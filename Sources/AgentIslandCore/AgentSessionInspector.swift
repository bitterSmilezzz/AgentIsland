import Foundation
import SQLite3

/// 从各家 Agent 的 JSONL/文本会话尾部提取「等待用户」与「本轮已完成」强语义。
///
/// 只检查结构字段（type/name/status/role/id），不拿普通消息正文做问句猜测：模型在解释
/// 代码时经常写出“是否……”，把任意问号当确认请求会制造大量误报。不同产品只要使用
/// request_user_input / AskUserQuestion / approval_request / task_complete 等常见协议名即可
/// 自动兼容；未知格式仍保留 CPU + 文件写入的原有降级路径。
public enum AgentSessionInspector {
    private static let requestNames: Set<String> = [
        "requestuserinput", "askuserquestion", "askfollowupquestion", "askuser", "askquestion",
        "requestpermission", "requestapproval", "requestconfirmation", "confirmwithuser",
        "approvalasked"
    ]
    private static let requestStates: Set<String> = [
        "approvalrequest", "approvalrequested", "permissionrequest", "permissionrequested",
        "confirmationrequest", "requiresconfirmation", "needsconfirmation", "pendingapproval",
        "awaitingapproval", "awaitinguserinput", "waitingforuser", "waitingforuserinput",
        "userinputrequest", "elicitation", "approvalasked"
    ]
    private static let completionTypes: Set<String> = [
        "taskcomplete", "taskcompleted", "turncomplete", "turncompleted",
        "sessioncomplete", "sessioncompleted", "runcomplete", "runcompleted",
        "turnend", "sessionend", "sessionendseed", "runend"
    ]
    private static let resolutionTypes: Set<String> = [
        "customtoolcalloutput", "toolresult", "userinputresponse", "approvalresponse",
        "permissionresponse", "confirmationresponse", "elicitationresponse", "approvaldecided"
    ]
    private static let resolutionStates: Set<String> = [
        "approved", "denied", "rejected", "cancelled", "canceled", "answered", "resolved"
    ]
    private static let activeTypes: Set<String> = [
        "reasoning", "thinking", "functioncall", "customtoolcall", "tooluse",
        "taskstarted", "turnstarted", "runstarted", "user_message", "usermessage",
        "stepstart", "turnstart", "toolcall", "assistantmessage"
    ]

    // MARK: - 活跃会话附加上下文（后台任务、子智能体、Token细分）
    private static let contextLock = NSLock()
    private static var activeContexts: [String: SessionActiveContext] = [:]

    public static func activeContext(for agentId: String) -> SessionActiveContext {
        contextLock.lock()
        defer { contextLock.unlock() }
        return activeContexts[agentId] ?? SessionActiveContext()
    }

    public static func setActiveContext(_ context: SessionActiveContext, for agentId: String) {
        contextLock.lock()
        defer { contextLock.unlock() }
        activeContexts[agentId] = context
    }

    public static func clearActiveContext(for agentId: String) {
        contextLock.lock()
        defer { contextLock.unlock() }
        activeContexts.removeValue(forKey: agentId)
    }

    /// 对 FileMonitor 已在后台定位出的每目录最新文件做有界尾读；不递归枚举目录。
    public static func inspect(profile: AgentProfile, activityFiles: [URL], now: Date = Date()) -> AgentSessionSignal? {
        // 专有 Agent 协议优先：拥有高保真结构化日志/专有解析器的 Agent（如 Antigravity、DSH、Cline、Roo）
        // 必须使用其专有解析器，避免被通用检测器的关键字深搜造成 attention/active 误判。
        switch profile.id {
        case "antigravity":
            return inspectAntigravitySession(now: now)
        case "dsh":
            return inspectDSHSession(now: now)
        case "cline", "roo-code":
            if let signal = inspectClineOrRooTasks(dirs: profile.sessionDirs, now: now) {
                return signal
            }
        default:
            break
        }

        let candidates = activityFiles.compactMap { url -> (URL, Date)? in
            guard let mtime = (try? url.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate else {
                return nil
            }
            return (url, mtime)
        }.sorted { $0.1 > $1.1 }

        for (file, mtime) in candidates {
            let age = max(0, now.timeIntervalSince(mtime))
            // 等待确认可以持续较久；完成态只保留一小段时间，之后自然显示“待机”。
            guard age <= 24 * 3600 else { continue }
            if file.lastPathComponent == "ui_messages.json" {
                if let data = try? Data(contentsOf: file),
                   let json = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]],
                   let signal = detectClineOrRoo(messages: json, fileAge: age) {
                    return signal
                }
            }
            let lines = LogTailReader.read(from: file, maxLines: 96, maxBytes: 262_144)
            guard let signal = detect(lines: lines) else { continue }
            if case .completed = signal, age > 15 * 60 { continue }
            return signal
        }
        // 部分桌面 Agent 把会话只写进 SQLite，FileMonitor 的 latest file 只能定位到
        // 二进制库本身。对已知 schema 做只读、索引命中的末条查询；失败即无信号。
        return inspectKnownDatabase(profile: profile, now: now)
    }

    /// 纯函数解析入口，供 fixture 测试与未来新增 Agent 格式时复用。
    public static func detect(lines: [String]) -> AgentSessionSignal? {
        var signal: AgentSessionSignal?
        var pendingCalls: [String: ActiveToolCall] = [:]
        var completedCallIds: Set<String> = []

        for (idx, line) in lines.enumerated() {
            guard let data = line.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) else {
                signal = applyPlainText(line, to: signal)
                continue
            }

            extractToolCalls(from: object, lineIndex: idx, into: &pendingCalls, completedIds: &completedCallIds)
            extractToolResolutions(from: object, into: &completedCallIds)

            if let request = signal?.attentionRequest,
               resolvesAttention(object, fingerprint: request.fingerprint) {
                signal = nil
                continue
            }

            // 尾窗可能只包含 AskUserQuestion 的 tool_result、而请求本身已落在窗口外；
            // 结果记录里的 toolName 不能被反向当成一条新请求。
            if isResolutionRecord(object) { continue }

            if let request = findRequest(in: object, sourceLine: line) {
                signal = .attention(request)
                continue
            }

            if let completion = findCompletion(in: object, sourceLine: line, depth: 0) {
                signal = .completed(fingerprint: completion)
                continue
            }

            // 新一轮用户消息、推理或工具调用使旧 task_complete 或旧 attention 失效。token_count / usage /
            // item_completed 等收尾记账事件是中性的，不会把状态立即冲掉。
            if startsOrContinuesWork(object) {
                if case .completed = signal { signal = nil }
                if case .attention = signal { signal = nil }
            }
        }

        // 关键防御：若模型产出了 completed 或处于普通待定，但仍有在途未决的执行类命令（如 Claude Code 的 Bash、Codex 的 exec_command 等）
        if signal?.attentionRequest == nil {
            let unresolved = pendingCalls.filter { !completedCallIds.contains($0.key) }
            // 只拦截真正的终端执行类命令调用，普通提问或读取类工具不破坏通用状态
            let unresolvedCommands = unresolved.values.filter { call in
                let n = normalized(call.name)
                return !requestNames.contains(n) && (n.contains("bash") || n.contains("command") || n.contains("terminal") || n.contains("exec") || call.command != nil)
            }
            if let lastCmd = unresolvedCommands.max(by: { $0.lineIndex < $1.lineIndex }) {
                let action: String
                if let cmd = lastCmd.command, !cmd.isEmpty {
                    action = "执行: \(cleanActionCommand(cmd))"
                } else if !lastCmd.name.isEmpty {
                    action = "执行: \(lastCmd.name)"
                } else {
                    action = "执行命令中"
                }
                return .active(fingerprint: "tool-\(lastCmd.id)", action: action)
            }
        }

        return signal
    }

    // MARK: - 工具调用与命令提取

    private struct ActiveToolCall {
        let id: String
        let name: String
        let command: String?
        let lineIndex: Int
    }

    private static func extractToolCalls(from value: Any, lineIndex: Int, into calls: inout [String: ActiveToolCall], completedIds: inout Set<String>) {
        if let dict = value as? [String: Any] {
            let type = normalized(dict["type"] as? String)
            let status = normalized(dict["status"] as? String)
            if type == "tooluse" || type == "functioncall" || type == "customtoolcall" || type == "toolcall" {
                if let id = identifier(in: dict) {
                    if status == "completed" || status == "done" || status == "success" || status == "failed" || status == "error" {
                        completedIds.insert(id)
                    } else {
                        let name = dict["name"] as? String ?? dict["function"] as? String ?? ""
                        let cmd = extractCommand(from: dict)
                        calls[id] = ActiveToolCall(id: id, name: name, command: cmd, lineIndex: lineIndex)
                    }
                }
            }
            for child in dict.values {
                extractToolCalls(from: child, lineIndex: lineIndex, into: &calls, completedIds: &completedIds)
            }
        } else if let array = value as? [Any] {
            for child in array {
                extractToolCalls(from: child, lineIndex: lineIndex, into: &calls, completedIds: &completedIds)
            }
        }
    }

    private static func extractToolResolutions(from value: Any, into completedIds: inout Set<String>) {
        if let dict = value as? [String: Any] {
            let type = normalized(dict["type"] as? String)
            let status = normalized(dict["status"] as? String)
            if type == "toolresult" || type == "functioncalloutput" || type == "customtoolcalloutput" || status == "completed" || status == "success" || status == "done" {
                for key in ["tool_use_id", "toolUseId", "call_id", "callId", "id"] {
                    if let id = dict[key] as? String, !id.isEmpty {
                        completedIds.insert(id)
                    }
                }
            }
            for child in dict.values {
                extractToolResolutions(from: child, into: &completedIds)
            }
        } else if let array = value as? [Any] {
            for child in array {
                extractToolResolutions(from: child, into: &completedIds)
            }
        }
    }

    private static func extractCommand(from dict: [String: Any]) -> String? {
        if let input = dict["input"] as? [String: Any] {
            if let cmd = input["command"] as? String ?? input["cmd"] as? String { return cmd }
        }
        if let args = dict["args"] as? [String: Any] {
            if let cmd = args["command"] as? String ?? args["cmd"] as? String ?? args["CommandLine"] as? String { return cmd }
        }
        if let arguments = dict["arguments"] as? String {
            if let data = arguments.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                if let cmd = obj["command"] as? String ?? obj["cmd"] as? String ?? obj["CommandLine"] as? String { return cmd }
            }
        }
        return nil
    }

    private static func cleanActionCommand(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("export ") || text.hasPrefix("SDKROOT=") {
            if let firstCmd = text.components(separatedBy: "&&").last {
                text = firstCmd.trimmingCharacters(in: .whitespacesAndNewlines)
            }
        }
        if text.hasPrefix("arch -arm64 ") {
            text = String(text.dropFirst("arch -arm64 ".count))
        }
        let maxLen = 36
        if text.count > maxLen {
            return String(text.prefix(maxLen)) + "…"
        }
        return text
    }

    // MARK: - Cline / Roo Code 专有解析器

    public static func detectClineOrRoo(messages: [[String: Any]], fileAge: TimeInterval) -> AgentSessionSignal? {
        guard !messages.isEmpty else { return nil }

        for msg in messages.reversed() {
            let type = msg["type"] as? String ?? ""
            let ts = msg["ts"] as? Double ?? Double(Date().timeIntervalSince1970 * 1000)
            let text = msg["text"] as? String ?? ""
            let fp = "cline-\(Int64(ts))"

            if type == "ask" {
                let ask = msg["ask"] as? String ?? ""
                if ask == "command" || ask == "command_output" {
                    let cmd = text.isEmpty ? "终端命令" : cleanActionCommand(text)
                    return .attention(AgentAttentionRequest(fingerprint: fp, message: "等待你批准命令: \(cmd)"))
                } else if ask == "followup" {
                    let question = text.isEmpty ? "等待你输入或回答" : clipDSHString(text, limit: 30)
                    return .attention(AgentAttentionRequest(fingerprint: fp, message: question))
                } else if ask == "tool" {
                    return .attention(AgentAttentionRequest(fingerprint: fp, message: "等待你批准工具操作"))
                } else {
                    return .attention(AgentAttentionRequest(fingerprint: fp, message: "等待你选择或确认"))
                }
            } else if type == "say" {
                let say = msg["say"] as? String ?? ""
                if say == "command" {
                    let cmd = text.isEmpty ? "执行命令中" : "执行命令: \(cleanActionCommand(text))"
                    return .active(fingerprint: fp, action: cmd)
                } else if say == "tool" {
                    let toolName = text.isEmpty ? "执行工具" : text
                    return .active(fingerprint: fp, action: "调用工具: \(toolName)")
                } else if say == "browser_action" {
                    return .active(fingerprint: fp, action: "执行浏览器操作")
                } else if say == "completion_result" || say == "task_completed" {
                    if fileAge <= 15 * 60 {
                        return .completed(fingerprint: fp)
                    } else {
                        return nil
                    }
                } else if say == "text" {
                    if fileAge <= 15 * 60 {
                        return .completed(fingerprint: fp)
                    } else {
                        return nil
                    }
                }
            }
        }
        return nil
    }

    public static func inspectClineOrRooTasks(dirs: [String], now: Date) -> AgentSessionSignal? {
        let fm = FileManager.default
        var newestURL: URL?
        var newestMtime: Date = .distantPast

        for dir in dirs {
            guard let taskDirs = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            for sub in taskDirs {
                let uiPath = "\(dir)/\(sub)/ui_messages.json"
                if fm.fileExists(atPath: uiPath),
                   let attrs = try? fm.attributesOfItem(atPath: uiPath),
                   let mtime = attrs[.modificationDate] as? Date {
                    if mtime > newestMtime {
                        newestMtime = mtime
                        newestURL = URL(fileURLWithPath: uiPath)
                    }
                }
            }
        }
        guard let url = newestURL else { return nil }
        let age = max(0, now.timeIntervalSince(newestMtime))
        guard age <= 24 * 3600 else { return nil }
        guard let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else {
            return nil
        }
        return detectClineOrRoo(messages: json, fileAge: age)
    }

    private static func findRequest(in value: Any, sourceLine: String) -> AgentAttentionRequest? {
        if let dict = value as? [String: Any] {
            let name = normalized(dict["name"] as? String)
            let type = normalized(dict["type"] as? String)
            let status = normalized(dict["status"] as? String)
            let state = normalized(dict["state"] as? String)
            let kind = normalized(dict["kind"] as? String)
            let marker = [name, type, status, state, kind].first {
                requestNames.contains($0) || requestStates.contains($0)
            }
            if let marker {
                let fingerprint = identifier(in: dict) ?? stableFingerprint(sourceLine)
                let approval = marker.contains("approval") || marker.contains("permission") || marker.contains("confirm")
                return AgentAttentionRequest(
                    fingerprint: fingerprint,
                    message: approval ? "等待你批准操作" : "等待你选择或确认"
                )
            }
            for child in dict.values {
                if let request = findRequest(in: child, sourceLine: sourceLine) { return request }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let request = findRequest(in: child, sourceLine: sourceLine) { return request }
            }
        }
        return nil
    }

    private static func resolvesAttention(_ value: Any, fingerprint: String) -> Bool {
        let roles = structuralValues(for: "role", in: value)
        if roles.contains("user") { return true }

        let sources = structuralValues(for: "source", in: value)
        if sources.contains("user") || sources.contains("userexplicit") { return true }

        let types = structuralValues(for: "type", in: value)
        let states = structuralValues(for: "status", in: value)
            .union(structuralValues(for: "state", in: value))
        let hasResolution = !types.isDisjoint(with: resolutionTypes)
            || !states.isDisjoint(with: resolutionStates)
        guard hasResolution else { return false }

        let ids = identifiers(in: value)
        // 有关联 ID 时必须对应当前请求；无 ID 的显式 approval_response 也可解除。
        return ids.isEmpty || ids.contains(fingerprint)
    }

    private static func isResolutionRecord(_ value: Any) -> Bool {
        let roles = structuralValues(for: "role", in: value)
        if roles.contains("toolresult") { return true }
        let types = structuralValues(for: "type", in: value)
        return !types.isDisjoint(with: resolutionTypes)
    }

    private static func findCompletion(in value: Any, sourceLine: String, depth: Int) -> String? {
        if let dict = value as? [String: Any] {
            let type = normalized(dict["type"] as? String)
            let subtype = normalized(dict["subtype"] as? String)
            if completionTypes.contains(type) || completionTypes.contains(subtype)
                || (depth == 0 && type == "result") {
                return identifier(in: dict) ?? stableFingerprint(sourceLine)
            }
            // WorkBuddy 等格式以 message.status=completed 表示本轮消息终态；普通工具调用
            // 的 status=completed 不会命中，因为其 type 是 tool_use/custom_tool_call。
            if depth <= 1, type == "message", normalized(dict["status"] as? String) == "completed" {
                return identifier(in: dict) ?? stableFingerprint(sourceLine)
            }
            for child in dict.values {
                if let completion = findCompletion(in: child, sourceLine: sourceLine, depth: depth + 1) { return completion }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let completion = findCompletion(in: child, sourceLine: sourceLine, depth: depth + 1) { return completion }
            }
        }
        return nil
    }

    private static func startsOrContinuesWork(_ value: Any) -> Bool {
        if structuralValues(for: "role", in: value).contains("user") { return true }
        let types = structuralValues(for: "type", in: value)
        return !types.isDisjoint(with: activeTypes)
    }

    private static func applyPlainText(_ line: String, to current: AgentSessionSignal?) -> AgentSessionSignal? {
        let lower = line.lowercased()
        // 纯文本降级只接受短状态行；大段终端输出/源码里偶然出现关键字不能改变状态。
        guard lower.count <= 512 else { return current }
        let requestPhrases = [
            "approval required", "approval requested", "permission required",
            "permission requested", "waiting for user input", "awaiting user input",
            "requires confirmation", "等待用户确认", "需要用户确认"
        ]
        if requestPhrases.contains(where: { lower.contains($0) }) {
            let approval = lower.contains("approval") || lower.contains("permission") || lower.contains("确认")
            return .attention(AgentAttentionRequest(
                fingerprint: stableFingerprint(line),
                message: approval ? "等待你批准操作" : "等待你选择或确认"
            ))
        }
        if current?.attentionRequest != nil {
            let resolved = ["approval granted", "permission granted", "user input received", "已确认", "已批准"]
            if resolved.contains(where: { lower.contains($0) }) { return nil }
        }
        let completionPhrases = ["task complete", "task completed", "turn complete", "turn completed", "任务已完成"]
        if completionPhrases.contains(where: { lower.contains($0) }) {
            return .completed(fingerprint: stableFingerprint(line))
        }
        return current
    }

    private static func structuralValues(for key: String, in value: Any) -> Set<String> {
        var result = Set<String>()
        if let dict = value as? [String: Any] {
            if let string = dict[key] as? String { result.insert(normalized(string)) }
            for child in dict.values { result.formUnion(structuralValues(for: key, in: child)) }
        } else if let array = value as? [Any] {
            for child in array { result.formUnion(structuralValues(for: key, in: child)) }
        }
        return result
    }

    private static func identifier(in dict: [String: Any]) -> String? {
        for key in ["call_id", "tool_use_id", "request_id", "toolCallId", "toolUseId",
                    "requestId", "id", "turn_id", "session_id", "turnId", "sessionId"] {
            if let id = dict[key] as? String, !id.isEmpty { return id }
        }
        if let data = dict["data"] as? [String: Any], let id = identifier(in: data) {
            return id
        }
        if let payload = dict["payload"] as? [String: Any], let id = identifier(in: payload) {
            return id
        }
        return nil
    }

    private static func identifiers(in value: Any) -> Set<String> {
        var result = Set<String>()
        if let dict = value as? [String: Any] {
            if let id = identifier(in: dict) { result.insert(id) }
            for child in dict.values { result.formUnion(identifiers(in: child)) }
        } else if let array = value as? [Any] {
            for child in array { result.formUnion(identifiers(in: child)) }
        }
        return result
    }

    private static func normalized(_ value: String?) -> String {
        guard let value else { return "" }
        return String(value.lowercased().unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) })
    }

    /// 64-bit FNV-1a：跨进程稳定，且只用于区分通知去重，不承担安全哈希职责。
    private static func stableFingerprint(_ value: String) -> String {
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        return String(hash, radix: 16)
    }

    // MARK: - 已知 SQLite 会话源

    private static func inspectKnownDatabase(profile: AgentProfile, now: Date) -> AgentSessionSignal? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        switch profile.id {
        case "dim":
            return inspectDimDatabase(path: "\(home)/.dimcode/v2/dimcode.sqlite", now: now)
        case "zcode":
            return inspectStatusDatabase(
                path: "\(home)/.zcode/v2/tasks-index.sqlite",
                sql: "SELECT id, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updated_at DESC LIMIT 1;",
                now: now
            )
        case "workbuddy":
            return inspectStatusDatabase(
                path: "\(home)/.workbuddy/workbuddy.db",
                sql: "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;",
                now: now
            )
        case "workbuddy-ai":
            return inspectStatusDatabase(
                path: "\(home)/.workbuddy-ai/workbuddy.db",
                sql: "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;",
                now: now
            )
        case "opencode":
            return inspectOpenCodeDatabase(path: "\(home)/.local/share/opencode/opencode.db", now: now)
        case "dsh":
            return inspectDSHSession(now: now)
        case "antigravity":
            return inspectAntigravitySession(now: now)
        default:
            return nil
        }
    }

    /// Dim 的确认请求是一条 assistant AskUserQuestion tool call，用户处理后追加关联
    /// tool_result；读取最新会话的末 32 条即可按 call id 配对，不碰问题/答案正文。
    private static func inspectDimDatabase(path: String, now: Date) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path) { db in
            let sql = """
            SELECT rowid, role, toolMetadata, parts
            FROM messages
            WHERE sessionId = (SELECT sessionId FROM messages ORDER BY rowid DESC LIMIT 1)
            ORDER BY rowid DESC LIMIT 32;
            """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }

            var rows: [(id: Int64, role: String, tool: Any?, parts: Any?)] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                let id = sqlite3_column_int64(stmt, 0)
                let role = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
                let toolText = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""
                let partsText = sqlite3_column_text(stmt, 3).map { String(cString: $0) } ?? ""
                let tool = toolText.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) }
                let parts = partsText.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) }
                rows.append((id, role, tool, parts))
            }
            guard let newest = rows.first else { return nil }

            let lines = rows.reversed().compactMap { row -> String? in
                var object: [String: Any] = ["role": row.role, "row_id": String(row.id)]
                if let tool = row.tool { object["toolMetadata"] = tool }
                if let parts = row.parts { object["parts"] = parts }
                guard JSONSerialization.isValidJSONObject(object),
                      let data = try? JSONSerialization.data(withJSONObject: object) else { return nil }
                return String(data: data, encoding: .utf8)
            }
            if case let .attention(request)? = detect(lines: lines) {
                return .attention(request)
            }

            // assistant 最新 part 有 endTime = 当前回复已封口。只看最后一个 part，不能因
            // 前面某段 thinking 已结束就把后续仍在跑的 tool_use 错判为任务完成。
            if newest.role == "assistant",
               let parts = newest.parts as? [[String: Any]],
               let last = parts.last,
               last["endTime"] != nil {
                guard fileAge(path, now: now) <= 15 * 60 else { return nil }
                return .completed(fingerprint: "dim-\(newest.id)")
            }
            return nil
        }
    }

    /// ZCode / WorkBuddy 的会话表直接提供终态 status。时间字段兼容毫秒与秒 epoch。
    private static func inspectStatusDatabase(path: String, sql: String, now: Date) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path) { db in
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }
            guard sqlite3_step(stmt) == SQLITE_ROW else { return nil }
            let id = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? stableFingerprint(path)
            let status = sqlite3_column_text(stmt, 1).map { normalized(String(cString: $0)) } ?? ""
            let rawTime = sqlite3_column_double(stmt, 2)
            let epoch = rawTime > 10_000_000_000 ? rawTime / 1000 : rawTime
            let age = epoch > 0 ? max(0, now.timeIntervalSince1970 - epoch) : fileAge(path, now: now)
            if requestStates.contains(status) {
                return .attention(AgentAttentionRequest(
                    fingerprint: id,
                    message: status.contains("approval") || status.contains("permission")
                        ? "等待你批准操作" : "等待你选择或确认"
                ))
            }
            if ["completed", "complete", "done", "succeeded", "success"].contains(status), age <= 15 * 60 {
                return .completed(fingerprint: id)
            }
            return nil
        }
    }

    private static func inspectOpenCodeDatabase(path: String, now: Date) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path) { db in
            let sql = """
            SELECT data FROM part
            WHERE session_id = (SELECT id FROM session ORDER BY time_updated DESC LIMIT 1)
            ORDER BY rowid DESC LIMIT 32;
            """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }
            var lines: [String] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                if let text = sqlite3_column_text(stmt, 0).map({ String(cString: $0) }), !text.isEmpty {
                    lines.append(text)
                }
            }
            guard let signal = detect(lines: Array(lines.reversed())) else { return nil }
            if case .completed = signal, fileAge(path, now: now) > 15 * 60 { return nil }
            return signal
        }
    }

    /// 解析 DeepSeek Harness (DSH) 官方会话投影缓存 (sessionProjectionCache)，提取活跃执行态与终态。
    public static func inspectDSHSession(baseDir: String? = nil, now: Date = Date()) -> AgentSessionSignal? {
        let projcacheDir: String
        if let baseDir {
            projcacheDir = baseDir
        } else {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            projcacheDir = "\(home)/.dsh/storages/session_projcache/sessions"
        }
        let fm = FileManager.default
        guard let files = try? fm.contentsOfDirectory(atPath: projcacheDir) else { return nil }

        var newestPath: String?
        var newestDate: Date = .distantPast
        for file in files where file.hasSuffix(".json") {
            let fullPath = "\(projcacheDir)/\(file)"
            if let attrs = try? fm.attributesOfItem(atPath: fullPath),
               let mdate = attrs[.modificationDate] as? Date {
                if mdate > newestDate {
                    newestDate = mdate
                    newestPath = fullPath
                }
            }
        }
        guard let path = newestPath else { return nil }
        let age = max(0, now.timeIntervalSince(newestDate))
        guard age <= 24 * 3600 else { return nil }

        guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let record = root["record"] as? [String: Any],
              let rows = record["rows"] as? [String: Any] else {
            return nil
        }

        let sessionId = URL(fileURLWithPath: path).deletingPathExtension().lastPathComponent

        // 提取任务标题
        let titleDict = rows["title"] as? [String: Any]
        let rawTitle = (titleDict?["val"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let cleanTitle = clipDSHString(rawTitle.replacingOccurrences(of: "\n", with: " "), limit: 26)

        // 检查审批/确认请求
        if let approvalRow = rows["approval"] as? [String: Any],
           let approvalVal = approvalRow["val"] as? [String: Any],
           let pendingId = approvalVal["id"] as? String, !pendingId.isEmpty {
            let toolName = approvalVal["toolName"] as? String ?? "操作"
            return .attention(AgentAttentionRequest(
                fingerprint: pendingId,
                message: "等待你批准执行: \(toolName)"
            ))
        }

        // 检查轮次与步骤
        let turnBoundary = rows["turnBoundary"] as? [String: Any]
        let tbVal = turnBoundary?["val"] as? [String: Any]
        let openTurnStartSeq = tbVal?["openTurnStartSeq"] as? Int

        let sessionStats = rows["sessionStats"] as? [String: Any]
        let statsVal = sessionStats?["val"] as? [String: Any]
        let openStep = statsVal?["openStep"] as? [String: Any]
        let currentStep = openStep?["step"] as? Int ?? statsVal?["steps"] as? Int
        let currentTurn = openStep?["turn"] as? Int ?? statsVal?["lastTurn"] as? Int ?? 1

        let isOpen = (openTurnStartSeq != nil) || (openStep != nil)

        if isOpen {
            // 处于活跃保护期内（30分钟内有更新），返回 active
            guard age <= 30 * 60 else { return nil }
            let actionDesc: String
            if !cleanTitle.isEmpty {
                if let step = currentStep, step > 0 {
                    actionDesc = "执行中: \(cleanTitle) (第 \(step) 步)"
                } else {
                    actionDesc = "执行中: \(cleanTitle)"
                }
            } else if let step = currentStep, step > 0 {
                actionDesc = "执行任务中 (第 \(step) 步)"
            } else {
                actionDesc = "执行任务中"
            }
            let fingerprint = "dsh-\(sessionId)-turn\(currentTurn)"
            return .active(fingerprint: fingerprint, action: actionDesc)
        } else {
            // 轮次已结束：若在完成后的 15 分钟内，返回 completed
            guard age <= 15 * 60 else { return nil }
            let totalSteps = statsVal?["steps"] as? Int ?? 0
            let fingerprint = "dsh-\(sessionId)-t\(currentTurn)-s\(totalSteps)"
            return .completed(fingerprint: fingerprint)
        }
    }

    /// 解析 Google Antigravity 轨迹日志 (.system_generated/logs/transcript.jsonl)，提取活跃执行态、提问确认与终态。
    public static func inspectAntigravitySession(baseDir: String? = nil, now: Date = Date()) -> AgentSessionSignal? {
        let brainDir: URL
        if let baseDir {
            brainDir = URL(fileURLWithPath: baseDir)
        } else {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            brainDir = URL(fileURLWithPath: "\(home)/.gemini/antigravity/brain")
        }
        guard let subdirs = try? FileManager.default.contentsOfDirectory(at: brainDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) else {
            return nil
        }
        var newestFile: URL?
        var newestTasksDir: URL?
        var newestTime: Date = .distantPast
        for sub in subdirs {
            let logFile = sub.appendingPathComponent(".system_generated/logs/transcript.jsonl")
            if let attrs = try? FileManager.default.attributesOfItem(atPath: logFile.path),
               let mtime = attrs[.modificationDate] as? Date {
                var effectiveTime = mtime
                let tasksDir = sub.appendingPathComponent(".system_generated/tasks")
                if let taskFiles = try? FileManager.default.contentsOfDirectory(at: tasksDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) {
                    for tf in taskFiles where tf.pathExtension == "log" {
                        if let tAttrs = try? FileManager.default.attributesOfItem(atPath: tf.path),
                           let tMTime = tAttrs[.modificationDate] as? Date,
                           tMTime > effectiveTime {
                            effectiveTime = tMTime
                        }
                    }
                }
                if effectiveTime > newestTime {
                    newestTime = effectiveTime
                    newestFile = logFile
                    newestTasksDir = tasksDir
                }
            }
        }
        guard let target = newestFile else { return nil }
        let age = max(0, now.timeIntervalSince(newestTime))
        guard age <= 24 * 3600 else { return nil }

        let lines = LogTailReader.read(from: target, maxLines: 120, maxBytes: 262_144)
        guard !lines.isEmpty else { return nil }

        return detectAntigravitySession(lines: lines, fileAge: age, now: now, tasksDir: newestTasksDir)
    }

    /// 解析 Antigravity transcript.jsonl 末尾若干行，推导当前状态信号
    public static func detectAntigravitySession(lines: [String], fileAge: TimeInterval, now: Date = Date(), tasksDir: URL? = nil) -> AgentSessionSignal? {
        // 预解析与状态扫描：
        // 1. 扫描所有行，收集 step 元数据与后台任务/子智能体生命周期
        var parsedMeta: [(stepIndex: Int, stepType: String, hasToolCalls: Bool)] = []
        var launchedTasks: [String: (desc: String, stepIndex: Int)] = [:]
        var finishedTaskIds: Set<String> = []
        var activeSubagents: Set<String> = []
        var finishedSubagents: Set<String> = []
        var subagentInfoMap: [String: AgentSubagentInfo] = [:]
        var subagentCandidateRoles: [(role: String, model: String)] = []

        var totalPromptTokens = 0
        var totalCompletionTokens = 0
        var totalCacheReadTokens = 0
        var totalCacheWriteTokens = 0
        var totalReasoningTokens = 0
        var totalTokens = 0

        for rawLine in lines {
            guard let data = rawLine.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }
            let si = obj["step_index"] as? Int ?? 0
            let st = obj["type"] as? String ?? ""
            let hasTC = (obj["tool_calls"] as? [[String: Any]])?.isEmpty == false
            parsedMeta.append((si, st, hasTC))

            // 提取 Token 统计细分
            if let usage = obj["usageMetadata"] as? [String: Any] ?? obj["usage"] as? [String: Any] ?? obj["token_count"] as? [String: Any] {
                let p = usage["promptTokenCount"] as? Int ?? usage["prompt_tokens"] as? Int ?? usage["input_tokens"] as? Int ?? 0
                let c = usage["candidatesTokenCount"] as? Int ?? usage["candidates_tokens"] as? Int ?? usage["output_tokens"] as? Int ?? 0
                let cr = usage["cachedContentTokenCount"] as? Int ?? usage["cache_read_tokens"] as? Int ?? 0
                let cw = usage["cache_write_tokens"] as? Int ?? 0
                let th = usage["thoughtsTokenCount"] as? Int ?? usage["reasoning_tokens"] as? Int ?? 0
                let tot = usage["totalTokenCount"] as? Int ?? usage["total_tokens"] as? Int ?? (p + c + cr + cw + th)
                totalPromptTokens += p
                totalCompletionTokens += c
                totalCacheReadTokens += cr
                totalCacheWriteTokens += cw
                totalReasoningTokens += th
                totalTokens += tot
            }

            let content = obj["content"] as? String ?? ""

            // (A) 后台任务启动 (Tool is running as a background task with task id: ...)
            if content.contains("Tool is running as a background task with task id:") {
                if let taskId = extractAntigravityTaskId(from: content) {
                    let desc = extractAntigravityTaskDescription(from: content)
                    launchedTasks[taskId] = (desc: desc, stepIndex: si)
                }
            }

            // (B) 后台任务完成 / 取消 / 中断检测
            if content.contains("finished with result:") ||
               content.contains("cancelled") ||
               content.contains("was killed") ||
               content.contains("Wait cancelled") {
                for taskId in launchedTasks.keys {
                    if content.contains(taskId) {
                        finishedTaskIds.insert(taskId)
                    }
                }
                if let finishedId = extractFinishedAntigravityTaskId(from: content) {
                    finishedTaskIds.insert(finishedId)
                }
            }

            // (C) manage_task 工具调用取消/杀掉任务
            if let toolCalls = obj["tool_calls"] as? [[String: Any]] {
                for tc in toolCalls {
                    let name = normalized(tc["name"] as? String)
                    if name.contains("managetask") || name.contains("manage_task") {
                        if let args = tc["args"] as? [String: Any],
                           let action = args["Action"] as? String, action == "kill",
                           let rawTid = args["TaskId"] as? String {
                            let tid = normalizeTaskId(rawTid)
                            finishedTaskIds.insert(tid)
                        }
                    } else if name.contains("invokesubagent") || name.contains("invoke_subagent") {
                        if let args = tc["args"] as? [String: Any],
                           let subs = args["Subagents"] as? [[String: Any]] {
                            for s in subs {
                                let r = s["Role"] as? String ?? s["TypeName"] as? String ?? "子任务"
                                let m = s["Model"] as? String ?? "inherit"
                                subagentCandidateRoles.append((role: r, model: m))
                            }
                        }
                    }
                }
            }

            // (D) 子智能体生命周期 (Created the following subagents:)
            if content.contains("Created the following subagents:") {
                let subIds = extractSubagentIds(from: content)
                for sid in subIds {
                    activeSubagents.insert(sid)
                    let cand = !subagentCandidateRoles.isEmpty ? subagentCandidateRoles.removeFirst() : (role: "子智能体", model: "inherit")
                    subagentInfoMap[sid] = AgentSubagentInfo(conversationId: sid, role: cand.role, model: cand.model, state: "running")
                }
            }
            if !activeSubagents.isEmpty {
                for sid in activeSubagents {
                    if content.contains("sender=\(sid)") || (content.contains(sid) && content.contains("finished")) {
                        finishedSubagents.insert(sid)
                    }
                }
            }
        }

        // 计算当前仍未交付完成的后台任务
        var unresolvedTasks: [(id: String, desc: String, stepIndex: Int)] = []
        for (id, info) in launchedTasks {
            if !finishedTaskIds.contains(id) {
                unresolvedTasks.append((id: id, desc: info.desc, stepIndex: info.stepIndex))
            }
        }
        unresolvedTasks.sort { $0.stepIndex > $1.stepIndex }

        let bgTaskModels = unresolvedTasks.map { task in
            AgentBackgroundTask(id: task.id, action: formatAntigravityBackgroundTaskAction(task.desc))
        }

        let pendingSubs = activeSubagents.subtracting(finishedSubagents)
        let subagentModels = pendingSubs.compactMap { sid -> AgentSubagentInfo? in
            subagentInfoMap[sid] ?? AgentSubagentInfo(conversationId: sid, role: "子智能体", model: "inherit", state: "running")
        }

        let tokenBreakdown: AgentTokenBreakdown? = totalTokens > 0 ? AgentTokenBreakdown(
            promptTokens: totalPromptTokens,
            completionTokens: totalCompletionTokens,
            cacheReadTokens: totalCacheReadTokens,
            cacheWriteTokens: totalCacheWriteTokens,
            reasoningTokens: totalReasoningTokens,
            totalTokens: totalTokens
        ) : nil

        let ctx = SessionActiveContext(
            backgroundTasks: bgTaskModels,
            subagents: subagentModels,
            tokenBreakdown: tokenBreakdown
        )
        setActiveContext(ctx, for: "antigravity")

        func makeActiveSignal(fingerprint: String, action: String) -> AgentSessionSignal {
            return .active(fingerprint: fingerprint, action: action)
        }

        // 从尾部逆序扫描推导当前最新状态信号
        for rawLine in lines.reversed() {
            guard let data = rawLine.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }

            let stepIndex = obj["step_index"] as? Int ?? 0
            let stepType = obj["type"] as? String ?? ""
            let fingerprint = "antigravity-step-\(stepIndex)"

            // 1. 等待用户选择或确认（如 ask_question）
            if let toolCalls = obj["tool_calls"] as? [[String: Any]], !toolCalls.isEmpty {
                if let askTool = toolCalls.first(where: {
                    let name = normalized($0["name"] as? String)
                    return requestNames.contains(name) || name.contains("askquestion") || name.contains("askuser")
                }) {
                    let alreadyAnswered = parsedMeta.contains { meta in
                        meta.stepIndex > stepIndex &&
                        (meta.stepType == "GENERIC" || meta.stepType == "USER_INPUT")
                    }
                    if alreadyAnswered { continue }

                    var questionMsg = "等待你选择或确认"
                    if let args = askTool["args"] as? [String: Any] {
                        if let questionsStr = args["questions"] as? String,
                           let qData = questionsStr.data(using: .utf8),
                           let qArray = try? JSONSerialization.jsonObject(with: qData) as? [[String: Any]],
                           let firstQ = qArray.first,
                           let qText = firstQ["question"] as? String, !qText.isEmpty {
                            questionMsg = clipDSHString(qText.replacingOccurrences(of: "\n", with: " "), limit: 30)
                        }
                    }
                    return .attention(AgentAttentionRequest(
                        fingerprint: fingerprint,
                        message: questionMsg
                    ))
                }

                // 正在执行工具调用
                guard fileAge <= 300 else { return nil }

                var actionText: String? = nil
                if let firstTool = toolCalls.first {
                    if let args = firstTool["args"] as? [String: Any] {
                        if let rawAction = args["toolAction"] as? String {
                            actionText = rawAction
                        } else if let rawSummary = args["toolSummary"] as? String {
                            actionText = rawSummary
                        }
                    }
                    if actionText == nil, let name = firstTool["name"] as? String {
                        actionText = name
                    }
                }
                let display = actionText.map { AgentActionInspector.cleanAntigravityAction($0) } ?? "执行工具中"
                return makeActiveSignal(fingerprint: fingerprint, action: display)
            }

            // 2. 规划响应与最终回答
            if stepType == "PLANNER_RESPONSE" {
                // 关键防御：若仍有后台任务在飞（如终端编译、测试运行、定时等待），
                // 此时 PLANNER_RESPONSE 只是模型对用户的阶段告知，绝非全流程完成，必须保持 active！
                if let activeTask = unresolvedTasks.first {
                    guard fileAge <= 15 * 60 else { return nil }
                    let action = formatAntigravityBackgroundTaskAction(activeTask.desc)
                    let bgFingerprint = "antigravity-bg-\(activeTask.id)-\(stepIndex)"
                    return makeActiveSignal(fingerprint: bgFingerprint, action: action)
                }

                let pendingSubagents = activeSubagents.subtracting(finishedSubagents)
                if let subagentId = pendingSubagents.first {
                    guard fileAge <= 15 * 60 else { return nil }
                    let subRole = subagentInfoMap[subagentId]?.role ?? "子智能体"
                    return makeActiveSignal(fingerprint: "antigravity-subagent-\(subagentId)-\(stepIndex)", action: "\(subRole) 执行中")
                }

                let content = obj["content"] as? String
                let status = obj["status"] as? String
                let hasActiveThinking = (obj["thinking"] as? String)?.isEmpty == false

                // 只有在所有后台任务和子智能体完全交付后，有内容输出给用户或无思考的明确 DONE 状态：才判定为轮次结束
                if (content != nil && !content!.isEmpty) || (status == "DONE" && !hasActiveThinking) {
                    if fileAge <= 15 * 60 {
                        return .completed(fingerprint: fingerprint)
                    } else {
                        return nil // 15 分钟后自然转入待机
                    }
                }

                // 只有思考而无工具调用也无最终内容：正在思考规划中
                if hasActiveThinking {
                    guard fileAge <= 300 else { return nil }
                    return makeActiveSignal(fingerprint: fingerprint, action: "思考规划中")
                }
            }

            // 3. 用户刚发送输入，模型正在启动准备
            if stepType == "USER_INPUT" {
                guard fileAge <= 300 else { return nil }
                return makeActiveSignal(fingerprint: fingerprint, action: "思考规划中")
            }

            // 4. 工具输出返回 (GENERIC)，正在等待下一拍模型调度或后台任务执行中
            if stepType == "GENERIC" {
                if let activeTask = unresolvedTasks.first {
                    guard fileAge <= 15 * 60 else { return nil }
                    let action = formatAntigravityBackgroundTaskAction(activeTask.desc)
                    let bgFingerprint = "antigravity-bg-\(activeTask.id)-\(stepIndex)"
                    return makeActiveSignal(fingerprint: bgFingerprint, action: action)
                }

                guard fileAge <= 300 else { return nil }
                var toolActionDesc: String? = nil
                for prevRaw in lines.reversed() {
                    if let pData = prevRaw.data(using: .utf8),
                       let pObj = try? JSONSerialization.jsonObject(with: pData) as? [String: Any],
                       let tCalls = pObj["tool_calls"] as? [[String: Any]],
                       let firstT = tCalls.first {
                        if let args = firstT["args"] as? [String: Any] {
                            toolActionDesc = (args["toolAction"] as? String) ?? (args["toolSummary"] as? String)
                        }
                        if toolActionDesc == nil, let tName = firstT["name"] as? String {
                            toolActionDesc = tName
                        }
                        break
                    }
                }
                if let toolActionDesc {
                    let cleaned = AgentActionInspector.cleanAntigravityAction(toolActionDesc)
                    return makeActiveSignal(fingerprint: fingerprint, action: "\(cleaned) (处理中)")
                }
                return makeActiveSignal(fingerprint: fingerprint, action: "处理中")
            }

            // 5. 系统通知或任务完成结果返回 (SYSTEM_MESSAGE)
            if stepType == "SYSTEM_MESSAGE" {
                guard fileAge <= 300 else { return nil }
                return makeActiveSignal(fingerprint: fingerprint, action: "处理任务结果中")
            }
        }

        return nil
    }

    /// 规范化后台任务文案格式（如将 arch / SDKROOT / Timer 等提炼为精炼动作）
    public static func formatAntigravityBackgroundTaskAction(_ desc: String) -> String {
        let trimmed = desc.trimmingCharacters(in: CharacterSet(charactersIn: "\" '`\t\n\r"))
        if trimmed.isEmpty {
            return "执行后台任务中"
        }
        if trimmed.hasPrefix("Timer:") {
            let parts = trimmed.split(separator: ",")
            if let firstPart = parts.first {
                let duration = firstPart.replacingOccurrences(of: "Timer:", with: "").trimmingCharacters(in: .whitespaces)
                return "后台定时中 (\(duration))"
            }
            return "后台定时中"
        }
        // 清除环境变量前缀
        var cmd = trimmed
        while let spaceIdx = cmd.firstIndex(of: " ") {
            let prefix = String(cmd[..<spaceIdx])
            if prefix.contains("=") && !prefix.contains(" ") {
                cmd = String(cmd[cmd.index(after: spaceIdx)...]).trimmingCharacters(in: .whitespaces)
            } else {
                break
            }
        }
        if cmd.hasPrefix("arch -arm64 ") {
            cmd = String(cmd.dropFirst("arch -arm64 ".count))
        }
        let cleaned = AgentActionInspector.cleanAntigravityAction(cmd)
        if cleaned.hasPrefix("后台任务:") || cleaned.hasPrefix("执行: 后台任务") {
            return cleaned
        }
        if cleaned.hasPrefix("执行: ") {
            return "后台任务: " + cleaned.dropFirst("执行: ".count)
        }
        return "后台任务: \(cleaned)"
    }

    private static func normalizeTaskId(_ raw: String) -> String {
        let trimmed = raw.trimmingCharacters(in: CharacterSet(charactersIn: "\" '`\t\n\r"))
        if let last = trimmed.split(separator: "/").last {
            return String(last)
        }
        return trimmed
    }

    private static func extractAntigravityTaskId(from content: String) -> String? {
        let marker = "Tool is running as a background task with task id:"
        guard let markerRange = content.range(of: marker) else { return nil }
        let after = content[markerRange.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
        let line = after.components(separatedBy: .newlines).first ?? ""
        let rawId = line.trimmingCharacters(in: .whitespaces)
        guard !rawId.isEmpty else { return nil }
        return normalizeTaskId(rawId)
    }

    private static func extractAntigravityTaskDescription(from content: String) -> String {
        let marker = "Task Description:"
        guard let markerRange = content.range(of: marker) else { return "" }
        let after = content[markerRange.upperBound...].trimmingCharacters(in: .whitespaces)
        let lines = after.components(separatedBy: .newlines)
        var descLines: [String] = []
        for line in lines {
            if line.contains("Task logs are available at:") ||
               line.contains("YOU MUST TAKE ONE OF THE FOLLOWING") {
                break
            }
            descLines.append(line)
        }
        return descLines.joined(separator: " ").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func extractFinishedAntigravityTaskId(from content: String) -> String? {
        if let range = content.range(of: "Task id \"") {
            let after = content[range.upperBound...]
            if let endQuote = after.firstIndex(of: "\"") {
                return normalizeTaskId(String(after[..<endQuote]))
            }
        }
        if let range = content.range(of: "Task \"") {
            let after = content[range.upperBound...]
            if let endQuote = after.firstIndex(of: "\"") {
                return normalizeTaskId(String(after[..<endQuote]))
            }
        }
        if let range = content.range(of: "sender=") {
            let after = content[range.upperBound...]
            let token = after.split(separator: " ").first ?? ""
            if token.contains("task-") {
                return normalizeTaskId(String(token))
            }
        }
        return nil
    }

    private static func extractSubagentIds(from content: String) -> [String] {
        var ids: [String] = []
        let marker = "\"conversationId\":"
        var searchRange = content.startIndex..<content.endIndex
        while let range = content.range(of: marker, range: searchRange) {
            let after = content[range.upperBound...].trimmingCharacters(in: .whitespaces)
            if let firstQuote = after.firstIndex(of: "\"") {
                let rest = after[after.index(after: firstQuote)...]
                if let secondQuote = rest.firstIndex(of: "\"") {
                    let cid = String(rest[..<secondQuote])
                    if !cid.isEmpty { ids.append(cid) }
                }
            }
            searchRange = range.upperBound..<content.endIndex
        }
        if ids.isEmpty, let textMarker = content.range(of: "Created the following subagents:") {
            let after = content[textMarker.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
            let tokens = after.components(separatedBy: CharacterSet(charactersIn: " ,;\n\r\t[](){}\""))
            for t in tokens {
                let trimmed = t.trimmingCharacters(in: .whitespaces)
                if !trimmed.isEmpty && (trimmed.contains("conv-") || trimmed.contains("subagent-") || trimmed.count >= 8) {
                    ids.append(trimmed)
                }
            }
        }
        return ids
    }

    private static func clipDSHString(_ text: String, limit: Int = 26) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > limit else { return trimmed }
        let index = trimmed.index(trimmed.startIndex, offsetBy: max(0, limit - 1))
        return String(trimmed[..<index]) + "…"
    }

    private static func fileAge(_ path: String, now: Date) -> TimeInterval {
        let attrs = try? FileManager.default.attributesOfItem(atPath: path)
        guard let date = attrs?[.modificationDate] as? Date else { return .infinity }
        return max(0, now.timeIntervalSince(date))
    }

    private static func withDB<T>(_ path: String, _ body: (OpaquePointer) -> T?) -> T? {
        ReadonlyDB.withConnection(path, body) ?? nil
    }
}

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

    /// 对 FileMonitor 已在后台定位出的每目录最新文件做有界尾读；不递归枚举目录。
    public static func inspect(profile: AgentProfile, activityFiles: [URL], now: Date = Date()) -> AgentSessionSignal? {
        // 专有 Agent 协议优先：拥有高保真结构化日志/专有解析器的 Agent（如 Antigravity、DSH）
        // 必须使用其专有解析器，避免被通用检测器的关键字深搜造成 attention/active 误判。
        switch profile.id {
        case "antigravity":
            return inspectAntigravitySession(now: now)
        case "dsh":
            return inspectDSHSession(now: now)
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

        for line in lines {
            guard let data = line.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) else {
                signal = applyPlainText(line, to: signal)
                continue
            }

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
        return signal
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
        var newestTime: Date = .distantPast
        for sub in subdirs {
            let logFile = sub.appendingPathComponent(".system_generated/logs/transcript.jsonl")
            if let attrs = try? FileManager.default.attributesOfItem(atPath: logFile.path),
               let mtime = attrs[.modificationDate] as? Date {
                if mtime > newestTime {
                    newestTime = mtime
                    newestFile = logFile
                }
            }
        }
        guard let target = newestFile else { return nil }
        let age = max(0, now.timeIntervalSince(newestTime))
        guard age <= 24 * 3600 else { return nil }

        let lines = LogTailReader.read(from: target, maxLines: 48, maxBytes: 131_072)
        guard !lines.isEmpty else { return nil }

        return detectAntigravitySession(lines: lines, fileAge: age, now: now)
    }

    /// 解析 Antigravity transcript.jsonl 末尾若干行，推导当前状态信号
    public static func detectAntigravitySession(lines: [String], fileAge: TimeInterval, now: Date = Date()) -> AgentSessionSignal? {
        // 预解析：收集所有行的 step_index 与 type，用于判断 ask_question 是否已被答复
        // 采用轻量元数据扫描，避免重复全量 JSON 解析
        var parsedMeta: [(stepIndex: Int, stepType: String, hasToolCalls: Bool)] = []
        for rawLine in lines {
            if let data = rawLine.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                let si = obj["step_index"] as? Int ?? 0
                let st = obj["type"] as? String ?? ""
                let hasTC = (obj["tool_calls"] as? [[String: Any]])?.isEmpty == false
                parsedMeta.append((si, st, hasTC))
            }
        }

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
                    // 如果该 ask_question step 之后已有 GENERIC 或 USER_INPUT step，
                    // 说明用户已经回复，不应再触发 attention；继续向前扫描更早的步骤。
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
                // 只有在最近 300 秒内发生文件变动才视为实时活跃执行；超时则视为挂起或已中断
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
                return .active(fingerprint: fingerprint, action: display)
            }

            // 2. 规划响应与最终回答
            if stepType == "PLANNER_RESPONSE" {
                let content = obj["content"] as? String
                let status = obj["status"] as? String
                let hasActiveThinking = (obj["thinking"] as? String)?.isEmpty == false

                // 如果有内容输出给用户，或者无思考的明确 DONE 状态：判定为轮次结束
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
                    return .active(fingerprint: fingerprint, action: "思考规划中")
                }
            }

            // 3. 用户刚发送输入，模型正在启动准备
            if stepType == "USER_INPUT" {
                guard fileAge <= 300 else { return nil }
                return .active(fingerprint: fingerprint, action: "思考规划中")
            }

            // 4. 工具输出返回 (GENERIC)，正在等待下一拍模型调度
            if stepType == "GENERIC" {
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
                    return .active(fingerprint: fingerprint, action: "\(cleaned) (处理中)")
                }
                return .active(fingerprint: fingerprint, action: "处理中")
            }
        }

        return nil
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

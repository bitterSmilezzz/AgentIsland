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

    // MARK: - 会话定位缓存

    /// 专有会话解析器要先在会话树里挑出「当前最新那个会话」的文件，再尾读它。
    /// 实测一趟遍历在真实机器上的量级：Antigravity 797 次 stat / 24ms，DSH 501 次 stat / 15ms。
    /// 而引擎每 2s 一拍，且「会话强语义」与「当前动作文案」两条链路各调一次同一棵树——
    /// 采样运行在 @MainActor 上，这些遍历直接把悬浮岛的展开动画卡出可感知的丢帧。
    ///
    /// 缓存的只是「选哪个文件」，尾读与解析每拍照常进行，因此状态转移（新写入、完成、
    /// 等待确认）不会有任何延迟。失效条件取两者较小值：
    /// - TTL 到期（兜底，防止长期不重定位）
    /// - 根目录 mtime 变化（新建会话目录必然刷新父目录 mtime → 立即重定位，新会话零延迟）
    /// 深层文件写入不改变根目录 mtime，所以只靠 mtime 会长期钉死在旧会话上。
    private struct LocatedSession {
        let file: URL
        var mtime: Date
        let sidecar: URL?   // Antigravity: 同一会话的 tasks 目录
    }

    private static let locateLock = NSLock()
    private static var locateCache: [String: (expires: Date, rootDate: Date?, located: LocatedSession?)] = [:]
    // TTL 取 10s：与 2s 采样节律错开（3s 会与每 2 拍一次的节律打拍，等于一半的拍仍在付费），
    // 而「新会话出现」由下面的根目录 mtime 令牌即时捕获。
    private static let locateTTL: TimeInterval = 10

    /// 读取（必要时重算）指定会话树的定位结果。
    /// - Parameters:
    ///   - key: 缓存键，须同时区分 Agent 与根目录（测试会注入临时目录）
    ///   - rootDir: mtime 作为失效令牌的目录；nil 表示无新会话可发现，只按 TTL 失效
    ///   - locate: 真正的遍历实现，仅在缓存失效时调用
    private static func locatedSession(key: String, rootDir: URL?, now: Date, locate: () -> LocatedSession?) -> LocatedSession? {
        locateLock.lock()
        defer { locateLock.unlock() }
        // 令牌取 stat() 而非 URL.resourceValues：后者有毫秒级缓存窗口，作为「目录变没过」
        // 的依据不够可靠（同一问题曾在尾读合并上实测误命中，见 LogTailReader）
        let rootDate = rootDir.flatMap { LogTailReader.statModificationDate($0.path) }
        if let cached = locateCache[key], now < cached.expires, cached.rootDate == rootDate {
            // 负结果同样命中：没定位到会话的 Agent 才是常态（装了但闲置），
            // 把 nil 挡在命中条件外等于每拍重跑一次全树 stat——正是本缓存要消掉的开销。
            // 但**只在有失效令牌时**才缓存 nil：rootDir 为 nil 的调用方（cline 的 tasks 根）
            // 令牌恒等于 nil，缓存负结果等于把「用户刚开的第一个会话」延迟到 TTL 到期才发现
            guard let hit = cached.located else { return nil }
            var fresh = hit
            // 只把时间戳往前修正：定位到的文件本身若有新写入要跟上；而 Antigravity 的
            // 有效时间可能来自同会话的后台任务日志（重算代价高），故绝不让它退化得更旧
            if let current = LogTailReader.statModificationDate(hit.file.path), current > fresh.mtime {
                fresh.mtime = current
            }
            return fresh
        }
        let located = locate()
        // 无令牌可依据时不写负结果（见上）；正结果照写——它只是把「已经找到的」复用
        if located != nil || rootDir != nil {
            locateCache[key] = (now.addingTimeInterval(locateTTL), rootDate, located)
        }
        return located
    }

    private static func modifiedDate(of url: URL) -> Date? {
        try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate
    }

    /// DSH 投影缓存目录：从档案 sessionDirs 里挑出投影那一层。
    /// 路径由注册表声明一次即可——解析器再硬编码一份，档案换目录时只会有一半生效。
    static func dshProjectionDir(in dirs: [String]) -> String? {
        dirs.first { URL(fileURLWithPath: $0).lastPathComponent == "sessions" && $0.contains("session_projcache") }
    }

    /// Antigravity 会话根（`~/.gemini/antigravity/brain`，同样只在注册表里写一次）
    static func antigravityBrainDir(in dirs: [String]) -> String? {
        dirs.first { URL(fileURLWithPath: $0).lastPathComponent == "brain" }
    }

    /// 探测一轮会话：优先消费 FileMonitor 已在后台定位出的每目录最新文件并做有界尾读。
    /// 专有解析器需要自行定位会话时，走带失效令牌的定位缓存（见 `locatedSession`），
    /// 因此调用方（@MainActor 采样）不会每拍重走会话树。
    public static func probe(profile: AgentProfile, activityFiles: [URL], now: Date = Date()) -> AgentSessionProbe {
        // 本轮探测的失败现场：用局部变量随 AgentSessionProbe 一起交回，不进任何按
        // agent id 索引的静态字典——那正是上一轮修掉的跨 Agent 串味形态（见 AgentSessionProbe 的说明）。
        // 一轮只留第一条原因：多条原因对用户的诊断价值相同，而「读不到」这件事本身只需说一次。
        var failure: SessionProbeHealth?
        let report: (SessionProbeHealth) -> Void = { health in
            if failure == nil { failure = health }
        }
        // 专有 Agent 协议优先：拥有高保真结构化日志/专有解析器的 Agent（如 Antigravity、DSH、Cline、Roo）
        // 必须使用其专有解析器，避免被通用检测器的关键字深搜造成 attention/active 误判。
        // 按档案声明的**方言**分派而非 agent id：Cline 与 Roo Code 共用一套格式，
        // 新增复用既有格式的 Agent 只改注册表。
        switch profile.sessionDialect {
        case .antigravityBrain:
            // Antigravity 读自家 tasks/log 尾窗，不涉及只读库与整体读入的大 JSON，
            // 本轮没有可上报的探测故障（其解析失败仍按「无信号」降级）
            return probeAntigravitySession(dirs: profile.sessionDirs, now: now)
        case .dshProjection:
            guard let dir = dshProjectionDir(in: profile.sessionDirs) else { return AgentSessionProbe() }
            return AgentSessionProbe(signal: inspectDSHSession(baseDir: dir, now: now, report: report),
                                     health: failure)
        case .qoderTranscript:
            return AgentSessionProbe(signal: inspectQoderTranscript(dirs: profile.sessionDirs, now: now, report: report),
                                     health: failure)
        case .clineTasks:
            if let signal = inspectClineOrRooTasks(dirs: profile.sessionDirs, now: now, report: report) {
                return AgentSessionProbe(signal: signal, health: failure)
            }
        case .genericTail:
            break
        }

        let candidates = activityFiles.compactMap { url -> (URL, Date)? in
            guard let mtime = modifiedDate(of: url) else { return nil }
            return (url, mtime)
        }.sorted { $0.1 > $1.1 }

        for (file, mtime) in candidates {
            let age = max(0, now.timeIntervalSince(mtime))
            // 等待确认可以持续较久；完成态只保留一小段时间，之后自然显示“待机”。
            guard age <= 24 * 3600 else { continue }
            if file.lastPathComponent == "ui_messages.json" {
                if let json = clineMessages(from: file, report: report),
                   let signal = detectClineOrRoo(messages: json, fileAge: age) {
                    return AgentSessionProbe(signal: signal, health: failure)
                }
            }
            let lines = LogTailReader.read(from: file, maxLines: 96, maxBytes: 262_144)
            guard let signal = detect(lines: lines) else { continue }
            if case .completed = signal, age > 15 * 60 { continue }
            return AgentSessionProbe(signal: signal, health: failure)
        }
        // 部分桌面 Agent 把会话只写进 SQLite，FileMonitor 的 latest file 只能定位到
        // 二进制库本身。对已知 schema 做只读、索引命中的末条查询；失败即无信号。
        return AgentSessionProbe(signal: inspectKnownDatabase(profile: profile, now: now, report: report),
                                 health: failure)
    }

    /// 大会话文件按内存映射读取：Cline 的 ui_messages.json、DSH 的投影缓存都会随会话
    /// 无上限增长，整块读进堆内存会在主线程上产生一次大拷贝（映射则由内核按需换页）。
    /// 超过上限直接放弃本轮信号，降级到 CPU/写入双信号判定，不能让解析拖垮采样。
    private static let maxSessionFileBytes = 32_000_000

    private static func readJSONFile(_ url: URL,
                                     report: (SessionProbeHealth) -> Void = { _ in }) -> Data? {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0
        guard size <= maxSessionFileBytes else {
            // 上限本身就是「本轮放弃」的现场：不记下来，这轮与「会话里真的没内容」在
            // 用户侧完全同形（都显示待机），而 32MB 的会话文件恰恰是最常见的那类会失明的
            report(SessionProbeHealth(failure: .oversizedFile, path: url.path))
            return nil
        }
        guard let data = try? Data(contentsOf: url, options: [.mappedIfSafe]) else {
            // 静默 return nil 的话，岛显示待机、doctor 说「结论可信」——而文件明明就在那里
            report(SessionProbeHealth(failure: .unreadableFile, path: url.path))
            return nil
        }
        return data
    }

    /// 读并解码 Cline/Roo 的 `ui_messages.json`。解码失败必须留证据：
    /// 「对方改版」与「这个会话确实没有待确认事项」在用户侧此前完全同形
    private static func clineMessages(from url: URL,
                                      report: (SessionProbeHealth) -> Void) -> [[String: Any]]? {
        guard let data = readJSONFile(url, report: report) else { return nil }
        if let json = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] {
            return json
        }
        report(SessionProbeHealth(failure: .undecodableFile, path: url.path))
        return nil
    }

    /// 用户主动中断本轮执行的系统提示（Claude Code 的 `[Request interrupted by user]`、
    /// Codex 的 turn_aborted 等）。只认「来自用户的短消息」，因为工具输出里也可能
    /// 原样出现这些字样（例如 Agent 正在读取自家的运行日志）。
    private static func isInterruptionNotice(_ line: String, facts: LineFacts) -> Bool {
        guard line.count <= 400, facts.roles.contains("user") || facts.types.contains("user") else { return false }
        let lowered = line.lowercased()
        return interruptionPhrases.contains { lowered.contains($0) }
    }

    private static let interruptionPhrases: [String] = [
        "interrupted by user", "request interrupted", "user interrupted",
        "turn_aborted", "turn aborted", "aborted by user",
        "cancelled by user", "canceled by user", "user cancelled", "user canceled",
    ]

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

            var facts = LineFacts()
            collectFacts(object, depth: 0, lineIndex: idx, sourceLine: line, into: &facts)
            for (id, call) in facts.toolCalls { pendingCalls[id] = call }
            completedCallIds.formUnion(facts.resolvedCallIds)

            // 用户中断（Ctrl-C / 点停止）会留下一条永不交付 tool_result 的执行类调用。
            // 不清掉的话：本行把 .completed 冲掉（下面的 mentionsActiveWork），而末尾的
            // 「在途命令拦截」又据这条僵尸调用返回 .active —— 引擎每拍都拿到 active，
            // 于是滞回与完成分支永远走不到，该 Agent 被钉死在 working，
            // 2s 快采样 + 高频全树扫描一并被锁住（耗电与 CPU 双输）。
            if isInterruptionNotice(line, facts: facts) {
                pendingCalls.removeAll()
                completedCallIds.removeAll()
            }

            if let request = signal?.attentionRequest, facts.resolves(request) {
                signal = nil
                continue
            }

            // 尾窗可能只包含 AskUserQuestion 的 tool_result、而请求本身已落在窗口外；
            // 结果记录里的 toolName 不能被反向当成一条新请求。
            if facts.isResolutionRecord { continue }

            if let marker = facts.requestMarker {
                signal = .attention(AgentAttentionRequest(
                    fingerprint: marker.fingerprint,
                    message: marker.approval ? "等待你批准操作" : "等待你选择或确认"
                ))
                continue
            }

            if let completion = facts.completionFingerprint {
                signal = .completed(fingerprint: completion)
                continue
            }

            // 新一轮用户消息、推理或工具调用使旧 task_complete 或旧 attention 失效。token_count / usage /
            // item_completed 等收尾记账事件是中性的，不会把状态立即冲掉。
            if facts.mentionsActiveWork {
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

    // MARK: - 单行事实收集（一次遍历替代十余次递归）

    struct ActiveToolCall {
        let id: String
        let name: String
        let command: String?
        let lineIndex: Int
    }

    /// 一行 JSON 解析后一次遍历所得的事实。
    ///
    /// 此前每行要跑约 13 趟全树递归（请求标记、完成标记、工具调用与结果、
    /// role/source/type/status/state 各一趟、标识符再一趟），实测 96 行尾窗
    /// 7.6ms/拍且全部发生在 @MainActor 上，其中 JSON 解析本身只占 0.27ms：
    /// 也就是说 96% 的开销是把同一棵已经解析好的树反复走完。
    /// 现在每行一趟，键值只在节点上读一次。
    private struct LineFacts {
        var requestMarker: (fingerprint: String, approval: Bool)?
        var completionFingerprint: String?
        var toolCalls: [String: ActiveToolCall] = [:]
        var resolvedCallIds: Set<String> = []
        var roles: Set<String> = []
        var sources: Set<String> = []
        var types: Set<String> = []
        /// status 与 state 两个键合并收集：只有「等待/已解决」类判定用到，二者语义相同
        var states: Set<String> = []
        var identifiers: Set<String> = []

        /// 结果由用户/用户显式动作产生（解除等待确认的依据之一）
        var attributedToUser: Bool { roles.contains("user") || sources.contains("user") || sources.contains("userexplicit") }
        var isResolutionRecord: Bool { roles.contains("toolresult") || !types.isDisjoint(with: resolutionTypes) }
        var mentionsActiveWork: Bool { roles.contains("user") || !types.isDisjoint(with: activeTypes) }
        var mentionsResolution: Bool {
            !types.isDisjoint(with: resolutionTypes) || !states.isDisjoint(with: resolutionStates)
        }

        /// 是否解除当前等待确认。判定顺序与原实现一致：先看是否出自用户，
        /// 再看是否携带明确的解决态；有关联 ID 时必须对应当前请求，无 ID 的显式
        /// approval_response 也可解除。
        func resolves(_ request: AgentAttentionRequest) -> Bool {
            if attributedToUser { return true }
            guard mentionsResolution else { return false }
            return identifiers.isEmpty || identifiers.contains(request.fingerprint)
        }
    }

    private static func collectFacts(_ value: Any, depth: Int, lineIndex: Int, sourceLine: String, into facts: inout LineFacts) {
        if let array = value as? [Any] {
            for child in array {
                collectFacts(child, depth: depth, lineIndex: lineIndex, sourceLine: sourceLine, into: &facts)
            }
            return
        }
        guard let dict = value as? [String: Any] else { return }

        // 每个键只读一次并归一化，供本节点的全部判定复用
        let type = normalized(dict["type"] as? String)
        let status = normalized(dict["status"] as? String)
        let role = normalized(dict["role"] as? String)
        let source = normalized(dict["source"] as? String)
        if !role.isEmpty { facts.roles.insert(role) }
        if !source.isEmpty { facts.sources.insert(source) }
        if !type.isEmpty { facts.types.insert(type) }
        let state = normalized(dict["state"] as? String)
        if !status.isEmpty { facts.states.insert(status) }
        if !state.isEmpty { facts.states.insert(state) }

        // identifier(in:) 会做 12 次键查找并向下钻 data/payload，每节点只算一次
        let nodeID = identifier(in: dict)
        if let id = nodeID { facts.identifiers.insert(id) }

        // 在途工具调用与其结果（终端类命令的生命周期，用于拦截虚假 completed）
        if type == "tooluse" || type == "functioncall" || type == "customtoolcall" || type == "toolcall" {
            if let id = nodeID {
                if status == "completed" || status == "done" || status == "success" || status == "failed" || status == "error" {
                    facts.resolvedCallIds.insert(id)
                } else {
                    let name = dict["name"] as? String ?? dict["function"] as? String ?? ""
                    facts.toolCalls[id] = ActiveToolCall(id: id, name: name, command: extractCommand(from: dict), lineIndex: lineIndex)
                }
            }
        }
        if type == "toolresult" || type == "functioncalloutput" || type == "customtoolcalloutput"
            || status == "completed" || status == "success" || status == "done" {
            for key in ["tool_use_id", "toolUseId", "call_id", "callId", "id"] {
                if let id = dict[key] as? String, !id.isEmpty {
                    facts.resolvedCallIds.insert(id)
                }
            }
        }

        // 请求标记与完成标记都取 DFS 首个命中，与原先各自早退的遍历语义一致
        if facts.requestMarker == nil {
            let kind = normalized(dict["kind"] as? String)
            let name = normalized(dict["name"] as? String)
            let marker = [name, type, status, state, kind].first {
                requestNames.contains($0) || requestStates.contains($0)
            }
            if let marker {
                let approval = marker.contains("approval") || marker.contains("permission") || marker.contains("confirm")
                facts.requestMarker = (nodeID ?? stableFingerprint(sourceLine), approval)
            }
        }
        if facts.completionFingerprint == nil {
            let subtype = normalized(dict["subtype"] as? String)
            let isCompletion = completionTypes.contains(type) || completionTypes.contains(subtype)
                || (depth == 0 && type == "result")
            // WorkBuddy 等格式以 message.status=completed 表示本轮消息终态；普通工具调用
            // 的 status=completed 不会命中，因为其 type 是 tool_use/custom_tool_call。
            let isCompletedMessage = depth <= 1 && type == "message" && status == "completed"
            if isCompletion || isCompletedMessage {
                facts.completionFingerprint = nodeID ?? stableFingerprint(sourceLine)
            }
        }

        for child in dict.values {
            collectFacts(child, depth: depth + 1, lineIndex: lineIndex, sourceLine: sourceLine, into: &facts)
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

    // MARK: - Qoder 专有解析器

    /// Qoder（阿里的 agentic IDE）：`~/.qoder/projects/<项目 slug>/<会话 uuid>.jsonl`，
    /// 逐行是 Anthropic 兼容的对话记录（`message.role` / `content[]` / `stop_reason` / `usage`），
    /// 工具名与 Claude Code 同源（Bash / Edit / Write / Read / Agent / AskUserQuestion）。
    ///
    /// 为什么不交给通用尾窗关键词扫描：Qoder 的「等待你回答」是 **`AskUserQuestion` 这个
    /// tool_use 还没有对应 tool_result**；回答完之后结果会立刻补上。只看关键字会在
    /// 「刚答完的那一拍」反向误报（本仓在 Claude 与 AskUserQuestion 上踩过同一形态），
    /// 而反向误报的代价是用户被叫回来发现什么都没发生。
    public static func inspectQoderTranscript(dirs: [String], now: Date,
                                              report: (SessionProbeHealth) -> Void = { _ in }) -> AgentSessionSignal? {
        let root = dirs.first.map { URL(fileURLWithPath: $0) }
        guard let found = locatedSession(key: "qoder|\(dirs.joined(separator: ","))",
                                         rootDir: root, now: now,
                                         locate: { walkQoderSessions(dirs: dirs) }) else { return nil }
        let age = max(0, now.timeIntervalSince(found.mtime))
        guard age <= 24 * 3600 else { return nil }
        // 单个会话文件实测可到 10MB（每行还挂 requestTokenAnchor 的整包请求/响应），
        // 必须走有界尾读：状态只取决于最近若干条消息
        let lines = LogTailReader.read(from: found.file, maxLines: 60, maxBytes: 262_144)
        guard !lines.isEmpty else { return nil }
        return detectQoder(lines: lines, fileAge: age)
    }

    /// 遍历 projects/<slug>/*.jsonl 取最近修改的一个。
    /// 走 `locatedSession` 的 TTL + 根 mtime 缓存：项目数与历史会话数会一直涨，
    /// 每拍全量 stat 正是当初给 Antigravity/DSH/Cline 加缓存的同一个理由。
    private static func walkQoderSessions(dirs: [String]) -> LocatedSession? {
        let fm = FileManager.default
        var newest: LocatedSession?
        for dir in dirs {
            guard let slugs = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            let base = URL(fileURLWithPath: dir)
            for slug in slugs {
                let project = base.appendingPathComponent(slug)
                guard let files = try? fm.contentsOfDirectory(atPath: project.path) else { continue }
                for name in files where name.hasSuffix(".jsonl") {
                    let url = project.appendingPathComponent(name)
                    guard let mtime = modifiedDate(of: url) else { continue }
                    if newest == nil || mtime > newest!.mtime {
                        newest = LocatedSession(file: url, mtime: mtime, sidecar: nil)
                    }
                }
            }
        }
        return newest
    }

    /// 这些工具在等**人**：没有 tool_result 时球在用户这边
    private static let qoderRequestTools: Set<String> = [
        "askuserquestion", "ask_question", "exitplanmode", "exit_plan_mode",
    ]

    public static func detectQoder(lines: [String], fileAge: TimeInterval) -> AgentSessionSignal? {
        struct Row {
            let id: String
            let role: String
            let stopReason: String
            let uses: [(name: String, id: String, hint: String)]
            let results: [String]
        }
        var rows: [Row] = []
        rows.reserveCapacity(lines.count)
        for raw in lines {
            guard let data = raw.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let message = obj["message"] as? [String: Any],
                  let role = message["role"] as? String else { continue }
            var uses: [(name: String, id: String, hint: String)] = []
            var results: [String] = []
            if let content = message["content"] as? [[String: Any]] {
                for block in content {
                    switch block["type"] as? String {
                    case "tool_use":
                        guard let name = block["name"] as? String else { continue }
                        uses.append((name, (block["id"] as? String) ?? "", qoderToolHint(block["input"])))
                    case "tool_result":
                        if let src = block["tool_use_id"] as? String { results.append(src) }
                    default: break
                    }
                }
            }
            rows.append(Row(id: (message["id"] as? String) ?? "", role: role,
                            stopReason: (message["stop_reason"] as? String) ?? "",
                            uses: uses, results: results))
        }
        guard let first = rows.first else { return nil }
        var answered = Set<String>()
        for row in rows { for id in row.results where !id.isEmpty { answered.insert(id) } }
        _ = first

        guard let lastAssistant = rows.last(where: { $0.role == "assistant" }) else { return nil }
        let pending = lastAssistant.uses.filter { !$0.id.isEmpty && !answered.contains($0.id) }

        if let ask = pending.first(where: { qoderRequestTools.contains($0.name.lowercased()) }) {
            return .attention(AgentAttentionRequest(
                fingerprint: "qoder-\(ask.id)",
                message: ask.name.lowercased().contains("plan") ? "等你确认下一步方案" : "等待你回答或选择"))
        }
        if let use = pending.first {
            return .active(fingerprint: "qoder-\(use.id)",
                           action: qoderActionText(name: use.name, hint: use.hint))
        }
        // 尾窗里的工具调用都已返回结果
        if lastAssistant.stopReason == "end_turn" || lastAssistant.stopReason == "stop_sequence" {
            // 完成态只保留一小段时间，之后自然回到「待机」——与其他方言同口径
            guard fileAge <= 15 * 60 else { return nil }
            return .completed(fingerprint: "qoder-\(lastAssistant.id)")
        }
        return .active(fingerprint: "qoder-cont-\(lastAssistant.id)", action: "继续处理中")
    }

    /// 从 tool_use 的 input 里取一条能给人看的线索（命令 / 文件名 / 任务描述）
    private static func qoderToolHint(_ input: Any?) -> String {
        guard let dict = input as? [String: Any] else { return "" }
        for key in ["command", "file_path", "path", "pattern", "prompt", "description", "toolName"] {
            if let text = dict[key] as? String, !text.isEmpty {
                return clipDSHString(text.replacingOccurrences(of: "\n", with: " "), limit: 40)
            }
        }
        return ""
    }

    private static func qoderActionText(name: String, hint: String) -> String {
        switch name.lowercased() {
        case "bash":            return hint.isEmpty ? "执行终端命令" : "运行: \(hint)"
        case "edit", "write",
             "multiedit":       return hint.isEmpty ? "修改文件" : "修改: \(hint)"
        case "read", "grep",
             "glob":            return hint.isEmpty ? "读取代码" : "读取: \(hint)"
        case "agent":           return hint.isEmpty ? "派生子任务处理中" : "子任务: \(hint)"
        case "mcp_call":        return hint.isEmpty ? "调用外部工具" : "调用工具: \(hint)"
        default:                return hint.isEmpty ? "正在执行 \(name)" : "\(name): \(hint)"
        }
    }

    // MARK: - Cline / Roo Code 专有解析器

    public static func detectClineOrRoo(messages: [[String: Any]], fileAge: TimeInterval) -> AgentSessionSignal? {
        guard !messages.isEmpty else { return nil }

        for msg in messages.reversed() {
            let type = msg["type"] as? String ?? ""
            let ts = msg["ts"] as? Double ?? Double(Date().timeIntervalSince1970 * 1000)
            let text = msg["text"] as? String ?? ""
            // 饱和转换：ts 来自第三方 ui_messages.json，一条 1e30 / -9.2e18 哨兵就能让
            // Int64(Double) 在 @MainActor 采样拍上当场 trap（岛直接消失，2s 后必复现）。
            // 同一 bug 类本仓已在 AgentLogStreamer / TokenUsageMonitor 修过并留了 SafeNumber。
            let fp = "cline-\(SafeNumber.saturatingInt(ts, source: "cline.ts"))"

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

    public static func inspectClineOrRooTasks(dirs: [String], now: Date,
                                              report: (SessionProbeHealth) -> Void = { _ in }) -> AgentSessionSignal? {
        // 任务数会随使用持续累积（每个任务一个目录 + 一次 stat），同样只在缓存失效时遍历
        guard let found = locatedSession(key: "cline|\(dirs.joined(separator: ","))", rootDir: nil, now: now, locate: {
            walkClineTasks(dirs: dirs)
        }) else { return nil }
        let age = max(0, now.timeIntervalSince(found.mtime))
        guard age <= 24 * 3600 else { return nil }
        guard let json = clineMessages(from: found.file, report: report) else {
            return nil
        }
        return detectClineOrRoo(messages: json, fileAge: age)
    }

    /// 遍历任务目录定位最近一次 ui_messages.json。
    private static func walkClineTasks(dirs: [String]) -> LocatedSession? {
        let fm = FileManager.default
        var newest: LocatedSession?
        for dir in dirs {
            guard let taskDirs = try? fm.contentsOfDirectory(atPath: dir) else { continue }
            for sub in taskDirs {
                let url = URL(fileURLWithPath: dir).appendingPathComponent(sub).appendingPathComponent("ui_messages.json")
                guard let mtime = modifiedDate(of: url) else { continue }
                if newest == nil || mtime > newest!.mtime {
                    newest = LocatedSession(file: url, mtime: mtime, sidecar: nil)
                }
            }
        }
        return newest
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

    /// 按档案声明的库位置与 schema 取会话终态；未登记 `sessionDatabase` 的 Agent 无此信号源。
    /// `report` 收下探测失败的原因：无信号 ≠ 探测成功，前者可能就是「我们瞎了」。
    private static func inspectKnownDatabase(
        profile: AgentProfile, now: Date,
        report: (SessionProbeHealth) -> Void = { _ in }
    ) -> AgentSessionSignal? {
        guard let database = profile.sessionDatabase else { return nil }
        switch database.schema {
        case .dimTasks:
            return inspectDimDatabase(path: database.path, now: now, report: report)
        case .statusIndex:
            guard let sql = database.statusSQL else { return nil }
            return inspectStatusDatabase(path: database.path, sql: sql, now: now, report: report)
        case .openCode:
            return inspectOpenCodeDatabase(path: database.path, now: now, report: report)
        }
    }

    /// Dim 的确认请求是一条 assistant AskUserQuestion tool call，用户处理后追加关联
    /// tool_result；读取最新会话的末 32 条即可按 call id 配对，不碰问题/答案正文。
    private static func inspectDimDatabase(path: String, now: Date,
                                           report: (SessionProbeHealth) -> Void) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path, report: report) { db, report in
            let sql = """
            SELECT rowid, role, toolMetadata, parts
            FROM messages
            WHERE sessionId = (SELECT sessionId FROM messages ORDER BY rowid DESC LIMIT 1)
            ORDER BY rowid DESC LIMIT 32;
            """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
                // schema 变了（例如 messages 表改名/整库迁移）：这恰是最需要留下证据的一刻，
                // 否则该 Agent 从此永远显示待机，无人知道为什么
                report(SessionProbeHealth(failure: .prepareFailed, path: path))
                return nil
            }
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
    private static func inspectStatusDatabase(path: String, sql: String, now: Date,
                                              report: (SessionProbeHealth) -> Void) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path, report: report) { db, report in
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
                report(SessionProbeHealth(failure: .prepareFailed, path: path))
                return nil
            }
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

    private static func inspectOpenCodeDatabase(path: String, now: Date,
                                                report: (SessionProbeHealth) -> Void) -> AgentSessionSignal? {
        guard fileAge(path, now: now) <= 24 * 3600 else { return nil }
        return withDB(path, report: report) { db, report in
            let sql = """
            SELECT data FROM part
            WHERE session_id = (SELECT id FROM session ORDER BY time_updated DESC LIMIT 1)
            ORDER BY rowid DESC LIMIT 32;
            """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
                report(SessionProbeHealth(failure: .prepareFailed, path: path))
                return nil
            }
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
    public static func inspectDSHSession(baseDir: String, now: Date = Date(),
                                         report: (SessionProbeHealth) -> Void = { _ in }) -> AgentSessionSignal? {
        let projcacheDir = URL(fileURLWithPath: baseDir)
        // 投影目录实测 500+ 会话文件，一趟 stat 约 15ms，不能每拍在 @MainActor 上重走
        let key = "dsh|\(projcacheDir.path)"
        guard let found = locatedSession(key: key, rootDir: projcacheDir, now: now, locate: {
            walkDSHProjections(in: projcacheDir)
        }) else { return nil }
        let age = max(0, now.timeIntervalSince(found.mtime))
        guard age <= 24 * 3600 else { return nil }

        guard let data = readJSONFile(found.file, report: report),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let record = root["record"] as? [String: Any],
              let rows = record["rows"] as? [String: Any] else {
            return nil
        }

        let sessionId = found.file.deletingPathExtension().lastPathComponent

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
        let openTurnStartSeq = SafeNumber.jsonInt(tbVal?["openTurnStartSeq"])

        let sessionStats = rows["sessionStats"] as? [String: Any]
        let statsVal = sessionStats?["val"] as? [String: Any]
        let openStep = statsVal?["openStep"] as? [String: Any]
        let currentStep = SafeNumber.jsonInt(openStep?["step"]) ?? SafeNumber.jsonInt(statsVal?["steps"])
        let currentTurn = SafeNumber.jsonInt(openStep?["turn"]) ?? SafeNumber.jsonInt(statsVal?["lastTurn"]) ?? 1

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
            let totalSteps = SafeNumber.jsonInt(statsVal?["steps"]) ?? 0
            let fingerprint = "dsh-\(sessionId)-t\(currentTurn)-s\(totalSteps)"
            return .completed(fingerprint: fingerprint)
        }
    }

    /// 遍历投影目录定位最新会话文件（仅在定位缓存失效时调用）。
    private static func walkDSHProjections(in dir: URL) -> LocatedSession? {
        let fm = FileManager.default
        guard let files = try? fm.contentsOfDirectory(atPath: dir.path) else { return nil }
        var newest: LocatedSession?
        for name in files where name.hasSuffix(".json") {
            let url = dir.appendingPathComponent(name)
            guard let mdate = modifiedDate(of: url) else { continue }
            if newest == nil || mdate > newest!.mtime {
                newest = LocatedSession(file: url, mtime: mdate, sidecar: nil)
            }
        }
        return newest
    }

    /// 解析 Google Antigravity 轨迹日志 (.system_generated/logs/transcript.jsonl)，
    /// 提取活跃执行态、提问确认与终态，以及本轮的后台任务/子智能体/Token 细分上下文。
    public static func probeAntigravitySession(dirs: [String], now: Date = Date()) -> AgentSessionProbe {
        guard let brain = antigravityBrainDir(in: dirs) else { return AgentSessionProbe() }
        let brainDir = URL(fileURLWithPath: brain)
        let key = "antigravity|\(brainDir.path)"
        guard let found = locatedSession(key: key, rootDir: brainDir, now: now, locate: {
            walkAntigravitySessions(in: brainDir)
        }) else {
            return AgentSessionProbe()
        }
        let age = max(0, now.timeIntervalSince(found.mtime))
        guard age <= 24 * 3600 else { return AgentSessionProbe() }

        let lines = LogTailReader.read(from: found.file, maxLines: 120, maxBytes: 262_144)
        guard !lines.isEmpty else { return AgentSessionProbe() }

        var context = SessionActiveContext()
        let signal = detectAntigravitySession(lines: lines, fileAge: age, now: now, tasksDir: found.sidecar) {
            context = $0
        }
        return AgentSessionProbe(signal: signal, context: context)
    }

    /// 遍历 brain 找出「有效最新」的会话：transcript 与它自己的后台任务日志取较新者，
    /// 这样长耗时后台命令（构建/测试）执行期间不会被误判为已完成。
    private static func walkAntigravitySessions(in brainDir: URL) -> LocatedSession? {
        guard let subdirs = try? FileManager.default.contentsOfDirectory(
            at: brainDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) else {
            return nil
        }
        var newestFile: URL?
        var newestTasksDir: URL?
        var newestTime: Date = .distantPast
        for sub in subdirs {
            let logFile = sub.appendingPathComponent(".system_generated/logs/transcript.jsonl")
            guard let mtime = modifiedDate(of: logFile) else { continue }
            var effectiveTime = mtime
            let tasksDir = sub.appendingPathComponent(".system_generated/tasks")
            if let taskFiles = try? FileManager.default.contentsOfDirectory(
                at: tasksDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) {
                for tf in taskFiles where tf.pathExtension == "log" {
                    if let tMTime = modifiedDate(of: tf), tMTime > effectiveTime {
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
        guard let target = newestFile else { return nil }
        return LocatedSession(file: target, mtime: newestTime, sidecar: newestTasksDir)
    }

    /// 解析 Antigravity transcript.jsonl 末尾若干行，推导当前状态信号。
    /// - Parameter contextSink: 收到本轮解析出的后台任务/子智能体/Token 细分上下文。
    ///   上下文与信号一样只在一次探测内存活，由调用方随 `AgentSessionProbe` 带出；
    ///   它曾经存放在按 agent id 索引的全局字典里，任何 Agent 的卡片都会因此串到
    ///   Antigravity 的残留上下文。只关心信号的调用方可忽略（测试即如此）。
    public static func detectAntigravitySession(
        lines: [String],
        fileAge: TimeInterval,
        now: Date = Date(),
        tasksDir: URL? = nil,
        contextSink: (SessionActiveContext) -> Void = { _ in }
    ) -> AgentSessionSignal? {
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
            let si = SafeNumber.jsonInt(obj["step_index"]) ?? 0
            let st = obj["type"] as? String ?? ""
            let hasTC = (obj["tool_calls"] as? [[String: Any]])?.isEmpty == false
            parsedMeta.append((si, st, hasTC))

            // 提取 Token 统计细分
            if let usage = obj["usageMetadata"] as? [String: Any] ?? obj["usage"] as? [String: Any] ?? obj["token_count"] as? [String: Any] {
                let p = SafeNumber.jsonInt(usage["promptTokenCount"]) ?? SafeNumber.jsonInt(usage["prompt_tokens"]) ?? SafeNumber.jsonInt(usage["input_tokens"]) ?? 0
                let c = SafeNumber.jsonInt(usage["candidatesTokenCount"]) ?? SafeNumber.jsonInt(usage["candidates_tokens"]) ?? SafeNumber.jsonInt(usage["output_tokens"]) ?? 0
                let cr = SafeNumber.jsonInt(usage["cachedContentTokenCount"]) ?? SafeNumber.jsonInt(usage["cache_read_tokens"]) ?? 0
                let cw = SafeNumber.jsonInt(usage["cache_write_tokens"]) ?? 0
                let th = SafeNumber.jsonInt(usage["thoughtsTokenCount"]) ?? SafeNumber.jsonInt(usage["reasoning_tokens"]) ?? 0
                let tot = SafeNumber.jsonInt(usage["totalTokenCount"]) ?? SafeNumber.jsonInt(usage["total_tokens"]) ?? (p + c + cr + cw + th)
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
        contextSink(ctx)

        func makeActiveSignal(fingerprint: String, action: String) -> AgentSessionSignal {
            return .active(fingerprint: fingerprint, action: action)
        }

        // 从尾部逆序扫描推导当前最新状态信号
        for rawLine in lines.reversed() {
            guard let data = rawLine.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }

            let stepIndex = SafeNumber.jsonInt(obj["step_index"]) ?? 0
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

    /// 只读会话库查询的统一入口：结果仍按「查不到就是 nil」降级，但把「为什么查不到」
    /// 交给 `report`——「解析器坏了」绝不能与「Agent 空闲」同形（见 SessionProbeHealth）。
    private static func withDB<T>(_ path: String,
                                  report: (SessionProbeHealth) -> Void = { _ in },
                                  _ body: @escaping (OpaquePointer, (SessionProbeHealth) -> Void) -> T?) -> T? {
        let box = HealthBox()
        var result = attempt(path, box, body)
        if box.containsPrepareFailure {
            // prepare 失败先当作**句柄陈旧**处理：原地重建（VACUUM / 截断）不改 inode，
            // 缓存的旧连接会被继续复用，此时直接定性为「对方改了表结构」是误诊。
            // 作废连接重试一次，两次都失败才对外上报——真改表仍会落到 prepareFailed。
            box.items.removeAll()
            ReadonlyDB.invalidate(path)
            result = attempt(path, box, body)
        }
        box.items.forEach(report)
        return result
    }

    /// 收集一次查询过程中的故障。用引用类型承载：捕获局部变量的嵌套函数
    /// 无法作为 `@escaping` 闭包传给连接层。
    private final class HealthBox {
        var items: [SessionProbeHealth] = []
        var containsPrepareFailure: Bool { items.contains { $0.failure == .prepareFailed } }
    }

    private static func attempt<T>(_ path: String,
                                   _ box: HealthBox,
                                   _ body: @escaping (OpaquePointer, (SessionProbeHealth) -> Void) -> T?) -> T? {
        ReadonlyDB.withConnection(path, onFailure: { failure in
            switch failure {
            case .missing:
                // 库从未创建（App 装了但没跑过会话）不是故障：报出来会把首次启动刷成一片红，
                // 而这恰恰是「还没有会话」的正常形态
                break
            case .openFailed:
                box.items.append(SessionProbeHealth(failure: .unreadableDB, path: path))
            }
        }, { db in body(db, { box.items.append($0) }) }) ?? nil
    }
}

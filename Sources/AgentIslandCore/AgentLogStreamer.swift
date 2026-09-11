import Foundation
import SQLite3

// MARK: - 智能体实时日志/事件流模型

public struct AgentLogEvent: Identifiable, Equatable, Sendable {
    public let id: String
    public let timestamp: Date
    public let kind: EventKind
    public let title: String
    public let detail: String?
    public let agentId: String

    public enum EventKind: String, Codable, Equatable, Sendable {
        case command    // 终端执行命令 (bash / sh / pnpm 等)
        case toolCall   // 工具调用 (mcp / search / fetch 等)
        case fileEdit   // 文件读写 / 编辑
        case thinking   // 思考规划 / reasoning
        case message    // 消息回复 / 评论
        case info       // 状态流转 / 会话启动

        public var label: String {
            switch self {
            case .command: return "EXEC"
            case .toolCall: return "TOOL"
            case .fileEdit: return "EDIT"
            case .thinking: return "THINK"
            case .message: return "MSG"
            case .info: return "INFO"
            }
        }
    }

    /// - Parameter id: 稳定标识。默认按「agentId + 时间戳 + 标题」推导（同一事件每次查询得到
    ///   相同 id），使视图能识别「还是同一条」而不全量重建。此前用随机 UUID，导致每 2 秒
    ///   刷新时 `result != events` 恒为真：用户展开的详情被强制折叠、滚动位置跳回顶部。
    public init(id: String? = nil,
                timestamp: Date = Date(),
                kind: EventKind,
                title: String,
                detail: String? = nil,
                agentId: String) {
        // 毫秒时间戳参与 id 拼接，来源是库字段（可能含 Int64 上限哨兵值）：
        // 直接 Int(Double) 会 trap，走饱和解析保证「打开流水页即崩」不再发生
        self.id = id ?? "\(agentId)-\(SafeNumber.saturatingInt(timestamp.timeIntervalSince1970 * 1000, source: "event.id"))-\(title)"
        self.timestamp = timestamp
        self.kind = kind
        self.title = title
        // detail 在 330pt 卡片内整段渲染：超长原始内容（Claude 的 64KB JSONL 行、
        // dim 的整段 parts JSON、完整消息文本）对用户没有诊断价值，全量渲染造成
        // 可感知的布局卡顿。在唯一构造点截断，覆盖全部 fetch 路径。
        if let detail, detail.count > Self.maxDetailCharacters {
            self.detail = String(detail.prefix(Self.maxDetailCharacters)) + "…[已截断]"
        } else {
            self.detail = detail
        }
        self.agentId = agentId
    }

    /// detail 渲染上界（字符）。完整内容本就无法在卡内滚动查看，截断无损
    public static let maxDetailCharacters = 4096
}

// MARK: - 实时日志流提取引擎

public enum AgentLogStreamer {

    /// ISO8601DateFormatter 初始化是已知昂贵对象（~0.5ms/个），流水页每 2s 刷新；
    /// 静态化复用（线程安全），与 TokenUsageMonitor 的 isoFormatter 同模式
    private static let isoFractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    /// 无毫秒的回退解析器（antigravity 两种时间格式并存）
    private static let isoPlain = ISO8601DateFormatter()

    /// 批次内 id 去重：同一消息内的多条 part（如连续两次同名 `tool_use`）共享同一
    /// `createdAt`，派生 id「agentId-毫秒-标题」会完全相同，用作 ForEach 身份与
    /// 展开态键时触发 SwiftUI 重复 id 未定义行为（行互相顶替）。
    /// 稳定性边界：同批内稳定（出现次序由数据行序决定）；新事件不改变既有事件的
    /// 编号；仅当重复组的**首个**出现被挤出列表窗口时，后续出现的编号会前移。
    /// 首次出现保持原 id（已展开的详情态不丢）。
    static func deduplicateIds(_ events: [AgentLogEvent]) -> [AgentLogEvent] {
        var used: Set<String> = []
        var nextSuffix: [String: Int] = [:]
        return events.map { event in
            guard used.contains(event.id) else {
                used.insert(event.id)
                return event
            }
            // 基础 id 可能本身以「#数字」结尾（标题含 #），固定序号会再撞——
            // 取「本批内下一个可用序号」，同时守住去重结果与原 id 的确定性
            var k = nextSuffix[event.id, default: 1]
            var candidate = "\(event.id)#\(k)"
            while used.contains(candidate) {
                k += 1
                candidate = "\(event.id)#\(k)"
            }
            nextSuffix[event.id] = k + 1
            used.insert(candidate)
            return AgentLogEvent(id: candidate,
                                 timestamp: event.timestamp,
                                 kind: event.kind,
                                 title: event.title,
                                 detail: event.detail,
                                 agentId: event.agentId)
        }
    }

    /// 提取指定 Agent 最近的结构化事件流水
    public static func fetchRecentEvents(agentId: String, limit: Int = 20) -> [AgentLogEvent] {
        let events: [AgentLogEvent]
        switch agentId {
        case "antigravity":
            events = fetchAntigravityEvents(limit: limit)
        case "codex":
            events = fetchCodexEvents(limit: limit)
        case "dim":
            events = fetchDimEvents(limit: limit)
        case "claude":
            events = fetchClaudeEvents(limit: limit)
        case "opencode":
            events = fetchOpenCodeEvents(limit: limit)
        case "zcode":
            events = fetchZCodeEvents(limit: limit)
        case "workbuddy":
            events = fetchWorkBuddyEvents(limit: limit)
        case "hermes":
            events = fetchHermesEvents(limit: limit)
        default:
            events = []
        }
        return deduplicateIds(events)
    }

    // MARK: - 1. Antigravity 日志流 (transcript.jsonl)

    public static func fetchAntigravityEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let brainDir = URL(fileURLWithPath: "\(home)/.gemini/antigravity/brain")
        guard let subdirs = try? FileManager.default.contentsOfDirectory(at: brainDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) else {
            return []
        }

        var newestFile: URL?
        var newestTime: Date = .distantPast
        for sub in subdirs {
            let logFile = sub.appendingPathComponent(".system_generated/logs/transcript.jsonl")
            if let attrs = try? FileManager.default.attributesOfItem(atPath: logFile.path),
               let mtime = attrs[.modificationDate] as? Date, mtime > newestTime {
                newestTime = mtime
                newestFile = logFile
            }
        }

        guard let target = newestFile else { return [] }
        let lines = readLastLines(from: target, maxLines: limit * 2)
        var events: [AgentLogEvent] = []

        for line in lines.reversed() {
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }

            let createdAtStr = obj["created_at"] as? String ?? ""
            let date = Self.isoFractional.date(from: createdAtStr) ?? Self.isoPlain.date(from: createdAtStr) ?? Date()

            if let toolCalls = obj["tool_calls"] as? [[String: Any]], !toolCalls.isEmpty {
                for tc in toolCalls {
                    let name = tc["name"] as? String ?? "tool"
                    var summary: String? = nil
                    var detail: String? = nil
                    if let args = tc["args"] as? [String: Any] {
                        summary = (args["toolAction"] as? String) ?? (args["toolSummary"] as? String)
                        if let cmd = args["CommandLine"] as? String {
                            detail = cmd
                        } else if let path = args["AbsolutePath"] as? String ?? (args["TargetFile"] as? String) {
                            detail = URL(fileURLWithPath: path).lastPathComponent
                        }
                    }
                    let title = summary ?? name
                    let kind: AgentLogEvent.EventKind
                    if name.contains("command") || name.contains("run") || name.contains("bash") {
                        kind = .command
                    } else if name.contains("file") || name.contains("write") || name.contains("replace") {
                        kind = .fileEdit
                    } else {
                        kind = .toolCall
                    }

                    events.append(AgentLogEvent(
                        timestamp: date,
                        kind: kind,
                        title: "\(name): \(title)",
                        detail: detail,
                        agentId: "antigravity"
                    ))
                }
            } else if let type = obj["type"] as? String {
                if type == "PLANNER_RESPONSE", let thinking = obj["thinking"] as? String, !thinking.isEmpty {
                    let firstLine = thinking.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? "思考分析中"
                    events.append(AgentLogEvent(
                        timestamp: date,
                        kind: .thinking,
                        title: firstLine.count > 40 ? String(firstLine.prefix(37)) + "..." : firstLine,
                        detail: thinking,
                        agentId: "antigravity"
                    ))
                } else if type == "USER_INPUT", let content = obj["content"] as? String {
                    let firstLine = content.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? "用户指令"
                    events.append(AgentLogEvent(
                        timestamp: date,
                        kind: .message,
                        title: "用户输入: " + (firstLine.count > 30 ? String(firstLine.prefix(27)) + "..." : firstLine),
                        detail: content,
                        agentId: "antigravity"
                    ))
                }
            }

            if events.count >= limit { break }
        }

        return events
    }

    // MARK: - 2. Codex 日志流 (rollout-*.jsonl)

    public static func fetchCodexEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let sessionsDir = URL(fileURLWithPath: "\(home)/.codex/sessions")
        guard let newestFile = LogTailReader.newestFile(in: sessionsDir, maxAge: 86400 * 3) else { return [] }

        let lines = readLastLines(from: newestFile, maxLines: limit * 2)
        var events: [AgentLogEvent] = []

        for line in lines.reversed() {
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }

            let timeStr = obj["timestamp"] as? String ?? ""
            let date = Self.isoFractional.date(from: timeStr) ?? Date()

            if let payload = obj["payload"] as? [String: Any] {
                let ptype = payload["type"] as? String ?? ""
                if ptype == "message", let content = payload["content"] as? [[String: Any]] {
                    for item in content {
                        if let text = item["text"] as? String, !text.isEmpty {
                            let first = text.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? "助手回复"
                            events.append(AgentLogEvent(
                                timestamp: date,
                                kind: .message,
                                title: first.count > 40 ? String(first.prefix(37)) + "..." : first,
                                detail: text,
                                agentId: "codex"
                            ))
                        }
                    }
                } else if ptype == "item_completed", let item = payload["item"] as? [String: Any] {
                    let itype = item["type"] as? String ?? ""
                    if let content = item["content"] as? [[String: Any]] {
                        for c in content {
                            if let text = c["text"] as? String, !text.isEmpty {
                                let first = text.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? itype
                                events.append(AgentLogEvent(
                                    timestamp: date,
                                    kind: .info,
                                    title: "完成: " + (first.count > 36 ? String(first.prefix(33)) + "..." : first),
                                    detail: text,
                                    agentId: "codex"
                                ))
                            }
                        }
                    }
                }
            }

            if events.count >= limit { break }
        }

        return events
    }

    // MARK: - 3. DimAgent 日志流 (dimcode.sqlite)

    public static func fetchDimEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.dimcode/v2/dimcode.sqlite"
        guard let db = openReadonly(dbPath) else { return [] }
        defer { sqlite3_close(db) }

        // 原查询对全表排序（EXPLAIN: SCAN + USE TEMP B-TREE FOR ORDER BY）：中型库上
        // 实测 160ms/次，而该页每 2s 刷新一次，等于持续空转读整库。
        let sql = dimEventsSQL(limit: limit)
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(stmt) }

        var events: [AgentLogEvent] = []

        while sqlite3_step(stmt) == SQLITE_ROW {
            let createdAtStr = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let role = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let partsStr = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""
            let toolMetaStr = sqlite3_column_text(stmt, 3).map { String(cString: $0) } ?? ""

            let date = Self.isoFractional.date(from: createdAtStr) ?? Date()

            if !toolMetaStr.isEmpty, let metaData = toolMetaStr.data(using: .utf8),
               let meta = try? JSONSerialization.jsonObject(with: metaData) as? [String: Any] {
                let toolName = meta["toolName"] as? String ?? "tool"
                var kind: AgentLogEvent.EventKind = .toolCall
                if toolName.contains("bash") || toolName.contains("run") || toolName.contains("exec") {
                    kind = .command
                } else if toolName.contains("write") || toolName.contains("edit") {
                    kind = .fileEdit
                }
                let status = meta["status"] as? String ?? "done"
                events.append(AgentLogEvent(
                    timestamp: date,
                    kind: kind,
                    title: "\(toolName) (\(status))",
                    detail: partsStr.isEmpty ? nil : partsStr,
                    agentId: "dim"
                ))
            } else if !partsStr.isEmpty, let partsData = partsStr.data(using: .utf8),
                      let parts = try? JSONSerialization.jsonObject(with: partsData) as? [[String: Any]] {
                for part in parts {
                    let ptype = part["type"] as? String ?? ""
                    if ptype == "thinking", let think = part["thinking"] as? String {
                        let first = think.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? "思考中"
                        events.append(AgentLogEvent(
                            timestamp: date,
                            kind: .thinking,
                            title: first.count > 36 ? String(first.prefix(33)) + "..." : first,
                            detail: think,
                            agentId: "dim"
                        ))
                    } else if ptype == "tool_use", let name = part["name"] as? String {
                        events.append(AgentLogEvent(
                            timestamp: date,
                            kind: .toolCall,
                            title: "调用工具: \(name)",
                            detail: "\(part["input"] ?? "")",
                            agentId: "dim"
                        ))
                    } else if ptype == "text", let text = part["text"] as? String {
                        let first = text.components(separatedBy: .newlines).first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }) ?? (role == "user" ? "用户提问" : "回复")
                        events.append(AgentLogEvent(
                            timestamp: date,
                            kind: role == "user" ? .message : .info,
                            title: (role == "user" ? "提问: " : "回答: ") + (first.count > 32 ? String(first.prefix(29)) + "..." : first),
                            detail: text,
                            agentId: "dim"
                        ))
                    }
                }
            }
        }

        return events
    }

    // MARK: - 4. Claude Code 日志流

    public static func fetchClaudeEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dirs = ["\(home)/.claude/sessions", "\(home)/.claude/projects"]
        for dir in dirs {
            let url = URL(fileURLWithPath: dir)
            guard let file = LogTailReader.newestFile(in: url, maxAge: 86400 * 3) else { continue }
            // 事件时间：优先行内 JSON 的 timestamp 字段，缺失回落文件 mtime。
            // 此前用 Date()——派生 id 拼接时间戳，导致每次刷新全部 id 都变，
            // 稳定 id 的目的（展开态保持、增量刷新）对 claude 完全落空
            let fileMtime = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?
                .contentModificationDate ?? Date()
            let lines = readLastLines(from: file, maxLines: limit)
            var events: [AgentLogEvent] = []
            for line in lines.reversed() {
                var kind: AgentLogEvent.EventKind = .info
                if line.contains("\"Bash\"") || line.contains("command") { kind = .command }
                else if line.contains("\"Edit\"") || line.contains("write") { kind = .fileEdit }
                else if line.contains("tool") { kind = .toolCall }

                let stamp = line.data(using: .utf8)
                    .flatMap { try? JSONSerialization.jsonObject(with: $0) }
                    .flatMap { $0 as? [String: Any] }
                    .flatMap { $0["timestamp"] as? String }
                    .flatMap { Self.isoFractional.date(from: $0) } ?? fileMtime

                let clean = line.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                events.append(AgentLogEvent(
                    timestamp: stamp,
                    kind: kind,
                    title: clean.count > 40 ? String(clean.prefix(37)) + "..." : clean,
                    detail: line,
                    agentId: "claude"
                ))
            }
            if !events.isEmpty { return events }
        }
        return []
    }

    // MARK: - 5. OpenCode 日志流 (opencode.db)

    public static func fetchOpenCodeEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.local/share/opencode/opencode.db"
        guard let db = openReadonly(dbPath) else { return [] }
        defer { sqlite3_close(db) }

        // 同 dim：rowid 窗口限流 + 时间排序（原全表排序实测 25ms/次，每 2s 一次）
        let sql = openCodeEventsSQL(limit: limit)
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(stmt) }

        var events: [AgentLogEvent] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let dataStr = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let timeCreatedMs = sqlite3_column_int64(stmt, 1)
            let date = SafeNumber.date(fromEpochMillis: timeCreatedMs, source: "opencode.part.time") ?? Date()

            if let data = dataStr.data(using: .utf8),
               let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                let type = json["type"] as? String ?? "part"
                var kind: AgentLogEvent.EventKind = .info
                var title = type
                if type == "reasoning" {
                    kind = .thinking
                    title = "深度推理规划"
                } else if type == "tool-call" {
                    kind = .toolCall
                    let name = json["toolName"] as? String ?? "tool"
                    title = "调用: \(name)"
                } else if type == "text" {
                    kind = .message
                    title = "文本响应"
                }

                events.append(AgentLogEvent(
                    timestamp: date,
                    kind: kind,
                    title: title,
                    detail: dataStr,
                    agentId: "opencode"
                ))
            }
        }
        return events
    }

    // MARK: - 6. ZCode 日志流 (tasks-index.sqlite)

    public static func fetchZCodeEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.zcode/v2/tasks-index.sqlite"
        guard let db = openReadonly(dbPath) else { return [] }
        defer { sqlite3_close(db) }

        let sql = "SELECT title, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updated_at DESC LIMIT \(limit);"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(stmt) }

        var events: [AgentLogEvent] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let status = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let updatedMs = sqlite3_column_int64(stmt, 2)
            let date = SafeNumber.date(fromEpochMillis: updatedMs, source: "zcode.task.updated") ?? Date()

            events.append(AgentLogEvent(
                timestamp: date,
                kind: status == "completed" ? .info : .command,
                title: "任务 [\(status)]: \(title)",
                detail: title,
                agentId: "zcode"
            ))
        }
        return events
    }

    // MARK: - 7. WorkBuddy 日志流 (workbuddy.db)

    public static func fetchWorkBuddyEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.workbuddy/workbuddy.db"
        guard let db = openReadonly(dbPath) else { return [] }
        defer { sqlite3_close(db) }

        let sql = "SELECT title, status, mode, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT \(limit);"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(stmt) }

        var events: [AgentLogEvent] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? "Session"
            let status = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let mode = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? "agent"
            let updatedMs = sqlite3_column_int64(stmt, 3)
            let date = SafeNumber.date(fromEpochMillis: updatedMs, source: "workbuddy.session.updated") ?? Date()

            events.append(AgentLogEvent(
                timestamp: date,
                kind: status.lowercased() == "active" ? .command : .info,
                title: "[\(mode)] \(title) (\(status))",
                detail: "状态: \(status), 模式: \(mode)",
                agentId: "workbuddy"
            ))
        }
        return events
    }

    // MARK: - 8. Hermes 日志流 (state.db)

    public static func fetchHermesEvents(limit: Int = 20) -> [AgentLogEvent] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.hermes/state.db"
        guard let db = openReadonly(dbPath) else { return [] }
        defer { sqlite3_close(db) }

        let sql = "SELECT title, last_activity_description, started_at FROM sessions ORDER BY started_at DESC LIMIT \(limit);"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
        defer { sqlite3_finalize(stmt) }

        var events: [AgentLogEvent] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? "Hermes"
            let desc = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let startedAtStr = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""

            events.append(AgentLogEvent(
                timestamp: Date(),
                kind: .toolCall,
                title: title,
                detail: desc.isEmpty ? startedAtStr : desc,
                agentId: "hermes"
            ))
        }
        return events
    }

    // MARK: - 辅助函数

    /// 流水页只关心「最近 N 条」，故先用 rowid 窗口把扫描限制在表尾，再按时间排序。
    /// 前提假设：rowid 越大插入越晚，「最近的事件」必落在尾部窗口内——该表实测
    /// rowid 顺序与 createdAt 仅毫秒级交错，而本查询只用于显示最近 N 条，无影响。
    /// 窗口按 limit 放大 25 倍并保底 500 行：调用方传更大 limit 时也不会截断。
    static func recentWindow(limit: Int) -> Int { max(limit * 25, 500) }

    /// dim 流水查询语句（独立成函数便于测试在 fixture 库上验证窗口语义）
    static func dimEventsSQL(limit: Int) -> String {
        """
        SELECT createdAt, role, parts, toolMetadata
        FROM messages
        WHERE rowid > (SELECT max(rowid) - \(recentWindow(limit: limit)) FROM messages)
        ORDER BY createdAt DESC LIMIT \(limit);
        """
    }

    /// opencode 流水查询语句（与 dim 同策略）
    static func openCodeEventsSQL(limit: Int) -> String {
        """
        SELECT p.data, p.time_created
        FROM part p
        WHERE p.rowid > (SELECT max(rowid) - \(recentWindow(limit: limit)) FROM part)
        ORDER BY p.time_created DESC LIMIT \(limit);
        """
    }

    /// 只读打开数据库（统一带 FULLMUTEX，与其他数据源一致）。
    /// 失败时同样必须 close：`sqlite3_open_v2` 在返回错误码前就已分配 handle
    /// （rc=14 等场景下 handle 非 NULL），不 close 会每次泄漏约 1.5KB。
    /// 这些调用位于主线程采样路径（working 2s / idle 5s），泄漏会持续累积。
    /// internal（非 private）以便测试直接覆盖失败路径的内存契约。
    static func openReadonly(_ dbPath: String) -> OpaquePointer? {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK,
              let db else {
            if let db { sqlite3_close(db) }
            return nil
        }
        return db
    }

    static func readLastLines(from file: URL, maxLines: Int = 20) -> [String] {
        LogTailReader.read(from: file, maxLines: maxLines, maxBytes: 65536)
    }
}

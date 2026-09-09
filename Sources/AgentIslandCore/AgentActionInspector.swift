import Foundation
import SQLite3

// MARK: - 实时动作透传检查器

public enum AgentActionInspector {

    /// 提取智能体当前正在执行的具体动作/命令/上下文
    public static func inspectAction(pid: Int32?, profile: AgentProfile, sessionDirs: [String]) -> String? {
        // 1. 优先检查进程级正在执行的子命令（最实时、零延迟）
        if let pid = pid, pid > 1 {
            if let cmd = activeChildCommand(of: pid) {
                return "正在执行: \(cmd)"
            }
        }

        // 2. 根据各 Agent 的专用会话数据库/日志提取最近动作
        if profile.id == "dim" {
            if let dimAction = inspectDimAction() {
                return dimAction
            }
        } else if profile.id == "codex" {
            if let codexAction = inspectCodexAction() {
                return codexAction
            }
        } else if profile.id == "claude" {
            if let claudeAction = inspectClaudeAction(sessionDirs: sessionDirs) {
                return claudeAction
            }
        } else if profile.id == "zcode" {
            if let zcodeAction = inspectZCodeAction() {
                return zcodeAction
            }
        } else if profile.id == "antigravity" {
            if let agyAction = inspectAntigravityAction() {
                return agyAction
            }
        } else if profile.id == "workbuddy" {
            if let wbAction = inspectWorkBuddyAction() {
                return wbAction
            }
        } else if profile.id == "opencode" {
            if let ocAction = inspectOpenCodeAction() {
                return ocAction
            }
        } else if profile.id == "dsh" {
            if let dshAction = inspectDSHAction(pid: pid) {
                return dshAction
            }
        } else if profile.id == "hermes" {
            if let hermesAction = inspectHermesAction() {
                return hermesAction
            }
        }

        return nil
    }

    // MARK: - 1. 进程级子命令探测

    public static func activeChildCommand(of ppid: Int32) -> String? {
        let pipe = Pipe()
        let pgrep = Process()
        pgrep.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        pgrep.arguments = ["-P", "\(ppid)"]
        pgrep.standardOutput = pipe
        guard (try? pgrep.run()) != nil else { return nil }
        pgrep.waitUntilExit()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        guard let str = String(data: data, encoding: .utf8) else { return nil }
        let pids = str.components(separatedBy: .newlines).compactMap { Int32($0.trimmingCharacters(in: .whitespaces)) }
        guard let firstChild = pids.first else { return nil }

        let psPipe = Pipe()
        let ps = Process()
        ps.executableURL = URL(fileURLWithPath: "/bin/ps")
        ps.arguments = ["-o", "command=", "-p", "\(firstChild)"]
        ps.standardOutput = psPipe
        guard (try? ps.run()) != nil else { return nil }
        ps.waitUntilExit()
        let psData = psPipe.fileHandleForReading.readDataToEndOfFile()
        guard let raw = String(data: psData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty else {
            return nil
        }
        return cleanCommand(raw)
    }

    public static func cleanCommand(_ raw: String) -> String {
        var cmd = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if let cRange = cmd.range(of: " -c ") {
            cmd = String(cmd[cRange.upperBound...]).trimmingCharacters(in: CharacterSet(charactersIn: "\"' "))
        }
        let parts = cmd.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        if let exe = parts.first, exe.contains("/") {
            let exeName = URL(fileURLWithPath: String(exe)).lastPathComponent
            if parts.count > 1 {
                cmd = "\(exeName) \(parts[1])"
            } else {
                cmd = exeName
            }
        }
        if cmd.count > 36 {
            cmd = String(cmd.prefix(33)) + "..."
        }
        return cmd
    }

    // MARK: - 2. DimAgent 会话数据库探测

    public static func inspectDimAction(dbPath: String? = nil, now: Date = Date()) -> String? {
        let path: String
        if let custom = dbPath {
            path = custom
        } else {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            path = "\(home)/.dimcode/v2/dimcode.sqlite"
        }
        guard FileManager.default.fileExists(atPath: path) else { return nil }

        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        // 查询最新的 1 条消息（按 createdAt 降序）
        let sql = """
        SELECT role, toolMetadata, parts, unixepoch(updatedAt), unixepoch(createdAt), updatedAt, createdAt 
        FROM messages 
        ORDER BY createdAt DESC LIMIT 1;
        """
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let role = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let toolMeta = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let parts = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""
            let updatedEpoch = sqlite3_column_int64(stmt, 3)
            let createdEpoch = sqlite3_column_int64(stmt, 4)

            var effectiveEpoch = max(Double(updatedEpoch), Double(createdEpoch))
            if effectiveEpoch <= 0 {
                let updatedStr = sqlite3_column_text(stmt, 5).map { String(cString: $0) } ?? ""
                let createdStr = sqlite3_column_text(stmt, 6).map { String(cString: $0) } ?? ""
                let formatter = ISO8601DateFormatter()
                formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
                let d1 = formatter.date(from: updatedStr)?.timeIntervalSince1970 ?? 0
                let d2 = formatter.date(from: createdStr)?.timeIntervalSince1970 ?? 0
                effectiveEpoch = max(d1, d2)
            }

            let age = now.timeIntervalSince1970 - effectiveEpoch
            return parseDimMessage(role: role, toolMeta: toolMeta, parts: parts, age: age)
        }
        return nil
    }

    /// 解析 DimCode 消息并提取实时运行动作（纯函数，隔离测试）
    public static func parseDimMessage(role: String, toolMeta: String, parts: String, age: TimeInterval) -> String? {
        // 1. 超过活跃窗口（90s）判定为完全闲置，不透传任何动作
        guard age >= 0 && age <= 90 else { return nil }

        // 2. 若用户刚发送 Prompt（role = user），等待模型响应中
        if role == "user" {
            return "思考规划中"
        }

        // 3. 若工具刚返回结果（role = tool_result），等待模型消化结果并规划下一步
        if role == "tool_result" {
            return "思考规划中"
        }

        // 4. 若为 assistant 角色，深度校验在途状态与思考/工具调用
        if role == "assistant" {
            // 4.1 优先检查 toolMetadata 是否包含正在执行（running / pending）的工具
            if !toolMeta.isEmpty {
                if let (name, status) = extractRunningDimToolCall(from: toolMeta) {
                    if status != "completed" && status != "success" && status != "error" {
                        return mapDimToolAction(name)
                    }
                }
            }

            // 4.2 深度解析 parts JSON
            if let data = parts.data(using: .utf8),
               let array = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]],
               let lastPart = array.last {

                let hasEndTime = lastPart["endTime"] != nil
                let type = lastPart["type"] as? String ?? ""

                // 核心修复：若最后一个 part 包含 endTime，说明当前回复阶段已全部完成！绝不误报「思考规划中」
                if hasEndTime {
                    return nil
                }

                if type == "thinking" {
                    return "思考规划中"
                } else if type == "text" {
                    return "正在生成回复"
                } else if type == "tool_use" {
                    let toolName = lastPart["name"] as? String ?? ""
                    return mapDimToolAction(toolName)
                }
            }

            // 4.3 parts 降级兜底：只有包含 thinking 且明确不含 endTime 时才报思考
            if parts.contains("\"thinking\"") && !parts.contains("\"endTime\"") {
                return "思考规划中"
            }
            if parts.contains("\"text\"") && !parts.contains("\"endTime\"") {
                return "正在生成回复"
            }
        }

        return nil
    }

    /// 从 toolMetadata 中提取首个工具及其状态
    public static func extractRunningDimToolCall(from json: String) -> (name: String, status: String)? {
        guard let data = json.data(using: .utf8),
              let dict = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        if let toolCalls = dict["toolCalls"] as? [[String: Any]] {
            for call in toolCalls {
                let name = call["name"] as? String ?? ""
                let status = call["status"] as? String ?? ""
                if !name.isEmpty {
                    return (name, status)
                }
            }
        }
        if let name = dict["toolName"] as? String {
            let status = dict["status"] as? String ?? ""
            return (name, status)
        }
        return nil
    }

    /// 映射 DimAgent 工具名称为高质自然语言文案
    public static func mapDimToolAction(_ rawName: String) -> String {
        let name = rawName.trimmingCharacters(in: .whitespacesAndNewlines)
        if name.isEmpty { return "正在调用工具" }
        let lower = name.lowercased()
        if lower.contains("write") || lower.contains("edit") || lower.contains("todowrite") || lower.contains("patch") {
            return "正在编辑代码"
        }
        if lower.contains("bash") || lower.contains("exec") || lower.contains("run") || lower.contains("command") {
            return "正在执行终端命令"
        }
        if lower.contains("view") || lower.contains("read") || lower.contains("glob") || lower.contains("list") {
            return "正在阅读代码"
        }
        if lower.contains("search") || lower.contains("websearch") {
            return "正在搜索网络"
        }
        return "正在使用工具: \(name)"
    }

    // MARK: - 3. Codex 会话探测

    public static func inspectCodexAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let sessionsDir = URL(fileURLWithPath: "\(home)/.codex/sessions")
        guard let newestFile = findNewestFile(in: sessionsDir, maxAge: 300) else { return nil }
        guard let lastLine = readLastNonEmptyLine(from: newestFile) else { return nil }

        if lastLine.contains("\"exec\"") || lastLine.contains("\"command\"") {
            return "正在执行命令"
        }
        if lastLine.contains("\"edit\"") || lastLine.contains("\"write\"") {
            return "正在修改文件"
        }
        if lastLine.contains("\"thinking\"") || lastLine.contains("\"reasoning\"") {
            return "思考规划中"
        }
        return nil
    }

    // MARK: - 4. Claude Code 会话探测

    public static func inspectClaudeAction(sessionDirs: [String]) -> String? {
        for dir in sessionDirs {
            let url = URL(fileURLWithPath: dir)
            guard let newestFile = findNewestFile(in: url, maxAge: 300) else { continue }
            guard let lastLine = readLastNonEmptyLine(from: newestFile) else { continue }
            if lastLine.contains("\"Bash\"") || lastLine.contains("\"bash\"") {
                return "正在执行命令"
            }
            if lastLine.contains("\"Edit\"") || lastLine.contains("\"Write\"") {
                return "正在修改文件"
            }
            if lastLine.contains("\"thinking\"") {
                return "思考规划中"
            }
        }
        return nil
    }

    // MARK: - 辅助函数

    private static func extractToolName(from json: String) -> String? {
        guard let startRange = json.range(of: "\"toolName\":\"") else { return nil }
        let sub = json[startRange.upperBound...]
        guard let endQuote = sub.firstIndex(of: "\"") else { return nil }
        let name = String(sub[..<endQuote])
        return name.isEmpty ? nil : name
    }

    private static func findNewestFile(in dir: URL, maxAge: TimeInterval) -> URL? {
        let fm = FileManager.default
        guard let enumerator = fm.enumerator(at: dir, includingPropertiesForKeys: [.contentModificationDateKey, .isRegularFileKey], options: [.skipsHiddenFiles]) else {
            return nil
        }
        var newestURL: URL?
        var newestDate: Date = .distantPast
        let threshold = Date().addingTimeInterval(-maxAge)

        for case let fileURL as URL in enumerator {
            guard let vals = try? fileURL.resourceValues(forKeys: [.contentModificationDateKey, .isRegularFileKey]),
                  vals.isRegularFile == true,
                  let mtime = vals.contentModificationDate,
                  mtime >= threshold,
                  mtime > newestDate else { continue }
            newestDate = mtime
            newestURL = fileURL
        }
        return newestURL
    }

    private static func readLastLines(from file: URL, maxLines: Int = 10) -> [String] {
        guard let handle = try? FileHandle(forReadingFrom: file) else { return [] }
        defer { try? handle.close() }
        let fileSize = handle.seekToEndOfFile()
        let readLen = min(fileSize, 16384)
        guard readLen > 0 else { return [] }
        handle.seek(toFileOffset: fileSize - readLen)
        let data = handle.readDataToEndOfFile()
        guard let content = String(data: data, encoding: .utf8) else { return [] }
        let lines = content.components(separatedBy: .newlines)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        return Array(lines.suffix(maxLines))
    }

    private static func readLastNonEmptyLine(from file: URL) -> String? {
        readLastLines(from: file, maxLines: 1).last
    }

    // MARK: - 5. ZCode 任务数据库探测

    public static func inspectZCodeAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.zcode/v2/tasks-index.sqlite"
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let twoHourAgoMs = nowMs - (2 * 60 * 60 * 1000)
        let sql = "SELECT title, task_status, updated_at FROM tasks WHERE deleted = 0 AND updated_at >= \(twoHourAgoMs) ORDER BY updated_at DESC LIMIT 1;"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let status = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            if !title.isEmpty {
                let cleanTitle = title.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = cleanTitle.count > 26 ? String(cleanTitle.prefix(23)) + "..." : cleanTitle
                if status == "completed" {
                    return "任务已完成: \(display)"
                } else {
                    return "正在处理: \(display)"
                }
            }
        }
        return nil
    }

    // MARK: - 6. Antigravity 轨迹日志探测

    public static func inspectAntigravityAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let brainDir = URL(fileURLWithPath: "\(home)/.gemini/antigravity/brain")
        guard let subdirs = try? FileManager.default.contentsOfDirectory(at: brainDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles]) else {
            return nil
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
        guard let target = newestFile, Date().timeIntervalSince(newestTime) < 300 else {
            return nil
        }

        let lines = readLastLines(from: target, maxLines: 10)
        guard !lines.isEmpty else { return nil }

        for line in lines.reversed() {
            guard let data = line.data(using: .utf8),
                  let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                continue
            }

            if let toolCalls = obj["tool_calls"] as? [[String: Any]], let firstTool = toolCalls.first {
                var actionText: String?
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

                if let text = actionText, !text.isEmpty {
                    return cleanAntigravityAction(text)
                }
            }

            if let type = obj["type"] as? String {
                if type == "PLANNER_RESPONSE", let thinking = obj["thinking"] as? String, !thinking.isEmpty {
                    return "思考规划中"
                }
            }
        }

        return nil
    }

    /// 清洗与汉化 Antigravity 动作文案（移除冗余前缀、动词本土化、长度归一）
    public static func cleanAntigravityAction(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: CharacterSet(charactersIn: "\" \t\n\r"))
        let prefixMap: [(String, String)] = [
            ("Viewing ", "查看: "),
            ("Reading ", "读取: "),
            ("Listing ", "列出: "),
            ("Editing ", "编辑: "),
            ("Writing ", "写入: "),
            ("Searching ", "搜索: "),
            ("Running ", "运行: "),
            ("Building ", "构建: "),
            ("Analyzing ", "分析: "),
            ("Checking ", "检查: "),
            ("Finding ", "查找: "),
            ("Sending ", "发送: "),
            ("Pushing ", "推送: "),
            ("Updating ", "更新: "),
            ("Executing ", "执行: ")
        ]
        for (eng, chn) in prefixMap {
            if text.hasPrefix(eng) {
                text = chn + text.dropFirst(eng.count)
                break
            }
        }
        if !text.contains(":") && !text.contains("：") && !text.hasPrefix("正在") && !text.hasPrefix("思考") {
            text = "执行: \(text)"
        }
        if text.count > 42 {
            text = String(text.prefix(39)) + "..."
        }
        return text
    }

    // MARK: - 7. WorkBuddy 会话数据库探测

    public static func inspectWorkBuddyAction(now: Date = Date()) -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.workbuddy/workbuddy.db"
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        // 查询未软删除的最新活跃/最近会话
        let sql = "SELECT id, COALESCE(NULLIF(custom_title, ''), NULLIF(title, ''), ''), status, mode, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let sessionId = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let title = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let status = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""
            let updatedAtMs = sqlite3_column_int64(stmt, 4)
            let nowMs = Int64(now.timeIntervalSince1970 * 1000)

            if !title.isEmpty {
                let cleanTitle = title.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = cleanTitle.count > 26 ? String(cleanTitle.prefix(23)) + "..." : cleanTitle
                let timeAgoMs = nowMs - updatedAtMs

                // 核心修复：会话闲置超过 120 秒（2分钟）或非 active，判定为非在途状态，返回 nil（防止在后台挂起时误报「待机」使引擎误判为 working）
                guard timeAgoMs <= 120 * 1000 && status.lowercased() == "active" else {
                    return nil
                }

                // 尝试细粒度探测会话 jsonl 日志中的最新动作
                if let detailedAction = inspectWorkBuddySessionLog(sessionId: sessionId, home: home) {
                    return detailedAction
                }

                return "正在: \(display)"
            }
        }
        return nil
    }

    /// 探测 WorkBuddy 会话日志获取具体动作
    private static func inspectWorkBuddySessionLog(sessionId: String, home: String) -> String? {
        guard !sessionId.isEmpty else { return nil }
        let projectsDir = URL(fileURLWithPath: "\(home)/.workbuddy/projects")
        let fm = FileManager.default
        guard let subdirs = try? fm.contentsOfDirectory(at: projectsDir, includingPropertiesForKeys: nil) else {
            return nil
        }
        for sub in subdirs {
            let jsonlURL = sub.appendingPathComponent("\(sessionId).jsonl")
            if fm.fileExists(atPath: jsonlURL.path) {
                if let lastLine = readLastNonEmptyLine(from: jsonlURL) {
                    if lastLine.contains("\"status\":\"completed\"") && lastLine.contains("\"type\":\"message\"") {
                        // 该轮已完成回复
                        return nil
                    }
                    if lastLine.contains("\"type\":\"reasoning\"") {
                        return "思考规划中"
                    }
                    if lastLine.contains("\"type\":\"function_call\"") {
                        if lastLine.contains("\"Edit\"") || lastLine.contains("\"Write\"") {
                            return "正在编辑代码"
                        }
                        if lastLine.contains("\"WebSearch\"") || lastLine.contains("\"Search\"") {
                            return "正在搜索网络"
                        }
                        if lastLine.contains("\"Bash\"") || lastLine.contains("\"Run\"") {
                            return "正在执行终端命令"
                        }
                        if let name = extractToolName(from: lastLine) {
                            return "正在使用工具: \(name)"
                        }
                        return "正在调用工具"
                    }
                }
                break
            }
        }
        return nil
    }

    // MARK: - 8. OpenCode 会话数据库探测

    public static func inspectOpenCodeAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.local/share/opencode/opencode.db"
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        let sql = "SELECT s.title, p.data, s.time_updated FROM session s LEFT JOIN part p ON p.session_id = s.id ORDER BY s.time_updated DESC, p.time_updated DESC LIMIT 1;"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let partDataStr = sqlite3_column_text(stmt, 1).map { String(cString: $0) }
            let timeUpdatedMs = sqlite3_column_int64(stmt, 2)
            let nowMs = Int64(Date().timeIntervalSince1970 * 1000)

            if let dataStr = partDataStr, let data = dataStr.data(using: .utf8),
               let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                if let type = json["type"] as? String {
                    if type == "reasoning" {
                        return "思考规划中"
                    } else if type == "tool-call", let toolName = json["toolName"] as? String {
                        return "正在调用: \(toolName)"
                    }
                }
            }

            if !title.isEmpty && !title.hasPrefix("New session -") {
                let cleanTitle = title.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = cleanTitle.count > 26 ? String(cleanTitle.prefix(23)) + "..." : cleanTitle
                if nowMs - timeUpdatedMs < 60 * 60 * 1000 {
                    return "会话: \(display)"
                }
            }
        }
        return nil
    }

    // MARK: - 9. DSH (DeepSeek Harness) 运行模式探测

    public static func inspectDSHAction(pid: Int32?) -> String? {
        if let pid = pid, pid > 1 {
            let pipe = Pipe()
            let ps = Process()
            ps.executableURL = URL(fileURLWithPath: "/bin/ps")
            ps.arguments = ["-o", "command=", "-p", "\(pid)"]
            ps.standardOutput = pipe
            if (try? ps.run()) != nil {
                ps.waitUntilExit()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                if let raw = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) {
                    if raw.contains(" web") {
                        return "Web 协作服务运行中"
                    } else if raw.contains(" run ") || raw.contains(" exec ") {
                        return "执行任务中: " + cleanCommand(raw)
                    }
                }
            }
        }
        return nil
    }

    // MARK: - 10. Hermes 会话数据库探测

    public static func inspectHermesAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.hermes/state.db"
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        let sql = "SELECT COALESCE(NULLIF(title, ''), NULLIF(last_activity_description, ''), ''), started_at FROM sessions ORDER BY started_at DESC LIMIT 1;"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let desc = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            if !desc.isEmpty {
                let clean = desc.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = clean.count > 26 ? String(clean.prefix(23)) + "..." : clean
                return "任务: \(display)"
            }
        }
        return nil
    }
}

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

    public static func inspectDimAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.dimcode/v2/dimcode.sqlite"
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK else {
            return nil
        }
        defer { sqlite3_close(db) }

        let sql = """
        SELECT role, toolMetadata, parts FROM messages 
        WHERE createdAt >= datetime('now', '-5 minutes')
        ORDER BY createdAt DESC LIMIT 1;
        """
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let toolMeta = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            let parts = sqlite3_column_text(stmt, 2).map { String(cString: $0) } ?? ""

            if !toolMeta.isEmpty {
                if toolMeta.contains("\"write\"") { return "正在编辑代码" }
                if toolMeta.contains("\"bash\"") || toolMeta.contains("\"run\"") { return "正在执行终端命令" }
                if toolMeta.contains("\"view\"") || toolMeta.contains("\"read\"") { return "正在阅读代码" }
                if let name = extractToolName(from: toolMeta) {
                    return "正在使用工具: \(name)"
                }
            }
            if parts.contains("\"thinking\"") {
                return "思考规划中"
            }
        }
        return nil
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
        let tenMinAgoMs = nowMs - (10 * 60 * 1000)
        let sql = "SELECT title, task_status FROM tasks WHERE updated_at >= \(tenMinAgoMs) ORDER BY updated_at DESC LIMIT 1;"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let title = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
            let status = sqlite3_column_text(stmt, 1).map { String(cString: $0) } ?? ""
            if !title.isEmpty {
                let cleanTitle = title.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = cleanTitle.count > 30 ? String(cleanTitle.prefix(27)) + "..." : cleanTitle
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

                if var text = actionText {
                    text = text.trimmingCharacters(in: CharacterSet(charactersIn: "\" \t\n\r"))
                    if !text.isEmpty {
                        return "正在: \(text)"
                    }
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
}

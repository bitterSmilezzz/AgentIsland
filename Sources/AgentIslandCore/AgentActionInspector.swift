import Foundation
import SQLite3

// MARK: - 实时动作透传检查器

public enum AgentActionInspector {

    /// 提取智能体当前正在执行的具体动作/命令/上下文
    /// - Parameter snapshot: 调用方已有的进程快照（引擎采样时传入）。传入可让子进程查找
    ///   零成本完成；不传则退化为现场枚举（--probe 等低频路径）。
    public static func inspectAction(pid: Int32?, profile: AgentProfile, sessionDirs: [String],
                                     snapshot: ProcessSnapshot? = nil) -> String? {
        // 1. 优先检查进程级正在执行的子命令（最实时、零延迟）
        if let pid = pid, pid > 1 {
            if let cmd = activeChildCommand(of: pid, in: snapshot) {
                return "正在执行: \(cmd)"
            }
        }

        // 2. OpenCode 方言家族（OpenCode 本体，以及沿用同一套 session/message/part 表结构的
        //    fork，如小米 MiMo Code）：按档案声明的 schema 路由，而不是按 id 逐个列——
        //    再接一个 fork 只改注册表，动作探测不会静默变成「没有当前动作」
        if profile.sessionDatabase?.schema == .openCode {
            return inspectOpenCodeAction(agentId: profile.id)
        }

        // 3. 其余按各 Agent 的专用会话数据库/日志提取最近动作
        switch profile.id {
        case "dim":
            return inspectDimAction(agentId: "dim")
        case "codex":
            return inspectCodexAction()
        case "claude":
            return inspectClaudeAction(sessionDirs: sessionDirs)
        case "zcode":
            return inspectZCodeAction(agentId: "zcode")
        case "antigravity":
            return inspectAntigravityAction(dirs: sessionDirs)
        case "workbuddy":
            return inspectWorkBuddyAction(agentId: "workbuddy")
        case "workbuddy-ai":
            return inspectWorkBuddyAction(agentId: "workbuddy-ai")
        case "dsh":
            return inspectDSHAction(pid: pid, snapshot: snapshot)
        case "hermes":
            return inspectHermesAction()
        default:
            return nil
        }
    }

    /// 会话库位置统一取自注册表档案（路径字面量在注册表只写一份）。
    /// 此前同一批 `.sqlite`/`.db` 路径在动作探测、会话探测、日志流与 Token 统计里各写一遍，
    /// 档案改名或换目录时只会有一处生效，其余静默读不到数据。
    static func sessionDatabasePath(for agentId: String) -> String? {
        AgentRegistry.databasePath(for: agentId)
    }

    // MARK: - 1. 进程级子命令探测

    /// 查找指定进程当前在跑的子命令（用于透传「正在执行: xxx」）
    ///
    /// 性能关键路径：引擎每 2s 采样一次、每个 Agent 调一次，必须零 fork。
    /// 早期实现用 `pgrep -P` + `ps -o command=` 两次 `Process` 调用并 `waitUntilExit()`，
    /// 实测单次 67ms，17 个 Agent 一轮 762ms 全部落在主线程——直接造成周期性 CPU 尖峰
    /// 与界面卡顿。现改为纯内存计算：进程表与命令行都由调用方的快照/一次性 sysctl 提供。
    ///
    /// - Parameter snapshot: 已有的进程快照；为 nil 时现场枚举一次（低频路径）。
    public static func activeChildCommand(of ppid: Int32, in snapshot: ProcessSnapshot? = nil) -> String? {
        let table = snapshot ?? ProcessProvider().snapshot()
        let children = table.entries.filter { $0.ppid == ppid && $0.pid > 0 }
        guard !children.isEmpty else { return nil }

        for child in children {
            guard let raw = commandLine(of: child.pid) ?? (child.path.isEmpty ? nil : child.path),
                  !raw.isEmpty else { continue }
            if isInternalHelperProcess(command: raw) { continue }
            // 长驻服务（MCP server / language server / 桥接守护）常驻于 Agent 生命周期内，
            // 并不代表「有任务在途」。早期只看「存在非辅助子进程」会导致任何挂了 MCP 的
            // Agent 恒被判 working（实测 DimAgent 的 openviking-bridge/server.js 常驻）。
            if isLongLivedService(command: raw) { continue }
            return cleanCommand(raw)
        }
        return nil
    }

    /// 长驻服务判定：这些进程随 Agent 启动而常驻，不作为「正在执行的任务」信号。
    /// 判定刻意保守——只认明确的工具/服务特征，避免误杀用户自己的
    /// `node server.js`、`npm run watcher`、`cargo run --bin daemon` 等真实任务命令。
    public static func isLongLivedService(command: String) -> Bool {
        let lower = command.lowercased()
        // 1. MCP 服务（路径或参数里出现 mcp 且是服务入口）
        if lower.contains("mcp") && (lower.contains("server") || lower.contains("bridge")) {
            return true
        }
        // 2. 语言服务 / 索引守护（这些名字本身就是常驻服务，不会出现在用户任务里）
        let serviceMarkers = [
            "language-server", "languageserver", "language_server", "lsp-server",
            "typescript-language", "tsserver", "pyright", "gopls", "rust-analyzer",
            "--liftoff-only",          // codegraph / v8 常驻索引进程
            "codegraph",               // 索引服务目录
        ]
        if serviceMarkers.contains(where: { lower.contains($0) }) { return true }
        return false
    }

    /// 读取进程完整命令行（sysctl KERN_PROCARGS2，纯系统调用，无子进程）。
    /// 返回形如 `/path/to/exe arg1 arg2`（与 `ps -o command=` 一致）。
    /// 失败返回 nil，调用方回退到快照里的可执行路径。
    /// 命令行短 TTL 缓存。dsh 匹配对系统里每个 node 候选每拍各做一次 KERN_PROCARGS2
    /// sysctl（args+env 整块拷贝解析，2-8KB 分配，node 进程多的开发机一拍十几次），
    /// 命中后 inspectDSHAction 还会对同一 pid 再调一次；dsh web 模式启动后命令行不变，
    /// 10s TTL 内复用无损。pid 退出后由容量上限粗粒度清除（512 条 × 短字符串，有界）。
    private static let commandLineCacheLock = NSLock()
    private static var commandLineCache: [Int32: (command: String, at: Date)] = [:]
    static func cachedCommandLine(of pid: Int32, ttl: TimeInterval = 10) -> String? {
        commandLineCacheLock.lock()
        defer { commandLineCacheLock.unlock() }
        if let hit = commandLineCache[pid], Date().timeIntervalSince(hit.at) < ttl {
            return hit.command
        }
        guard let command = commandLine(of: pid) else {
            commandLineCache[pid] = nil   // pid 已消失：不缓存负结果，同时清残留
            return nil
        }
        if commandLineCache.count > 512 {
            commandLineCache.removeAll(keepingCapacity: true)
        }
        commandLineCache[pid] = (command, Date())
        return command
    }

    public static func commandLine(of pid: Int32) -> String? {
        var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
        var size = 0
        guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctl(&mib, 3, &buffer, &size, nil, 0) == 0 else { return nil }

        // 布局：[argc(int32)][exec_path\0][padding\0...][argv0\0][argv1\0]...[env0\0]...
        // 必须按 argc 截断：参数区之后紧跟环境变量区，仅靠空字节分隔会把
        // `PATH=...`、`USER=...` 误当成用户命令（实测曾因此漏掉所有真实子命令）。
        // argv[0] 必须保留：isInternalHelperProcess 的过滤标记（/frameworks/、
        // .app/contents/、helper.app、bare-modifier-monitor 等）都来自可执行路径，
        // 丢掉它会让 ChatGPT 的内部辅助进程被当成用户命令透传。
        let headerSize = MemoryLayout<Int32>.size
        guard size > headerSize else { return nil }
        let argc = buffer.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) }
        guard argc > 0, argc < 4096 else { return nil }
        let args = buffer.withUnsafeBytes { raw -> [String] in
            var offset = headerSize
            let bytes = raw.bindMemory(to: UInt8.self)
            // 跳过 exec_path 与对齐用的空字节
            while offset < size, bytes[offset] != 0 { offset += 1 }
            while offset < size, bytes[offset] == 0 { offset += 1 }
            var result: [String] = []
            var current: [UInt8] = []
            while offset < size, result.count < Int(argc) {
                let b = bytes[offset]
                if b == 0 {
                    result.append(String(decoding: current, as: UTF8.self))
                    current.removeAll(keepingCapacity: true)
                } else {
                    current.append(b)
                }
                offset += 1
            }
            return result
        }
        // 防御：缓冲区被内核截断时可能一个参数都读不到（第二次读取 size 不足），
        // 直接返回 nil 而非崩溃
        guard let exe = args.first, !exe.isEmpty else { return nil }
        let rest = args.dropFirst()
        return rest.isEmpty ? exe : "\(exe) \(rest.joined(separator: " "))"
    }

    /// 判定是否为桌面应用/Electron/Chromium 自身的内部辅助进程、渲染器或守护服务（非用户任务执行命令）
    public static func isInternalHelperProcess(command: String) -> Bool {
        let trimmed = command.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return true }
        let lower = trimmed.lowercased()

        // 1. 排除应用内部 Frameworks、Helpers、Plugins、Resources 路径及各类内部 helper app
        if lower.contains("/frameworks/") || lower.contains("/helpers/") ||
           lower.contains(".framework/") || lower.contains("helper.app") ||
           lower.contains(".app/contents/") {
            return true
        }

        // 2. 排除 Chromium / Electron / WebKit 核心类型标记（如渲染、GPU、实用程序进程等）
        if lower.contains("--type=utility") || lower.contains("--type=renderer") ||
           lower.contains("--type=gpu-process") || lower.contains("--type=crashpad-handler") ||
           lower.contains("--type=zygote") || lower.contains("--type=watcher") ||
           lower.contains("--user-data-dir=") {
            return true
        }

        // 3. 排除各类辅助保活/守护/心跳子命令
        if lower.contains("crashpad") || lower.contains("bare-modifier-monitor") ||
           lower.contains("app-server") || lower.contains("code_mode_host") ||
           lower.contains("code-mode-host") ||
           lower.contains("webprocess") || lower.contains("networkservice") ||
           lower.contains("codex (service)") || lower.contains("codex (renderer)") {
            return true
        }

        return false
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

    public static func inspectDimAction(agentId: String = "dim", now: Date = Date()) -> String? {
        // 库位置取自注册表档案；库缺失 / open 失败 → withDB 返回 nil
        // （原 fileExists 快路径由 ReadonlyDB.connection 的 inode 检查覆盖）
        guard let path = sessionDatabasePath(for: agentId) else { return nil }
        return withDB(path) { db in
            // 查询最新的 1 条消息。
            // 性能关键：不能 `ORDER BY createdAt DESC LIMIT 1`——messages 表没有 createdAt
            // 索引（现有索引均以 sessionId 打头），实测数万行 / 数百 MB 会退化成
            // 「全表扫描 + 临时 B 树排序」，单次约 220ms，而本函数每 2 秒在主线程调用一次。
            // 用 max(rowid) 定位最新行（rowid 是隐含主键，O(1)），再按主键精确取该行。
            let sql = """
            SELECT role, toolMetadata, parts, unixepoch(updatedAt), unixepoch(createdAt), updatedAt, createdAt 
            FROM messages 
            WHERE rowid = (SELECT max(rowid) FROM messages);
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
        guard let newestFile = LogTailReader.newestFile(in: sessionsDir, maxAge: 300) else { return nil }
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
            guard let newestFile = LogTailReader.newestFile(in: url, maxAge: 300) else { continue }
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

    /// ReadonlyDB.withConnection 的本文件内薄封装：body 收窄为 `-> T?`，5 个探测闭包
    /// 得以保留原 `return nil / return "..."` 语义。直接调 withConnection 时，外层
    /// String? 上下文会让求解器把 T 钉成 String（`T? == String?` 取恒等解且不回溯），
    /// 闭包内 `return nil` 无法通过类型检查（Swift 6.3 实测）；此处统一在边界把
    /// `T??` 展平回 `T?`（`?? nil` 恒等展平，库缺失/open 失败的 nil 原样透传）。
    /// open 失败关句柄的契约在 ReadonlyDB.connection 内统一执行。
    private static func withDB<T>(_ path: String, _ body: (OpaquePointer) -> T?) -> T? {
        ReadonlyDB.withConnection(path, body) ?? nil
    }

    /// 标题截断（列表行展示）：超限截断补省略号。4 个探测源共用，避免口径漂移
    private static func clipTitle(_ title: String, limit: Int = 26) -> String {
        title.count > limit ? String(title.prefix(limit - 3)) + "..." : title
    }

    static func readLastLines(from file: URL, maxLines: Int = 10) -> [String] {
        LogTailReader.read(from: file, maxLines: maxLines, maxBytes: 16384)
    }

    private static func readLastNonEmptyLine(from file: URL) -> String? {
        readLastLines(from: file, maxLines: 1).last
    }

    // MARK: - 5. ZCode 任务数据库探测

    public static func inspectZCodeAction(agentId: String = "zcode") -> String? {
        guard let dbPath = sessionDatabasePath(for: agentId) else { return nil }
        return withDB(dbPath) { db in
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
                    let display = clipTitle(cleanTitle)
                    if status == "completed" {
                        return "任务已完成: \(display)"
                    } else {
                        return "正在处理: \(display)"
                    }
                }
            }
            return nil
        }
    }

    // MARK: - 6. Antigravity 轨迹日志探测

    public static func inspectAntigravityAction(dirs: [String]) -> String? {
        // 优先从 Antigravity 原生会话探测中读取当前正在活跃执行的动作；
        // 处于已完成、待审批或待机状态时返回 nil，绝不残留回溯上一轮旧命令。
        if let signal = AgentSessionInspector.probeAntigravitySession(dirs: dirs).signal,
           case let .active(_, actionText) = signal,
           let actionText, !actionText.isEmpty {
            return actionText
        }
        return nil
    }

    /// 清洗与汉化 Antigravity 动作文案（移除冗余前缀、动词本土化、长度归一）
    public static func cleanAntigravityAction(_ raw: String) -> String {
        var text = raw.trimmingCharacters(in: CharacterSet(charactersIn: "\" \t\n\r"))
        let directToolMap: [String: String] = [
            "run_command": "执行终端命令",
            "view_file": "查看文件",
            "replace_file_content": "编辑文件代码",
            "write_to_file": "写入文件",
            "grep_search": "搜索代码正则",
            "find_by_name": "查找文件",
            "list_dir": "浏览目录",
            "search_web": "联网搜索",
            "read_url_content": "读取网页内容",
            "ask_question": "等待用户确认",
            "manage_task": "管理后台任务",
            "schedule": "调度定时任务",
            "invoke_subagent": "调用子智能体",
            "manage_subagents": "管理子智能体",
            "define_subagent": "定义子智能体",
            "generate_image": "生成图像素材"
        ]
        if let direct = directToolMap[text] {
            return direct
        }

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

    public static func inspectWorkBuddyAction(agentId: String = "workbuddy", now: Date = Date()) -> String? {
        guard let dbPath = sessionDatabasePath(for: agentId) else { return nil }
        // 会话 jsonl 落在库的同级目录，由注册的库路径推出，避免再写一份 home 字面量
        let dataDir = (dbPath as NSString).deletingLastPathComponent
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return withDB(dbPath) { db in
            // 查询未软删除的最新活跃/最近会话
            //
            // 不要用 max(rowid) 替掉这里的 ORDER BY updated_at DESC（本文件 dim 探测器
            // 235 行那一招在这里**不成立**，本机真实库可反驳）：sessions 行会被就地 UPDATE，
            // rowid 顺序不代表 updated_at 顺序。实测 ~/.workbuddy/workbuddy.db（5 行）:
            //   ORDER BY updated_at DESC LIMIT 1 → rowid 1 / 1789832472978
            //   WHERE rowid = max(rowid)         → rowid 5 / 1788920103273（早 10.6 天）
            // 取错会话即读错 status 与 updated_at，本函数下游的「闲置 >120s 即判非在途」
            // 门限会直接失真。代价侧也没有可省的：EXPLAIN QUERY PLAN 是 SCAN sessions +
            // USE TEMP B-TREE FOR ORDER BY，LIMIT 1 让临时 B 树只存一行，实测 5 行库
            // 0.16µs——瓶颈是全表扫本身，而 updated_at 无索引是第三方应用的库结构决定，
            // 只读连接加不了索引。
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
                    let display = clipTitle(cleanTitle)
                    let timeAgoMs = nowMs - updatedAtMs

                    // 核心修复：会话闲置超过 120 秒（2分钟）或非 active，判定为非在途状态，返回 nil（防止在后台挂起时误报「待机」使引擎误判为 working）
                    guard timeAgoMs <= 120 * 1000 && status.lowercased() == "active" else {
                        return nil
                    }

                    // 尝试细粒度探测会话 jsonl 日志中的最新动作
                    if let detailedAction = inspectWorkBuddySessionLog(sessionId: sessionId, home: home, dataDir: dataDir) {
                        return detailedAction
                    }

                    return "正在: \(display)"
                }
            }
            return nil
        }
    }

    /// 探测 WorkBuddy 会话日志获取具体动作
    private static func inspectWorkBuddySessionLog(sessionId: String, home: String, dataDir: String) -> String? {
        guard !sessionId.isEmpty else { return nil }
        let projectsDir = URL(fileURLWithPath: "\(dataDir)/projects")
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

    public static func inspectOpenCodeAction(agentId: String = "opencode") -> String? {
        guard let dbPath = sessionDatabasePath(for: agentId) else { return nil }
        return withDB(dbPath) { db in
            // 性能关键：原实现 `LEFT JOIN part ... ORDER BY s.time_updated DESC, p.time_updated
            // DESC LIMIT 1` 的相关子查询对 part 按 time_updated 排序（无索引 → 临时 B 树，
            // 随活跃会话 part 数线性退化，万级 part 实测量级 2-5ms，每 2s 一次全在主线程）。
            // 两步化：① session 表百行级，ORDER BY time_updated 取最新会话可接受；
            // ② part 用 rowid 定位该会话最新行（插入序 O(1)；rowid 与 time_created 的
            // 毫秒级交错对「最新一条动作」无影响，与 AgentLogStreamer 流水窗口同一取舍）。
            let sessionSQL = "SELECT id, title, time_updated FROM session ORDER BY time_updated DESC LIMIT 1;"
            var sessionId: String?
            var sessionTitle: String?
            var timeUpdatedMs: Int64 = 0
            var sessionStmt: OpaquePointer?
            if sqlite3_prepare_v2(db, sessionSQL, -1, &sessionStmt, nil) == SQLITE_OK {
                defer { sqlite3_finalize(sessionStmt) }
                if sqlite3_step(sessionStmt) == SQLITE_ROW {
                    sessionId = sqlite3_column_text(sessionStmt, 0).map { String(cString: $0) }
                    sessionTitle = sqlite3_column_text(sessionStmt, 1).map { String(cString: $0) }
                    timeUpdatedMs = sqlite3_column_int64(sessionStmt, 2)
                }
            }
            guard let sessionId else { return nil }

            let partSQL = "SELECT data FROM part WHERE session_id = ? ORDER BY rowid DESC LIMIT 1;"
            var partStmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, partSQL, -1, &partStmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(partStmt) }
            // TRANSIENT 而非 nil(=SQLITE_STATIC)：sessionId 是 Swift String，桥出的 C 缓冲区
            // 只在本行有效，而 step 在下一行
            sqlite3_bind_text(partStmt, 1, sessionId, -1, ReadonlyDB.transientDestructor)
            var partDataStr: String?
            if sqlite3_step(partStmt) == SQLITE_ROW {
                partDataStr = sqlite3_column_text(partStmt, 0).map { String(cString: $0) }
            }

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

            if let title = sessionTitle, !title.isEmpty && !title.hasPrefix("New session -") {
                let cleanTitle = title.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                let display = clipTitle(cleanTitle)
                if nowMs - timeUpdatedMs < 60 * 60 * 1000 {
                    return "会话: \(display)"
                }
            }
            return nil
        }
    }

    // MARK: - 9. DSH (DeepSeek Harness) 运行模式探测

    public static func inspectDSHAction(pid: Int32?, snapshot: ProcessSnapshot? = nil, sessionDirs: [String] = []) -> String? {
        // 1. 优先从 DSH 官方投影缓存中提取正在运行的活跃任务与步骤
        if let projection = AgentSessionInspector.dshProjectionDir(in: sessionDirs),
           let signal = AgentSessionInspector.inspectDSHSession(baseDir: projection),
           case let .active(_, actionText) = signal,
           let actionText, !actionText.isEmpty {
            return actionText
        }

        // 2. 进程命令行辅助回退
        if let pid = pid, pid > 1 {
            // 用 libproc 直读命令行，避免 fork /bin/ps（单次 ~67ms 的主线程开销）；
            // 走 TTL 缓存——匹配器命中 dsh 时刚为本 pid 探测过，二次 sysctl 纯浪费
            let raw = cachedCommandLine(of: pid)
                ?? snapshot?.entries.first { $0.pid == pid }?.path
            if let raw, !raw.isEmpty {
                // 用词边界匹配：commandLine 以可执行路径开头（如 "/path/dsh run foo"），
                // 直接 contains(" run ") 在子命令紧跟路径时会漏判
                let tokens = raw.split(separator: " ").map(String.init)
                if tokens.contains("web") {
                    return "Web 协作服务运行中"
                } else if tokens.contains("run") || tokens.contains("exec") {
                    return "执行任务中: " + cleanCommand(raw)
                }
            }
        }
        return nil
    }

    // MARK: - 10. Hermes 会话数据库探测

    public static func inspectHermesAction() -> String? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let dbPath = "\(home)/.hermes/state.db"
        return withDB(dbPath) { db in
            let sql = "SELECT COALESCE(NULLIF(title, ''), NULLIF(last_activity_description, ''), ''), started_at FROM sessions ORDER BY started_at DESC LIMIT 1;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }

            if sqlite3_step(stmt) == SQLITE_ROW {
                let desc = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
                if !desc.isEmpty {
                    let clean = desc.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
                    let display = clipTitle(clean)
                    return "任务: \(display)"
                }
            }
            return nil
        }
    }
}

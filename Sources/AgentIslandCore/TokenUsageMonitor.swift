import Foundation
import SQLite3

// MARK: - Token 用量统计（只读双 SQLite 数据源）
//
// 数据源参考 vibe-usage 的采集思路（本机只读、不碰凭证）：
// 1. DimAgent: ~/.dimcode/v2/dimcode.sqlite → usage_ledger（token 全，cost 全 NULL）
// 2. OpenCode: ~/.local/share/opencode/opencode.db → message（token + cost）
// 24h 口径 = 时间窗口内的记录之和；累计 = 全表之和。
// 60s 后台轮询足够（token 用量无需秒级实时）。
// OpenCode 不用 tokens.total（它含 cache.read，多轮会话重复计费虚高），
// 统一用 input+output+reasoning 净消耗口径，与 DimAgent 的 prompt+completion 对齐。

public struct TokenUsage: Equatable {
    public var tokens24h: Int = 0
    public var tokensTotal: Int = 0
    public var cost24h: Double = 0
    public var costTotal: Double = 0

    public var isEmpty: Bool { tokensTotal == 0 && costTotal == 0 }

    public init() {}
    public init(tokens24h: Int, tokensTotal: Int, cost24h: Double, costTotal: Double) {
        self.tokens24h = tokens24h
        self.tokensTotal = tokensTotal
        self.cost24h = cost24h
        self.costTotal = costTotal
    }

    public static func + (lhs: TokenUsage, rhs: TokenUsage) -> TokenUsage {
        TokenUsage(tokens24h: lhs.tokens24h + rhs.tokens24h,
                   tokensTotal: lhs.tokensTotal + rhs.tokensTotal,
                   cost24h: lhs.cost24h + rhs.cost24h,
                   costTotal: lhs.costTotal + rhs.costTotal)
    }

    /// 1.23M / 45.6k / 890 式紧凑格式
    public static func compact(_ n: Int) -> String {
        switch n {
        case 1_000_000_000...: return String(format: "%.2fB", Double(n) / 1_000_000_000)
        case 1_000_000...: return String(format: "%.2fM", Double(n) / 1_000_000)
        case 10_000...: return String(format: "%.1fk", Double(n) / 1_000)
        default: return "\(n)"
        }
    }

    /// $1.23 / $0.45 / <$0.01；0 或负返回空串
    public static func cost(_ c: Double) -> String {
        guard c > 0 else { return "" }
        if c < 0.01 { return "<$0.01" }
        return String(format: "$%.2f", c)
    }
}

// MARK: - 详情页数据行

public struct ModelUsage: Identifiable, Equatable {
    public let modelId: String
    public let messages: Int
    public let tokens: Int
    public let cost: Double
    public var id: String { modelId }
}

public struct SessionUsage: Identifiable, Equatable {
    public let sessionId: String
    public let directory: String?   // Finder 跳转目标（不存在为 nil）
    public let messages: Int
    public let tokens: Int
    public let cost: Double
    public let lastTime: Date?
    public var id: String { sessionId }
}

// MARK: - SQL 字符串转义（单引号翻倍，防会话/模型名带引号炸查询）

extension String {
    var escaped: String { replacingOccurrences(of: "'", with: "''") }
}

/// @unchecked Sendable：全部可变状态由 NSLock + dbQueue 串行队列保护，可跨线程调用
public final class TokenUsageMonitor: @unchecked Sendable {
    private let lock = NSLock()
    private var _usage: [String: TokenUsage] = [:]   // agentId → 用量
    private var _grandTotal = TokenUsage()
    /// 增量缓存（阿证低）：文件未变时的上次结果副本，避免全表聚合重算
    private var lastUsage: [String: TokenUsage] = [:]
    private var lastDimStamp = ""
    private var lastOpenCodeStamp = ""
    private var timer: Timer?
    /// 刷新完成后的主线程回调（引擎用它触发重采样，让卡片高度/徽标及时跟上）
    public var onRefresh: (@MainActor () -> Void)?

    /// 各 agent 的用量快照（引擎采样时取走）
    public var usage: [String: TokenUsage] {
        lock.lock(); defer { lock.unlock() }
        return _usage
    }
    /// 所有数据源总和（汇总栏）
    public var grandTotal: TokenUsage {
        lock.lock(); defer { lock.unlock() }
        return _grandTotal
    }

    private let dimAgentDB = NSString(string: "~/.dimcode/v2/dimcode.sqlite").expandingTildeInPath
    private let openCodeDB = NSString(string: "~/.local/share/opencode/opencode.db").expandingTildeInPath

    /// SQLite 只读连接缓存（复用避免每查询 open/close）；查询统一走串行队列保证连接线程安全
    private var dbConnections: [String: OpaquePointer] = [:]
    /// 连接打开时的文件 inode（外部替换主文件后据此失效缓存连接，阿证中）
    private var dbInodes: [String: UInt64] = [:]
    private let dbQueue = DispatchQueue(label: "com.agentisland.tokenusage.db")

    public init() {}

    public func start(interval: TimeInterval = 60.0) {
        guard timer == nil else { return }
        refreshAsync()   // 先刷一次，主卡汇总栏启动即有数据
        let t = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
            self?.refreshAsync()
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    public func stop() {
        timer?.invalidate()
        timer = nil
        closeConnectionsAsync()
    }

    /// 暂停后台轮询（面板 docked 时无展示需求，省掉整条查询链路与 onRefresh 重采样）
    /// 注意：不关闭连接（阿证中1：每次 expanded 重开 + 全表重扫产生 41.6% 尖峰；
    /// 只读连接可长驻复用，SQLite 对同库持续写入安全）；stop() 才彻底清理
    public func pause() {
        guard timer != nil else { return }
        timer?.invalidate()
        timer = nil
    }

    /// 恢复后台轮询（面板 expanded 时），并立即刷新一次保证 UI 最新
    public func resume(interval: TimeInterval = 60.0) {
        guard timer == nil else { return }
        refreshAsync()
        let t = Timer(timeInterval: interval, repeats: true) { [weak self] _ in
            self?.refreshAsync()
        }
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    /// 异步关闭连接：不阻塞主线程（若详情页大查询在飞，等其自然结束；dbQueue 串行保证安全）
    private func closeConnectionsAsync() {
        dbQueue.async { [weak self] in
            guard let self else { return }
            for (_, db) in self.dbConnections {
                sqlite3_close(db)
            }
            self.dbConnections.removeAll()
            self.dbInodes.removeAll()
        }
    }

    public func refreshAsync() {
        DispatchQueue.global(qos: .utility).async { [weak self] in
            self?.refresh()
        }
    }

    public func refresh() {
        // 增量缓存（阿证低）：两库文件 mtime/inode 未变则跳过全表聚合重算。
        // 只读连接 + stat 微秒级；token 表行数到十万级时全表 SUM 可达数百 ms，
        // 文件未变时无谓重算应避免
        let dimStamp = fileStamp(dimAgentDB)
        let openCodeStamp = fileStamp(openCodeDB)
        if dimStamp == lastDimStamp, openCodeStamp == lastOpenCodeStamp,
           !lastUsage.isEmpty {
            return
        }

        let cutoffISO = Self.iso24hAgo()
        let cutoffMs = Int64(Date().timeIntervalSince1970 - 86_400) * 1000

        var usage: [String: TokenUsage] = [:]
        usage["dim"] = queryDimAgent(cutoffISO: cutoffISO)
        usage["opencode"] = queryOpenCode(cutoffMs: cutoffMs)

        let total = usage.values.reduce(TokenUsage(), +)
        lock.lock()
        _usage = usage.filter { !$0.value.isEmpty }
        _grandTotal = total
        lastUsage = _usage
        lock.unlock()
        lastDimStamp = dimStamp
        lastOpenCodeStamp = openCodeStamp
        if let onRefresh {
            Task { @MainActor in onRefresh() }
        }
    }

    /// 文件变更戳（inode+mtime+size 组合；任一变化即视为被替换/写入）
    private func fileStamp(_ path: String) -> String {
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: path) else { return "" }
        let inode = (attrs[.systemFileNumber] as? UInt64) ?? 0
        let mtime = (attrs[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        let size = (attrs[.size] as? UInt64) ?? 0
        return "\(inode)-\(Int(mtime))-\(size)"
    }

    // MARK: - 详情页数据（按需查询，后台线程执行，主线程回调）

    /// 按模型拆分（累计口径，按 token 降序）
    public func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            var rows: [ModelUsage] = []
            switch agentId {
            case "dim":
                let sql = """
                SELECT modelId, COUNT(*),
                       SUM(json_extract(usage,'$.promptTokens'))+SUM(json_extract(usage,'$.completionTokens')),
                       COALESCE(SUM(cost),0)
                FROM usage_ledger GROUP BY modelId ORDER BY 3 DESC
                """
                rows = rawRows(sql, dbPath: dimAgentDB, cols: 4).map {
                    ModelUsage(modelId: $0[0], messages: Int($0[1]) ?? 0,
                               tokens: Int($0[2]) ?? 0, cost: Double($0[3]) ?? 0)
                }
            case "opencode":
                let sql = """
                SELECT json_extract(data,'$.modelID'), COUNT(*),
                       SUM(json_extract(data,'$.tokens.input'))+SUM(json_extract(data,'$.tokens.output'))+SUM(json_extract(data,'$.tokens.reasoning')),
                       COALESCE(SUM(json_extract(data,'$.cost')),0)
                FROM message WHERE json_extract(data,'$.role')='assistant'
                GROUP BY 1 ORDER BY 3 DESC
                """
                rows = rawRows(sql, dbPath: openCodeDB, cols: 4).map {
                    ModelUsage(modelId: $0[0], messages: Int($0[1]) ?? 0,
                               tokens: Int($0[2]) ?? 0, cost: Double($0[3]) ?? 0)
                }
            default:
                rows = []
            }
            Task { @MainActor in completion(rows) }
        }
    }

    /// 某模型下的会话列表（按最后活动降序）
    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            var rows: [SessionUsage] = []
            switch agentId {
            case "dim":
                let dirPrefix = NSString(string: "~/.dimcode/v2/data/sessions").expandingTildeInPath
                let sql = """
                SELECT sessionId, COUNT(*),
                       SUM(json_extract(usage,'$.promptTokens'))+SUM(json_extract(usage,'$.completionTokens')),
                       COALESCE(SUM(cost),0), MAX(createdAt)
                FROM usage_ledger WHERE modelId = '\(modelId.escaped)'
                GROUP BY sessionId ORDER BY 5 DESC LIMIT 200
                """
                rows = rawRows(sql, dbPath: dimAgentDB, cols: 5).map { r in
                    let dir = dirPrefix + "/" + r[0]
                    return SessionUsage(sessionId: r[0],
                                        directory: FileManager.default.fileExists(atPath: dir) ? dir : nil,
                                        messages: Int(r[1]) ?? 0, tokens: Int(r[2]) ?? 0,
                                        cost: Double(r[3]) ?? 0,
                                        lastTime: Self.parseISO(r[4]))
                }
            case "opencode":
                let sql = """
                SELECT m.session_id, COUNT(*),
                       SUM(json_extract(m.data,'$.tokens.input'))+SUM(json_extract(m.data,'$.tokens.output'))+SUM(json_extract(m.data,'$.tokens.reasoning')),
                       COALESCE(SUM(json_extract(m.data,'$.cost')),0),
                       MAX(m.time_created), s.directory
                FROM message m LEFT JOIN session s ON s.id = m.session_id
                WHERE json_extract(m.data,'$.role')='assistant' AND json_extract(m.data,'$.modelID')='\(modelId.escaped)'
                GROUP BY m.session_id ORDER BY 5 DESC LIMIT 200
                """
                rows = rawRows(sql, dbPath: openCodeDB, cols: 6).map { r in
                    // 与 dim 对齐：目录已删除则置 nil（点击不再显示文件夹图标）
                    let rawDir = r[5]
                    let dir = (!rawDir.isEmpty && FileManager.default.fileExists(atPath: rawDir)) ? rawDir : nil
                    return SessionUsage(sessionId: r[0],
                                        directory: dir,
                                        messages: Int(r[1]) ?? 0, tokens: Int(r[2]) ?? 0,
                                        cost: Double(r[3]) ?? 0,
                                        lastTime: Double(r[4]).map { Date(timeIntervalSince1970: $0 / 1000) })
                }
            default:
                rows = []
            }
            Task { @MainActor in completion(rows) }
        }
    }

    // MARK: - 24h/累计 汇总查询

    private func queryDimAgent(cutoffISO: String) -> TokenUsage {
        // createdAt 是 ISO8601 UTC 字符串（同格式字符串比较即时间比较）；cost 全表 SUM（NULL 记 0）
        let sql24h = """
        SELECT COALESCE(SUM(json_extract(usage,'$.promptTokens')),0)
             + COALESCE(SUM(json_extract(usage,'$.completionTokens')),0),
               COALESCE(SUM(cost),0)
        FROM usage_ledger WHERE createdAt >= '\(cutoffISO)'
        """
        let sqlTotal = """
        SELECT COALESCE(SUM(json_extract(usage,'$.promptTokens')),0)
             + COALESCE(SUM(json_extract(usage,'$.completionTokens')),0),
               COALESCE(SUM(cost),0)
        FROM usage_ledger
        """
        var u = TokenUsage()
        if let (t24, c24) = scalarSum(sql24h, dbPath: dimAgentDB),
           let (tAll, cAll) = scalarSum(sqlTotal, dbPath: dimAgentDB) {
            u = TokenUsage(tokens24h: t24, tokensTotal: tAll, cost24h: c24, costTotal: cAll)
        }
        return u
    }

    private func queryOpenCode(cutoffMs: Int64) -> TokenUsage {
        let tokensExpr = """
        COALESCE(SUM(json_extract(data,'$.tokens.input')),0)
        + COALESCE(SUM(json_extract(data,'$.tokens.output')),0)
        + COALESCE(SUM(json_extract(data,'$.tokens.reasoning')),0)
        """
        let roleFilter = "json_extract(data,'$.role')='assistant'"
        let sql24h = "SELECT \(tokensExpr), COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(roleFilter) AND time_created >= \(cutoffMs)"
        let sqlTotal = "SELECT \(tokensExpr), COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(roleFilter)"
        var u = TokenUsage()
        if let (t24, c24) = scalarSum(sql24h, dbPath: openCodeDB),
           let (tAll, cAll) = scalarSum(sqlTotal, dbPath: openCodeDB) {
            u = TokenUsage(tokens24h: t24, tokensTotal: tAll, cost24h: c24, costTotal: cAll)
        }
        return u
    }

    // MARK: - SQLite 底层

    /// 两列标量查询：(token, cost)；查询失败返回 nil
    private func scalarSum(_ sql: String, dbPath: String) -> (Int, Double)? {
        guard let row = rawRows(sql, dbPath: dbPath, cols: 2).first else { return nil }
        return (Int(row[0]) ?? 0, Double(row[1]) ?? 0)
    }

    /// 通用查询：全部列转字符串返回（数值/文本统一处理，空结果返回 []）
    /// 线程安全：所有查询在 dbQueue 串行执行，连接按路径缓存复用（只读，应用生命周期内不关闭）
    private func rawRows(_ sql: String, dbPath: String, cols: Int) -> [[String]] {
        dbQueue.sync {
            let fm = FileManager.default
            guard fm.fileExists(atPath: dbPath) else {
                debugPrint("TokenUsage: db not found \(dbPath)")
                return []
            }
            // inode 失效检测（阿证中）：外部工具原子替换/重建主文件（VACUUM 后 rename、
            // 备份恢复、删后重建）后，缓存只读句柄永久指向旧 inode，统计静默陈旧。
            // 命中缓存时对比 systemFileNumber，不一致则关闭重开
            let db: OpaquePointer
            if let cached = dbConnections[dbPath] {
                if isCurrentInode(dbPath, openedInode: dbInodes[dbPath]) {
                    db = cached
                } else {
                    sqlite3_close(cached)
                    dbConnections[dbPath] = nil
                    dbInodes[dbPath] = nil
                    guard let reopened = openReadonly(dbPath) else { return [] }
                    db = reopened
                }
            } else {
                guard let handle = openReadonly(dbPath) else { return [] }
                db = handle
            }

            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
                debugPrint("TokenUsage: prepare failed: \(String(cString: sqlite3_errmsg(db)))")
                return []
            }
            defer { sqlite3_finalize(stmt) }

            var rows: [[String]] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                var row: [String] = []
                row.reserveCapacity(cols)
                for i in 0..<cols {
                    if let c = sqlite3_column_text(stmt, Int32(i)) {
                        row.append(String(cString: c))
                    } else {
                        row.append("")
                    }
                }
                rows.append(row)
            }
            // 中途出错（SQLITE_ERROR/BUSY）不应把部分行当完整结果（阿证低）
            if sqlite3_errcode(db) != SQLITE_OK && sqlite3_errcode(db) != SQLITE_DONE && sqlite3_errcode(db) != SQLITE_ROW {
                debugPrint("TokenUsage: step error \(String(cString: sqlite3_errmsg(db)))")
                return []
            }
            return rows
        }
    }

    /// 只读打开并记录 inode
    private func openReadonly(_ dbPath: String) -> OpaquePointer? {
        var handle: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &handle, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let handle else {
            if let handle { sqlite3_close(handle) }
            debugPrint("TokenUsage: open failed \(dbPath)")
            return nil
        }
        dbConnections[dbPath] = handle
        dbInodes[dbPath] = currentInode(dbPath)
        return handle
    }

    /// 当前文件 inode（stat systemFileNumber；文件缺失返回 nil）
    private func currentInode(_ dbPath: String) -> UInt64? {
        (try? FileManager.default.attributesOfItem(atPath: dbPath))?[.systemFileNumber] as? UInt64
    }

    /// 缓存连接对应的 inode 是否仍与磁盘一致
    private func isCurrentInode(_ dbPath: String, openedInode: UInt64?) -> Bool {
        guard let now = currentInode(dbPath), let openedInode else { return false }
        return now == openedInode
    }

    /// 静态化：ISO8601DateFormatter 初始化昂贵且线程安全，避免每行/每次调用新建
    private static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    static func iso24hAgo(now: Date = Date()) -> String {
        isoFormatter.string(from: now.addingTimeInterval(-86_400))
    }

    static func parseISO(_ s: String) -> Date? {
        isoFormatter.date(from: s)
    }
}

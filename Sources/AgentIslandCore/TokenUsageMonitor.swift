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

// MARK: - dim 净消耗 SQL 片段（唯一事实来源，汇总/模型/会话三处共用）
//
// usage_ledger.usage.promptTokens 含缓存命中部分（cacheReadTokens），直接相加会
// 把缓存重复计入，累计用量被大幅虚高（数十倍量级）。净消耗 =
// (prompt - cacheRead) + completion，逐行钳制非负（个别行缺失/异常时不产生负值）。

enum DimUsageSQL {
    /// 净 token 表达式（对 usage 列逐行求值，供 SUM 使用）
    static let netTokens = """
    COALESCE(SUM(MAX(COALESCE(json_extract(usage,'$.promptTokens'),0) - COALESCE(json_extract(usage,'$.cacheReadTokens'),0), 0)),0)
         + COALESCE(SUM(json_extract(usage,'$.completionTokens')),0)
    """
}

// MARK: - 数值解析防护
//
// SQLite 数值列在 Swift 侧统一以字符串取出，而 `Int(Double(...))` 对越界值、
// Infinity、NaN 会**直接 fatalError**（整个进程 trap，用户表现为面板打开即消失）：
// 数据源把用量写成 `1e19`、`"99999999999999999999"`，或两行 1e308 相加令 SUM 溢出为
// `Inf`，都会走到这条路径。所以所有外部数值一律经这里「饱和解析」——
// 任何输入都返回可安全参与运算的值，绝不 trap；超出真实量级的输入被钳制并留下告警。
//
// 告警不做去重：这些数据源本就是异常态（写入端 bug 或哨兵值），
// 每轮刷新各告警一次，频率上限即轮询节律，足以定位且不会刷屏。

enum SafeNumber {
    /// 「不可能触及」的量级上限：token 用量、消息条数、毫秒时间戳都远小于该值
    /// （触及即说明数据源写入异常、聚合溢出或写入哨兵值），超过一律钳制，
    /// 避免污染汇总与后续运算。
    static let magnitudeCeiling = 1_000_000_000_000_000   // 1e15

    /// 金额上限（美元）：同为「不可能触及」的量级。
    static let costCeiling = 1_000_000_000.0

    /// Double → Int 饱和转换：越界 / Inf / NaN 一律钳制在 ±ceiling，绝不 trap。
    static func saturatingInt(_ value: Double, ceiling: Int = magnitudeCeiling, source: String) -> Int {
        guard value.isFinite else {
            // NaN 两个比较都不成立，落到 0
            let clamped = value > 0 ? ceiling : (value < 0 ? -ceiling : 0)
            warn(source: source, detail: "非有限值 \(value) 已钳制为 \(clamped)")
            return clamped
        }
        if value >= Double(ceiling) {
            warn(source: source, detail: "越界值 \(value) 已钳制为 \(ceiling)")
            return ceiling
        }
        if value <= -Double(ceiling) {
            warn(source: source, detail: "越界值 \(value) 已钳制为 \(-ceiling)")
            return -ceiling
        }
        return Int(value)
    }

    /// 字符串 → Int 饱和解析：Int 优先（精确、无精度损失），
    /// 失败再按 Double（SQLite REAL 聚合会输出 "1.9e+07" 这类文本）。
    /// 两者都失败（空串 / 非数值）按 0 处理，与「字段缺失」语义一致。
    static func parseInt(_ raw: String, ceiling: Int = magnitudeCeiling, source: String) -> Int {
        if let exact = Int(raw) {
            if exact > ceiling {
                warn(source: source, detail: "越界值 \(exact) 已钳制为 \(ceiling)")
                return ceiling
            }
            if exact < -ceiling {
                warn(source: source, detail: "越界值 \(exact) 已钳制为 \(-ceiling)")
                return -ceiling
            }
            return exact
        }
        guard let value = Double(raw) else { return 0 }
        return saturatingInt(value, ceiling: ceiling, source: source)
    }

    /// 字符串 → 金额：非有限值（Inf / NaN）归 0（否则汇总栏会显示 $inf / $nan），
    /// 超出上限钳制；负值与 0 保持原样（不改变既有口径）。
    static func parseCost(_ raw: String, ceiling: Double = costCeiling, source: String) -> Double {
        guard let value = Double(raw) else { return 0 }
        guard value.isFinite else {
            warn(source: source, detail: "非有限值 \(value) 已归零")
            return 0
        }
        if value > ceiling {
            warn(source: source, detail: "越界值 \(value) 已钳制为 \(ceiling)")
            return ceiling
        }
        return value
    }

    /// 毫秒时间戳文本 → Date。非法值（0 / 负数 / Inf / NaN / 超出合理纪元范围）
    /// 返回 nil，而不是构造出一个会让视图错乱的非法 Date。
    static func date(fromMillisText raw: String, source: String) -> Date? {
        guard let ms = Double(raw) else { return nil }
        guard ms.isFinite, ms > 0, ms < 1e14 else {
            warn(source: source, detail: "异常毫秒时间戳 \(raw) 已忽略")
            return nil
        }
        return Date(timeIntervalSince1970: ms / 1000)
    }

    /// SQLite INTEGER 列取出的毫秒纪元 → Date。同样拒收哨兵值 / 越界值
    /// （例如 Int64 上限），避免事件时间显示成公元数亿年。
    static func date(fromEpochMillis ms: Int64, source: String) -> Date? {
        guard ms > 0, ms < 100_000_000_000_000 else {
            warn(source: source, detail: "异常毫秒时间戳 \(ms) 已忽略")
            return nil
        }
        return Date(timeIntervalSince1970: Double(ms) / 1000)
    }

    private static func warn(source: String, detail: String) {
        debugPrint("SafeNumber[\(source)]: \(detail)")
    }
}

// MARK: - Token 子系统 seam（轮询面 / 查询面）
// 引擎与测试只依赖这两个小 interface；TokenUsageMonitor 是现网 adapter，
// 测试替身为本文件末尾的 FakeTokenUsageMonitor。

/// 轮询面：生命周期 + 缓存读取 + 刷新回调（引擎消费）
public protocol TokenUsagePolling: AnyObject {
    /// 各 agent 的用量快照（agentId → 用量；查询失败时保留上次成功值）
    var usage: [String: TokenUsage] { get }
    /// 所有数据源总和（汇总栏 / 高度判断）
    var grandTotal: TokenUsage { get }
    /// 刷新完成后的主线程回调（引擎接线：触发重采样）
    var onRefresh: (@MainActor () -> Void)? { get set }
    func start(interval: TimeInterval)
    func stop()
    /// 暂停轮询（连接保留；「呈现活跃」失活时由引擎调用）
    func pause()
}

/// 默认轮询节律（唯一来源：类签名默认参数与无参便利方法共用）
public enum TokenUsagePollingDefaults {
    public static let interval: TimeInterval = 60.0
}

public extension TokenUsagePolling {
    /// 协议要求不带默认参数；无参形式走默认节律
    func start() { start(interval: TokenUsagePollingDefaults.interval) }
}

/// 查询面：详情页按需下钻（引擎转发时消费）
public protocol TokenUsageQuerying: AnyObject {
    func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void)
    func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void)
}

/// @unchecked Sendable：全部可变状态由 NSLock + dbQueue 串行队列保护，可跨线程调用
public final class TokenUsageMonitor: TokenUsagePolling, TokenUsageQuerying, @unchecked Sendable {
    private let lock = NSLock()
    private var _usage: [String: TokenUsage] = [:]   // agentId → 用量
    private var _grandTotal = TokenUsage()
    /// 串行化整次刷新，避免并发查询交错发布旧结果。
    private let refreshLock = NSLock()
    private var lastDimRefresh: Date?
    private var lastOpenCodeRefresh: Date?
    private var lastDimStamp = ""
    private var lastOpenCodeStamp = ""
    private var timer: Timer?
    /// 刷新完成后的主线程回调（引擎用它触发重采样，让卡片高度/徽标及时跟上）。
    /// 后台刷新线程读、主线程写，故与其余可变状态一样纳入 lock（读写都在锁内取值，
    /// 回调本身在锁外调用，避免持锁执行引擎代码造成死锁）。
    private var _onRefresh: (@MainActor () -> Void)?
    public var onRefresh: (@MainActor () -> Void)? {
        get { lock.lock(); defer { lock.unlock() }; return _onRefresh }
        set { lock.lock(); defer { lock.unlock() }; _onRefresh = newValue }
    }

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

    private let dimAgentDB: String
    private let openCodeDB: String

    /// SQLite 只读连接缓存（复用避免每查询 open/close）；查询统一走串行队列保证连接线程安全
    private var dbConnections: [String: OpaquePointer] = [:]
    /// 连接打开时的文件 inode（外部替换主文件后据此失效缓存连接）
    private var dbInodes: [String: UInt64] = [:]
    private let dbQueue = DispatchQueue(label: "com.agentisland.tokenusage.db")

    /// 数据库路径构造注入（接受依赖，不自行定位；默认现网双源路径，测试传 fixture 临时库）
    public init(dimAgentDB: String = NSString(string: "~/.dimcode/v2/dimcode.sqlite").expandingTildeInPath,
                openCodeDB: String = NSString(string: "~/.local/share/opencode/opencode.db").expandingTildeInPath) {
        self.dimAgentDB = dimAgentDB
        self.openCodeDB = openCodeDB
    }

    public func start(interval: TimeInterval = TokenUsagePollingDefaults.interval) {
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

    /// 暂停后台轮询（「呈现活跃」失活时由引擎调用，省掉整条查询链路与 onRefresh 重采样）
    /// 注意：不关闭连接（每次 expanded 重开 + 全表重扫产生 41.6% 尖峰；
    /// 只读连接可长驻复用，SQLite 对同库持续写入安全）；stop() 才彻底清理。
    /// 暂停后重启走 start()：timer 已空则立即首刷并重建定时器，连接不受影响
    public func pause() {
        guard timer != nil else { return }
        timer?.invalidate()
        timer = nil
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
        refresh(now: Date())
    }

    /// 同一次刷新共用时间边界；文件未变也需定期推进 24h 窗口。
    func refresh(now: Date) {
        refreshLock.lock()
        defer { refreshLock.unlock() }
        let dimStamp = fileStamp(dimAgentDB)
        let openCodeStamp = fileStamp(openCodeDB)
        func isFresh(_ date: Date?) -> Bool {
            guard let date else { return false }
            let age = now.timeIntervalSince(date)
            return age >= 0 && age < TokenUsagePollingDefaults.interval
        }
        let refreshDim = lastDimStamp != dimStamp || !isFresh(lastDimRefresh)
        let refreshOpenCode = lastOpenCodeStamp != openCodeStamp || !isFresh(lastOpenCodeRefresh)
        guard refreshDim || refreshOpenCode else { return }

        var updated = usage
        var succeeded = false
        if refreshDim, let value = queryDimAgent(cutoffISO: Self.iso24hAgo(now: now)) {
            updated["dim"] = value
            lastDimStamp = dimStamp
            lastDimRefresh = now
            succeeded = true
        }
        // 时间戳同样可能是脏值（now 由调用方注入，测试可传任意 Date）：饱和后再转 Int64
        let cutoffSeconds = now.addingTimeInterval(-86_400).timeIntervalSince1970
        let cutoffMs = Int64(SafeNumber.saturatingInt(cutoffSeconds * 1000, source: "opencode.cutoff"))
        if refreshOpenCode,
           let value = queryOpenCode(cutoffMs: cutoffMs) {
            updated["opencode"] = value
            lastOpenCodeStamp = openCodeStamp
            lastOpenCodeRefresh = now
            succeeded = true
        }
        // 打开成功不等于查询成功：失败源保留旧值与旧戳，下次刷新重试。
        guard succeeded else { return }
        lock.lock()
        let previousUsage = _usage
        let previousTotal = _grandTotal
        let newUsage = updated.filter { !$0.value.isEmpty }
        let newTotal = updated.values.reduce(TokenUsage(), +)
        _usage = newUsage
        _grandTotal = newTotal
        lock.unlock()
        // 数据库仍被轮询但结果没有变化时，不触发引擎重采样；
        // 只有统计值真的变化（含 24h 窗口过期）才刷新 UI。
        guard previousUsage != newUsage || previousTotal != newTotal else { return }
        if let onRefresh {
            Task { @MainActor in onRefresh() }
        }
    }

    /// 文件变更戳（inode+mtime+size 组合；任一变化即视为被替换/写入）。
    /// 必须纳入 `-wal` 文件（WAL 模式连接持有中 commit 只写 wal，
    /// 主文件戳在 checkpoint 前不变——长时间未 checkpoint 的库只查主文件
    /// 会导致活跃写入期间统计静默冻结）；mtime 用 Double 避免整秒截断漏检
    private func fileStamp(_ path: String) -> String {
        func stamp(_ p: String) -> String {
            guard let attrs = try? FileManager.default.attributesOfItem(atPath: p) else { return "missing" }
            let inode = (attrs[.systemFileNumber] as? UInt64) ?? 0
            let mtime = (attrs[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
            let size = (attrs[.size] as? UInt64) ?? 0
            return "\(inode)-\(mtime)-\(size)"
        }
        return stamp(path) + "|wal:" + stamp(path + "-wal")
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
                       \(DimUsageSQL.netTokens),
                       COALESCE(SUM(cost),0)
                FROM usage_ledger GROUP BY modelId ORDER BY 3 DESC
                """
                rows = rawRows(sql, dbPath: dimAgentDB, cols: 4).map {
                    ModelUsage(modelId: $0[0],
                               messages: SafeNumber.parseInt($0[1], source: "dim.model.messages"),
                               tokens: SafeNumber.parseInt($0[2], source: "dim.model.tokens"),
                               cost: SafeNumber.parseCost($0[3], source: "dim.model.cost"))
                }
            case "opencode":
                let sql = """
                SELECT json_extract(data,'$.modelID'), COUNT(*),
                       COALESCE(SUM(json_extract(data,'$.tokens.input')),0)+COALESCE(SUM(json_extract(data,'$.tokens.output')),0)+COALESCE(SUM(json_extract(data,'$.tokens.reasoning')),0),
                       COALESCE(SUM(json_extract(data,'$.cost')),0)
                FROM message WHERE json_extract(data,'$.role')='assistant'
                GROUP BY 1 ORDER BY 3 DESC
                """
                rows = rawRows(sql, dbPath: openCodeDB, cols: 4).map {
                    ModelUsage(modelId: $0[0],
                               messages: SafeNumber.parseInt($0[1], source: "opencode.model.messages"),
                               tokens: SafeNumber.parseInt($0[2], source: "opencode.model.tokens"),
                               cost: SafeNumber.parseCost($0[3], source: "opencode.model.cost"))
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
                       \(DimUsageSQL.netTokens),
                       COALESCE(SUM(cost),0), MAX(createdAt)
                FROM usage_ledger WHERE modelId = '\(modelId.escaped)'
                GROUP BY sessionId ORDER BY 5 DESC LIMIT 200
                """
                rows = rawRows(sql, dbPath: dimAgentDB, cols: 5).map { r in
                    let dir = dirPrefix + "/" + r[0]
                    return SessionUsage(sessionId: r[0],
                                        directory: FileManager.default.fileExists(atPath: dir) ? dir : nil,
                                        messages: SafeNumber.parseInt(r[1], source: "dim.session.messages"),
                                        tokens: SafeNumber.parseInt(r[2], source: "dim.session.tokens"),
                                        cost: SafeNumber.parseCost(r[3], source: "dim.session.cost"),
                                        lastTime: Self.parseISO(r[4]))
                }
            case "opencode":
                let sql = """
                SELECT m.session_id, COUNT(*),
                       COALESCE(SUM(json_extract(m.data,'$.tokens.input')),0)+COALESCE(SUM(json_extract(m.data,'$.tokens.output')),0)+COALESCE(SUM(json_extract(m.data,'$.tokens.reasoning')),0),
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
                                        messages: SafeNumber.parseInt(r[1], source: "opencode.session.messages"),
                                        tokens: SafeNumber.parseInt(r[2], source: "opencode.session.tokens"),
                                        cost: SafeNumber.parseCost(r[3], source: "opencode.session.cost"),
                                        lastTime: SafeNumber.date(fromMillisText: r[4], source: "opencode.session.lastTime"))
                }
            default:
                rows = []
            }
            Task { @MainActor in completion(rows) }
        }
    }

    // MARK: - 24h/累计 汇总查询

    private func queryDimAgent(cutoffISO: String) -> TokenUsage? {
        // createdAt 是 ISO8601 UTC 字符串（同格式字符串比较即时间比较）；cost 全表 SUM（NULL 记 0）
        let sql24h = """
        SELECT \(DimUsageSQL.netTokens),
               COALESCE(SUM(cost),0)
        FROM usage_ledger WHERE createdAt >= '\(cutoffISO)'
        """
        let sqlTotal = """
        SELECT \(DimUsageSQL.netTokens),
               COALESCE(SUM(cost),0)
        FROM usage_ledger
        """
        var u: TokenUsage?
        if let (t24, c24) = scalarSum(sql24h, dbPath: dimAgentDB, source: "dim"),
           let (tAll, cAll) = scalarSum(sqlTotal, dbPath: dimAgentDB, source: "dim") {
            u = TokenUsage(tokens24h: t24, tokensTotal: tAll, cost24h: c24, costTotal: cAll)
        }
        return u
    }

    private func queryOpenCode(cutoffMs: Int64) -> TokenUsage? {
        let tokensExpr = """
        COALESCE(SUM(json_extract(data,'$.tokens.input')),0)
        + COALESCE(SUM(json_extract(data,'$.tokens.output')),0)
        + COALESCE(SUM(json_extract(data,'$.tokens.reasoning')),0)
        """
        let roleFilter = "json_extract(data,'$.role')='assistant'"
        let sql24h = "SELECT \(tokensExpr), COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(roleFilter) AND time_created >= \(cutoffMs)"
        let sqlTotal = "SELECT \(tokensExpr), COALESCE(SUM(json_extract(data,'$.cost')),0) FROM message WHERE \(roleFilter)"
        var u: TokenUsage?
        if let (t24, c24) = scalarSum(sql24h, dbPath: openCodeDB, source: "opencode"),
           let (tAll, cAll) = scalarSum(sqlTotal, dbPath: openCodeDB, source: "opencode") {
            u = TokenUsage(tokens24h: t24, tokensTotal: tAll, cost24h: c24, costTotal: cAll)
        }
        return u
    }

    // MARK: - SQLite 底层

    /// 两列标量查询：(token, cost)；查询失败返回 nil
    private func scalarSum(_ sql: String, dbPath: String, source: String) -> (Int, Double)? {
        guard let row = rawRows(sql, dbPath: dbPath, cols: 2).first else { return nil }
        // 经 Double 中转：SQLite 对 REAL 列求和会输出 "19067783.5" 这类带小数文本，
        // 直接 Int("...") 会返回 nil 并被 ?? 0 静默归零（统计整体消失且无任何报错）。
        // 兜底转换必须走 SafeNumber：脏数据（1e19 / Inf / NaN）在该路径上会直接 trap。
        let tokens = SafeNumber.parseInt(row[0], source: "\(source).total.tokens")
        return (tokens, SafeNumber.parseCost(row[1], source: "\(source).total.cost"))
    }

    /// 通用查询：全部列转字符串返回（数值/文本统一处理，空结果返回 []）
    /// 线程安全：所有查询在 dbQueue 串行执行，连接按路径缓存复用（只读，应用生命周期内不关闭）
    private func rawRows(_ sql: String, dbPath: String, cols: Int) -> [[String]] {
        dbQueue.sync {
            let fm = FileManager.default
            guard fm.fileExists(atPath: dbPath) else {
                return []
            }
            // inode 失效检测：外部工具原子替换/重建主文件（VACUUM 后 rename、
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
            // 中途出错（SQLITE_ERROR/BUSY）不应把部分行当完整结果
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
        // 降低瞬态 BUSY：只读连接遇到写锁立即返回 BUSY，
        // 等待最多 1s 再失败，避免偶发把整次查询打成失败
        sqlite3_busy_timeout(handle, 1000)
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

// MARK: - 测试用假实现（MainActor 内使用；记录轮询面调用序列，零 I/O）

public final class FakeTokenUsageMonitor: TokenUsagePolling, TokenUsageQuerying {
    /// 轮询面调用序列："start" / "stop" / "pause"
    public private(set) var calls: [String] = []
    public var usage: [String: TokenUsage] = [:]
    public var grandTotal = TokenUsage()
    public var onRefresh: (@MainActor () -> Void)?

    public init() {}

    public func start(interval: TimeInterval) { calls.append("start") }

    public func stop() { calls.append("stop") }

    public func pause() { calls.append("pause") }

    public func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        calls.append("modelBreakdown")   // 同步记录：调用即刻可断言；回调异步送达
        Task { @MainActor in completion([]) }
    }

    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        calls.append("sessions")   // 同步记录：调用即刻可断言；回调异步送达
        Task { @MainActor in completion([]) }
    }
}

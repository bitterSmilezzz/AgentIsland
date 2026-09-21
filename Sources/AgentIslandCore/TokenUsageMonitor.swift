import Foundation
import SQLite3

// MARK: - Token 用量统计（只读 SQLite + 结构化 JSONL 数据源）
//
// 数据源参考 vibe-usage 的采集思路（本机只读、不碰凭证）：
// 1. DimAgent: ~/.dimcode/v2/dimcode.sqlite → usage_ledger（token 全，cost 全 NULL）
// 2. OpenCode: ~/.local/share/opencode/opencode.db → message（token + cost）
// 3. Codex: ~/.codex/sessions → token_usage_record
// 4. Claude / WorkBuddy: 各自 projects/sessions JSONL → message.usage
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

// MARK: - 时间分析数据

/// 时间分析范围。每个范围控制固定桶数，保证 330pt 图表不会因数据量增长而变密。
public enum TokenTimeRange: String, CaseIterable, Identifiable, Equatable {
    case day
    case week
    case month

    public var id: String { rawValue }

    public var duration: TimeInterval {
        switch self {
        case .day: return 24 * 3_600
        case .week: return 7 * 24 * 3_600
        case .month: return 30 * 24 * 3_600
        }
    }

    public var bucketCount: Int {
        switch self {
        case .day: return 24       // 每小时
        case .week: return 28      // 每 6 小时
        case .month: return 30     // 每天
        }
    }

    public var label: String {
        switch self {
        case .day: return "最近 24 小时 (24h)"
        case .week: return "最近 7 天 (7d)"
        case .month: return "最近 30 天 (30d)"
        }
    }
}

public struct TokenUsagePoint: Identifiable, Equatable {
    public let start: Date
    public let tokens: Int
    public let cost: Double
    public var id: Date { start }
}

public struct TokenSourceUsage: Identifiable, Equatable {
    public let agentId: String
    public let tokens: Int
    public let cost: Double
    /// 已发现该工具的本地统计源；false 表示当前机器没有可读取的明细，而不是用量为 0。
    public let isAvailable: Bool
    public var id: String { agentId }
}

/// Token 来源行是否可进入详情的导航决策。
///
/// 用量来源与实时监控是两条独立链路：例如 Codex 被 ChatGPT 桌面端承载时，
/// 不应在实时 Agent 列表中重复出现，却仍能从 `~/.codex/sessions` 读取历史用量。
public enum TokenSourceDetailRoute {
    public static func target(for source: TokenSourceUsage,
                              detailCapableAgentIDs: Set<String>) -> String? {
        // 实时快照只描述进程监控，不能决定历史用量是否可查看。内嵌 Codex 会被
        // 有意排除出独立快照以免与 ChatGPT 重复计数，但仍保留可读的 JSONL 用量。
        guard source.isAvailable, detailCapableAgentIDs.contains(source.agentId) else { return nil }
        return source.agentId
    }
}

public struct TokenUsageTimeline: Equatable {
    public let range: TokenTimeRange
    public let points: [TokenUsagePoint]
    public let sources: [TokenSourceUsage]
    public let tokens: Int
    public let cost: Double
    public let previousTokens: Int
    public let previousCost: Double

    public static func empty(for range: TokenTimeRange, now: Date = Date()) -> TokenUsageTimeline {
        let step = range.duration / Double(range.bucketCount)
        let start = now.addingTimeInterval(-range.duration)
        return TokenUsageTimeline(
            range: range,
            points: (0..<range.bucketCount).map {
                TokenUsagePoint(start: start.addingTimeInterval(Double($0) * step), tokens: 0, cost: 0)
            },
            sources: [], tokens: 0, cost: 0, previousTokens: 0, previousCost: 0
        )
    }
}

/// 数据源查询后统一进入纯聚合器；时间边界和分桶规则因此可以脱离 SQLite 精确测试。
struct TokenUsageRecord: Equatable {
    let agentId: String
    let time: Date
    let tokens: Int
    let cost: Double
}

enum TokenTimelineBuilder {
    static func build(records: [TokenUsageRecord], range: TokenTimeRange,
                      now: Date,
                      supportedSourceIds: [String] = [],
                      availableSourceIds: Set<String> = []) -> TokenUsageTimeline {
        let duration = range.duration
        let currentStart = now.addingTimeInterval(-duration)
        let previousStart = now.addingTimeInterval(-2 * duration)
        let step = duration / Double(range.bucketCount)
        var tokenBuckets = Array(repeating: 0, count: range.bucketCount)
        var costBuckets = Array(repeating: 0.0, count: range.bucketCount)
        var sources: [String: (tokens: Int, cost: Double)] = [:]
        var previousTokens = 0
        var previousCost = 0.0

        for record in records where record.time >= previousStart && record.time <= now {
            let tokens = max(record.tokens, 0)
            let cost = max(record.cost.isFinite ? record.cost : 0, 0)
            if record.time < currentStart {
                previousTokens = safeTokenSum(previousTokens, tokens)
                previousCost = safeCostSum(previousCost, cost)
                continue
            }

            let rawIndex = Int(record.time.timeIntervalSince(currentStart) / step)
            let index = min(max(rawIndex, 0), range.bucketCount - 1)
            tokenBuckets[index] = safeTokenSum(tokenBuckets[index], tokens)
            costBuckets[index] = safeCostSum(costBuckets[index], cost)
            let old = sources[record.agentId] ?? (0, 0)
            sources[record.agentId] = (safeTokenSum(old.tokens, tokens), safeCostSum(old.cost, cost))
        }

        let points = (0..<range.bucketCount).map { index in
            TokenUsagePoint(
                start: currentStart.addingTimeInterval(Double(index) * step),
                tokens: tokenBuckets[index],
                cost: costBuckets[index]
            )
        }
        let sourceIds = supportedSourceIds.isEmpty
            ? Set(sources.keys)
            : Set(supportedSourceIds).union(sources.keys)
        let sourceRows = sourceIds.map { agentId in
            let value = sources[agentId] ?? (0, 0)
            return TokenSourceUsage(
                agentId: agentId,
                tokens: value.tokens,
                cost: value.cost,
                isAvailable: availableSourceIds.isEmpty
                    ? sources[agentId] != nil
                    : availableSourceIds.contains(agentId)
            )
        }.sorted {
            if $0.tokens != $1.tokens { return $0.tokens > $1.tokens }
            if $0.isAvailable != $1.isAvailable { return $0.isAvailable }
            return $0.agentId < $1.agentId
        }
        return TokenUsageTimeline(
            range: range,
            points: points,
            sources: sourceRows,
            tokens: tokenBuckets.reduce(0, safeTokenSum),
            cost: costBuckets.reduce(0, safeCostSum),
            previousTokens: previousTokens,
            previousCost: previousCost
        )
    }

    private static func safeTokenSum(_ lhs: Int, _ rhs: Int) -> Int {
        let (sum, overflow) = lhs.addingReportingOverflow(rhs)
        if overflow || sum > SafeNumber.magnitudeCeiling { return SafeNumber.magnitudeCeiling }
        return max(sum, 0)
    }

    private static func safeCostSum(_ lhs: Double, _ rhs: Double) -> Double {
        let sum = lhs + rhs
        guard sum.isFinite else { return SafeNumber.costCeiling }
        return min(max(sum, 0), SafeNumber.costCeiling)
    }
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

    /// JSON 整数字段容错读取。`value as? Int` 在第三方改版时会静默返回 nil：
    /// `"step_index": "3"`（字符串）或 `3.0`（浮点）都拿不到值，于是
    /// —— Antigravity 每行指纹退化成 `step-0`（进度判定与「已回答」比较全部失效）
    /// —— DSH 的 openTurnStartSeq 变 nil，在途状态被降级成「已完成」
    /// 三种写法都认；越界仍走饱和钳制，绝不 trap
    static func jsonInt(_ value: Any?) -> Int? {
        switch value {
        case let n as Int: return n
        case let d as Double: return d.isFinite ? saturatingInt(d, source: "jsonInt") : nil
        case let s as String:
            let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines)
            if let i = Int(trimmed) { return i }
            if let d = Double(trimmed), d.isFinite { return saturatingInt(d, source: "jsonInt.string") }
            return nil
        default: return nil
        }
    }

    /// 饱和乘：两个可能来自外部数据的 Int 相乘，溢出时取 Int.max 而不是 trap。
    /// Swift 的 `*` 在 Int 溢出时是运行时崩溃（实测 `tokens --budget 9e18` 走到
    /// `dailyBudget * 当月天数` 即 SIGTRAP，且不留任何诊断输出）
    static func product(_ lhs: Int, _ rhs: Int) -> Int {
        let (value, overflow) = lhs.multipliedReportingOverflow(by: rhs)
        guard overflow else { return value }
        // 溢出时按符号钳到两端，正负由操作数决定
        return (lhs < 0) != (rhs < 0) ? Int.min : Int.max
    }

    /// 金额上限（美元）：同为「不可能触及」的量级。
    static let costCeiling = 1_000_000_000.0

    /// 饱和乘（金额口径）：cost 全链路都钳在 `costCeiling`，唯独「月末预估费用」原先是裸的
    /// `Double × 天数`——脏 cost 会让它跑到 Inf，界面上出现「累计 $1e9，预估月末
    /// $3.1e10」这种自相矛盾的数（本该同量级的两个数差了 31 倍）。
    /// 钳制方向与 `saturatingInt` 同一套口径：Inf → 上限、NaN → 0、负数 → 0。
    static func costProduct(_ lhs: Double, _ rhs: Int) -> Double {
        let days = Double(max(0, rhs))
        guard days > 0 else { return 0 }
        if lhs.isInfinite { return lhs > 0 ? costCeiling : 0 }
        guard lhs.isFinite else { return 0 }
        let value = max(lhs, 0) * days
        guard value.isFinite else { return costCeiling }
        return min(value, costCeiling)
    }

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
        AppLog.warn("SafeNumber[\(source)]: \(detail)")
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
    /// 按需单次刷新（R34/F6）：菜单栏 popover 等第三消费方打开时调用
    func refreshAsync()
    /// 同步单次刷新：一次性 CLI（status / report / doctor）必须在采样前把用量取回来，
    /// 否则每个 Agent 的用量列都会显示 0——把「没去取」呈现成「没有」
    func refreshSync()
}

/// 默认轮询节律（唯一来源：类签名默认参数与无参便利方法共用）
public enum TokenUsagePollingDefaults {
    public static let interval: TimeInterval = 60.0
}

public extension TokenUsagePolling {
    /// 协议要求不带默认参数；无参形式走默认节律
    func start() { start(interval: TokenUsagePollingDefaults.interval) }
    /// 测试替身与不提供同步取数的实现退回异步（下一次采样自然带上）
    func refreshSync() { refreshAsync() }
}

/// 查询面：详情页按需下钻（引擎转发时消费）
public protocol TokenUsageQuerying: AnyObject {
    func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void)
    func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void)
    func timeline(range: TokenTimeRange, now: Date,
                  completion: @escaping @MainActor (TokenUsageTimeline) -> Void)
}

public extension TokenUsageQuerying {
    /// 旧测试替身和不提供历史数据的 adapter 仍可使用空时间线，不需要伪造历史。
    func timeline(range: TokenTimeRange, now: Date = Date(),
                  completion: @escaping @MainActor (TokenUsageTimeline) -> Void) {
        Task { @MainActor in completion(.empty(for: range, now: now)) }
    }
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
    private let structuredIndex: StructuredTokenUsageIndex
    /// 当前能提供稳定本地 Token 明细的工具；时间页始终列出，缺源时明确标注。
    static let supportedToolIds = ["dim", "codex", "claude", "workbuddy", "workbuddy-ai", "opencode"]
    private var configuredToolIds: [String] {
        let structuredIds = structuredIndex.configuredToolIds
        return Self.supportedToolIds.filter {
            $0 == "dim" || $0 == "opencode" || structuredIds.contains($0)
        }
    }
    /// isFresh 宽限（R25）：略大于轮询间隔，吸收定时器抖动
    static let stampGrace: TimeInterval = 5
    /// 数据源主库连续缺失计数（R9：达到阈值视为「源已消失」并置空该源）
    private var dimMissingStreak = 0
    private var openCodeMissingStreak = 0
    /// 连续缺失阈值：60s 轮询下约 3 分钟——瞬时空窗（原子替换/迁移）不触发
    static let sourceMissingLimit = 3

    /// SQLite 只读连接缓存（复用避免每查询 open/close）；查询统一走串行队列保证连接线程安全
    private var dbConnections: [String: OpaquePointer] = [:]
    /// 连接打开时的文件 inode（外部替换主文件后据此失效缓存连接）
    private var dbInodes: [String: UInt64] = [:]
    /// 一次性连接（stop 后迟到的重建；rawRows 收尾统一关闭，不写回缓存）
    private var transientHandles: [OpaquePointer] = []
    private let dbQueue = DispatchQueue(label: "com.agentisland.tokenusage.db")

    /// 现网构造：SQLite + 可审计 JSONL，全部只读且不触碰凭证/正文。
    /// 库与采集根目录一律取自注册表档案（`sessionDatabase` / `tokenRoots`）——同样的路径
    /// 若在这里再写一份字面量，档案换目录或改名后只有一半会生效（`dimcode.sqlite` 与
    /// WorkBuddy 的 `projects/` 之前就不在注册表里）。
    public convenience init() {
        func database(_ id: String) -> String { AgentRegistry.databasePath(for: id) ?? "" }
        // 采集根取自档案的 `tokenRoots`：它与 sessionDirs 分开声明，因为 WorkBuddy 一类的
        // 明细目录与心跳目录不在同一子树
        func tokenRoots(_ id: String) -> [String] { AgentRegistry.profile(id)?.tokenRoots ?? [] }
        self.init(
            dimAgentDB: database("dim"),
            openCodeDB: database("opencode"),
            structuredSources: [
                StructuredTokenSource(agentId: "codex", roots: tokenRoots("codex"), format: .codex),
                StructuredTokenSource(agentId: "claude", roots: tokenRoots("claude"), format: .anthropic),
                StructuredTokenSource(agentId: "workbuddy", roots: tokenRoots("workbuddy"), format: .anthropic),
                StructuredTokenSource(agentId: "workbuddy-ai", roots: tokenRoots("workbuddy-ai"), format: .anthropic),
                // Qoder 刻意**不**接进来。它的逐行记录长得和 Anthropic 一模一样
                // （message.usage.input_tokens / cache_read_input_tokens / output_tokens），
                // 但本机实测 1,033 条 usage 记录里这四个字段全是 0——真值只在 `credits`
                // （合计 303.37）与 `context_usage_ratio` 里。接进来的代价实测是冷跑
                // `tokens` 从 ~2.05s 涨到 ~2.9–4.0s（会话文件单个可达 10MB），
                // 换回 0 条可用数据。等 credits 有了明确的展示口径再接。
            ]
        )
    }

    /// Fixture 构造默认关闭用户目录采集，保证测试不被开发机真实日志污染。
    public convenience init(dimAgentDB: String, openCodeDB: String) {
        self.init(dimAgentDB: dimAgentDB, openCodeDB: openCodeDB, structuredSources: [])
    }

    init(dimAgentDB: String, openCodeDB: String, structuredSources: [StructuredTokenSource]) {
        self.dimAgentDB = dimAgentDB
        self.openCodeDB = openCodeDB
        self.structuredIndex = StructuredTokenUsageIndex(sources: structuredSources)
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

    /// 连接代际（R34/F9）：stop() 递增后，在飞 refresh 的 rawRows 不再把重建的
    /// 连接写回缓存（stop 后无人再关，违背「彻底清理」契约）
    private var dbGeneration = 0
    public func stop() {
        lock.lock(); dbGeneration += 1; lock.unlock()
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

    /// 在飞/排队中的 refreshAsync 合并（R34/G4）：start() 首刷与 60s Timer 相邻触发
    /// 两次会让第二个线程在 refreshLock 上白等（纯排队）。入队前置位、refresh 收尾清位
    private var refreshQueued = false
    public func refreshAsync() {
        lock.lock()
        if refreshQueued {
            lock.unlock()
            return
        }
        refreshQueued = true
        lock.unlock()
        DispatchQueue.global(qos: .utility).async { [weak self] in
            self?.refresh()
            self?.lock.lock()
            self?.refreshQueued = false
            self?.lock.unlock()
        }
    }

    public func refresh() {
        refresh(now: Date())
    }

    /// 同步刷一次：调用线程直接把 DB/文件读完后返回（一次性 CLI 用）
    public func refreshSync() { refresh() }

    /// 同一次刷新共用时间边界；文件未变也需定期推进 24h 窗口。
    func refresh(now: Date) {
        refreshLock.lock()
        defer { refreshLock.unlock() }
        let dimStamp = fileStamp(dimAgentDB)
        let openCodeStamp = fileStamp(openCodeDB)
        func isFresh(_ date: Date?) -> Bool {
            guard let date else { return false }
            let age = now.timeIntervalSince(date)
            // 宽限 5s（R25/S9）：阈值恰等于轮询间隔时，下一拍 age 恒 ≥ interval，
            // isFresh 永不成立 → 戳比对被 OR 短路成永久无效功（每次 4 次 stat）。
            // 稳态（戳未变）跳过重查是本条件的意图；24h 窗口在有写入（戳变化）时即重算
            return age >= 0 && age < TokenUsagePollingDefaults.interval + Self.stampGrace
        }
        let refreshDim = lastDimStamp != dimStamp || !isFresh(lastDimRefresh)
        let refreshOpenCode = lastOpenCodeStamp != openCodeStamp || !isFresh(lastOpenCodeRefresh)
        guard refreshDim || refreshOpenCode || structuredIndex.isEnabled else { return }

        var updated = usage
        var succeeded = false
        // 数据源活性终态（R9）：主库文件连续多次缺失时该源置空。「查询失败保留上次
        // 成功值」契约只覆盖瞬时失败；「源已消失」若不清空，面板会永久显示陈旧数字。
        // 连续计数区分瞬时缺失（原子替换的短暂空窗 / 迁移中）与永久删除——单拍缺失
        // 只累积不清空（数据库暂时缺失保留旧统计的契约不变）。注意判定必须用主库
        // 文件存在性而非 fileStamp 字符串——组合戳里的 "-wal:missing" 子串是合法形态
        func noteMissing(_ exists: Bool, counter: inout Int) -> Bool {
            if exists {
                counter = 0
                return false
            }
            counter += 1
            return counter >= Self.sourceMissingLimit
        }
        let dimGone = noteMissing(FileManager.default.fileExists(atPath: dimAgentDB),
                                  counter: &dimMissingStreak)
        let openCodeGone = noteMissing(FileManager.default.fileExists(atPath: openCodeDB),
                                       counter: &openCodeMissingStreak)
        if dimGone {
            updated["dim"] = nil
            succeeded = true
        }
        if openCodeGone {
            updated["opencode"] = nil
            succeeded = true
        }
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
        if structuredIndex.isEnabled {
            let structured = structuredIndex.snapshot(now: now)
            let structuredUsage = structured.usage(now: now)
            for agentId in structuredIndex.configuredToolIds {
                updated[agentId] = structuredUsage[agentId]
            }
            // 扫描成功即允许发布：即使全部为零，也要能清除已删除日志留下的旧值。
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

    /// 会话下钻一次最多取多少条。列表页的标题必须按这个数说话：取满了就不能说
    /// 「N 个会话」——那是把「只看了前 200 条」讲成「一共就 200 条」。
    public static let sessionDrilldownLimit = 200

    /// 会话列表的标题口径（纯函数，便于测）：取满上限时明说是「最近 N 个，还有更多」
    public static func sessionListSubtitle(count: Int) -> String {
        count >= sessionDrilldownLimit
            ? "最近 \(count) 个会话（还有更多未列出）"
            : "\(count) 个会话"
    }

    /// 某模型下的会话列表（按最后活动降序）
    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            var rows: [SessionUsage] = []
            switch agentId {
            case "dim":
                // 会话目录同样取自档案：这里不再抄一份 `~/...` 字面量
                let dirPrefix = AgentRegistry.profile("dim")?.tokenRoots.first ?? ""
                let sql = """
                SELECT sessionId, COUNT(*),
                       \(DimUsageSQL.netTokens),
                       COALESCE(SUM(cost),0), MAX(createdAt)
                FROM usage_ledger WHERE modelId = '\(modelId.escaped)'
                GROUP BY sessionId ORDER BY 5 DESC LIMIT \(Self.sessionDrilldownLimit)
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
                GROUP BY m.session_id ORDER BY 5 DESC LIMIT \(Self.sessionDrilldownLimit)
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

    /// 全局 Token 时间线（净消耗口径）。SQLite 与结构化日志合并后统一分桶，
    /// UI 切换范围时在后台读取最近两个等长周期，用于当前趋势、环比与分工具统计。
    public func timeline(range: TokenTimeRange, now: Date = Date(),
                         completion: @escaping @MainActor (TokenUsageTimeline) -> Void) {
        DispatchQueue.global(qos: .userInitiated).async { [self] in
            let previousStart = now.addingTimeInterval(-2 * range.duration)
            let lowerISO = Self.isoFormatter.string(from: previousStart)
            let upperISO = Self.isoFormatter.string(from: now)
            let dimRowTokens = """
            MAX(COALESCE(json_extract(usage,'$.promptTokens'),0)
                - COALESCE(json_extract(usage,'$.cacheReadTokens'),0), 0)
                + COALESCE(json_extract(usage,'$.completionTokens'),0)
            """
            let dimSQL = """
            SELECT createdAt, \(dimRowTokens), COALESCE(cost,0)
            FROM usage_ledger
            WHERE createdAt >= '\(lowerISO.escaped)' AND createdAt <= '\(upperISO.escaped)'
            ORDER BY createdAt
            """
            let dimRecords = rawRows(dimSQL, dbPath: dimAgentDB, cols: 3).compactMap { row -> TokenUsageRecord? in
                guard let time = Self.parseISO(row[0]) else { return nil }
                return TokenUsageRecord(
                    agentId: "dim", time: time,
                    tokens: SafeNumber.parseInt(row[1], source: "dim.timeline.tokens"),
                    cost: SafeNumber.parseCost(row[2], source: "dim.timeline.cost")
                )
            }

            let lowerMs = Int64(SafeNumber.saturatingInt(
                previousStart.timeIntervalSince1970 * 1_000, source: "opencode.timeline.lower"
            ))
            let upperMs = Int64(SafeNumber.saturatingInt(
                now.timeIntervalSince1970 * 1_000, source: "opencode.timeline.upper"
            ))
            let openCodeSQL = """
            SELECT time_created,
                   COALESCE(json_extract(data,'$.tokens.input'),0)
                     + COALESCE(json_extract(data,'$.tokens.output'),0)
                     + COALESCE(json_extract(data,'$.tokens.reasoning'),0),
                   COALESCE(json_extract(data,'$.cost'),0)
            FROM message
            WHERE json_extract(data,'$.role')='assistant'
              AND time_created >= \(lowerMs) AND time_created <= \(upperMs)
            ORDER BY time_created
            """
            let openCodeRecords = rawRows(openCodeSQL, dbPath: openCodeDB, cols: 3).compactMap { row -> TokenUsageRecord? in
                guard let time = SafeNumber.date(fromMillisText: row[0], source: "opencode.timeline.time") else {
                    return nil
                }
                return TokenUsageRecord(
                    agentId: "opencode", time: time,
                    tokens: SafeNumber.parseInt(row[1], source: "opencode.timeline.tokens"),
                    cost: SafeNumber.parseCost(row[2], source: "opencode.timeline.cost")
                )
            }

            let structured = structuredIndex.snapshot(now: now)
            var availableSourceIds = structured.availableToolIds
            if FileManager.default.fileExists(atPath: dimAgentDB) { availableSourceIds.insert("dim") }
            if FileManager.default.fileExists(atPath: openCodeDB) { availableSourceIds.insert("opencode") }

            let result = TokenTimelineBuilder.build(
                records: dimRecords + openCodeRecords + structured.records,
                range: range,
                now: now,
                supportedSourceIds: configuredToolIds,
                availableSourceIds: availableSourceIds
            )
            Task { @MainActor in completion(result) }
        }
    }

    // MARK: - 24h/累计 汇总查询

    private func queryDimAgent(cutoffISO: String) -> TokenUsage? {
        // createdAt 是 ISO8601 UTC 字符串（同格式字符串比较即时间比较）；cost 全表 SUM（NULL 记 0）
        //
        // 一趟同时出「24h」与「累计」两个口径。此前是两条独立查询：usage_ledger 上没有
        // createdAt 打头的索引（只有 (sessionId, createdAt, ledgerId)），两条都是全表 SCAN，
        // 而净 token 表达式要逐行 json_extract(usage)——同一遍表扫两次、同一批 JSON 解两遍。
        // 实测（本机、只读、prepare 后 min-of-20 次 step）：1,208 行的真实 dimcode.sqlite
        // 两趟 0.05 + 0.60 = 0.65ms、合并 0.65ms（打平，省下的那次全表扫被逐行 CASE 比
        // 较抵消）；把同一张表按行放大到 77,312 行（44MB）后两趟 8.24 + 44.26 = 52.5ms、
        // 合并 48.1ms（-8%，四列输出与两趟逐项相同）。省的是一遍全表扫描，随行数线性增长。
        // 等价性（逐条对照原式，真实库与夹具库均已对拍）：
        // · 子查询把每行净 token 先落成标量 t，两个口径只是对同一批 t 分别「全表求和」与
        //   「CASE 过滤求和」，加数集合与原两条查询一致；
        // · completionTokens 缺失时原式靠 SUM 跳过 NULL，此处显式 COALESCE 成 0——同为加 0；
        // · cost 为 NULL 时同理（COALESCE(cost,0)）；空表 / 窗口内无行都经 COALESCE 归 0；
        // · MAX(a,b) 是标量 max（任一参数 NULL 即返回 NULL），两个参数都已 COALESCE 故不为 NULL。
        let sql = """
        SELECT COALESCE(SUM(t),0),
               COALESCE(SUM(c),0),
               COALESCE(SUM(CASE WHEN createdAt >= ? THEN t ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN createdAt >= ? THEN c ELSE 0 END),0)
        FROM (
            SELECT createdAt,
                   COALESCE(cost,0) AS c,
                   MAX(COALESCE(json_extract(usage,'$.promptTokens'),0)
                     - COALESCE(json_extract(usage,'$.cacheReadTokens'),0), 0)
                     + COALESCE(json_extract(usage,'$.completionTokens'),0) AS t
            FROM usage_ledger
        )
        """
        guard let row = rawRows(sql, dbPath: dimAgentDB, cols: 4, textParams: [cutoffISO, cutoffISO]).first else {
            return nil
        }
        return TokenUsage(tokens24h: Self.tokenColumn(row[2], source: "dim.24h.tokens"),
                          tokensTotal: Self.tokenColumn(row[0], source: "dim.total.tokens"),
                          cost24h: Self.costColumn(row[3], source: "dim.24h.cost"),
                          costTotal: Self.costColumn(row[1], source: "dim.total.cost"))
    }

    private func queryOpenCode(cutoffMs: Int64) -> TokenUsage? {
        // 与 dim 同构：单趟出两个口径。role 过滤后逐行 json_extract(data) 三次，
        // 两条查询等于把这些 JSON 解两遍。实测（本机 opencode.db 缺失，故用与
        // TokenFixture 同 schema 的夹具库对拍：数值逐项一致，见「SQLite 单趟汇总」用例）。
        // cutoffMs 是饱和后的 Int64 字面量（无注入面），24h 条件用 CASE 复用同一次扫描。
        // 净 token 沿用原口径：input+output+reasoning 三项直加（cache.read 不参与）。
        let sql = """
        SELECT COALESCE(SUM(t),0),
               COALESCE(SUM(c),0),
               COALESCE(SUM(CASE WHEN time_created >= \(cutoffMs) THEN t ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN time_created >= \(cutoffMs) THEN c ELSE 0 END),0)
        FROM (
            SELECT time_created,
                   COALESCE(json_extract(data,'$.cost'),0) AS c,
                   COALESCE(json_extract(data,'$.tokens.input'),0)
                     + COALESCE(json_extract(data,'$.tokens.output'),0)
                     + COALESCE(json_extract(data,'$.tokens.reasoning'),0) AS t
            FROM message
            WHERE json_extract(data,'$.role')='assistant'
        )
        """
        guard let row = rawRows(sql, dbPath: openCodeDB, cols: 4).first else { return nil }
        return TokenUsage(tokens24h: Self.tokenColumn(row[2], source: "opencode.24h.tokens"),
                          tokensTotal: Self.tokenColumn(row[0], source: "opencode.total.tokens"),
                          cost24h: Self.costColumn(row[3], source: "opencode.24h.cost"),
                          costTotal: Self.costColumn(row[1], source: "opencode.total.cost"))
    }

    // MARK: - SQLite 底层


    /// 汇总列 → Int。经 Double 中转：SQLite 对 REAL 列求和会输出 "19067783.5" 这类
    /// 带小数文本，直接 Int("...") 会返回 nil 并被 ?? 0 静默归零（统计整体消失且无任何
    /// 报错）。兜底转换必须走 SafeNumber：脏数据（1e19 / Inf / NaN）在该路径上会直接 trap。
    private static func tokenColumn(_ raw: String, source: String) -> Int {
        SafeNumber.parseInt(raw, source: source)
    }

    /// 汇总列 → 金额（同 tokenColumn 的防护口径）
    private static func costColumn(_ raw: String, source: String) -> Double {
        SafeNumber.parseCost(raw, source: source)
    }

    /// 通用查询：全部列转字符串返回（数值/文本统一处理，空结果返回 []）
    /// 线程安全：所有查询在 dbQueue 串行执行，连接按路径缓存复用（只读，应用生命周期内不关闭）
    private func rawRows(_ sql: String, dbPath: String, cols: Int, textParams: [String] = []) -> [[String]] {
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

            // 一次性连接（stop 后迟到重建）用毕即关（R34/F9）。注册位置有两处讲究：
            // ① 必须在 prepare 之前——Swift 的 defer 只对「注册之后」的控制流生效，
            //    放在 prepare 之后就仍然漏掉 prepare 失败这条 return（而「一直失败」
            //    恰恰是最需要收句柄的场景）；
            // ② 必须早于下面 finalize 的 defer——defer 逆序执行，晚注册会先跑，
            //    在未 finalize 的连接上 sqlite3_close 只会返回 SQLITE_BUSY 并把连接留下，
            //    而数组已清空，等于永不重试。close_v2 再兜一层。
            defer {
                lock.lock()
                let pending = transientHandles
                transientHandles.removeAll()
                lock.unlock()
                for handle in pending { sqlite3_close_v2(handle) }
            }

            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
                AppLog.warn("TokenUsage: prepare failed: \(String(cString: sqlite3_errmsg(db)))")
                return []
            }
            defer { sqlite3_finalize(stmt) }

            // 文本参数按位绑定（1 起）：汇总查询用同一 SQL 同时出 24h 与累计两个口径，
            // cutoff 必须以参数进入——字面量拼接会把它写进 SQL 两次（且难以比对）。
            for (offset, value) in textParams.enumerated() {
                sqlite3_bind_text(stmt, Int32(offset + 1), value, -1, ReadonlyDB.transientDestructor)
            }

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
                AppLog.warn("TokenUsage: step error \(String(cString: sqlite3_errmsg(db)))")
                return []
            }
            return rows
        }
    }

    /// 只读打开并记录 inode
    private func openReadonly(_ dbPath: String) -> OpaquePointer? {
        var handle: OpaquePointer?
        // 代际必须在 open **之前**取：open 之后再读，比较的就是同一个值读两遍，
        // 恒等成立——「stop() 发生在打开期间」这一种恰恰判不出来，迟到新建的连接
        // 会被写进 dbConnections，而 closeConnectionsAsync 早已跑完，从此无人再关
        lock.lock()
        let generationBeforeOpen = dbGeneration
        lock.unlock()
        guard sqlite3_open_v2(dbPath, &handle, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let handle else {
            if let handle { sqlite3_close(handle) }
            AppLog.warn("TokenUsage: open failed \(dbPath)")
            return nil
        }
        // 降低瞬态 BUSY：只读连接遇到写锁立即返回 BUSY，
        // 等待最多 1s 再失败，避免偶发把整次查询打成失败
        sqlite3_busy_timeout(handle, 1000)
        // 代际校验（R34/F9）：打开**期间**发生 stop() 时不得写回缓存——closeConnectionsAsync
        // 已经跑过，写回来就无人再关；一次性连接用完即关。
        // 边界如实说明：整次查询完全发生在 stop() 之后的（代际前后一致）仍会写回缓存，
        // 当前唯一的 stop 点是应用退出，fd 随进程一起没，故不再补 lastClosedGeneration 机制
        lock.lock()
        let generationAfterOpen = dbGeneration
        lock.unlock()
        if generationBeforeOpen == generationAfterOpen {
            dbConnections[dbPath] = handle
            dbInodes[dbPath] = currentInode(dbPath)
            return handle
        }
        // 已停止：返回一次性连接（调用方用完后由 rawRows 关闭）——用代际标记另行处理
        transientHandles.append(handle)
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
    private static let isoFormatterWithoutFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    static func iso24hAgo(now: Date = Date()) -> String {
        isoFormatter.string(from: now.addingTimeInterval(-86_400))
    }

    static func parseISO(_ s: String) -> Date? {
        isoFormatter.date(from: s) ?? isoFormatterWithoutFraction.date(from: s)
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

    public func refreshAsync() { calls.append("refreshAsync") }

    public func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        calls.append("modelBreakdown")   // 同步记录：调用即刻可断言；回调异步送达
        Task { @MainActor in completion([]) }
    }

    public func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        calls.append("sessions")   // 同步记录：调用即刻可断言；回调异步送达
        Task { @MainActor in completion([]) }
    }

    public func timeline(range: TokenTimeRange, now: Date,
                         completion: @escaping @MainActor (TokenUsageTimeline) -> Void) {
        calls.append("timeline")
        Task { @MainActor in completion(.empty(for: range, now: now)) }
    }
}

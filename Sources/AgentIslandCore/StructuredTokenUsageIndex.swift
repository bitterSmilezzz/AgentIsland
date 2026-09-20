import Foundation

/// JSONL 工具的 Token 明细格式。索引只保留时间与计数，不保留对话内容。
enum StructuredTokenLogFormat {
    /// Codex rollout：`type=token_usage_record`，单次响应位于 payload.usage。
    case codex
    /// Claude Code / WorkBuddy：单次响应位于 message.usage。
    case anthropic
}

struct StructuredTokenSource {
    let agentId: String
    let roots: [String]
    let format: StructuredTokenLogFormat
}

struct StructuredTokenUsageSnapshot {
    let records: [TokenUsageRecord]
    let availableToolIds: Set<String>
    /// records 按 agentId 分桶；**桶内保持 records 里的原始先后顺序**，因此每个桶的累加
    /// 序列与「线性扫全表 + 按 agentId 取数」逐字一致，饱和/钳制的生效时机不变。
    let recordsByAgent: [AgentBucket]

    struct AgentBucket {
        let agentId: String
        let records: [TokenUsageRecord]
    }

    /// 24h / 累计两个口径。按桶累加而非逐条查字典：本机实测 5,778 条记录时旧写法
    /// （线性扫 + 每条一次 `result[agentId] ?? TokenUsage()` 读写）0.99ms/轮，
    /// 分桶后 0.53ms/轮，且每轮只剩每桶一次字典写入——与累计会话数无关的增长被切断。
    func usage(now: Date) -> [String: TokenUsage] {
        let cutoff = now.addingTimeInterval(-86_400)
        var result: [String: TokenUsage] = [:]
        for bucket in recordsByAgent {
            var value = TokenUsage()
            var counted = false
            for record in bucket.records where record.time <= now {
                counted = true
                value.tokensTotal = Self.safeSum(value.tokensTotal, record.tokens)
                value.costTotal = Self.safeCostSum(value.costTotal, record.cost)
                if record.time >= cutoff {
                    value.tokens24h = Self.safeSum(value.tokens24h, record.tokens)
                    value.cost24h = Self.safeCostSum(value.cost24h, record.cost)
                }
            }
            // 与线性扫一致：一条记录都没落进 `time <= now` 的工具不出现在结果里
            if counted { result[bucket.agentId] = value }
        }
        return result
    }

    private static func safeSum(_ lhs: Int, _ rhs: Int) -> Int {
        let (sum, overflow) = lhs.addingReportingOverflow(max(rhs, 0))
        if overflow || sum > SafeNumber.magnitudeCeiling { return SafeNumber.magnitudeCeiling }
        return max(sum, 0)
    }

    private static func safeCostSum(_ lhs: Double, _ rhs: Double) -> Double {
        let sum = lhs + max(rhs.isFinite ? rhs : 0, 0)
        guard sum.isFinite else { return SafeNumber.costCeiling }
        return min(sum, SafeNumber.costCeiling)
    }
}

/// 本地结构化日志的只读增量索引。
///
/// 每轮只 stat JSONL；文件未变化直接复用解析结果，避免 60s 轮询反复重读完整会话。
/// 同一响应可能因 fork/恢复出现在多个文件中，Codex response_id 和 WorkBuddy id 会全局去重。
///
/// 稳态开销的最后一道闸是「全库戳备忘录」：解析结果按文件缓存后，剩下的摊平/去重/分桶
/// 仍随**生命周期内累计会话数**线性增长（本机 5,778 条记录实测 8.2ms/轮），而绝大多数
/// 轮询里整棵日志树一个字节都没变——戳逐字相同即直接复用上一趟的快照对象。
final class StructuredTokenUsageIndex: @unchecked Sendable {
    private struct Event {
        let uniqueId: String
        let record: TokenUsageRecord
    }

    /// 单文件的变更戳（inode 防「同名文件被整体替换」，mtime+size 防原地改写）。
    /// 三个事实来自同一次 stat(2)。
    private struct FileStamp: Equatable {
        let inode: UInt64
        let modifiedAt: TimeInterval
        let size: Int
    }

    private struct CachedFile {
        let stamp: FileStamp
        let endedWithNewline: Bool
        let events: [Event]
    }

    /// 全库戳 → 已构建快照。戳逐字相同 ⇒ 各文件事件集合与顺序都不变 ⇒ 摊平、去重、
    /// 分桶的结果必然逐字相同，直接复用（数组按引用共享，不再逐元素拷贝）。
    private struct Memo {
        let stamps: [String: FileStamp]
        let snapshot: StructuredTokenUsageSnapshot
    }

    private let sources: [StructuredTokenSource]
    private let lock = NSLock()
    private var cache: [String: CachedFile] = [:]
    private var memo: Memo?

    init(sources: [StructuredTokenSource]) {
        self.sources = sources
    }

    var isEnabled: Bool { !sources.isEmpty }
    var configuredToolIds: Set<String> { Set(sources.map(\.agentId)) }

    func snapshot() -> StructuredTokenUsageSnapshot {
        lock.lock()
        defer { lock.unlock() }

        var availableToolIds = Set<String>()
        var seenCacheKeys = Set<String>()
        var stamps: [String: FileStamp] = [:]
        // 按遍历顺序持有各文件的事件**数组**（每个文件一次 append，共享底层存储）：
        // 戳未变的常态下这一趟只有这点开销，摊平与去重整套跳过
        var fileEvents: [[Event]] = []

        for source in sources {
            for root in source.roots where FileManager.default.fileExists(atPath: root) {
                availableToolIds.insert(source.agentId)
                guard let enumerator = FileManager.default.enumerator(
                    at: URL(fileURLWithPath: root, isDirectory: true),
                    includingPropertiesForKeys: nil,
                    options: [.skipsHiddenFiles, .skipsPackageDescendants]
                ) else { continue }

                for case let url as URL in enumerator where url.pathExtension.lowercased() == "jsonl" {
                    // 一次 stat(2) 拿齐 (普通文件, mtime, size, inode)。此前这里先
                    // `resourceValues` 取 mtime/size、再 `attributesOfItem` 取 inode——
                    // 同一条元数据付两遍：本机实测 attributesOfItem 单次 26µs（为一次比对
                    // 组装 NSDictionary/NSNumber），stat(2) 只要 0.6µs；且 resourceValues
                    // 还有毫秒级陈旧窗口（同一问题曾咬到尾读备忘，见 LogTailReader 开头注释）。
                    guard let facts = LogTailReader.statRegularFile(url.path) else { continue }
                    let cacheKey = source.agentId + "|" + url.path
                    let stamp = FileStamp(inode: facts.inode,
                                          modifiedAt: facts.mtime.timeIntervalSince1970,
                                          size: Int(truncatingIfNeeded: facts.size))
                    seenCacheKeys.insert(cacheKey)

                    // stamps[键] 一律记「**实际贡献了事件的那份 CachedFile 的戳**」——
                    // 备忘录只在「每个贡献者的戳都没变」时才命中，这样「上次读失败、
                    // 这次同一 mtime/size 下读成功」不会被误判成没变（那正是统计静默
                    // 丢一整个文件的场景）。
                    if let cached = cache[cacheKey], cached.stamp == stamp {
                        stamps[cacheKey] = cached.stamp
                        fileEvents.append(cached.events)
                        continue
                    }

                    let old = cache[cacheKey]
                    let canAppend = old?.stamp.inode == stamp.inode
                        && old?.endedWithNewline == true && stamp.size > (old?.stamp.size ?? 0)
                    let offset = canAppend ? (old?.stamp.size ?? 0) : 0
                    if let parsed = Self.parse(url: url, source: source, offset: offset,
                                               limit: stamp.size) {
                        // 戳里的 size 换成**本轮真正承认过的字节数**：stat 与读之间文件被原地
                        // 截断时，缓存虚高的 size 会让下一轮的 offset 落在从未解析的字节之后
                        // （与修掉的「重复计数」正好对称的另一侧）
                        let consumed = parsed.consumedThrough
                        let settled = FileStamp(inode: stamp.inode,
                                                modifiedAt: stamp.modifiedAt,
                                                size: consumed)
                        let events = canAppend ? (old?.events ?? []) + parsed.events : parsed.events
                        cache[cacheKey] = CachedFile(stamp: settled,
                                                     endedWithNewline: parsed.endedWithNewline,
                                                     events: events)
                        stamps[cacheKey] = settled
                        fileEvents.append(events)
                    } else if let cached = old {
                        // 文件正被写入或瞬时不可读时沿用上一份成功结果，避免统计闪回零。
                        // 戳沿用旧那份（而不是本次观测到的新戳）→ 下一趟仍会重试解析
                        stamps[cacheKey] = cached.stamp
                        fileEvents.append(cached.events)
                    }
                }
            }
        }

        cache = cache.filter { seenCacheKeys.contains($0.key) }

        // 稳态：整棵日志树一个字节都没变（绝大多数轮询都是这一支）。
        // availableToolIds 也要比：某个 root 里一个 jsonl 都没有时，戳集合会保持不变，
        // 而活性集合却会随该 root 出现/消失而变。
        if let memo, memo.stamps == stamps, memo.snapshot.availableToolIds == availableToolIds {
            return memo.snapshot
        }

        var uniqueIds = Set<String>()
        var records: [TokenUsageRecord] = []
        var agentOrder: [String] = []
        var byAgent: [String: [TokenUsageRecord]] = [:]
        for events in fileEvents {
            for event in events where uniqueIds.insert(event.uniqueId).inserted {
                records.append(event.record)
                if byAgent[event.record.agentId] == nil { agentOrder.append(event.record.agentId) }
                byAgent[event.record.agentId, default: []].append(event.record)
            }
        }
        var buckets: [StructuredTokenUsageSnapshot.AgentBucket] = []
        buckets.reserveCapacity(agentOrder.count)
        for agentId in agentOrder {
            buckets.append(StructuredTokenUsageSnapshot.AgentBucket(
                agentId: agentId, records: byAgent[agentId] ?? []))
        }
        let snapshot = StructuredTokenUsageSnapshot(records: records,
                                                    availableToolIds: availableToolIds,
                                                    recordsByAgent: buckets)
        memo = Memo(stamps: stamps, snapshot: snapshot)
        return snapshot
    }

    /// 解析 `[offset, limit)` 这一段，并回吐真正承认到的位置 `consumedThrough`。
    /// `limit` 必须是**扫描时量到的那个 size**：本函数在 stat 之后才 mmap 读文件，
    /// 期间第三方还能继续追加。若按读到的实际长度解析、却把旧的较小 size 记进缓存，
    /// 下一轮的 offset 就落在已解析过的字节上——而没有 eventId 的行，其去重键
    /// `path#offset-lineIndex` 恰随 offset 一起外移，去重失效、Token 静默膨胀。
    private static func parse(url: URL, source: StructuredTokenSource, offset: Int, limit: Int)
        -> (events: [Event], endedWithNewline: Bool, consumedThrough: Int)? {
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        let rawEnd = min(limit, data.count)
        guard offset >= 0, offset <= rawEnd else { return nil }
        // 只承认到最后一个完整行。段尾截在半行上会让 endedWithNewline=false，
        // 于是下一轮 canAppend 不成立 → offset 归零 → 整文件重解析：
        // 正在追加的日志里这几乎每轮都发生，比收口之前更慢。
        // 回退上限与下面「跳过 >1MB 巨行」的口径一致
        var end = rawEnd
        var backed = 0
        while end > offset && data[data.startIndex + end - 1] != 0x0A {
            end -= 1
            backed += 1
            if backed > 1_000_000 { end = rawEnd; break }
        }
        var events: [Event] = []
        let relevantMarker = source.format == .codex
            ? Data("token_usage_record".utf8)
            : Data("\"usage\"".utf8)
        // dropFirst/prefix 是切片，不拷贝：subdata 会把 mmap 段整体搬到堆上，
        // 正好抵消上面 .mappedIfSafe 的意图
        for (lineIndex, line) in data.dropFirst(offset).prefix(end - offset)
            .split(separator: 0x0A, omittingEmptySubsequences: true).enumerated() {
            // 对话正文可能极长；Token 记录本身很小，跳过异常巨型行避免临时解析峰值。
            let lineData = Data(line)
            guard line.count <= 1_000_000, lineData.range(of: relevantMarker) != nil,
                  let object = try? JSONSerialization.jsonObject(with: lineData),
                  let json = object as? [String: Any],
                  let parsed = parse(json: json, format: source.format) else { continue }

            let fallbackId = url.path + "#\(offset)-\(lineIndex)"
            events.append(Event(
                uniqueId: source.agentId + "|" + (parsed.eventId ?? fallbackId),
                record: TokenUsageRecord(
                    agentId: source.agentId,
                    time: parsed.time,
                    tokens: parsed.tokens,
                    cost: 0
                )
            ))
        }
        return (events, end > 0 && data[data.startIndex + end - 1] == 0x0A, end)
    }

    private static func parse(json: [String: Any], format: StructuredTokenLogFormat)
        -> (eventId: String?, time: Date, tokens: Int)? {
        switch format {
        case .codex:
            guard json["type"] as? String == "token_usage_record",
                  let payload = json["payload"] as? [String: Any],
                  let usage = payload["usage"] as? [String: Any],
                  let time = date(json["timestamp"]) else { return nil }
            let tokens = netTokens(
                usage: usage,
                cacheKey: "cached_input_tokens",
                source: "codex.jsonl"
            )
            guard tokens > 0 else { return nil }
            return (payload["response_id"] as? String, time, tokens)

        case .anthropic:
            guard let message = json["message"] as? [String: Any],
                  let usage = message["usage"] as? [String: Any],
                  let time = date(json["timestamp"]) else { return nil }
            let tokens = netTokens(
                usage: usage,
                cacheKey: "cache_read_input_tokens",
                source: "anthropic.jsonl"
            )
            guard tokens > 0 else { return nil }
            return ((json["id"] as? String) ?? (json["uuid"] as? String), time, tokens)
        }
    }

    /// 与 SQLite 源统一：净消耗 = max(input-cacheRead, 0) + output。
    /// Codex 的 output_tokens 已包含 reasoning token，不再重复相加。
    private static func netTokens(usage: [String: Any], cacheKey: String, source: String) -> Int {
        let input = number(usage["input_tokens"], source: source + ".input")
        let cached = number(usage[cacheKey], source: source + ".cache")
        let output = number(usage["output_tokens"], source: source + ".output")
        let uncachedInput = max(input - min(cached, input), 0)
        let (sum, overflow) = uncachedInput.addingReportingOverflow(max(output, 0))
        if overflow || sum > SafeNumber.magnitudeCeiling { return SafeNumber.magnitudeCeiling }
        return max(sum, 0)
    }

    private static func number(_ value: Any?, source: String) -> Int {
        if let number = value as? NSNumber {
            return SafeNumber.saturatingInt(number.doubleValue, source: source)
        }
        if let string = value as? String {
            return SafeNumber.parseInt(string, source: source)
        }
        return 0
    }

    private static func date(_ value: Any?) -> Date? {
        if let string = value as? String { return TokenUsageMonitor.parseISO(string) }
        guard let number = value as? NSNumber else { return nil }
        let raw = number.doubleValue
        guard raw.isFinite, raw > 0 else { return nil }
        let seconds = raw > 100_000_000_000 ? raw / 1_000 : raw
        guard seconds < 100_000_000_000 else { return nil }
        return Date(timeIntervalSince1970: seconds)
    }
}

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
    /// 保留窗口内的明细。掉出窗口（或超出单文件上限）的响应不在这里，而在
    /// `rolledUpTokens` 里——时间分析页最宽只看 30 天，多出来的历史只服务「累计」。
    let records: [TokenUsageRecord]
    let availableToolIds: Set<String>
    /// records 按 agentId 分桶；**桶内保持 records 里的原始先后顺序**，因此每个桶的累加
    /// 序列与「线性扫全表 + 按 agentId 取数」逐字一致，饱和/钳制的生效时机不变。
    /// 只剩折入记录的工具也会出现（桶内 records 为空），否则它的累计会凭空消失。
    let recordsByAgent: [AgentBucket]
    /// 折出明细的历史合计（agentId → 净 token / 响应条数）。**不是「数据不全」的免责
    /// 声明**：这些 token 照旧计入 `usage(now:)` 的累计口径，只是不再逐条留存、不再能
    /// 按天铺开。
    let rolledUpTokens: [String: Int]
    let rolledUpCount: [String: Int]

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
            // 折入的历史只加进累计：折入线在 70 天前，24h 窗口不可能落到它里面。
            if let rolled = rolledUpTokens[bucket.agentId] {
                counted = true
                value.tokensTotal = Self.safeSum(value.tokensTotal, rolled)
            }
            // 与线性扫一致：既没记录落进 `time <= now`、也没折入历史的工具不出现在结果里
            if counted { result[bucket.agentId] = value }
        }
        return result
    }

    static func safeSum(_ lhs: Int, _ rhs: Int) -> Int {
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
/// 稳态开销的第一道闸是「全库戳备忘录」：解析结果按文件缓存后，剩下的摊平/去重/分桶
/// 仍随**生命周期内累计会话数**线性增长（本机 5,778 条记录实测 8.2ms/轮），而绝大多数
/// 轮询里整棵日志树一个字节都没变——戳逐字相同即直接复用上一趟的快照对象。
///
/// 第二道闸是**保留窗口**（`detailRetention`），切断备忘录管不到的那两半：备忘录只在
/// 什么都没变时命中，而**任何一次写入**都要重摊平整棵历史树；留存数组也只增不减。实测
/// （release、合成语料、`phys_footprint` 差值口径）每条留存明细约 780B 常驻、整趟重建约
/// 0.63ms/千条，两者都随磁盘语料总量线性上升且永不回落；折进合计的那部分每条 ≈0B。
/// 掉出窗口的响应就折成按工具的合计：累计口径逐字不变（折入是保和的），每轮成本从
/// 「历史总量」变成「近 70 天数量」。取舍与被否决的方案见 `docs/adr/0007-*.md`。
final class StructuredTokenUsageIndex: @unchecked Sendable {
    /// 明细保留窗口。**70 天不是拍的**：分析页最宽 30 天，而那一档还要往前读同样长的
    /// 「上一周期」做对比（`TokenTimelineBuilder.previousStart = now - 2 × duration`），
    /// SQLite 那两个源本来就能查到 60 天前——JSONL 这边若只留 30 天，「较上一周期 ±N%」
    /// 就会被静默少算一块，而图表上一点看不出来（图只画 30 天）。所以取
    /// 2×最宽档 + 10 天余量（余量免得刚好掉出边界的明细被反复折出/折回，折回要重读
    /// 整份文件）。动这个数之前先看用例「保留窗口必须盖住最宽分析档的「上一周期」」。
    static let detailRetention: TimeInterval = 70 * 86_400

    /// 单文件明细条数硬上限：一个会话文件在窗口内挤出几十万条响应时的第二道保险。
    /// 触顶时折掉该文件**最早**的明细——累计不受影响，代价是最宽分析窗口可能少画一段。
    static let maxDetailEventsPerFile = 20_000

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
        /// 窗口内的明细，按解析顺序
        let events: [Event]
        /// 本文件已折出明细的那部分：净 token 合计与响应条数。同一份文件里被抄了两遍
        /// 的响应（fork/恢复会这么做）在折入时按 uniqueId 去过重，所以这份合计不会把
        /// 同一响应的两份拷贝各算一次。
        let rolledTokens: Int
        let rolledCount: Int
        /// 留存明细里最新一条的时间。戳未变的复用分支靠它一次比较决定「整份折掉」，
        /// 不必再遍历数组——长期不动的会话正是留存存量的大头。
        let newestKeptTime: TimeInterval?
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

    /// `now` 决定保留窗口的位置。由调用方（`refresh(now:)`、时间线查询）传入与本轮其他
    /// 口径同一个时间边界；测试也由此能把窗口钉死。
    func snapshot(now: Date = Date()) -> StructuredTokenUsageSnapshot {
        lock.lock()
        defer { lock.unlock() }

        let cutoffTime = now.timeIntervalSince1970 - Self.detailRetention
        var availableToolIds = Set<String>()
        var seenCacheKeys = Set<String>()
        var stamps: [String: FileStamp] = [:]
        // 按遍历顺序持有各文件的事件**数组**（每个文件一次 append，共享底层存储）：
        // 戳未变的常态下这一趟只有这点开销，摊平与去重整套跳过
        var fileEvents: [[Event]] = []
        var rolledTokens: [String: Int] = [:]
        var rolledCount: [String: Int] = [:]
        // 本趟把某个文件的明细折成了合计 ⇒ 备忘录里那份的明细已经偏多，不能继续用
        var collapsed = false

        func account(agentId: String, tokens: Int, count: Int) {
            if tokens != 0 {
                rolledTokens[agentId] = StructuredTokenUsageSnapshot.safeSum(
                    rolledTokens[agentId] ?? 0, tokens)
            }
            if count != 0 {
                let (sum, overflow) = (rolledCount[agentId] ?? 0).addingReportingOverflow(count)
                rolledCount[agentId] = overflow ? Int.max : sum
            }
        }

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
                        let expired = cached.newestKeptTime.map { $0 < cutoffTime } ?? false
                        if !expired {
                            account(agentId: source.agentId,
                                    tokens: cached.rolledTokens, count: cached.rolledCount)
                            stamps[cacheKey] = cached.stamp
                            fileEvents.append(cached.events)
                            continue
                        }
                        // 明细整体掉出窗口 ⇒ 就地折成合计，数组当场释放。这一步与「文件
                        // 有没有被写过」无关，是长期不动的会话能被回收的原因。
                        //
                        // 只在**这个文件还没折过任何东西**时就地折：折入的去重看得见
                        // 传给它的那些事件，看不见的恰好是先前已经折掉的那一段键。真折过
                        // 一段（老会话被重新追加、或上一轮被上限裁过），就只能走下面的整份
                        // 重读——那才是重新看得见这个文件全部内容的姿势。
                        if cached.rolledTokens == 0 {
                            let folded = Self.fold(events: cached.events, carryOver: [],
                                                   cutoffTime: cutoffTime)
                            let entry = CachedFile(stamp: cached.stamp,
                                                   endedWithNewline: cached.endedWithNewline,
                                                   events: folded.kept,
                                                   rolledTokens: folded.tokens,
                                                   rolledCount: folded.count,
                                                   newestKeptTime: folded.newestKept)
                            cache[cacheKey] = entry
                            stamps[cacheKey] = entry.stamp
                            fileEvents.append(entry.events)
                            account(agentId: source.agentId,
                                    tokens: entry.rolledTokens, count: entry.rolledCount)
                            if entry.events.count < cached.events.count { collapsed = true }
                            continue
                        }
                    }

                    let old = cache[cacheKey]
                    // 增量条件里带上「没折过任何东西」，理由同上：`fold` 的去重只看得见
                    // 传给它的事件，已经折掉的那一段键不在其中。这条条件是**载荷性**的——
                    // 下面的折入结果直接顶掉旧合计、不做任何搬移，删掉它等于把折掉的历史
                    // 从合计里抹掉（变异验证过：三个用例同时变红）。
                    // 代价落在罕见路径上：跨 70 天还在写的会话文件、以及被上限裁过的文件。
                    let canAppend = (old?.rolledTokens ?? 0) == 0
                        && old?.stamp.inode == stamp.inode
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
                        // 增量路径带上上一轮留下的明细一起折：两段合起来才是这个文件全部的
                        // 已知事件，同一响应被抄了两遍时只有第一份会被算进去。整份重解析
                        // （!canAppend）看得见整个文件，carryOver 留空即可。
                        // 这里不需要把上一轮的折入合计搬过来：有折入的文件走不了增量。
                        let folded = Self.fold(events: parsed.events,
                                               carryOver: canAppend ? (old?.events ?? []) : [],
                                               cutoffTime: cutoffTime)
                        let entry = CachedFile(stamp: settled,
                                               endedWithNewline: parsed.endedWithNewline,
                                               events: folded.kept,
                                               rolledTokens: folded.tokens,
                                               rolledCount: folded.count,
                                               newestKeptTime: folded.newestKept)
                        cache[cacheKey] = entry
                        stamps[cacheKey] = settled
                        fileEvents.append(entry.events)
                        account(agentId: source.agentId,
                                tokens: entry.rolledTokens, count: entry.rolledCount)
                    } else if let cached = old {
                        // 文件正被写入或瞬时不可读时沿用上一份成功结果，避免统计闪回零。
                        // 戳沿用旧那份（而不是本次观测到的新戳）→ 下一趟仍会重试解析
                        stamps[cacheKey] = cached.stamp
                        fileEvents.append(cached.events)
                        account(agentId: source.agentId,
                                tokens: cached.rolledTokens, count: cached.rolledCount)
                    }
                }
            }
        }

        cache = cache.filter { seenCacheKeys.contains($0.key) }

        // 稳态：整棵日志树一个字节都没变（绝大多数轮询都是这一支）。
        // availableToolIds 也要比：某个 root 里一个 jsonl 都没有时，戳集合会保持不变，
        // 而活性集合却会随该 root 出现/消失而变。数字上备忘录可以容忍刚折过一次的明细
        // （折入保和），但那份快照仍握着本趟刚刚释放的数组，所以 collapsed 时作废旧账。
        if let memo, !collapsed, memo.stamps == stamps,
           memo.snapshot.availableToolIds == availableToolIds {
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
        // 明细全被折掉的工具也要有桶，否则它的累计无处安放
        for agentId in rolledTokens.keys.sorted() where byAgent[agentId] == nil {
            agentOrder.append(agentId)
        }
        var buckets: [StructuredTokenUsageSnapshot.AgentBucket] = []
        buckets.reserveCapacity(agentOrder.count)
        for agentId in agentOrder {
            buckets.append(StructuredTokenUsageSnapshot.AgentBucket(
                agentId: agentId, records: byAgent[agentId] ?? []))
        }
        let snapshot = StructuredTokenUsageSnapshot(records: records,
                                                    availableToolIds: availableToolIds,
                                                    recordsByAgent: buckets,
                                                    rolledUpTokens: rolledTokens,
                                                    rolledUpCount: rolledCount)
        memo = Memo(stamps: stamps, snapshot: snapshot)
        return snapshot
    }

    // MARK: - 保留窗口

    /// 按保留窗口切分一个文件的事件：`time < cutoff` 的折进合计，其余留在明细里；
    /// 明细超过 `maxDetailEventsPerFile` 时把最早的折掉。
    ///
    /// **调用方必须把这个文件当前全部已知事件喂进来**（`carryOver` 带着上一轮留下的明细，
    /// 或者干脆整份重读）。去重看得见参数里的每一条，看不见的正是已经折掉的那一段键——
    /// 少喂一段，同一响应被再抄一遍时就会各算一次。带折入历史的文件因此走不了增量。
    private static func fold(events: [Event], carryOver: [Event], cutoffTime: TimeInterval)
        -> (kept: [Event], tokens: Int, count: Int, newestKept: TimeInterval?) {
        var kept: [Event] = []
        var seen: Set<String> = []
        var tokens = 0
        var count = 0
        var newestKept: TimeInterval?
        kept.reserveCapacity(events.count)

        func keep(_ event: Event) {
            kept.append(event)
            let time = event.record.time.timeIntervalSince1970
            if newestKept == nil || time > newestKept! { newestKept = time }
        }
        for event in carryOver + events {
            guard seen.insert(event.uniqueId).inserted else { continue }
            if event.record.time.timeIntervalSince1970 < cutoffTime {
                tokens = StructuredTokenUsageSnapshot.safeSum(tokens, event.record.tokens)
                count += 1
            } else {
                keep(event)
            }
        }
        // 上限只裁明细、不裁累计：折掉最早的，留住最近那一段（图表用得上的部分）
        if kept.count > maxDetailEventsPerFile {
            let doomed = Set(kept.sorted { $0.record.time < $1.record.time }
                .prefix(kept.count - maxDetailEventsPerFile).map(\.uniqueId))
            var survivors: [Event] = []
            survivors.reserveCapacity(maxDetailEventsPerFile)
            for event in kept {
                if doomed.contains(event.uniqueId) {
                    tokens = StructuredTokenUsageSnapshot.safeSum(tokens, event.record.tokens)
                    count += 1
                } else {
                    survivors.append(event)
                }
            }
            kept = survivors
            newestKept = kept.map { $0.record.time.timeIntervalSince1970 }.max()
        }
        return (kept, tokens, count, newestKept)
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
            // 先在切片上找标记、命中才 `Data(line)`：日志里 99% 以上的行是对话正文
            // （本机 200MB 语料只对应 1,215 条响应），先拷贝等于把整段 mmap 逐行搬上堆。
            guard line.count <= 1_000_000, line.range(of: relevantMarker) != nil else { continue }
            let lineData = Data(line)
            guard let object = try? JSONSerialization.jsonObject(with: lineData),
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

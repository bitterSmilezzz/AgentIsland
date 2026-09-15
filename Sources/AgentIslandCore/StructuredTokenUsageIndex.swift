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

    func usage(now: Date) -> [String: TokenUsage] {
        let cutoff = now.addingTimeInterval(-86_400)
        var result: [String: TokenUsage] = [:]
        for record in records where record.time <= now {
            var value = result[record.agentId] ?? TokenUsage()
            value.tokensTotal = Self.safeSum(value.tokensTotal, record.tokens)
            value.costTotal = Self.safeCostSum(value.costTotal, record.cost)
            if record.time >= cutoff {
                value.tokens24h = Self.safeSum(value.tokens24h, record.tokens)
                value.cost24h = Self.safeCostSum(value.cost24h, record.cost)
            }
            result[record.agentId] = value
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
final class StructuredTokenUsageIndex: @unchecked Sendable {
    private struct Event {
        let uniqueId: String
        let record: TokenUsageRecord
    }

    private struct CachedFile {
        let inode: UInt64
        let modifiedAt: TimeInterval
        let size: Int
        let endedWithNewline: Bool
        let events: [Event]
    }

    private let sources: [StructuredTokenSource]
    private let lock = NSLock()
    private var cache: [String: CachedFile] = [:]

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
        var allEvents: [Event] = []

        for source in sources {
            for root in source.roots where FileManager.default.fileExists(atPath: root) {
                availableToolIds.insert(source.agentId)
                guard let enumerator = FileManager.default.enumerator(
                    at: URL(fileURLWithPath: root, isDirectory: true),
                    includingPropertiesForKeys: [.isRegularFileKey, .contentModificationDateKey, .fileSizeKey],
                    options: [.skipsHiddenFiles, .skipsPackageDescendants]
                ) else { continue }

                for case let url as URL in enumerator where url.pathExtension.lowercased() == "jsonl" {
                    guard let values = try? url.resourceValues(
                        forKeys: [.isRegularFileKey, .contentModificationDateKey, .fileSizeKey]
                    ), values.isRegularFile == true else { continue }

                    let cacheKey = source.agentId + "|" + url.path
                    seenCacheKeys.insert(cacheKey)
                    let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
                    let inode = (attributes?[.systemFileNumber] as? UInt64) ?? 0
                    let modifiedAt = values.contentModificationDate?.timeIntervalSince1970 ?? 0
                    let size = values.fileSize ?? 0
                    if let cached = cache[cacheKey],
                       cached.inode == inode, cached.modifiedAt == modifiedAt, cached.size == size {
                        allEvents.append(contentsOf: cached.events)
                        continue
                    }

                    let old = cache[cacheKey]
                    let canAppend = old?.inode == inode && old?.endedWithNewline == true && size > (old?.size ?? 0)
                    let offset = canAppend ? (old?.size ?? 0) : 0
                    if let parsed = Self.parse(url: url, source: source, offset: offset) {
                        let events = canAppend ? (old?.events ?? []) + parsed.events : parsed.events
                        cache[cacheKey] = CachedFile(
                            inode: inode,
                            modifiedAt: modifiedAt,
                            size: size,
                            endedWithNewline: parsed.endedWithNewline,
                            events: events
                        )
                        allEvents.append(contentsOf: events)
                    } else if let cached = cache[cacheKey] {
                        // 文件正被写入或瞬时不可读时沿用上一份成功结果，避免统计闪回零。
                        allEvents.append(contentsOf: cached.events)
                    }
                }
            }
        }

        cache = cache.filter { seenCacheKeys.contains($0.key) }

        var uniqueIds = Set<String>()
        let records = allEvents.compactMap { event -> TokenUsageRecord? in
            guard uniqueIds.insert(event.uniqueId).inserted else { return nil }
            return event.record
        }
        return StructuredTokenUsageSnapshot(records: records, availableToolIds: availableToolIds)
    }

    private static func parse(url: URL, source: StructuredTokenSource, offset: Int)
        -> (events: [Event], endedWithNewline: Bool)? {
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        guard offset >= 0, offset <= data.count else { return nil }
        var events: [Event] = []
        let relevantMarker = source.format == .codex
            ? Data("token_usage_record".utf8)
            : Data("\"usage\"".utf8)
        for (lineIndex, line) in data.dropFirst(offset)
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
        return (events, data.last == 0x0A)
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

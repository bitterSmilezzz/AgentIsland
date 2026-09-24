import Foundation
import Security

// MARK: - 可信自报通道（02 号票 / spec 第 3 节）
//
// 岛此前只知道「这个进程在跑、CPU 高、日志刚写过」，状态是**推断**出来的。自报通道让
// Agent 自己说「我在等什么」，代价是要先把「谁在说话」这件事办实：
// 一条没有出处的申报一旦被采信，就比推断更糟——它会盖掉进程表本来能证明的事实。
// 所以这里的每个判定都是**拒绝方向**的：验不了就不采信，而不是「大概率是真的」。
//
// 三条口径（都写在 spec 第 3 节，落在这里而不是调用方，避免出现第二份判断）：
// 1. 未知 agent id 一律拒收，**绝不自动建档**（建档等于让外部进程往注册表里塞东西）；
// 2. 可信度只由令牌建立。顺序是先令牌后 pid——pid 匹配不能替代令牌；
// 3. TTL 到期只是**退回推断**：记录不删、卡片不消失，多盖一个 `selfReportExpired` 的戳。
//    「过期即清空」会把「它刚才还在说自己在等确认」这条证据抹掉，而那是用户判断的关键。

// MARK: - 状态

/// 自报的状态。`idle` 与 `completed` 不同义：前者是「活着、没活」，后者是「这一轮结束了」。
/// 混成一个值会让卡片在任务结束后一直显示工作态（推断侧就吃过这个亏）。
public enum SelfReportState: String, Codable, CaseIterable, Equatable {
    case working
    case attention
    case completed
    case idle
}

// MARK: - 载荷：两套形状都收
//
// 01 号票核实过：Qoder / Claude Code / Codex 的 http hook 会把事件**原样** POST 到配置里
// 的 URL，字段是 `session_id` / `hook_event_name` / `agent_id` 那一套，而不是我们自己定义的
// `{agent, session, state}`。若 `/session` 只认后者，「配一条 http hook 即接入」这条最省的路
// 要用不上，得退化成再写一个桥接脚本。

/// 一条归一化之后的申报。`ttl` 已钳进 `[15, 600]`，`detail`/`ask` 已按上限截断——
/// 截断而不是拒绝：超限属于「说太多」，不是「说假话」。
public struct SelfReportSubmission: Equatable {
    public let agentID: String
    public let sessionID: String
    public let pid: Int32?
    public let state: SelfReportState
    public let detail: String?
    public let ask: String?
    public let ttl: TimeInterval

    public init(agentID: String, sessionID: String, pid: Int32?, state: SelfReportState,
                detail: String?, ask: String?, ttl: TimeInterval) {
        self.agentID = agentID
        self.sessionID = sessionID
        self.pid = pid
        self.state = state
        self.detail = detail
        self.ask = ask
        self.ttl = ttl
    }

    /// 上限来自 spec：detail 进「正在做什么」那一行，再长就是往岛上灌文本
    public static let detailLimit = 200
    public static let askLimit = 200
    public static let ttlRange: ClosedRange<TimeInterval> = 15...600
    public static let defaultTTL: TimeInterval = 90
}

public enum SelfReportRejection: String, Equatable {
    /// 缺 `agent`/`session`，或 JSON 根本解不开
    case malformed
    /// §3：未知档案 id ⇒ 不自动建档。**带令牌时**才回 400；不带令牌时这条与 `malformed`
    /// 一律收敛成同一个 `noToken` 响应，否则 `/session` 就成了「这台机器装了哪些 Agent」
    /// 的免费枚举口（macOS 13 那条构建路径下端口对局域网可达）
    case unknownAgent
    /// `state` 不在四个值里，也不是该档案已知的 hook 事件名
    case unknownState
}

/// `identity()` 的失败形态。Result 的 Failure 必须是 Error，
/// 而这里想带的东西和 rejection 一样：一个枚举原因 + 一句人读的说明
public struct SelfReportParseFailure: Error, Equatable {
    public let reason: SelfReportRejection
    public let detail: String
    public init(reason: SelfReportRejection, detail: String) {
        self.reason = reason
        self.detail = detail
    }
}

/// `identity` 与 `parse` 之间传的中间形状：坐标 + 原始 JSON。刻意**不**公开——
/// 公开出去就是第二条「自己从正文里读字段」的路，会绕开 `state`/`detail`/`ttl` 的归一化
struct SelfReportRawPayload {
    let identity: SelfReportIdentity
    let json: [String: Any]
}

public enum SelfReportOutcome: Equatable {
    case accepted(SelfReportSubmission)
    case rejected(SelfReportRejection, detail: String)
}

/// 各家 hook 的事件名 → 我们的状态。
///
/// **是一张 allowlist，不是一张全局表**——这是 01 号票口径（「宁缺毋滥：认不出就不收」）
/// 的直接落地。第一版写成「全局表 + 一条 deny」，于是每接入一家都在**默认放行 Claude 的语义**：
/// Trae 的 `Notification` 一个事件同时覆盖「等确认」与「任务完成」（正文注明 triggered
/// asynchronously），套上全局表就把「任务完成」升格成「要你确认」——凭空一个假警报，
/// 而它比不映射更糟，因为它顶着可信自报的头衔。Cline / Cursor 的 `Notification` 语义本轮
/// 同样没逐字核实，所以它们也不该在表里。查不到 ⇒ 400 `unknownState`，接入方看得见。
public enum SelfReportEventMapping {
    /// PascalCase 那一族事件名。三家同构是 01 号票逐字核过的，别的档案不许套用
    static let shared: [String: SelfReportState] = [
        "SessionStart": .working,
        "UserPromptSubmit": .working,
        "PreToolUse": .working,
        "PostToolUse": .working,
        "Notification": .attention,
        "Stop": .completed,
        "SessionEnd": .completed,
    ]
    /// 允许套用 `shared` 的档案。`SubagentStop` **不在** `shared` 里：那条事件的
    /// `session_id` 是**主会话**的（按各家 hook 的通用形状推，本轮没有实样 ⇒ 研究清单待核实），
    /// 映射成 `.completed` 等于把还在等子代理结果的父会话标成「这一轮结束了」——
    /// 又是凭空造出一个终态。没核实过就不收。
    static let allowlist: [String: [String: SelfReportState]] = [
        "claude": shared, "codex": shared, "qoder": shared
    ]

    public static func state(hookEventName raw: String, profileID: String) -> SelfReportState? {
        let name = raw.trimmingCharacters(in: .whitespaces)
        if let hit = allowlist[profileID]?[name] { return hit }
        // OpenCode 走 plugin 事件总线，事件名是 `session.idle` 这一族小写点分名。
        // 这一族不按档案分家：名字自己就把状态说了（idle 就是 idle），
        // 而 PascalCase 那一族的同名事件在不同厂商下语义不同——分家的理由只适用后者。
        switch name.lowercased() {
        case "session.idle": return .idle
        case "session.error": return .attention
        default: return nil
        }
    }
}

/// 撤销只需要「哪条申报」这两个坐标，不需要 state——把 state 也要求齐，
/// 等于让 DELETE 必须先重发一遍声明（实测第一次接就是这样：撤销回了 unknownState）
public struct SelfReportIdentity: Equatable {
    public let agentID: String
    public let sessionID: String
    public init(agentID: String, sessionID: String) {
        self.agentID = agentID
        self.sessionID = sessionID
    }
}

public enum SelfReportPayload {
    /// 解析 `/session` 的请求体。`knownAgentIDs` 由调用方给出**当前启用的**注册表，
    /// 因为「未知档案」是业务判定，不该由解析器自己查内置表（那会绕过用户的自定义档案，
    /// 也会让 `AgentRegistry.profile(id:)` 那条「勿当通用入口」的警告成真）。
    ///
    /// `queryAgent` 是 URL 上带的那个 `?agent=`：hook 的 http 配置**只会原样 POST 事件正文**，
    /// 主会话的 payload 里没有我们的档案 id（`agent_id` 只在子代理场景出现）。不带这个口子，
    /// 「配一条 http hook 即接入」就接不进来——而那条路径是 01 号票里最省的一笔。
    /// 它不降低任何要求：仍然要在注册表里，仍然不许自动建档。
    public static func parse(_ data: Data, knownAgentIDs: Set<String>, queryAgent: String? = nil)
        -> SelfReportOutcome {
        switch identityAndJSON(data, knownAgentIDs: knownAgentIDs, queryAgent: queryAgent) {
        case .failure(let f): return .rejected(f.reason, detail: f.detail)
        case .success(let raw):
            let id = raw.identity
            let json = raw.json
            var state: SelfReportState?
            if let raw = name(json["state"]) {
                state = SelfReportState(rawValue: raw.lowercased())
                    ?? SelfReportEventMapping.state(hookEventName: raw, profileID: id.agentID)
            } else if let hook = name(json["hook_event_name"]) {
                state = SelfReportEventMapping.state(hookEventName: hook, profileID: id.agentID)
            }
            guard let resolved = state else {
                return .rejected(.unknownState,
                                 detail: "state 既不是 working/attention/completed/idle，也不是该档案已知的 hook 事件名")
            }
            // `pid` 是「我声称自己是这个进程」的**可选**主张。声称了却读不出来的那些形态
            // （字符串带空格、布尔、对象、数组、越界整数）第一版一律当成「没给」，
            // 于是整条冒名顶替防护被静默跳过——这与 `SelfReport.swift` 开头立的规矩
            // （验不了就不采信）相反。现在：给了就必须是一个能用的进程号。
            var pid: Int32?
            let pidRaw = json["pid"]
            if pidRaw != nil && !(pidRaw is NSNull) {
                guard let claimed = SelfReportPayload.parsedPID(pidRaw) else {
                    return .rejected(.malformed,
                                     detail: "pid 给了就必须是一个 (0, \(Int32.max)) 内的正整数进程号，"
                                         + "或干脆不要带这个字段")
                }
                pid = claimed
            }
            return .accepted(SelfReportSubmission(
                agentID: id.agentID, sessionID: id.sessionID, pid: pid, state: resolved,
                detail: clamped(name(json["detail"]), SelfReportSubmission.detailLimit),
                ask: clamped(name(json["ask"]), SelfReportSubmission.askLimit),
                ttl: clampedTTL((json["ttl"] as? NSNumber)?.doubleValue
                                    ?? (json["ttl"] as? String).flatMap(Double.init))))
        }
    }

    /// JSON 里的进程号只收两种形态：数字（先查是 bool、再查范围——`4294967298` 经
    /// `NSNumber.int32Value` 会**截断成 2**，截断后的 pid 再去比对名字是靠运气）与
    /// 纯数字字符串。其余一律 nil ⇒ 调用方拒绝，而不是静默当作没提。
    static func parsedPID(_ raw: Any?) -> Int32? {
        if let n = raw as? NSNumber {
            if String(cString: n.objCType) == "c" { return nil }   // JSON 布尔值
            let v = n.int64Value
            return (v > 0 && v < Int64(Int32.max)) ? Int32(v) : nil
        }
        if let s = raw as? String,
           let v = Int32(s.trimmingCharacters(in: .whitespaces)), v > 0 { return v }
        return nil
    }

    /// 只解「哪条申报」这两个坐标。POST 与 DELETE 共用它，两条路径于是天然同口径：
    /// 档案必须存在、名字必须是字符串、URL 的 `?agent=` 只当兜底。
    /// 返回**不**带原始 JSON：那等于公开一条绕过归一化的「第二份正文读法」
    /// （`state` 不许走撤销路径、`detail` 截断、`ttl` 钳制都会被它跳开），
    /// 需要 JSON 的规范化路径走下面那个内部重载。
    public static func identity(_ data: Data, knownAgentIDs: Set<String>, queryAgent: String?)
        -> Result<SelfReportIdentity, SelfReportParseFailure> {
        switch identityAndJSON(data, knownAgentIDs: knownAgentIDs, queryAgent: queryAgent) {
        case .success(let raw): return .success(raw.identity)
        case .failure(let f): return .failure(f)
        }
    }

    static func identityAndJSON(_ data: Data, knownAgentIDs: Set<String>, queryAgent: String?)
        -> Result<SelfReportRawPayload, SelfReportParseFailure> {
        // 空串按「没给」处理：`?agent=` 拼错成空值时，报错要说「缺 agent」而不是「档案不存在」
        let fromQuery = queryAgent?.trimmingCharacters(in: .whitespaces)
        let queryAgent = (fromQuery?.isEmpty == false) ? fromQuery : nil
        guard let obj = try? JSONSerialization.jsonObject(with: data),
              let json = obj as? [String: Any] else {
            return .failure(SelfReportParseFailure(reason: .malformed, detail: "请求体不是 JSON 对象"))
        }
        // 只认字符串形态的名字字段：`{"agent": 1}` 不该被收成 id "1"——注册表的键是名字，
        // 数字键是调用方的 bug，不是「一个恰好还没建档的 Agent」
        guard let agent = name(json["agent"]) ?? name(json["agent_id"]) ?? queryAgent else {
            return .failure(SelfReportParseFailure(reason: .malformed,
                                                   detail: "缺 agent（正文或 URL 的 ?agent= 至少给一个）"))
        }
        guard knownAgentIDs.contains(agent) else {
            return .failure(SelfReportParseFailure(reason: .unknownAgent,
                                                   detail: "注册表里没有档案「\(agent)」，不会自动建档"))
        }
        guard let session = name(json["session"]) ?? name(json["session_id"]) else {
            return .failure(SelfReportParseFailure(reason: .malformed, detail: "缺 session"))
        }
        return .success(SelfReportRawPayload(identity: SelfReportIdentity(agentID: agent,
                                                                          sessionID: session),
                                             json: json))
    }

    /// 名字字段的**唯一**读法：只认非空字符串，并且返回 trim 后的值。
    /// 原先正文不 trim、URL trim，于是 `{"agent":" claude "}` 报 unknownAgent，
    /// 而 `?agent=%20claude%20` 能过——同一个值两条口径，手抄的人看不出为什么。
    static func name(_ raw: Any?) -> String? {
        guard let v = raw as? String else { return nil }
        let trimmed = v.trimmingCharacters(in: .whitespaces)
        return trimmed.isEmpty ? nil : trimmed
    }

    static func clamped(_ s: String?, _ limit: Int) -> String? {
        guard let s else { return nil }
        return s.count <= limit ? s : String(s.prefix(limit))
    }
    static func clampedTTL(_ raw: Double?) -> TimeInterval {
        guard let raw else { return SelfReportSubmission.defaultTTL }
        guard raw > 0 else { return SelfReportSubmission.defaultTTL }
        return min(max(raw, SelfReportSubmission.ttlRange.lowerBound),
                   SelfReportSubmission.ttlRange.upperBound)
    }
}

// MARK: - 令牌
//
// 放在 `~/Library/Application Support/AgentIsland/report.token`（0600，首启生成）。
// 不进钥匙串是有意的：接入命令要把它抄进第三方 Agent 的 settings 里，用户得**看得见**才能配；
// 而它的作用域只有本机回环的一个「能不能声明状态」的判定。红线照旧：值永不进日志、
// 不进仓库、不进任何输出（`masked()` 是唯一允许出现在文案里的形态）。

/// 「这条请求过了令牌校验」的**唯一凭证**。init 是 internal ⇒ `AgentIsland` 那个 target
/// 里没有任何写法能凭空造出它，于是「服务端把可信度硬编码成 true」这一类改动
/// 从「源码棘轮 grep 得到的一种拼写」变成了**编译不过**。
/// （上一轮 `bindSelfReport` 收的是裸 `Bool`：实测把 `matches(...)` 换成
/// `providedToken != nil`——任意非空 header 即得可信自报——在 484 条全绿下存活。）
public struct SelfReportCredential: Equatable, Hashable {
    init() {}
}

/// 令牌文件**读不出来**的原因。把它们混成同一个 nil 是本轮最贵的一类错误：
/// 「没有令牌」和「令牌是真的但形态被外部动作弄坏了」在用户侧是完全不同的两件事——
/// 前者要配接入命令，后者只是权限位被人改了，而旧令牌其实还在。
public enum SelfReportTokenDefect: String, Equatable {
    case missing
    case notRegular
    case foreignOwner
    case looseMode
    case badShape
}

public struct SelfReportTokenStore {
    /// header 名唯一来源：接入命令文案（06 号票）、服务端读取、测试都走这一条
    public static let headerName = "X-AgentIsland-Token"

    public let tokenURL: URL
    private let fileManager: FileManager
    /// 「谁算本机用户」。做成注入值而不是直接调 `getuid()`：属主检查是唯一一条
    /// 在测试里造不出反例的判据（测试没法以别的 uid 建文件），而那恰恰是最需要
    /// 被证明「真会拒绝」的一条
    private let ownerUID: () -> Int

    public init(directory: URL? = nil, fileManager: FileManager = .default,
                ownerUID: @escaping () -> Int = { Int(getuid()) }) {
        let dir = directory ?? SelfReportTokenStore.defaultDirectory
        self.tokenURL = dir.appendingPathComponent("report.token")
        self.fileManager = fileManager
        self.ownerUID = ownerUID
    }

    public static var defaultDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
            .appendingPathComponent("AgentIsland", isDirectory: true)
    }

    /// 取令牌；不存在则生成。返回 nil 表示**这次拿不到可信通道**（目录建不出来、磁盘写不进），
    /// 调用方必须走「拒绝可信自报」那条分支，不能当成「令牌不匹配」。
    /// 现在只有一个调用方（`LocalEventServer.start`），它把 nil 记进 `selfReportChannelUp`，
    /// 于是「整条通道在这台机器上永远 noToken」在岛内是可看见的，而不只是注释里的一句要求。
    public func ensure() -> String? {
        let (existing, defect) = inspect()
        if let existing, !existing.isEmpty { return existing }
        if let defect, defect != .missing {
            // 「静默作废」是这条通道最坏的失效方式：用户看到的是「昨天还好使，今天全废」，
            // 而真实原因只是有人对令牌目录跑了 chmod -R / 同步盘把权限写宽了
            AppLog.warn("report.token 形态不合格(\(defect.rawValue))，已换成新生成的令牌"
                        + "——接入命令需要重新配置")
        }
        // 目录不存在时 createFile 只会默默失败——那等于整条通道在这台机器上**永远**返回
        // noToken，而每个接入方看到的都是「令牌不对」，没人会想到是这里没建目录
        if !fileManager.fileExists(atPath: tokenURL.deletingLastPathComponent().path) {
            do {
                try fileManager.createDirectory(at: tokenURL.deletingLastPathComponent(),
                                                withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: NSNumber(value: Self.ownerOnlyDirMode)])
            } catch { return nil }
        }
        // 符号链接占位不需要在这里特殊处理：实测 `createFile` 会先把链接本身解掉、
        // 再建 0600 的普通文件（目标内容一字不动，悬空链接也一样）。这条口径由
        // 「ensure 不许把新生成的令牌写穿符号链接」那两条用例钉着——真哪天 Foundation
        // 改成穿过链接写，红的是那条测试，而不是这里多一段永远不会被观察到的删除
        guard let generated = SelfReportTokenStore.randomToken() else { return nil }
        guard fileManager.createFile(atPath: tokenURL.path, contents: Data(generated.utf8),
                                     attributes: [.posixPermissions: NSNumber(value: Self.ownerOnlyMode)])
        else { return nil }
        // createFile 的权限属性会被 umask 影响，落盘后**显式**再收一次；
        // 0600 是这个文件唯一的合法形态——它是「谁在说话」的全部依据
        try? fileManager.setAttributes([.posixPermissions: NSNumber(value: Self.ownerOnlyMode)],
                                       ofItemAtPath: tokenURL.path)
        return generated
    }

    public func read() -> String? { inspect().token }

    /// 「文件里有串就用」在这台机器上是不成立的假设：`Application Support/AgentIsland/`
    /// 在首启之前归用户可写，任何本机进程都能抢先把 report.token 写成它已知的那个值，
    /// 于是它拿着「令牌」就能绑一条可信自报，而我们以为是自己生成的。
    /// 所以先验明正身：常规文件（非符号链接）、属主是当前用户、权限不带组/其他位、
    /// 内容是一段足够长的十六进制。任何一条不对都当作**没有令牌**，并把原因带出去——
    /// `matches()` 一律为假，而 `ensure()` 会换成真正由我们生成的那一份（换之前先告警）。
    /// 诚实的边界：这验的是**形态**不是**来源**——同一个用户、同样 0600 的合法十六进制串
    /// 仍然可能被抢先写好，本机要真做到不可伪造得把令牌放进钥匙串并放弃「抄进第三方配置」。
    public func inspect() -> (token: String?, defect: SelfReportTokenDefect?) {
        guard let attrs = try? fileManager.attributesOfItem(atPath: tokenURL.path) else {
            return (nil, .missing)
        }
        guard (attrs[.type] as? FileAttributeType) == .typeRegular else { return (nil, .notRegular) }
        if let uid = attrs[.ownerAccountID] as? NSNumber, uid.intValue != ownerUID() {
            return (nil, .foreignOwner)
        }
        let mode = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? -1
        guard mode & 0o077 == 0 else { return (nil, .looseMode) }
        guard let data = try? Data(contentsOf: tokenURL),
              let text = String(data: data, encoding: .utf8) else { return (nil, .badShape) }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count >= 32, trimmed.allSatisfy({ $0.isHexDigit }) else { return (nil, .badShape) }
        return (trimmed, nil)
    }

    /// 重置：旧令牌直接作废（不是「再加一个」），用户拿新命令重配即可
    public func reset() -> String? {
        guard let generated = SelfReportTokenStore.randomToken() else { return nil }
        return fileManager.createFile(atPath: tokenURL.path, contents: Data(generated.utf8),
                                      attributes: [.posixPermissions: NSNumber(value: Self.ownerOnlyMode)])
            ? generated : nil
    }

    /// 权限位（十进制 384 = 0o600）。测试要直接比对它，所以留成公开常量而不是注释里的字面量
    public static let ownerOnlyMode: Int = 0o600
    /// 令牌**目录**的权限：0o700。写成 0o600 的话目录没有执行位，
    /// 里面根本创建不了文件——第一版就是这么「修好了目录、弄坏了生成」的
    public static let ownerOnlyDirMode: Int = 0o700

    /// 定长比较。累加必须是 `|=` 而不是 `^=`：XOR 校验和只要求「差异两两相消」，
    /// 于是**把真令牌每个字节都翻转**得到的是 `diff == 0` ⇒ 判定为正确令牌。
    /// 实测（把这一段的上一版直接编成可执行体跑）：20000 次随机同长猜测里有上百次被接受
    /// （≈3%），第一次假接受最少 3 次猜测就到——48 位十六进制本来该有的不可猜测性归零，
    /// 而这条判断是整条通道唯一的边界。
    public func matches(_ provided: String?) -> Bool {
        guard let want = read(), let provided else { return false }
        let a = Array(want.utf8), b = Array(provided.trimmingCharacters(in: .whitespaces).utf8)
        guard a.count == b.count else { return false }
        var diff: UInt8 = 0
        for i in 0..<a.count { diff |= a[i] ^ b[i] }
        return diff == 0
    }

    /// 唯一的「能不能声明可信状态」出口：过令牌才产出凭证。
    /// 有 `matches()` 在，这里为什么还包一层？因为**服务端只该拿到能力，不该拿到判定结果**——
    /// 拿到 Bool 的那一层下一步就会写成 `if providedToken != nil`。
    public func authorize(_ provided: String?) -> SelfReportCredential? {
        matches(provided) ? SelfReportCredential() : nil
    }

    /// 任何要显示给用户的东西只能用它：前 4 后 4，中间打码
    public func masked() -> String {
        guard let t = read() else { return "（尚未生成）" }
        return SelfReportTokenStore.mask(t)
    }
    /// 打码形式的类型级入口：值还没进参数就只剩前缀了——告警文案里要带的是「哪一把」，
    /// 而「哪一把」永远不该以明文出现（红线：令牌值不进日志）
    public static func mask(_ value: String) -> String {
        let chars = Array(value)
        guard chars.count > 12 else { return "····" }
        return String(chars.prefix(4)) + "····" + String(chars.suffix(4))
    }

    static func randomToken() -> String? {
        var bytes = [UInt8](repeating: 0, count: 24)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return nil }
        return bytes.map { String(format: "%02x", $0) }.joined()
    }
}

// MARK: - 绑定判定

/// 进程表探针的注入缝。生产走 libproc（`ProcessTerminator` 那一个出口），测试给字典。
/// 之所以是结构体而不是直接调 `ProcessTerminator`：绑定这件事必须可测，
/// 而「探活 + 比对可执行文件名」在真机上是不可控的。
public struct SelfReportProcessProbe {
    public var isLive: (Int32) -> Bool
    /// 当前可执行文件名（小写）。nil 表示取不到（权限、正在退出）。
    public var executableName: (Int32) -> String?

    public init(isLive: @escaping (Int32) -> Bool, executableName: @escaping (Int32) -> String?) {
        self.isLive = isLive
        self.executableName = executableName
    }

    public init() {
        self.isLive = { ProcessTerminator.isAlive(pid: $0) }
        self.executableName = { ProcessTerminator.executableName(of: $0) }
    }
}

/// `/session` 的绑定结果。**只有 `.bound` 会写入可信自报**，其余一律退回现状（推断）。
/// 这里**没有** `.noToken` 那一格：`bind` 要求出示凭证（`SelfReportCredential` 非可选），
/// 所以「没令牌却走到 pid 比对」不是一条会被写错的分支，而是一个编译错误。
/// 上一轮这里是 `tokenAccepted: Bool` + `.noToken`，于是「先令牌后 pid」只是一句注释
/// 和一次 grep——实测把服务端的 `matches(...)` 换成 `providedToken != nil` 全绿存活。
public enum SelfReportBinding: Equatable {
    case bound
    /// 给了 pid 却对不上：可能是 pid 复用，也可能是冒名顶替
    case pidMismatch(reason: String)
    /// 申报指向的档案已经不在了。这一条**不叫** `pidMismatch`：`reason` 是给脚本读的枚举，
    /// 把「档案没了」塞进 pid 那一格会让接入方去查自己写对的进程号
    case profileGone(reason: String)
}

public enum SelfReportBinder {
    /// 口径：**可信度只由令牌建立，pid 只能否决、不能授予**。
    /// 凭证非可选就是这条口径的实现——任何本机进程都能报一个真实存在的 pid，
    /// 如果把 pid 放在前面，等于把安全边界换成一个可被猜中的整数。
    public static func bind(_ submission: SelfReportSubmission, credential: SelfReportCredential,
                            profile: AgentProfile?, probe: SelfReportProcessProbe) -> SelfReportBinding {
        guard let pid = submission.pid else { return .bound }
        guard probe.isLive(pid) else {
            return .pidMismatch(reason: "pid \(pid) 不在进程表里（或已是僵尸）")
        }
        guard let profile else {
            return .profileGone(reason: "档案 \(submission.agentID) 已不在注册表里")
        }
        // 档案没有进程名约束时不做路径比对：这类档案（纯 MCP/纯目录型）本来就匹配不出名字，
        // 拒了它等于禁掉它的自报，而它也没声称自己是某个进程
        guard !profile.processNames.isEmpty else { return .bound }
        guard let got = probe.executableName(pid) else {
            // 与 `isAlive(expectedPath:)` 的方向**相反**且是故意的：那边是「终止后再复核」，
            // 读不到路径就宁可当它还活着（避免谎报清理成功）；这边是授予可信度，
            // 证明不了身份就是不采信。共用一个 libproc 出口，但两个方向的保守侧不同。
            return .pidMismatch(reason: "读不到 pid \(pid) 的可执行路径，无法确认它是 \(profile.name)")
        }
        let names = Set(profile.processNames.map { $0.lowercased() })
        // 与进程匹配同一个谓词（词边界前缀）。这里若写成 `got.hasPrefix(name)`，
        // 一个叫 claudex 的进程就能顶替 claude 拿到可信自报
        guard ProcessMatcher.matchesProcessNames(names, basename: got) else {
            return .pidMismatch(reason: "pid \(pid) 的可执行文件是「\(got)」，不匹配 \(profile.name) 的进程名集")
        }
        return .bound
    }
}

// MARK: - 存储（引擎侧）

/// 一条可信自报。`expiredAt` 一旦被盖章就**不再清除**，续报会重新计时并把它抹回 nil——
/// 「过期」是记录的状态，不是记录的删除。
public struct SelfReportRecord: Equatable {
    public let agentID: String
    public let sessionID: String
    public let pid: Int32?
    public var state: SelfReportState
    public var detail: String?
    public var ask: String?
    public var receivedAt: Date
    /// 申报时给的续报窗口（秒）。重锚时要靠它把 expiresAt 推回去，所以得留在记录里
    public let ttl: TimeInterval
    public var expiresAt: Date
    /// 最近一次 TTL 到期的时刻。nil = 仍在续报窗口内。
    public var expiredAt: Date?

    public func isBelievable(now: Date) -> Bool { expiredAt == nil && now < expiresAt }
}

/// 引擎持有的自报登记表。刻意**不**并进 `resetTracking` 那 12 个字典：
/// 那些是「每 Agent 的推断计时器」，进程消失就该清零；而自报是**外部声明**，
/// 它主张的事实（「它说它在等确认」）过期之后仍是要给用户看的证据。
public struct SelfReportStore {
    public private(set) var records: [String: SelfReportRecord] = [:]
    /// 上限存在的理由：会话 id 是外部随便起的，不去重就会无界增长；
    /// 而一张岛只可能同时关心个位数的会话。
    public static let capacity = 64
    /// 过期证据的保质期：再老就没有解释价值了
    public static let evidenceRetention: TimeInterval = 24 * 3600

    public init() {}

    public static func key(agentID: String, sessionID: String) -> String { "\(agentID)\u{1}\(sessionID)" }

    /// 续报/新报都走这里：同一个 (agent, session) 覆盖而不是追加
    @discardableResult
    public mutating func ingest(_ s: SelfReportSubmission, now: Date) -> SelfReportRecord {
        let key = SelfReportStore.key(agentID: s.agentID, sessionID: s.sessionID)
        let record = SelfReportRecord(agentID: s.agentID, sessionID: s.sessionID, pid: s.pid,
                                      state: s.state, detail: s.detail, ask: s.ask,
                                      receivedAt: now, ttl: s.ttl,
                                      expiresAt: now.addingTimeInterval(s.ttl), expiredAt: nil)
        records[key] = record
        // 写入之后顺手盖一次章。诚实的理由只有一个：`expiredEvidence`（04 号票要给用户看的
        // 「它说过什么」）只在记录带上 `expiredAt` 之后才可见，不盖就要等下一拍采样。
        // **不是**「不然 prune 会看到一批本该过期却还光鲜的记录」——prune 的淘汰键是
        // `expiresAt` 而非 `expiredAt`，实测同一批过期记录盖不盖章结果完全一致
        _ = sweepExpired(now: now)
        prune(now: now)
        return record
    }

    /// 撤销一条申报（`DELETE /session`）。撤销**不**留过期证据：那是「我说过」被主动收回，
    /// 与「说过但没续上」是两件事。
    @discardableResult
    public mutating func revoke(agentID: String, sessionID: String) -> Bool {
        records.removeValue(forKey: SelfReportStore.key(agentID: agentID, sessionID: sessionID)) != nil
    }

    /// 把到期的记录盖章，返回本轮**新**过期的那些（重复扫描不会重复记账）。
    @discardableResult
    public mutating func sweepExpired(now: Date) -> [SelfReportRecord] {
        var fresh: [SelfReportRecord] = []
        for (key, record) in records where record.expiredAt == nil && now >= record.expiresAt {
            var stamped = record
            stamped.expiredAt = now
            records[key] = stamped
            fresh.append(stamped)
        }
        return fresh
    }

    /// 按 (agent, session) 精确取一条。响应体必须用它自己那条申报的到期时刻——
    /// 原先回复走 `believable(agentID:)`，那是「跨会话挑最晚过期的那条」，
    /// 一个 Agent 开两个会话时就会把**另一条**的 expiresAt 回给调用方
    public func record(agentID: String, sessionID: String, now: Date) -> SelfReportRecord? {
        let r = records[SelfReportStore.key(agentID: agentID, sessionID: sessionID)]
        return (r?.isBelievable(now: now) == true) ? r : nil
    }

    /// 某 Agent 当前仍可信的那条申报（取最晚过期的）。多会话时这就是「卡片该显示哪条」的口径。
    public func believable(agentID: String, now: Date) -> SelfReportRecord? {
        records.values.filter { $0.agentID == agentID && $0.isBelievable(now: now) }
            .max { $0.expiresAt < $1.expiresAt }
    }

    /// 某 Agent 最近一条过期证据——04 号票的「自报说 X，进程表说 Y」双陈述读这个
    public func expiredEvidence(agentID: String) -> SelfReportRecord? {
        records.values.filter { $0.agentID == agentID && $0.expiredAt != nil }
            .max { ($0.expiredAt ?? .distantPast) < ($1.expiredAt ?? .distantPast) }
    }

    /// 时钟回拨重锚（与引擎给 `workingSince`/`highCpuSince` 做的那条防御同一个理由）：
    /// 回拨之后 `now` 会落在所有到期时刻之前，于是 TTL **永远**判不出过期——
    /// 一条申报就此长生不老地压着推断。把落在未来的记录重锚到本拍，窗口按它自己的 ttl 重来。
    /// 只管**还没过期**的那些：第一版顺手 `expiredAt = nil`，实测「申报 → 断心跳被盖章 →
    /// NTP 回拨 25s」之后 `believable` 又返回了那条 `attention`，而这中间没有任何新上报。
    /// 那是把第 3 条口径反着违反了——不是「过期即清空」，而是更糟的「过期即复活」。
    /// 一个已经过了期、又没人清空的戳，本身就是「它说过、后来断了」的证据，不许擦。
    mutating func reanchorIfClockRewound(now: Date) {
        for (key, record) in records where record.receivedAt > now && record.expiredAt == nil {
            var fixed = record
            fixed.receivedAt = now
            fixed.expiresAt = now.addingTimeInterval(record.ttl)
            records[key] = fixed
        }
    }

    /// 每档案的会话上限。总量 64 是全档案共用的，所以只看总量的淘汰可以被一个说话者
    /// 用光：实测灌 64 条 `qoder/flood*` 之后，刚写进来的 `claude/real`（ttl 600）直接没了。
    /// 每档案封顶之后，洪水最多挤掉**同一个说话者**的记录。
    public static let perAgentCapacity = 8

    public mutating func dropAgent(_ agentID: String) {
        records = records.filter { $0.value.agentID != agentID }
    }

    mutating func prune(now: Date) {
        for (key, r) in records {
            // 保质期从**真正过期的那一刻**算，不是从「我们哪天才发现」算。
            // 用 expiredAt 的话，一条从未被采样扫到的记录会永远新鲜——盖章时刻总是 now
            if !r.isBelievable(now: now),
               now.timeIntervalSince(r.expiresAt) > SelfReportStore.evidenceRetention {
                records[key] = nil
            }
        }
        // 先按档案封顶，再按总量封顶；两次排序都**先保可信窗口内的**再比新旧。
        // 只按 receivedAt 保新的话，24 小时保质期内的过期证据能把正在跑的会话挤掉
        for (agentID, group) in Dictionary(grouping: records.values, by: { $0.agentID })
        where group.count > SelfReportStore.perAgentCapacity {
            for drop in mostBelievableFirst(group, now: now).dropFirst(SelfReportStore.perAgentCapacity) {
                records[SelfReportStore.key(agentID: agentID, sessionID: drop.sessionID)] = nil
            }
        }
        guard records.count > SelfReportStore.capacity else { return }
        let keep = mostBelievableFirst(Array(records.values), now: now).prefix(SelfReportStore.capacity)
        records = Dictionary(uniqueKeysWithValues: keep.map {
            (SelfReportStore.key(agentID: $0.agentID, sessionID: $0.sessionID), $0)
        })
    }

    /// 淘汰顺序：仍在可信窗口内的优先，其次按最近申报时间新→旧
    private func mostBelievableFirst(_ group: [SelfReportRecord], now: Date) -> [SelfReportRecord] {
        group.sorted { lhs, rhs in
            switch (lhs.isBelievable(now: now), rhs.isBelievable(now: now)) {
            case (true, false): return true
            case (false, true): return false
            default: return lhs.receivedAt > rhs.receivedAt
            }
        }
    }
}

// MARK: - 协议形状（HTTP 那一层的两个纯函数）
//
// 它们住在 Core 而不是 `LocalEventServer` 里，唯一理由是**可测**：`AgentIsland` 是
// executable target，测试 runner 链接不到它，于是上一轮 `SelfReportQuery.parse`、
// header 取值、状态码表——也就是票 02 的 Done-when 那一面（「curl 三条命令各得到什么」）
// ——一行自动化守卫都没有。把它们搬过来之后，服务端只剩路由与发送。

/// `DELETE /session` 的结果。撤销**为什么**没成是一个要回给脚本的枚举，
/// 而「带没带有效令牌」不进这里：那条判定只体现为**能不能调用撤销**
/// （`revokeSelfReport(credential:)` 的凭证参数非可选，没令牌就调不动它）。
public enum SelfReportRevokeOutcome: Equatable {
    case revoked
    /// 没有这条申报（可能从来没申报过，也可能已经被容量/保质期收走）
    case notFound
}

/// `/session` 的 URL 参数。只解一个 `agent`：它是「这条 URL 属于哪个档案」的唯一新增信息，
/// 而 hook 配置能带它的地方也只有 URL（正文由对方运行时决定，多一个字段就多一处不兼容）。
/// 刻意不做通用表单解码——`detail`/`ask`/`state` 从 URL 走会绕过长度截断与形状校验。
public struct SelfReportQuery: Equatable {
    public let agent: String?

    public static func parse(from fullPath: String) -> SelfReportQuery {
        guard let q = fullPath.split(separator: "?").dropFirst().first else { return SelfReportQuery(agent: nil) }
        var agent: String?
        for pair in q.components(separatedBy: "&") {
            let kv = pair.components(separatedBy: "=")
            guard kv.count == 2, kv[0] == "agent" else { continue }
            // 片段要摘掉：`/session?agent=claude#frag` 此前把 `claude#frag` 整段当 id
            // 送进注册表比对，回给调用方一个莫须有的 `unknownAgent`
            let raw = String(kv[1]).components(separatedBy: "#").first ?? ""
            // 与正文那条路径同口径：解码完就 trim。留空格到这里，`?agent=%20claude%20`
            // 能过而 `{"agent":" claude "}` 不能过，同一个值两副面孔
            if let decoded = raw.removingPercentEncoding {
                agent = decoded.trimmingCharacters(in: .whitespaces)
            }
        }
        return SelfReportQuery(agent: agent)
    }
}

/// 从请求头块里取一个字段（大小写不敏感）。此前 headerPart 直接被丢掉，
/// `/session` 是第一个**必须读 header** 的端点。
public enum SelfReportHeaders {
    public static func value(_ headerBlock: String, name: String) -> String? {
        let want = name.lowercased()
        for line in headerBlock.components(separatedBy: "\r\n").dropFirst() {
            let pair = line.components(separatedBy: ":")
            guard pair.count >= 2 else { continue }
            if pair[0].trimmingCharacters(in: .whitespaces).lowercased() == want {
                return pair[1...].joined(separator: ":").trimmingCharacters(in: .whitespaces)
            }
        }
        return nil
    }
}

// MARK: - 对外形状（响应码、reason、给接入方看的那句话）
//
// 这三条规则原先是服务端里的字面量，而它们恰恰是本轮唯一**行为级**的对外承诺：
// 「未鉴权时不透露载荷为什么被拒」「没投递不许说成已投递」。
// 写在 executable target 里就等于没有守卫（测试 runner 链接不到它），所以搬到能测的地方。

public enum SelfReportWire {
    /// 载荷被拒时回给调用方的 `reason`。
    /// 唯一的非显然规则：**没带凭证时一律 `noToken`**——否则依次试 `{"agent":"claude"}`
    /// 与 `{"agent":"zzz"}` 就能拿到一份「这台机器装了哪些编码 Agent」的清单，
    /// 而 macOS 13 那条构建路径下这个探测还能从局域网做。
    /// §3 的「未知 id ⇒ 400」仍然成立，只是它现在只对**带令牌**的请求说
    public static func reason(for rejection: SelfReportRejection, trusted: Bool) -> SelfReportReason {
        guard trusted else { return .noToken }
        switch rejection {
        case .malformed: return .malformed
        case .unknownAgent: return .unknownAgent
        case .unknownState: return .unknownState
        }
    }

    /// 与 `reason` 同一条规则的状态码面：未鉴权时不是「你的请求有问题」（400），
    /// 而是「我收下但不采信」（200 + 落回通道）
    public static func status(for rejection: SelfReportRejection, trusted: Bool) -> Int {
        trusted ? 400 : 200
    }

    /// 未采信那条申报给接入方看的话。分叉的唯一依据是 `post` 的**返回值**：
    /// 上一轮这里不分叉，于是 `state:"working"`（持续状态，本来就不转事件）
    /// 得到的回复是「已按未采信事件投递」——把「没送」讲成「送了」，
    /// 而接入方据此会认为通道在работает
    public static func untrustedMessage(posted: Bool, state: SelfReportState) -> String {
        posted
            ? "没有有效令牌：本条已按未采信事件投递（不改变状态来源）。"
            : "没有有效令牌，且 \(state.rawValue) 是持续状态、不转事件：本条未被记录，也不改变状态来源。"
    }
}

// MARK: - 未采信申报的落回通道
//
// spec §3：「无令牌 ⇒ 拒绝可信自报，落回现有 externallyDelivered 通道」。
// 这条映射单独成函数，是因为它是**语义翻译**而不是安全判定：状态 → 事件类型。
// 放在调用方会让每个拒绝分支各自抄一份映射，而事件类型的口径本来只有一处。

public enum SelfReportFallback {
    /// 该申报值得转成一条通知吗。`working` / `idle` 是**持续状态**而不是事件——
    /// 把它们转成事件会让同一条「我在干活」在每次续报时都响一次。
    public static func eventKind(for state: SelfReportState) -> AgentTaskEvent.EventType? {
        switch state {
        case .attention: return .attention
        case .completed: return .completed
        case .working, .idle: return nil
        }
    }

    /// 转成一条**未采信**的事件并投递。返回是否真的投递了。
    @MainActor @discardableResult
    public static func post(_ submission: SelfReportSubmission, to engine: ActivityEngine) -> Bool {
        guard let kind = eventKind(for: submission.state) else { return false }
        engine.postEvent(AgentTaskEvent(
            agentId: submission.agentID,
            agentName: submission.agentID,
            eventType: kind,
            duration: 0,
            timestamp: Date(),
            pid: submission.pid,
            message: submission.ask ?? submission.detail,
            detail: submission.detail,
            externallyDelivered: true))
        return true
    }
}

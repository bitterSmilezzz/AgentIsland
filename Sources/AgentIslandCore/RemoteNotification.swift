import Foundation
import Security

// MARK: - 远程通知：消息模型、外发策略与凭据存放

/// 一条要送出本机的事件。字段刻意极少——外发的每一字节都是离开这台机器的信息，
/// 默认只带「哪个 Agent + 什么状态」，命令内容/文件路径必须由用户显式勾选才进得来。
public struct OutboundMessage: Equatable, Sendable {
    public let title: String
    public let body: String
    /// true 表示值得打断用户（等待确认、消耗告警）；false 只是知会（任务完成）
    public let urgent: Bool

    public init(title: String, body: String, urgent: Bool) {
        self.title = title
        self.body = body
        self.urgent = urgent
    }
}

/// 外发结果。绝不出现「没发出去却报成功」——本仓已经为此修过清理进程与 CLI 投递两处。
public enum OutboundOutcome: Equatable, Hashable, Sendable {
    case delivered
    /// 被策略挡下（总开关关、该类事件关、节流命中、静默时段）——不算失败，也不该记成功
    case suppressed(reason: String)
    /// 配置不完整（没填 key/主机/收件人等）
    case notConfigured(reason: String)
    /// 送出但对方没接受：带可给人看的短说明（HTTP 状态码或 SMTP 回复码）。
    /// `permanent` = 对端**明确拒绝**（404 主题不存在、535 授权码错、550 拒绝中继）：
    /// 再试一次也是同一个结果。这一位既决定要不要重试，也决定界面怎么说——
    /// 把「配置就是错的」显示成「链路在抖」会把人引向完全错误的排查方向。
    case failed(reason: String, permanent: Bool = false)

    public var isDelivered: Bool {
        if case .delivered = self { return true }
        return false
    }

    /// 只对 `.failed` 有意义；`.suppressed` / `.notConfigured` 不是「对端拒绝」
    public var isPermanent: Bool {
        if case .failed(_, let permanent) = self { return permanent }
        return false
    }

    /// 给人看的一句话原因（日志与设置页共用）。绝不返回「未知错误」这类空壳
    public var shortReason: String {
        switch self {
        case .delivered: return "已送达"
        case .suppressed(let reason): return reason
        case .notConfigured(let reason): return reason
        case .failed(let reason, _): return reason
        }
    }
}

/// 通道种类。ntfy 的 HTTP 形态按官方文档核实过（POST 到 `https://ntfy.sh/<主题>`，
/// 标题走 `X-Title`、优先级走 `X-Priority`，公开服务器单条上限 4096 字节）；
/// 微信系（Server酱 / PushPlus / WxPusher）与企微/钉钉/飞书一律走「自定义模板」，
/// 端点与字段由用户从自己那家的控制台/文档粘贴——本仓不预置未核实的字段名。
public enum RemoteChannelKind: String, CaseIterable, Codable, Sendable {
    case ntfy
    case customHTTP
    case smtpEmail

    public var label: String {
        switch self {
        case .ntfy: return "ntfy 推送"
        case .customHTTP: return "自定义 HTTP（微信 Server酱 / PushPlus / 企微 / 钉钉…）"
        case .smtpEmail: return "邮箱（SMTP）"
        }
    }

    /// 该通道的密钥在钥匙串里的固定条目名。
    /// 刻意不做成可配字段：「密钥存了但条目名对不上」是查不出来的失效——
    /// 界面全绿、每次发送都失败。条目名由通道种类唯一决定。
    public var defaultSecretName: String { "remote.\(rawValue)" }

    /// 端点是否明文过网。填 http:// 时密钥与通知内容会明文穿过路径上的每一跳。
    /// 与「配齐」分开的独立警告：配齐检查里混进安全提示会让两条都变模糊
    public func insecureEndpoint(config: RemoteChannelConfig) -> String? {
        let url: String
        switch self {
        case .ntfy: url = config.topicOrURL
        case .customHTTP: url = config.urlTemplate
        case .smtpEmail: return nil   // 只支持 465，一定是 TLS
        }
        return url.lowercased().hasPrefix("http://")
            ? "地址是明文 http://：密钥与通知内容会明文经过路径上的每一跳，建议换成 https://" : nil
    }

    /// 地址里像是**直接粘了凭据**（没有 {key} 占位，却含 key= / .send / 高熵片段）。
    /// 这种填法会让凭据以明文进 UserDefaults、并显示在设置页与预览里；
    /// 掩码只能保证「看得见的那份」不泄漏，落盘那份救不回来，所以要说破
    public func plaintextSecretInTemplate(config: RemoteChannelConfig) -> String? {
        let url = self == .ntfy ? config.topicOrURL : config.urlTemplate
        guard !url.isEmpty, !url.contains("{key}") else { return nil }
        let lowered = url.lowercased()
        let suspicious = lowered.contains("key=") || lowered.contains("token=")
            || lowered.contains(".send")
            || url.split(separator: "/").contains { RemoteSecret.looksLikeToken($0) }
        return suspicious
            ? "地址里像是直接粘了密钥：它会明文存进配置并显示在预览里。请把密钥那段改成 {key}，值放钥匙串"
            : nil
    }

    /// 该通道是否已经配齐到「可以试发」。
    /// `hasSecret` 由调用方给（发送路径用注入的读取器，设置页用 `RemoteSecret.exists`），
    /// 判据本身保持纯函数：它每次外发都会被调，不该在里面碰钥匙串。
    public func missingField(config: RemoteChannelConfig, hasSecret: Bool) -> String? {
        switch self {
        case .ntfy:
            let value = config.topicOrURL.trimmingCharacters(in: .whitespacesAndNewlines)
            if value.isEmpty { return "缺主题名或服务器地址" }
            // 判定只写 `hasPrefix("http")` 的话，`ntfy.mine.local/island`（少贴了协议头）
            // 会被拼成 https://ntfy.sh/ntfy.mine.local/island——内容跑到一台
            // 用户没选过的公网服务器上，主题名还是他的内网主机名
            if value.contains("/"), !value.lowercased().contains("://") {
                return "像是要自建服务器的地址，但少了 http:// 或 https:// 前缀"
            }
            if value.contains(" ") || value.unicodeScalars.contains(where: { !$0.isASCII }) {
                return "主题名只能是无空格的 ASCII 字符"
            }
            return nil
        case .customHTTP:
            if config.urlTemplate.isEmpty { return "缺发送地址" }
            // 空模板时本可以自己组一套字段名，但那等于把没核实过的字段名
            // （body / content / desp）写成预设——各家中转服务的字段必须用户自己填
            if config.bodyTemplate.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return "缺请求体模板：各家字段名不同，请填写类似 \"title={title}&desp={body}\" 的形状"
            }
            if RemoteSecret.containsPlaceholder(config.urlTemplate) && !hasSecret {
                return "地址里有 {key} 占位，但钥匙串里还没有密钥"
            }
            return nil
        case .smtpEmail:
            if config.smtpHost.isEmpty { return "缺 SMTP 服务器地址" }
            if config.smtpUser.isEmpty { return "缺发信账号" }
            if config.smtpTo.isEmpty { return "缺收件地址" }
            // 端口先于密钥检查：非 465 是「这条路根本不走」，缺授权码只是「还差最后一步」
            if config.smtpPort != 465 { return "仅支持 465（隐式 TLS）；25/587 的 STARTTLS 不支持" }
            if !hasSecret { return "缺 SMTP 密码/授权码（存钥匙串）" }
            return nil
        }
    }
}

/// 一个通道的非密钥配置。密钥（sendkey / token / webhook 里的 key / SMTP 授权码）
/// **不进这个结构**，一律走 `RemoteSecret` 存钥匙串：UserDefaults 是明文 plist，
/// 会被 Time Machine、iCloud 同步与任何 dump 工具读到，而 webhook URL 本身就是通行证。
public struct RemoteChannelConfig: Codable, Equatable, Sendable {
    /// ntfy：主题名或完整地址；customHTTP：含 `{title}` / `{body}` 占位的地址
    public var topicOrURL: String = ""
    public var urlTemplate: String = ""
    /// POST 时的请求体模板。customHTTP 必填（各家中转服务的字段名不同，本仓不预置），
    /// ntfy 忽略它——ntfy 的正文就是裸文本
    public var bodyTemplate: String = ""
    public var useJSONBody: Bool = true
    /// SMTP
    public var smtpHost: String = ""
    public var smtpPort: Int = 465
    public var smtpUser: String = ""
    public var smtpTo: String = ""
    /// 是否把「最后一条动作」一起送出（默认否：那会把命令内容送出这台机器）
    public var includeActionDetail: Bool = false

    public init() {}

    /// 与 `RemoteNotifyPolicy` 同一口径：每个字段都 `decodeIfPresent` + 回落默认值。
    /// 用合成解码器的话，以后每加一个字段，旧存档就整条解不开——而配置的默认值是
    /// 「全空 + 关闭」，用户会看到自己配好的通道凭空清空。
    /// 单个字段类型写错（手改 plist 把端口写成字符串）也只作废那一个字段，
    /// 其余照样读回：坏一个字段就丢整条配置，等于把「一处笔误」放大成「全部重填」
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func text(_ key: CodingKeys) -> String {
            (try? c.decodeIfPresent(String.self, forKey: key)) ?? nil ?? ""
        }
        topicOrURL = text(.topicOrURL)
        urlTemplate = text(.urlTemplate)
        bodyTemplate = text(.bodyTemplate)
        useJSONBody = (try? c.decodeIfPresent(Bool.self, forKey: .useJSONBody)) ?? nil ?? true
        smtpHost = text(.smtpHost)
        smtpUser = text(.smtpUser)
        smtpTo = text(.smtpTo)
        smtpPort = (try? c.decodeIfPresent(Int.self, forKey: .smtpPort)) ?? nil ?? 465
        includeActionDetail =
            (try? c.decodeIfPresent(Bool.self, forKey: .includeActionDetail)) ?? nil ?? false
    }

    private enum CodingKeys: String, CodingKey {
        case topicOrURL, urlTemplate, bodyTemplate, useJSONBody
        case smtpHost, smtpPort, smtpUser, smtpTo, includeActionDetail
    }
}

/// 外发策略：总开关、事件类型过滤、节流、静默时段。
public struct RemoteNotifyPolicy: Codable, Equatable, Sendable {
    public var masterEnabled: Bool = false
    public var sendCompleted: Bool = true
    public var sendAttention: Bool = true
    public var sendCostSpike: Bool = true
    /// 同一 Agent 同一状态在该秒数内只发一次（一次任务会连发多条同类事件）
    public var throttleSeconds: Int = 90
    /// 静默时段（本地时区的「时:分」，start > end 表示跨零点）
    public var quietStart: String = ""
    public var quietEnd: String = ""
    /// 只在人不在机器前时外发。
    /// 光靠「锁屏 / 显示器睡眠」在用户的真实场景里是错的：他用 Windows 远程桌面连着 Mac
    /// 时，会话既不会锁定也不会熄屏，人走开了这两个信号全为 false——实测本机正是这个形态
    /// （无键 CGSSessionScreenIsLocked、显示器未睡，而键盘已 88 秒没动）。
    /// 所以判定必须带上「无输入多久」这一条，见 `awayIdleSeconds`。
    public var onlyWhenAway: Bool = false
    /// 无输入超过这么多秒就算「人不在」。默认 120s：短于它会在你只是去倒水时误报，
    /// 长于它则等到忘了这回事才通知
    public var awayIdleSeconds: Int = 120

    public init(masterEnabled: Bool = false, sendCompleted: Bool = true, sendAttention: Bool = true,
                sendCostSpike: Bool = true, throttleSeconds: Int = 90,
                quietStart: String = "", quietEnd: String = "",
                onlyWhenAway: Bool = false, awayIdleSeconds: Int = 120) {
        self.masterEnabled = masterEnabled; self.sendCompleted = sendCompleted
        self.sendAttention = sendAttention; self.sendCostSpike = sendCostSpike
        self.throttleSeconds = throttleSeconds; self.quietStart = quietStart
        self.quietEnd = quietEnd; self.onlyWhenAway = onlyWhenAway
        self.awayIdleSeconds = awayIdleSeconds
    }

    static let normalizedThrottle = 15...3600
    static let normalizedIdle = 30...3600

    public func normalized() -> RemoteNotifyPolicy {
        var copy = self
        copy.throttleSeconds = min(max(throttleSeconds, Self.normalizedThrottle.lowerBound),
                                   Self.normalizedThrottle.upperBound)
        copy.awayIdleSeconds = min(max(awayIdleSeconds, Self.normalizedIdle.lowerBound),
                                   Self.normalizedIdle.upperBound)
        // 写坏的静默时段必须退化成「不静默」，而不是整天把通知吞掉——
        // 后者是静默失效，用户永远看不见，正是最难查的那类 bug
        if !Self.isHHmm(copy.quietStart) { copy.quietStart = "" }
        if !Self.isHHmm(copy.quietEnd) { copy.quietEnd = "" }
        if copy.quietStart.isEmpty != copy.quietEnd.isEmpty {
            copy.quietStart = ""
            copy.quietEnd = ""
        }
        return copy
    }

    static func isHHmm(_ text: String) -> Bool {
        let parts = text.components(separatedBy: ":")
        guard parts.count == 2, let h = Int(parts[0]), let m = Int(parts[1]),
              (0...23).contains(h), (0...59).contains(m) else { return false }
        return true
    }

    /// 当前时刻是否落在静默时段（start > end 表示跨零点；未配置即不静默）
    public func inQuietHours(_ now: Date, calendar: Calendar = .current) -> Bool {
        guard !quietStart.isEmpty, !quietEnd.isEmpty else { return false }
        let comps = calendar.dateComponents([.hour, .minute], from: now)
        let minutes = (comps.hour ?? 0) * 60 + (comps.minute ?? 0)
        let start = Self.minutes(quietStart), end = Self.minutes(quietEnd)
        if start == end { return false }
        return start < end ? (minutes >= start && minutes < end)
                           : (minutes >= start || minutes < end)
    }

    private static func minutes(_ hhmm: String) -> Int {
        let parts = hhmm.components(separatedBy: ":")
        guard parts.count == 2, let h = Int(parts[0]), let m = Int(parts[1]) else { return -1 }
        return h * 60 + m
    }

    /// 人是否不在机器前：锁屏 / 显示器睡眠 / 无输入超过阈值，任一成立即算离开。
    ///
    /// 三条里对远程桌面真正起作用的是第三条。取不到信号时按「已离开」处理：
    /// 这一条 fail-open 与节流那条相反——宁可多发一条，也不要出现
    /// 「开关看着开了、其实永远不发」那种查不出来的失效
    public func isAway(_ signals: PresenceSignals) -> Bool {
        if signals.screenLocked || signals.displayAsleep { return true }
        guard let idle = signals.idleSeconds else { return true }
        return idle >= Double(normalized().awayIdleSeconds)
    }

    /// 判成「有人在」时给出依据（写进「最近外发」，被挡下的通知要说得出为什么被挡）
    public func presentReason(_ signals: PresenceSignals) -> String {
        guard let idle = signals.idleSeconds else { return "取不到输入时长" }
        return "距上次输入 \(Int(idle)) 秒，未达 \(normalized().awayIdleSeconds) 秒"
    }

    /// 该事件类型是否允许外发
    public func allows(_ kind: RemoteEventKind) -> Bool {
        switch kind {
        case .completed: return sendCompleted
        case .attention: return sendAttention
        case .costSpike: return sendCostSpike
        }
    }
}

/// 手写解码：每个字段都 `decodeIfPresent` 回落默认值。
/// 合成实现要求 JSON 含全部键，于是**以后每加一个字段，旧存档就整条解不开**——
/// `loadPolicy` 的回落是「总开关关闭」，用户看到的是通知悄悄不再外发，最难查的一类失效。
extension RemoteNotifyPolicy {
    private enum CodingKeys: String, CodingKey {
        case masterEnabled, sendCompleted, sendAttention, sendCostSpike
        case throttleSeconds, quietStart, quietEnd, onlyWhenAway, awayIdleSeconds
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        masterEnabled = try c.decodeIfPresent(Bool.self, forKey: .masterEnabled) ?? false
        sendCompleted = try c.decodeIfPresent(Bool.self, forKey: .sendCompleted) ?? true
        sendAttention = try c.decodeIfPresent(Bool.self, forKey: .sendAttention) ?? true
        sendCostSpike = try c.decodeIfPresent(Bool.self, forKey: .sendCostSpike) ?? true
        throttleSeconds = try c.decodeIfPresent(Int.self, forKey: .throttleSeconds) ?? 90
        quietStart = try c.decodeIfPresent(String.self, forKey: .quietStart) ?? ""
        quietEnd = try c.decodeIfPresent(String.self, forKey: .quietEnd) ?? ""
        onlyWhenAway = try c.decodeIfPresent(Bool.self, forKey: .onlyWhenAway) ?? false
        awayIdleSeconds = try c.decodeIfPresent(Int.self, forKey: .awayIdleSeconds) ?? 120
    }
}

public enum RemoteEventKind: String, Sendable {
    case completed, attention, costSpike
}

/// 「这台机器前现在有没有人」的原始信号。判定逻辑在 `RemoteNotifyPolicy.isAway(_:)`
/// （纯函数、可离线测），取信号那一层不掺判断——它要碰窗口服务器，测不了。
public struct PresenceSignals: Equatable, Sendable {
    public var screenLocked: Bool
    public var displayAsleep: Bool
    /// 距上一次键盘/鼠标输入的秒数；nil = 取不到
    public var idleSeconds: TimeInterval?

    public init(screenLocked: Bool = false, displayAsleep: Bool = false,
                idleSeconds: TimeInterval? = nil) {
        self.screenLocked = screenLocked; self.displayAsleep = displayAsleep
        self.idleSeconds = idleSeconds
    }

    /// 一个信号都取不到（非 GUI 进程、窗口服务器拒绝等）
    public static let unavailable = PresenceSignals(idleSeconds: nil)
}

// MARK: - 钥匙串

/// 密钥只存 macOS 钥匙串。本仓红线：源码与任何落盘配置里零硬编码 secret；
/// 密钥由用户自己录入，界面上永远只显示掩码，日志里绝不写值。
public enum RemoteSecret {
    public static let service = "com.agentisland.remote"

    /// 模板里允许出现的密钥占位（值永远不进 UserDefaults，运行时从钥匙串取出替换）
    public static func containsPlaceholder(_ text: String) -> Bool { text.contains("{key}") }

    /// 写入结果。`refused` 必须原样显示给用户：本 App 走 ad-hoc 签名，每次重新出包
    /// 代码标识都变，钥匙串可能弹「允许访问」甚至直接拒绝——静默吞掉就等于
    /// 用户以为存上了，之后每次外发都失败且没有任何线索。
    public enum WriteResult: Equatable, Sendable {
        case ok
        case refused(reason: String)
    }

    /// 写入即覆盖同名条目
    public static func write(_ value: String, for name: String) -> WriteResult {
        guard !name.isEmpty else { return .refused(reason: "条目名为空") }
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: name,
        ]
        // 刻意不设 kSecAttrAccessible：SecItem.h 写明它在 macOS 经典钥匙串上
        // 「除非同时给 kSecAttrSynchronizable，否则不支持」，传进去只会换来 errSecParam
        // 或一个不生效的字段。「不跟着 iCloud 账号跑」由 kSecAttrSynchronizable 的
        // 默认值 false 保证，不需要额外声明。
        query[kSecValueData as String] = Data(value.utf8)
        var status = SecItemAdd(query as CFDictionary, nil)
        if status == errSecDuplicateItem {
            // 只有确认要替换时才删旧值。反过来（先删再加）在 ad-hoc 签名下很危险：
            // 旧条目能删、新条目却被钥匙串拒，结果是一条没存上、旧值也没了，
            // 而界面只能显示「写入被拒」——用户以为自己还有凭据
            delete(name)
            status = SecItemAdd(query as CFDictionary, nil)
        }
        guard status == errSecSuccess else { return .refused(reason: describe(status)) }
        return .ok
    }

    /// 把 OSStatus 翻成人话；拿不到系统描述时退回状态码，绝不显示成「未知错误」
    static func describe(_ status: OSStatus) -> String {
        if status == errSecInteractionNotAllowed { return "钥匙串已锁定或要求交互但被拒绝" }
        if status == errSecAuthFailed { return "钥匙串访问被拒（代码标识变更后需在弹窗点「始终允许」）" }
        if let msg = SecCopyErrorMessageString(status, nil) as String?, !msg.isEmpty {
            return "\(msg)（\(status)）"
        }
        return "OSStatus \(status)"
    }

    public static func read(_ name: String) -> String? {
        guard !name.isEmpty else { return nil }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: name,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess,
              let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// 「存过没有」——设置页用它显示绿/红状态，**不把值读进界面内存**。
    /// 刻意不带 kSecReturnData：只要存在性
    public static func exists(_ name: String) -> Bool {
        guard !name.isEmpty else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: name,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        return SecItemCopyMatching(query as CFDictionary, nil) == errSecSuccess
    }

    @discardableResult
    public static func delete(_ name: String) -> Bool {
        guard !name.isEmpty else { return false }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: name,
        ]
        return SecItemDelete(query as CFDictionary) == errSecSuccess
    }

    /// 像不像一条凭据：16 位以上的纯字母数字。ntfy 主题这类用户自取的短名不会命中
    static func looksLikeToken(_ text: Substring) -> Bool {
        text.count >= 16 && text.allSatisfy { $0.isLetter || $0.isNumber }
    }

    /// 留首尾各 2 位，中间一律 •
    static func maskToken(_ text: Substring) -> String {
        "\(text.prefix(2))•••\(text.suffix(2))"
    }

    /// 掩码：只用于界面回显，绝不用它做日志或诊断输出
    public static func masked(_ value: String) -> String {
        guard value.count > 4 else { return String(repeating: "•", count: max(value.count, 1)) }
        let head = value.prefix(2)
        let tail = value.suffix(2)
        return "\(head)\(String(repeating: "•", count: min(value.count - 4, 12)))\(tail)"
    }
}


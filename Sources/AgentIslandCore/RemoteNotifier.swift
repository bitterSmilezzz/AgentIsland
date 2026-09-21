import Foundation

// MARK: - 外发调度：策略 → 渲染 → 传输 → 结果记账

/// 一条外发记录的结果（供设置页显示「最近一次：成功/失败/被抑制」，
/// 也供岛内提示——失败必须看得见）
public struct OutboundAttempt: Equatable, Hashable, Sendable {
    public let at: Date
    public let title: String
    public let outcome: OutboundOutcome
    public init(at: Date, title: String, outcome: OutboundOutcome) {
        self.at = at; self.title = title; self.outcome = outcome
    }

    public var shortText: String {
        switch outcome {
        case .delivered: return "已送达"
        case .suppressed(let reason): return "未发：\(reason)"
        case .notConfigured(let reason): return "未配置：\(reason)"
        case .failed(let reason): return "失败：\(reason)"
        }
    }}

/// 渲染出来的消息（不含密钥），供「发送预览」原样展示
public struct OutboundPreview: Equatable, Sendable {
    public var title: String
    public var body: String
    public var requestSummary: String
}

/// 事件 → 外发的调度器。
///
/// 三条硬规矩在这里落地：
/// ① **默认只带最小内容**（Agent 名 + 状态 + 多久前）。命令文本、文件路径这类
///    会离开这台机器的信息，必须 `includeActionDetail` 显式打开才进得来；
/// ② **失败如实**：任何非 delivered 都记账并回传，设置页看得到，绝不显示「已发送」；
/// ③ **密钥不出内存之外**：URL/授权码从钥匙串取，渲染结果只以掩码形式给界面。
public final class RemoteNotifier: @unchecked Sendable {
    public struct Inputs: Sendable {
        public var agentName: String
        /// 节流键用这个而不是 `agentName`：显示名可以被外部投递路径随便填，
        /// 每次换一个名字就等于绕过节流
        public var agentId: String
        public var kind: RemoteEventKind
        /// completed 当作「本次任务用时」；其余类型当作「事件发生至今」。
        /// 两条语义分开是因为外发的那一刻同类事件总是刚发生，写成「3 分前」是谎报
        public var seconds: TimeInterval
        /// 仅当通道配置打开 includeActionDetail 时才会进入正文
        public var actionDetail: String?
        public var message: String?

        public init(agentName: String, agentId: String? = nil, kind: RemoteEventKind,
                    seconds: TimeInterval, actionDetail: String? = nil, message: String? = nil) {
            self.agentName = agentName
            // 调用点拿不到 id 时退回显示名（自定义 Agent 与假事件就是这种情况），
            // 语义仍然是「同一个来源的同类事件在窗口内只发一次」
            self.agentId = agentId ?? agentName
            self.kind = kind; self.seconds = seconds
            self.actionDetail = actionDetail; self.message = message
        }
    }

    private let lock = NSLock()
    private var lastSent: [String: Date] = [:]       // key = agent + kind
    private var history: [OutboundAttempt] = []      // 有界，见 record(_:)
    /// 注入以便离线测试；生产用 HTTPTransport()
    private let transport: RemoteTransport
    /// 密钥解析（生产 = 钥匙串；测试 = 查表）
    private let secretReader: (String) -> String?

    public init(transport: RemoteTransport = HTTPTransport(),
                secretReader: @escaping (String) -> String? = { RemoteSecret.read($0) }) {
        self.transport = transport
        self.secretReader = secretReader
    }

    /// 纯函数：策略 + 配置 → 要发的内容。拆出来是因为它能被完整测试（不碰网络、不读钥匙串）
    public static func render(inputs: Inputs, config: RemoteChannelConfig,
                              kind: RemoteChannelKind) -> OutboundMessage {
        let state: String
        switch inputs.kind {
        case .completed: state = "任务完成"
        case .attention: state = "等待你确认"
        case .costSpike: state = "消耗告警"
        }
        let title = "\(inputs.agentName) · \(state)"
        let qualifier = inputs.kind == .completed
            ? "用时 \(DurationText.short(inputs.seconds))" : "刚刚"
        // 正文首行自带 Agent 名：X-Title 这类头字段在 ASCII 约束下可能被接收端截断，
        // 而正文是裸 UTF-8 字节，一定到得了
        var lines = ["\(inputs.agentName) · \(state)（\(qualifier)）"]
        if config.includeActionDetail, let detail = inputs.actionDetail, !detail.isEmpty {
            lines.append("动作：\(String(detail.prefix(80)))")
        }
        if config.includeActionDetail, let msg = inputs.message, !msg.isEmpty {
            lines.append(String(msg.prefix(120)))
        }
        // 不带 includeActionDetail 时，正文里绝不出现动作/路径/命令——
        // 这是「哪些字节离开这台机器」的唯一开关
        return OutboundMessage(title: title, body: lines.joined(separator: "\n"),
                              urgent: inputs.kind != .completed)
    }

    /// 渲染成实际请求（含密钥替换）。`masked` 为 true 时用于预览
    public static func renderRequest(message: OutboundMessage, kind: RemoteChannelKind,
                                     config: RemoteChannelConfig, secret: String?,
                                     masked: Bool) -> RenderedRequest {
        switch kind {
        case .ntfy:
            // 形态按官方文档：POST https://<服务器>/<主题>，标题与优先级走 X-Title / X-Priority
            let base = config.topicOrURL.hasPrefix("http") ? config.topicOrURL : "https://ntfy.sh"
            let topic = config.topicOrURL.hasPrefix("http") ? "" : config.topicOrURL
            let url = topic.isEmpty ? base : "\(base)/\(topic)"
            // 公开服务器单条上限 4096 字节，超了直接 400：先收标题再收正文，
            // 而不是整条丢出去拿一个用户看不懂的失败
            let capped = Self.clamp(message: message, url: url)
            return RenderedRequest(url: capped.url, method: "POST",
                                   headers: [HTTPField("X-Title", capped.title),
                                             HTTPField("X-Priority", message.urgent ? "4" : "3")],
                                   body: capped.body)
        case .customHTTP:
            let key = secret ?? ""
            // 地址里的值一律百分号编码；密钥本身按原值替换（各家都把 SendKey 放在路径里，
            // 它本来就是 URL 的一部分）
            let filled = Self.fill(config.urlTemplate, key: masked ? RemoteSecret.masked(key) : key,
                                   title: WireText.queryValue(message.title),
                                   body: WireText.queryValue(message.body))
            var headers: [HTTPField] = []
            let body: String
            if config.useJSONBody {
                // JSON 里的值必须走 JSON 转义：正文来自被监控的 Agent，一个裸引号
                // 或换行就能让整条 JSON 非法，而接收端只回一个看不懂的 4xx
                if config.bodyTemplate.isEmpty {
                    body = "{\"title\":\(WireText.jsonString(message.title)),"
                        + "\"body\":\(WireText.jsonString(message.body))}"
                } else {
                    body = Self.fill(config.bodyTemplate, key: masked ? RemoteSecret.masked(key) : key,
                                     title: WireText.jsonString(message.title).droppingJSONQuotes,
                                     body: WireText.jsonString(message.body).droppingJSONQuotes)
                }
                headers.append(HTTPField("Content-Type", "application/json"))
            } else {
                // 表单编码：值必须编码，否则正文里的 `&` 会凭空多出一个字段、
                // 一个换行会把请求体切成两段
                body = config.bodyTemplate.isEmpty
                    ? "title=\(WireText.formValue(message.title))&content=\(WireText.formValue(message.body))"
                    : Self.fill(config.bodyTemplate, key: masked ? RemoteSecret.masked(key) : key,
                                title: WireText.formValue(message.title),
                                body: WireText.formValue(message.body))
                headers.append(HTTPField("Content-Type", "application/x-www-form-urlencoded"))
            }
            return RenderedRequest(url: filled, method: "POST", headers: headers, body: body)
        case .smtpEmail:
            guard let secret else {
                return RenderedRequest(headers: [HTTPField("Subject", message.title)], body: message.body)
            }
            let target = SMTPTarget(host: config.smtpHost, port: config.smtpPort,
                                    user: config.smtpUser, password: masked ? "••••••" : secret,
                                    from: config.smtpUser, to: config.smtpTo)
            // Subject 放头里只为预览可读；真正发信由 SMTPClient 用 message.title 组头
            return RenderedRequest(method: "SMTP", headers: [HTTPField("Subject", message.title)],
                                   body: message.body, smtp: target)
        }
    }

    /// 模板替换：`{key}` 原值/掩码，`{title}`/`{body}` 由调用方**先按目标格式编码好**再传进来。
    /// 编码规则放在调用方而不是这里，是因为同一对占位符在查询串、表单体和 JSON 里
    /// 需要三种完全不同的转义（%XX / + / \")
    private static func fill(_ template: String, key: String, title: String, body: String) -> String {
        template.replacingOccurrences(of: "{key}", with: key)
            .replacingOccurrences(of: "{title}", with: title)
            .replacingOccurrences(of: "{body}", with: body)
    }

    /// ntfy 公开服务器单条消息上限 4096 字节（超出直接 400）。
    /// 截断优先保住首行（哪个 Agent + 什么状态），因为那是唯一必须到得了的信息。
    static let ntfyByteLimit = 4096

    private static func clamp(message: OutboundMessage, url: String)
        -> (url: String, title: String, body: String) {
        func utf8Clip(_ text: String, _ limit: Int) -> String {
            var bytes = 0
            var out = ""
            for scalar in text.unicodeScalars {
                bytes += scalar.utf8.count
                if bytes > limit { break }
                out.unicodeScalars.append(scalar)
            }
            return out
        }
        var body = message.body
        if body.utf8.count > ntfyByteLimit {
            let lines = body.components(separatedBy: "\n")
            let head = lines.first ?? ""
            let room = max(0, ntfyByteLimit - head.utf8.count - 1)
            body = lines.count == 1 ? utf8Clip(head, ntfyByteLimit)
                                    : head + "\n" + utf8Clip(lines.dropFirst().joined(separator: "\n"), room)
        }
        // 先编码再截：头值必须 ASCII（不编码时 URLSession 会静默丢掉整段中文），
        // 而编码后的长度是原文的三倍量级，所以预算按编码后的字节算
        return (url, utf8Clip(WireText.headerValue(message.title), 255), body)
    }

    /// 预览：不发送、不读真实密钥以外的东西，且返回值已脱敏
    public func preview(inputs: Inputs, kind: RemoteChannelKind, config: RemoteChannelConfig) -> OutboundPreview {
        let message = Self.render(inputs: inputs, config: config, kind: kind)
        let secret = secretReader(kind.defaultSecretName)
        let request = Self.renderRequest(message: message, kind: kind, config: config,
                                        secret: secret, masked: true)
        return OutboundPreview(title: message.title, body: message.body,
                               requestSummary: request.maskedPreview)
    }

    /// 发一条。返回真实结果；调用方据此决定要不要提示失败。
    /// `away` = 人是否不在机器前，仅在策略打开 `onlyWhenAway` 时参与判定。
    /// 默认 `.unavailable`（判定不了 → 按「已离开」照常发）：「以为开了其实永远不发」
    /// 是本仓最难查的那类失效，代价只是坐在机器前多收一条推送。
    @discardableResult
    public func deliver(inputs: Inputs, kind: RemoteChannelKind, config: RemoteChannelConfig,
                        policy: RemoteNotifyPolicy, presence: PresenceSignals = .unavailable,
                        now: Date = Date()) async -> OutboundOutcome {
        await send(inputs: inputs, kind: kind, config: config, policy: policy, now: now,
                   bypassPolicy: false, presence: presence)
    }

    /// 这条通道现在到底能不能发（与 `attempt` 用同一判据）。
    /// 岛内每拍都要先问这一句再决定要不要起 Task：不这么做的话，
    /// 「功能没配好」会被写成每次都跑一遍策略 + 读一次钥匙串的常规路径
    public func isConfigured(kind: RemoteChannelKind, config: RemoteChannelConfig) -> Bool {
        let secret = secretReader(kind.defaultSecretName) ?? ""
        return kind.missingField(config: config, hasSecret: !secret.isEmpty) == nil
    }

    /// 设置页的「发送测试」：绕过节流与静默时段（用户按下按钮就是要立刻看到结果），
    /// 但**总开关仍然管用**——它是这个功能的隐私闸门，若按一次测试就能出本机，
    /// 开关本身就不可信。测试结果同样进历史，且同样真的送出本机。
    @discardableResult
    public func testDeliver(kind: RemoteChannelKind, config: RemoteChannelConfig,
                            policy: RemoteNotifyPolicy, now: Date = Date()) async -> OutboundOutcome {
        let inputs = Inputs(agentName: "AgentIsland", kind: .attention, seconds: 0,
                            message: "这是一条测试消息")
        return await send(inputs: inputs, kind: kind, config: config,
                          policy: policy, now: now, bypassPolicy: true, presence: .unavailable)
    }

    private func send(inputs: Inputs, kind: RemoteChannelKind, config: RemoteChannelConfig,
                      policy: RemoteNotifyPolicy, now: Date, bypassPolicy: Bool,
                      presence: PresenceSignals) async -> OutboundOutcome {
        let outcome = await attempt(inputs: inputs, kind: kind, config: config, policy: policy,
                                    now: now, bypassPolicy: bypassPolicy, presence: presence)
        record(OutboundAttempt(at: now, title: Self.render(inputs: inputs, config: config, kind: kind).title,
                               outcome: outcome))
        return outcome
    }

    private func attempt(inputs: Inputs, kind: RemoteChannelKind, config: RemoteChannelConfig,
                         policy: RemoteNotifyPolicy, now: Date, bypassPolicy: Bool,
                         presence: PresenceSignals) async -> OutboundOutcome {
        let normalized = policy.normalized()
        // 总开关连「发送测试」一起挡：它是这个功能的隐私闸门，若按一次测试就能出本机，
        // 开关本身就不可信。也正因为排在前面，关掉时不会去碰钥匙串
        guard normalized.masterEnabled else { return .suppressed(reason: "总开关未开") }
        let secret = secretReader(kind.defaultSecretName) ?? ""
        let hasSecret = !secret.isEmpty
        // 事件类型开关排在绕过范围之外：关掉「等待你确认」的人不该被测试按钮代发一条
        guard normalized.allows(inputs.kind) else { return .suppressed(reason: "该类事件已关闭") }
        if !bypassPolicy {
            // 绕过只覆盖「什么时候打扰用户」这三条里的后两条（节流/静默/在场）
            if normalized.inQuietHours(now) { return .suppressed(reason: "静默时段") }
            // 与静默时段同级：都挡在配置检查之前，否则坐在机器前时会看到「缺主题」
            // 这种根本没走到的提示
            if normalized.onlyWhenAway, !normalized.isAway(presence) {
                return .suppressed(reason: "有人在机器前（\(normalized.presentReason(presence))）")
            }
        }
        // 配置检查放在节流之前：配置坏了是每次都发不出去，
        // 若先判节流，用户会看到「节流命中」而实际是根本没配好
        if let missing = kind.missingField(config: config, hasSecret: hasSecret) {
            return .notConfigured(reason: missing)
        }
        let throttleKey = "\(inputs.agentId)|\(inputs.kind.rawValue)"
        // 检查与登记必须在一个锁里完成：分两次取锁时，同一键的两个并发尝试会双双通过
        // 检查、各发一条，节流窗口形同两个窗口宽
        if !bypassPolicy, !claimThrottle(throttleKey, at: now,
                                         window: normalized.throttleSeconds) {
            return .suppressed(reason: "节流命中")
        }

        let message = Self.render(inputs: inputs, config: config, kind: kind)
        let request = Self.renderRequest(message: message, kind: kind, config: config,
                                        secret: hasSecret ? secret : nil, masked: false)
        let outcome = await transport.perform(request)
        // 只有真送达才保留节流占位：claimThrottle 已经预登记，失败必须回滚，
        // 否则一次网络抖动会吞掉后面一整段（默认 90 秒）的通知
        if !outcome.isDelivered { releaseThrottle(throttleKey, at: now) }
        return outcome
    }

    /// 原子地「查节流 + 预登记」。返回 false 表示落在窗口内，本次不该发
    private func claimThrottle(_ key: String, at: Date, window: Int) -> Bool {
        lock.lock(); defer { lock.unlock() }
        if let last = lastSent[key], at.timeIntervalSince(last) < Double(window) { return false }
        lastSent[key] = at
        // 键空间 = Agent 数 × 事件类型，但用户会增删档案；顺手收个上界
        if lastSent.count > 200 {
            for (k, v) in lastSent where at.timeIntervalSince(v) > 86_400 { lastSent[k] = nil }
        }
        return true
    }

    /// 没发出去就撤掉预登记：失败必须能立刻重试
    private func releaseThrottle(_ key: String, at: Date) {
        lock.lock(); defer { lock.unlock() }
        if lastSent[key] == at { lastSent[key] = nil }
    }

    /// 最近若干次外发结果（有界）
    public var recentAttempts: [OutboundAttempt] {
        lock.lock(); defer { lock.unlock() }
        return history
    }

    private func record(_ attempt: OutboundAttempt) {
        lock.lock(); defer { lock.unlock() }
        history.insert(attempt, at: 0)
        if history.count > 20 { history.removeLast(history.count - 20) }
    }
}

private extension String {
    /// 去掉 `WireText.jsonString` 加上的首尾引号：模板里的 `{title}` 通常在引号内部
    var droppingJSONQuotes: String {
        count >= 2 && hasPrefix("\"") && hasSuffix("\"")
            ? String(dropFirst().dropLast()) : self
    }
}

/// 「3 分前 / 2 小时前」——与岛内文案同口径，但这里必须自己实现（Core 不能依赖 UI）
public enum DurationText {
    public static func short(_ seconds: TimeInterval) -> String {
        let s = Int(max(0, seconds))
        if s < 60 { return "\(s)秒" }
        let m = s / 60
        if m < 60 { return "\(m)分" }
        let h = m / 60
        if h < 24 { return "\(h)小时" }
        return "\(h / 24)天"
    }
}

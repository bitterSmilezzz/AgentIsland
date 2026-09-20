import Foundation

// MARK: - 外发通道：渲染请求 + HTTP 传输 + SMTP 会话

/// 通道把「消息」渲染成一个**可读的请求描述**，两件事靠它完成：
/// ① 设置页的「发送预览」要让用户在真发之前看见原文（哪些字节离开这台机器）；
/// ② 失败时能报出「发到哪、什么状态码」而不是笼统的「发送失败」。
/// 注意 `headers` / `body` 里可能含密钥，预览与日志都必须走掩码。
/// 一个请求头。用结构体而不是元组：元组不参与 Equatable 的合成，
/// 而「渲染结果」必须可比较（测试要断言渲染出的请求逐字段一致）
public struct HTTPField: Equatable, Sendable {
    public var name: String
    public var value: String
    public init(_ name: String, _ value: String) { self.name = name; self.value = value }
}

public struct RenderedRequest: Equatable, Sendable {
    public var url: String
    public var method: String
    public var headers: [HTTPField]
    public var body: String
    /// SMTP 用：连接与认证参数（HTTP 通道忽略）
    public var smtp: SMTPTarget?

    public init(url: String = "", method: String = "POST", headers: [HTTPField] = [],
                body: String = "", smtp: SMTPTarget? = nil) {
        self.url = url
        self.method = method
        self.headers = headers
        self.body = body
        self.smtp = smtp
    }

    /// 供界面显示的脱敏文本：密钥/授权码/含 key 的 URL 一律打码
    public var maskedPreview: String {
        var lines = ["\(method) \(RemoteSecret.maskedURL(url))"]
        for field in headers {
            lines.append("\(field.name): \(SensitiveFields.shouldMask(header: field.name) ? RemoteSecret.masked(field.value) : field.value)")
        }
        if let smtp {
            lines.append("SMTP \(smtp.host):\(smtp.port) 发信=\(smtp.from) 收信=\(smtp.to)")
            lines.append("密码=••••••")
        }
        if !body.isEmpty { lines.append(""); lines.append(body) }
        return lines.joined(separator: "\n")
    }
}

public struct SMTPTarget: Equatable, Sendable {
    public var host: String
    public var port: Int
    public var user: String
    public var password: String
    public var from: String
    public var to: String
    /// 465 是隐式 TLS（连上就握手）；25/587 走 STARTTLS 升级
    public var implicitTLS: Bool { port == 465 }

    public init(host: String, port: Int, user: String, password: String, from: String, to: String) {
        self.host = host; self.port = port; self.user = user
        self.password = password; self.from = from; self.to = to
    }
}

public enum SensitiveFields {
    /// 预览里必须打码的请求头（其余如 X-Title / X-Priority 原样显示才有意义）
    public static func shouldMask(header name: String) -> Bool {
        let n = name.lowercased()
        return n.contains("authorization") || n.contains("token") || n.contains("key")
            || n == "x-webhook-key"
    }
}

public extension RemoteSecret {
    /// URL 里的密钥常见形态：`?token=xxx` / `&key=xxx` / `.../<SendKey>.send`。
    /// 预览与日志都必须过这一层——密钥泄漏最常见的路径就是「把完整 URL 打印出来了」
    static func maskedURL(_ url: String) -> String {
        var out = url
        for query in ["access_token", "token", "sendkey", "key", "webhook"] {
            var searchRange = out.startIndex..<out.endIndex
            while let hit = out.range(of: query + "=", options: .literal, range: searchRange) {
                let tail = out[hit.upperBound...]
                let valueEnd = tail.firstIndex(of: "&") ?? tail.endIndex
                let value = String(tail[..<valueEnd])
                if !value.isEmpty {
                    out.replaceSubrange(hit.upperBound..<valueEnd, with: masked(value))
                }
                searchRange = out.index(after: hit.lowerBound)..<out.endIndex
            }
        }
        let comps = out.split(separator: "/", omittingEmptySubsequences: true)
        guard comps.count >= 2 else { return out }
        // 一条规则同时管主机名与路径：Server酱 把 SendKey 放在主机名首段
        // （https://<32位key>.send），各家 webhook 把它放在路径段（/notify/<32位token>），
        // 而帮助文本就叫用户「把控制台给出的地址整段粘进来」——两条路都会被走到。
        // comps[0] 是 "https:"，所以从 [1] 起都是「主机名 + 各路径段」。
        // 判据是「按 . ? & = : / 切开后的某一段是 16+ 位纯字母数字」：
        // sctapi.ftqq.com 与 my-agent-island-topic 都不会命中，正常域名与主题名照原样显示
        for seg in comps[1...] {
            // 段里可能跟着 .send 之类的后缀，只遮其中的高熵片段
            guard let token = seg.split(whereSeparator: { ".?=&:".contains($0) })
                .first(where: { RemoteSecret.looksLikeToken($0) }) else { continue }
            out = out.replacingOccurrences(of: String(token),
                                           with: RemoteSecret.maskToken(token))
        }
        return out
    }
}

// MARK: - 传输

public protocol RemoteTransport: Sendable {
    func perform(_ request: RenderedRequest) async -> OutboundOutcome
}

/// 只用来拒绝重定向：URL 里带着 SendKey/token，跟着 302 走就是把凭据发给
/// 一个用户从没配置过的主机，而对方回 3xx 是常见做法而不是攻击
private final class NoRedirectDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

/// 真实 HTTP 传输。超时固定 10s：外发在后台队列跑，绝不能让一次卡住的请求拖住事件管线
///
/// 端口由用户填的 URL 决定，本层不额外限制（只有 SMTP 受 465 约束，见 `SMTPTarget`）。
/// 非 465 的邮箱地址会走到 STARTTLS 分支，而 `SMTPSocketConnection` 无法原地升级 TLS，
/// 于是如实回 `.failed(reason: "连不上 …")` ——不是静默丢弃。`missingField` 会在配置层先拦住
public struct HTTPTransport: RemoteTransport {
    public init() {}

    public func perform(_ request: RenderedRequest) async -> OutboundOutcome {
        if let smtp = request.smtp {
            return await SMTPClient.deliver(target: smtp,
                                            subject: request.headers.first(where: { $0.name == "Subject" })?.value ?? "",
                                            textBody: request.body)
        }
        guard let url = URL(string: request.url) else { return .failed(reason: "地址无法解析") }
        var req = URLRequest(url: url)
        req.httpMethod = request.method
        req.timeoutInterval = 10
        for field in request.headers { req.setValue(field.value, forHTTPHeaderField: field.name) }
        if !request.body.isEmpty { req.httpBody = Data(request.body.utf8) }
        // 带 delegate 的 session 必须配 async 版 `data(for:)`：实测用 completionHandler 版
        // `dataTask(with:)` 时每次都拿到 NSURLErrorDomain -999，请求根本没出门
        // （Apple 的规则是二者取其一，设了 delegate 就不该再指望闭包回调）。
        // 这个 delegate 只为拒绝重定向而存在：URL 里带着 SendKey/token，
        // 跟着 302 走等于把凭据发给一个用户从没配置过的主机
        let sessionConfig = URLSessionConfiguration.ephemeral
        sessionConfig.requestCachePolicy = .reloadIgnoringLocalCacheData
        sessionConfig.timeoutIntervalForRequest = 10
        let session = URLSession(configuration: sessionConfig, delegate: NoRedirectDelegate(),
                                 delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        do {
            let (_, response) = try await session.data(for: req)
            guard let http = response as? HTTPURLResponse else {
                return .failed(reason: "无 HTTP 响应")
            }
            if (200..<300).contains(http.statusCode) {
                // 注意语义：这是「对方接受了这条请求」。多数中转服务即使内部失败
                // 也回 200 + 一段错误 JSON，本层不去猜——设置页的文案对此写明
                return .delivered
            }
            return .failed(reason: "服务端返回 \(http.statusCode)")
        } catch let error as URLError where error.code == .cancelled {
            return .failed(reason: "请求被取消：目标回了重定向，本 App 不跟着走（密钥不外送给第三方）")
        } catch let error as URLError {
            return .failed(reason: "网络错误：\(error.localizedDescription)")
        } catch {
            return .failed(reason: "网络错误：\((error as NSError).domain) \((error as NSError).code)")
        }
    }
}

/// SMTP 会话抽象。测试把假服务器塞进来，就能断言命令序列而不碰网络。
public protocol SMTPSessionIO: AnyObject, Sendable {
    /// 读一行（含 CRLF）；超时返回 nil
    func readLine(timeout: TimeInterval) async -> String?
    /// 写一行（自带 CRLF）；连接已断返回 false
    func writeLine(_ text: String) async -> Bool
    func close()
}

/// 极简 SMTP 客户端。只实现「发一封纯文本信」所需的最小集合：不做 pipelining、
/// 不做多收件人优化，也**不做 STARTTLS**——`Network.framework` 不能在已建立的 TCP 上
/// 原地升级 TLS，所以非 465 的连接在 `SMTPSocketConnection.connect()` 就返回 false，
/// 这里没有第二条路可走（保留半条 STARTTLS 分支等于留一段永远不会执行的代码）。
public enum SMTPClient {
    /// 跑完一次会话并**保证**关掉连接：失败分支若不关，每条走 SMTP 的通道
    /// 每次失败都留下一条活的 NWConnection（长时运行下会攒出几十个 socket）
    public static func run(io: SMTPSessionIO, target: SMTPTarget, subject: String,
                           textBody: String, now: Date = Date(),
                           timeout: TimeInterval = 10) async -> OutboundOutcome {
        let outcome = await session(io: io, target: target, subject: subject,
                                    textBody: textBody, now: now, timeout: timeout)
        // 关掉连接归这里：session 的每条失败分支都是直接 return，不关就留下一条
        // 活的 TLS 连接（连同已解密的授权码）。放在 run 而不是调用方，
        // 是因为只有这一层能被离线测试观察到「失败路径也关了」
        io.close()
        return outcome
    }

    /// 与服务器对话的纯逻辑，全部 IO 走注入的 `SMTPSessionIO`——这是它能被离线测试的原因
    private static func session(io: SMTPSessionIO, target: SMTPTarget, subject: String,
                                textBody: String, now: Date,
                                timeout: TimeInterval) async -> OutboundOutcome {
        func expect(_ codes: [Int], _ label: String) async -> OutboundOutcome? {
            guard var line = await io.readLine(timeout: timeout) else {
                return .failed(reason: "\(label)：服务器无响应（超时）")
            }
            let code = Int(line.prefix(3)) ?? -1
            guard codes.contains(code) else { return .failed(reason: "\(label)：回复 \(code)") }
            // 「状态码-」是续行，必须读到出现「状态码空格」的那一行为止。
            // 只读一行的话，多行回复会把剩下的行留在流里，后面每一次读取全部错位
            while line.count >= 4, line.hasPrefix("\(code)-") {
                guard let next = await io.readLine(timeout: timeout) else {
                    return .failed(reason: "\(label)：续行后无响应")
                }
                line = next
                if !line.hasPrefix("\(code)") { return .failed(reason: "\(label)：回复流被打断") }
            }
            return nil
        }
        func ehlo() async -> OutboundOutcome? {
            guard await io.writeLine("EHLO \(SMTPHello.hostName)") else {
                return .failed(reason: "EHLO：连接已断开")
            }
            // 多行回复要读到「单空格续行结束」为止，否则后面的读全部错位
            while let line = await io.readLine(timeout: timeout) {
                if line.count >= 4, line.hasPrefix("250-") { continue }
                if line.hasPrefix("250") { return nil }
                return .failed(reason: "EHLO：回复异常")
            }
            return .failed(reason: "EHLO：服务器无响应")
        }

        if let fail = await expect([220], "问候") { return fail }
        if let fail = await ehlo() { return fail }

        guard await io.writeLine("AUTH LOGIN") else { return .failed(reason: "AUTH：连接已断开") }
        if let fail = await expect([334], "AUTH LOGIN") { return fail }
        guard await io.writeLine(Data(target.user.utf8).base64EncodedString()) else {
            return .failed(reason: "AUTH：连接已断开")
        }
        if let fail = await expect([334], "AUTH 用户名") { return fail }
        guard await io.writeLine(Data(target.password.utf8).base64EncodedString()) else {
            return .failed(reason: "AUTH：连接已断开")
        }
        if let fail = await expect([235], "AUTH 结果") { return fail }

        guard await io.writeLine("MAIL FROM:<\(WireText.smtpLine(target.from))>") else { return .failed(reason: "MAIL FROM：连接已断开") }
        if let fail = await expect([250], "MAIL FROM") { return fail }
        guard await io.writeLine("RCPT TO:<\(WireText.smtpLine(target.to))>") else { return .failed(reason: "RCPT TO：连接已断开") }
        if let fail = await expect([250, 251], "RCPT TO") { return fail }
        guard await io.writeLine("DATA") else { return .failed(reason: "DATA：连接已断开") }
        if let fail = await expect([354], "DATA") { return fail }

        for line in SMTPMessage.lines(subject: subject, body: textBody, target: target, now: now) {
            guard await io.writeLine(line) else { return .failed(reason: "正文发送中断") }
        }
        if let fail = await expect([250], "收件确认") { return fail }
        _ = await io.writeLine("QUIT")
        return .delivered
    }

    public static func deliver(target: SMTPTarget, subject: String, textBody: String) async -> OutboundOutcome {
        let io = SMTPSocketConnection(host: target.host, port: target.port, implicitTLS: target.implicitTLS)
        // 每条失败路径都直接 return，不关连接就会留下一个活的 NWConnection 与
        // 若干挂起的 send 续体（`writeLine` 用的是 .contentProcessed，只有取消才结算）
        defer { io.close() }
        guard await io.connect() else { return .failed(reason: "连不上 \(target.host):\(target.port)") }
        return await run(io: io, target: target, subject: subject, textBody: textBody)
    }
}

/// 邮件文本的构造与点填充（dot-stuffing）。单独拆出来是因为这两处最容易写错且完全可测。
public enum SMTPMessage {
    /// 返回要逐行写给服务器的内容，最后一行是终止符 "."
    public static func lines(subject: String, body: String, target: SMTPTarget, now: Date) -> [String] {
        var lines: [String] = [
            "From: \(WireText.smtpLine(target.from))",
            "To: \(WireText.smtpLine(target.to))",
            "Subject: =?UTF-8?B?\(Data(subject.utf8).base64EncodedString())?=",
            "Date: \(RFC822Date.string(from: now))",
            "MIME-Version: 1.0",
            "Content-Type: text/plain; charset=utf-8",
            "Content-Transfer-Encoding: 8bit",
            "",
        ]
        // 正文按 CR/LF 归一化，再以 CRLF 逐行发；行首的 "." 必须加倍，
        // 否则一行以 "." 开头的正文会被服务器当成邮件结束
        let normalized = body.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
        for line in normalized.components(separatedBy: "\n") {
            lines.append(line.hasPrefix(".") ? "." + line : line)
        }
        lines.append(".")
        return lines
    }
}

public enum SMTPHello {
    /// EHLO 主机名：拿系统给的本地主机名，拿不到就用一个通用值
    public static var hostName: String {
        let name = ProcessInfo.processInfo.hostName
        return name.isEmpty ? "localhost" : name
    }
}

/// RFC 822 日期（多数邮件服务器只认这个格式）
public enum RFC822Date {
    private static let formatter: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "EEE, dd MMM yyyy HH:mm:ss Z"
        return f
    }()
    public static func string(from date: Date) -> String { formatter.string(from: date) }
}

import Foundation

// MARK: - 本机 HTTP 入口的形状（切分、收完判定、路由、状态码、事件类型映射）
//
// 这些原先全写在 executable target 的 `LocalEventServer` 里，而测试 runner 链接不到它——
// 「什么样的请求会得到哪个状态码、哪句话」是本机对外的契约（03 号票要的就是这一层）。
// 写在测不到的地方就等于没写：本仓已经在 `/session` 上吃过一次「响应形状散在服务端、
// 只有 DTO 层断言」的亏。服务端从此只管传输：收字节、判断收没收完、把结论发出去。
//
// 安全边界不变：这里**不做任何可信判定**。令牌、pid、TTL、未知档案全在引擎那一侧，
// 本文件只决定「这条请求长得对不对、该走哪条路、回哪个状态码」。

public enum LocalEventHTTP {
    /// 一条请求最多收这么多字节。无鉴权端口上「慢慢发」也能长期占住 fd 与内存，
    /// 上限换一个明确的 413，而不是让内存跟着对端的手速走。
    public static let maxRequestBytes = 65_536
    /// HTTP 头部与正文之间的结束符（CRLF CRLF）
    private static let headerEnd = Data([13, 10, 13, 10])

    public struct Request: Equatable {
        public let method: String
        public let path: String
        /// 带查询串的那一份，`/session` 的 `?agent=` 要靠它
        public let fullPath: String
        public let headerPart: String
        public let body: String
    }

    /// 一条请求的落点。状态码与响应体都在这里给出——**服务端不许再自己写一遍数字**。
    /// （刻意不写 `Equatable`：`CLINotifyRequestDTO` 与事件类型都不带等值语义，
    /// 为了比较而给它们加协议等于为了测试改生产类型）
    public enum Outcome {
        /// 还没收完：传输层继续收，别急着答（分块的 POST 走到这里就答，等于把
        /// 「TCP 把它拆成几段」当成「客户端发了个畸形请求」）
        case needMoreData
        /// 超出 `maxRequestBytes`
        case tooLarge
        /// 通用端点的直接回复（200 / 400 / 404 / 405 / 503）
        case reply(status: Int, body: String)
        /// 交给 `/session` 那条链：判定在引擎，这里只保证方法是对的
        case session(Request)
        /// 交给 `/notify`：正文已经解得开，事件类型的映射也在这里定好了
        case notify(Request, CLINotifyRequestDTO, eventType: AgentTaskEvent.EventType)

        /// 引擎没就绪：那是**服务端**的状态，不该报成调用方的请求有问题（503 而不是 400）
        public static var noEngineReply: Outcome {
            .reply(status: SelfReportWire.noEngineStatus, body: LocalEventHTTP.json(
                CLISessionResultDTO(bound: false, reason: .noEngine, message: "引擎尚未就绪，本条未被记录")))
        }

        /// 超限那条回脸（413 而不是 400：正文没畸形，只是太大）
        public static var tooLargeReply: Outcome {
            .reply(status: 413, body: LocalEventHTTP.json(CLINotifyResultDTO(
                success: false,
                message: "Request body too large (limit \(LocalEventHTTP.maxRequestBytes) bytes)")))
        }
    }

    /// `Content-Length` 是判定「收完没有」的唯一依据。
    /// 没有这个头就按「头部结束即收完」处理——本机的调用方（curl、hook、CLI）都带这个头，
    /// 而 `Transfer-Encoding: chunked` 在这条回环链路上今天没有使用者，
    /// 支持它得先把分块解码写出来，别在没有需求时留一条没人验的路径。
    public static func contentLength(_ headerPart: String) -> Int? {
        guard let raw = SelfReportHeaders.value(headerPart, name: "Content-Length"),
              let value = Int(raw.trimmingCharacters(in: .whitespaces)),
              value >= 0 else { return nil }
        return value
    }

    public static func parse(_ data: Data, streamEnded: Bool = false) -> Outcome {
        guard let sep = data.range(of: headerEnd) else {
            if data.count > maxRequestBytes { return .tooLarge }
            // 头部结束符都没出现：连接还在发就继续等；已经发完/断了才回畸形
            return streamEnded ? .reply(status: 400, body: json(
                CLINotifyResultDTO(success: false, message: "Malformed HTTP request")))
                : .needMoreData
        }
        let headerData = data.subdata(in: data.startIndex..<sep.lowerBound)
        let bodyData = data.subdata(in: sep.upperBound..<data.endIndex)
        // 头与正文**分开解码**，但真正把「多字节被 TCP 切开」挡住的是上面那道收完判定：
        // 上一版一次 receive 就整包 `String(data:encoding:.utf8)`，一个切在中间的汉字让它返回 nil，
        // 于是合法的中文正文被拒成 400「Invalid encoding」。两步缺一步都会红（变异 m41 红，
        // 而只把这里改回整包解是**等价变异** m42：不红）——别把功劳记在其中一半头上
        guard let headerText = String(data: headerData, encoding: .utf8) else {
            return .reply(status: 400, body: json(
                CLINotifyResultDTO(success: false, message: "Invalid encoding")))
        }
        if let want = contentLength(headerText), bodyData.count < want {
            if data.count > maxRequestBytes { return .tooLarge }
            return streamEnded ? .reply(status: 400, body: json(
                CLINotifyResultDTO(success: false, message: "Malformed HTTP request")))
                : .needMoreData
        }
        guard let bodyText = String(data: bodyData, encoding: .utf8) else {
            return .reply(status: 400, body: json(
                CLINotifyResultDTO(success: false, message: "Invalid encoding")))
        }
        return route(LocalEventHTTP.Request(
            method: "", path: "", fullPath: "", headerPart: headerText, body: bodyText))
    }

    /// 请求行与路由。单独拆出来是为了能拿「已经切好的形状」直接测，不必拼字节。
    public static func route(_ raw: Request) -> Outcome {
        let requestLines = raw.headerPart.components(separatedBy: "\r\n")
        let tokens = requestLines.first.map { $0.components(separatedBy: " ") } ?? []
        guard tokens.count >= 2 else {
            // 原先这里还有一条「request line 为空」的 guard，那是不可达分支：
            // components(separatedBy:) 对空串也会返回 [""]，first 永远不为 nil
            return .reply(status: 400, body: json(
                CLINotifyResultDTO(success: false, message: "Invalid request line")))
        }
        let request = Request(method: tokens[0].uppercased(),
                              path: String(tokens[1].prefix { $0 != "?" }),
                              fullPath: tokens[1],
                              headerPart: raw.headerPart,
                              body: raw.body)
        switch (request.method, request.path) {
        case ("OPTIONS", _):
            return .reply(status: 200, body: "OK")
        case ("GET", "/health"), ("GET", "/ping"):
            return .reply(status: 200, body: #"{"status":"ok","service":"agentisland"}"#)
        case (_, "/session"):
            guard request.method == "POST" || request.method == "DELETE" else {
                return .reply(status: SelfReportWire.badMethodStatus, body: json(
                    CLISessionResultDTO(bound: false, reason: .badMethod,
                                        message: "/session 只接受 POST（声明/续报）与 DELETE（撤销）")))
            }
            return .session(request)
        case ("POST", "/notify"), ("POST", "/event"):
            guard let bodyData = request.body.data(using: .utf8),
                  let payload = try? JSONDecoder().decode(CLINotifyRequestDTO.self, from: bodyData) else {
                return .reply(status: 400, body: json(CLINotifyResultDTO(
                    success: false, message: "Invalid JSON payload. Expected CLINotifyRequestDTO")))
            }
            return .notify(request, payload, eventType: AgentTaskEvent.externalType(from: payload.type))
        default:
            return .reply(status: 404, body: json(
                CLINotifyResultDTO(success: false, message: "Not Found")))
        }
    }

    /// 状态码 → 原因短语。**逐码给**：把 405 写成 "Bad Request" 会让接入方以为是自己
    /// 正文写错了；而新增一格状态码（本轮的 413 就是）如果忘了跟着加，
    /// 老写法会把它悄悄降级成 "Bad Request"。认不出的码按**类别**给，不假装知道具体含义。
    public static func statusText(for code: Int) -> String {
        switch code {
        case 200: return "OK"
        case 400: return "Bad Request"
        case 404: return "Not Found"
        case 405: return "Method Not Allowed"
        case 413: return "Payload Too Large"
        case 503: return "Service Unavailable"
        default:
            if code >= 500 { return "Server Error" }
            if code >= 400 { return "Client Error" }
            return "OK"
        }
    }

    /// 编码成对外那串 JSON。失败时给一个**能看的**兜底而不是空串：空串会让客户端
    /// 收到一个 200 而正文是零字节，curl 上看不出任何异常。
    public static func json<T: Encodable>(_ value: T) -> String {
        guard let data = try? JSONEncoder().encode(value),
              let text = String(data: data, encoding: .utf8) else {
            return #"{"success":false}"#
        }
        return text
    }
}

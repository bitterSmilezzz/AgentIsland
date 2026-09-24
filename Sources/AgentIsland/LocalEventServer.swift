import Foundation
import Network
import AgentIslandCore

// MARK: - 本地事件接收端 (LocalEventServer - v0.0.77)
// 零外部依赖（基于 Apple 原生 Network.framework NWListener），只接受发往回环地址的连接。
// 允许本地脚本、CI/CD 任务、第三方工具（如 git hook、curl 或 agentisland notify）
// 毫秒级直推智能体完成/需要关注事件。
//
// 安全边界（v0.0.93 收紧）：`/notify` 没有鉴权——任何能连上它的进程都能往岛上写
// 「任务完成 / 需要你确认」。此前注释写着「仅绑定 127.0.0.1」，但 `NWParameters.tcp`
// 不设 `requiredLocalEndpoint` 实际监听的是 `*:41999`（lsof 实测 IPv6 双栈通配），
// 也就是同一局域网内任意主机都能伪造事件。现在能绑回环就只绑回环（macOS 14+ 的
// requiredLocalEndpoint）；绑不了的那条路（macOS 13）**没有**第二道复核，端口仍是局域网
// 可达，只 NSLog 一条警告——见 start/handleConnection，别把这里当成已经双向兜住了。

public final class LocalEventServer: @unchecked Sendable {
    public static let defaultPort: UInt16 = 41999
    private var listener: NWListener?
    private let queue = DispatchQueue(label: "com.agentisland.eventserver", qos: .utility)
    /// 在飞连接上限：无鉴权端口上，一条「只连不发」的 TCP 就能永久占住一个 fd。
    /// 用活连接表而不是计数器——计数一旦漏减就会把好端端的 notify 关门
    private let maxLiveConnections = 16
    private let connectionsLock = NSLock()
    private var liveConnections: [ObjectIdentifier: NWConnection] = [:]
    private weak var engine: ActivityEngine?

    public init(engine: ActivityEngine?) {
        self.engine = engine
    }

    public func start(port: UInt16 = defaultPort) {
        guard listener == nil else { return }
        // 令牌在「即将开始接受自报」这一刻生成，而不是等到第一个请求：接入命令
        // （06 号票的设置页）要能立刻把值带出来。生成走引擎那一份 store——
        // 两处各开一个文件句柄，就会有「一份看得到、一份看不到」的那天
        if let engine = engine {
            // nil 不是「令牌不匹配」而是「这台机器上拿不到可信通道」：不写一行日志，
            // 现象就是所有接入方长期收到 noToken，而没有任何地方指向目录或磁盘
            Task { @MainActor in
                if engine.selfReportTokens.ensure() == nil {
                    NSLog("[LocalEventServer] 可信自报令牌不可用（目录建不出来或写不进），"
                          + "/session 只会返回 noToken")
                }
            }
        }

        do {
            let parameters = NWParameters.tcp
            parameters.allowLocalEndpointReuse = true
            let endpointPort = NWEndpoint.Port(rawValue: port) ?? NWEndpoint.Port(rawValue: Self.defaultPort)!
            // 只绑 IPv4 回环：不给局域网留监听面。requiredLocalEndpoint 是 macOS 14+ 的公开 API，
            // 13 上设不了——此时监听面仍是通配，显式告警而不是假装安全。
            // 注意端口来源：设了 requiredLocalEndpoint 就**不能**再传 `on:`，两者同时给会让
            // `NWListener(using:on:)` 直接构造失败（实测），于是 notify 端点整个不监听。
            let newListener: NWListener
            if #available(macOS 14.0, *) {
                parameters.requiredLocalEndpoint = .hostPort(host: .ipv4(.loopback), port: endpointPort)
                newListener = try NWListener(using: parameters)
            } else {
                NSLog("[LocalEventServer] macOS < 14 无法限定回环监听：\(port) 对局域网可达，"
                      + "而 /notify 无鉴权（任何能连上它的进程都能向岛内伪造事件）")
                newListener = try NWListener(using: parameters, on: endpointPort)
            }

            newListener.newConnectionHandler = { [weak self] connection in
                self?.handleConnection(connection)
            }

            newListener.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    #if DEBUG
                    NSLog("[LocalEventServer] 监听服务已就绪: 127.0.0.1:%d", port)
                    #endif
                case .failed(let error):
                    NSLog("[LocalEventServer] 启动失败: %@", error.localizedDescription)
                case .cancelled:
                    #if DEBUG
                    NSLog("[LocalEventServer] 服务已停止")
                    #endif
                default:
                    break
                }
            }

            newListener.start(queue: queue)
            self.listener = newListener
        } catch {
            NSLog("[LocalEventServer] 异常: %@", error.localizedDescription)
        }
    }

    public func stop() {
        listener?.cancel()
        listener = nil
    }

    private func handleConnection(_ connection: NWConnection) {
        // 只接受回环来源由监听端的 requiredLocalEndpoint 保证（见 start）：
        // NWConnection 不公开 channel/localEndpoint，accept 后再复核这条路在公开 API 里不存在
        guard admit(connection) else {
            connection.cancel()
            return
        }
        // 只连不发的客户端：5s 后回收（否则该 fd 与相关对象一直自持）
        queue.asyncAfter(deadline: .now() + 5) { [weak self] in
            guard self?.isAdmitted(connection) == true else { return }
            connection.cancel()
        }
        connection.start(queue: queue)
        receiveNext(connection)
    }

    /// 记入活连接表并判断是否还在上限内（先把已终止的摘掉，保证上限不会因漏减而关门）
    private func admit(_ connection: NWConnection) -> Bool {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        liveConnections = liveConnections.filter { _, existing in
            switch existing.state {
            case .cancelled, .failed: return false
            default: return true
            }
        }
        guard liveConnections.count < maxLiveConnections else { return false }
        liveConnections[ObjectIdentifier(connection)] = connection
        return true
    }

    private func isAdmitted(_ connection: NWConnection) -> Bool {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        return liveConnections[ObjectIdentifier(connection)] != nil
    }

    private func dismiss(_ connection: NWConnection) {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        liveConnections[ObjectIdentifier(connection)] = nil
    }

    private func receiveNext(_ connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self = self else {
                connection.cancel()
                return
            }

            if let data = data, !data.isEmpty {
                self.processHTTPPayload(data, connection: connection)
                self.dismiss(connection)   // processHTTPPayload 各分支都以 cancel 收尾
                return
            }

            if isComplete || error != nil {
                connection.cancel()
            }
        }
    }

    private func processHTTPPayload(_ data: Data, connection: NWConnection) {
        guard let text = String(data: data, encoding: .utf8) else {
            sendResponse(connection: connection, statusCode: 400, body: #"{"success":false,"message":"Invalid encoding"}"#)
            return
        }

        // 简易 HTTP 协议切分
        let parts = text.components(separatedBy: "\r\n\r\n")
        guard parts.count >= 2 else {
            sendResponse(connection: connection, statusCode: 400, body: #"{"success":false,"message":"Malformed HTTP request"}"#)
            return
        }

        let headerPart = parts[0]
        let bodyPart = parts[1...].joined(separator: "\r\n\r\n")
        let requestLines = headerPart.components(separatedBy: "\r\n")
        guard let requestLine = requestLines.first else {
            sendResponse(connection: connection, statusCode: 400, body: #"{"success":false,"message":"Empty request line"}"#)
            return
        }

        let tokens = requestLine.components(separatedBy: " ")
        guard tokens.count >= 2 else {
            sendResponse(connection: connection, statusCode: 400, body: #"{"success":false,"message":"Invalid request line"}"#)
            return
        }

        let method = tokens[0].uppercased()
        let fullPath = tokens[1]
        let path = String(fullPath.prefix { $0 != "?" })

        // 跨域预检与健康检查
        if method == "OPTIONS" {
            sendResponse(connection: connection, statusCode: 200, body: "OK")
            return
        }

        if (path == "/health" || path == "/ping") && method == "GET" {
            sendResponse(connection: connection, statusCode: 200, body: #"{"status":"ok","service":"agentisland"}"#)
            return
        }

        if path == "/session" {
            handleSession(method: method, headerPart: headerPart, body: bodyPart,
                          query: SelfReportQuery.parse(from: fullPath),
                          connection: connection)
            return
        }

        guard method == "POST" && (path == "/notify" || path == "/event") else {
            sendResponse(connection: connection, statusCode: 404, body: #"{"success":false,"message":"Not Found"}"#)
            return
        }

        guard let bodyData = bodyPart.data(using: .utf8),
              let req = try? JSONDecoder().decode(CLINotifyRequestDTO.self, from: bodyData) else {
            sendResponse(connection: connection, statusCode: 400, body: #"{"success":false,"message":"Invalid JSON payload. Expected CLINotifyRequestDTO"}"#)
            return
        }

        let eventType: AgentTaskEvent.EventType
        switch req.type.lowercased() {
        case "attention", "confirm", "wait":
            eventType = .attention
        case "costspike", "cost", "budget", "alert":
            eventType = .costSpike
        default:
            eventType = .completed
        }

        let event = AgentTaskEvent(
            agentId: req.agent,
            agentName: req.agent,
            eventType: eventType,
            duration: 0,
            timestamp: Date(),
            pid: nil,
            message: req.message,
            detail: req.detail,
            // 这条路径与深链一样是「本机任意进程都能写」的入口：不带标记的话，
            // 岛内看不出是伪造的，远程外发的「外部投递一律不转发」闸门也会失效
            // （外发没有 shouldPeek 那类分级，直接就把文字送出去了）
            externallyDelivered: true
        )

        Task { @MainActor [weak self] in
            self?.engine?.postEvent(event)
        }

        let resultDTO = CLINotifyResultDTO(success: true, eventId: event.id.uuidString, message: "Event accepted")
        let responseData = (try? JSONEncoder().encode(resultDTO)) ?? Data()
        let responseString = String(data: responseData, encoding: .utf8) ?? #"{"success":true}"#

        sendResponse(connection: connection, statusCode: 200, body: responseString)
    }

    // MARK: - 可信自报入口（02 号票 / spec 第 3 节）
    //
    // 这里**只做 HTTP 形状**：路由、取值、状态码。所有判定（令牌、pid 绑定、TTL、
    // 未知档案、撤销成没成）都留在 ActivityEngine 与 SelfReportBinder 里——一旦在这里再判一次
    // 「这条可信吗」，就有了第二个答案，而两边对不上的那天就是用户看到的谎。
    // 令牌在这里只以一枚 `SelfReportCredential?` 的形式存在：那个类型的 init 是 internal，
    // 本 target 构造不出「已授权」，也写不出 `tokenAccepted: true` 这种拼写。

    private func handleSession(method: String, headerPart: String, body: String,
                               query: SelfReportQuery, connection: NWConnection) {
        guard method == "POST" || method == "DELETE" else {
            send(connection: connection, status: 405, text: json(
                CLISessionResultDTO(bound: false, reason: .badMethod,
                                    message: "/session 只接受 POST（声明/续报）与 DELETE（撤销）")))
            return
        }
        let providedToken = SelfReportHeaders.value(headerPart, name: SelfReportTokenStore.headerName)
        let bodyData = Data(body.utf8)
        // 注册表、令牌、绑定判定都在引擎那一侧（@MainActor），所以整条处理走一次跳转：
        // 若把令牌校验留在本队列上先判，就得让服务端自己持有第二份令牌文件句柄
        Task { @MainActor in
            guard let engine = self.engine else {
                // 引擎没就绪是**服务端**的状态，不该报成调用方的请求有问题（503 而不是 400）
                self.send(connection: connection, status: 503, text: self.json(CLISessionResultDTO(
                    bound: false, reason: .noEngine, message: "引擎尚未就绪，本条未被记录")))
                return
            }
            let credential = engine.authorizeSelfReport(providedToken)
            let ids = Set(engine.allProfiles.map { $0.id })
            if method == "DELETE" {
                // 没带凭证就**先不解析正文**：否则 400/`unknownAgent` 与 200/`noToken`
                // 的差集就是「这台机器装了哪些 Agent」的免费清单（POST 那边同理，见下）
                guard let credential else {
                    self.send(connection: connection, status: 200, text: self.json(
                        CLISessionRevokeDTO(revoked: false, reason: .noToken)))
                    return
                }
                // 撤销不该被迫重发一遍 state
                switch SelfReportPayload.identity(bodyData, knownAgentIDs: ids, queryAgent: query.agent) {
                case .failure(let rejection):
                    // 走到这里必然带凭证（上面那条 guard 已经把无凭据的 DELETE 答完了），
                    // 所以载荷为什么被拒可以照实说
                    self.send(connection: connection,
                              status: SelfReportWire.status(for: rejection.reason, trusted: true),
                              text: self.json(CLISessionRevokeDTO(
                                revoked: false,
                                reason: SelfReportWire.reason(for: rejection.reason, trusted: true))))
                case .success(let id):
                    switch engine.revokeSelfReport(agentID: id.agentID, sessionID: id.sessionID,
                                                   credential: credential) {
                    case .notFound:
                        // 「没有这条记录」进枚举，人话另给：脚本 switch reason，读 message 的是人
                        self.send(connection: connection, status: 200, text: self.json(
                            CLISessionRevokeDTO(revoked: false, reason: .notFound,
                                                message: "没有这条申报（可能从来没有，也可能已被容量/保质期收走）")))
                    case .revoked:
                        self.send(connection: connection, status: 200, text: self.json(
                            CLISessionRevokeDTO(revoked: true)))
                    }
                }
                return
            }
            let parsed = SelfReportPayload.parse(bodyData, knownAgentIDs: ids, queryAgent: query.agent)
            guard let credential else {
                // §3：无令牌不是「丢掉」，而是**落回**既有的无鉴权事件通道；
                // 而响应的形状（reason / 状态码）不取决于载荷——这两条都由 SelfReportWire 说，
                // 因为它是要被测试的那类口径，不是路由细节
                var message = "没有有效令牌，本条未记录、也不改变状态来源。"
                if case .accepted(let submission) = parsed {
                    // 悄悄丢弃是最坏结果（接入方看到 200 会以为通了）；反过来，
                    // 把「没投递」讲成「已投递」同样是谎 ⇒ 分叉的依据是 `post` 的返回值
                    message = SelfReportWire.untrustedMessage(
                        posted: SelfReportFallback.post(submission, to: engine),
                        state: submission.state)
                } else if case .rejected = parsed {
                    message += "带上有效令牌才会回具体原因（无凭据的请求一律只报 noToken，"
                        + "免得注册表被当字典查）。落回通道也因此没走。"
                }
                self.send(connection: connection, status: 200, text: self.json(
                    CLISessionResultDTO(bound: false, reason: .noToken, message: message)))
                return
            }
            guard case .accepted(let submission) = parsed else {
                // 解析与档案判定**先于**可信度：指向不存在档案的申报，配什么令牌都不该收
                //（§3「未知 agent id 直接 400，绝不自动建档」）
                if case .rejected(let reason, let detail) = parsed {
                    self.send(connection: connection,
                              status: SelfReportWire.status(for: reason, trusted: true),
                              text: self.json(CLISessionResultDTO(
                                bound: false,
                                reason: SelfReportWire.reason(for: reason, trusted: true),
                                message: detail)))
                }
                return
            }
            // 凭证到这里必然存在（上面那条 `guard let credential else` 已经把没带的请求
            // 连同落回通道一起答完了），而 `bindSelfReport` 要的就是非可选的它：
            // 不存在「带错令牌却走进绑定」的那条路
            switch engine.bindSelfReport(submission, credential: credential) {
            case .bound:
                // 拿**这条申报**的到期时刻，不是「该 Agent 最晚过期的那条」
                let record = engine.selfReportRecord(agentID: submission.agentID,
                                                     sessionID: submission.sessionID)
                self.send(connection: connection, status: 200, text: self.json(CLISessionResultDTO(
                    bound: true, expiresAt: record.map { $0.expiresAt.timeIntervalSince1970 })))
            case .pidMismatch(let why):
                self.send(connection: connection, status: 200, text: self.json(
                    CLISessionResultDTO(bound: false, reason: .pidMismatch, message: why)))
            case .profileGone(let why):
                self.send(connection: connection, status: 400, text: self.json(
                    CLISessionResultDTO(bound: false, reason: .profileGone, message: why)))
            }
        }
    }

    private func send(connection: NWConnection, status: Int, text: String) {
        sendResponse(connection: connection, statusCode: status, body: text)
    }

    private func json<T: Encodable>(_ dto: T) -> String {
        String(data: (try? JSONEncoder().encode(dto)) ?? Data(), encoding: .utf8) ?? "{}"
    }


    private func sendResponse(connection: NWConnection, statusCode: Int, body: String) {
        // 逐码给文案：405 报成 "Bad Request" 会让接入方以为是自己 body 写错了
        let statusText: String
        switch statusCode {
        case 200: statusText = "OK"
        case 404: statusText = "Not Found"
        case 405: statusText = "Method Not Allowed"
        case 503: statusText = "Service Unavailable"
        default: statusText = "Bad Request"
        }
        let bodyData = body.data(using: .utf8) ?? Data()
        let headers = [
            "HTTP/1.1 \(statusCode) \(statusText)",
            "Content-Type: application/json; charset=utf-8",
            "Content-Length: \(bodyData.count)",
            "Access-Control-Allow-Origin: *",
            "Connection: close",
            "",
            ""
        ].joined(separator: "\r\n")

        var fullData = headers.data(using: .utf8) ?? Data()
        fullData.append(bodyData)

        connection.send(content: fullData, completion: .contentProcessed { _ in
            connection.cancel()
        })
    }
}

// MARK: - `/session` 的 URL 参数与 header 取值
//
// 这两个纯函数住在 `AgentIslandCore.SelfReportQuery` / `SelfReportHeaders`：本 target 是
// executable，测试 runner 链接不到它，留在原地就等于「票的 Done-when 那一面零守卫」。

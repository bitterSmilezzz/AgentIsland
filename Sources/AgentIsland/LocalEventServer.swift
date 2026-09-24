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
    /// 分块到达的请求按连接累在这里。生命周期跟着 `liveConnections` 走：
    /// 答完、被回收、超时取消都会清掉（`dismiss` 与 `clearBuffer`），
    /// 上限由 `LocalEventHTTP.maxRequestBytes` 在解析前判——不然内存跟着对端的手速涨
    private var buffers: [ObjectIdentifier: Data] = [:]
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
        // 缓冲跟着连接走：漏一处就是「每条被超时回收的连接都留着一块内存」
        buffers[ObjectIdentifier(connection)] = nil
    }

    private func receiveNext(_ connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self = self else {
                connection.cancel()
                return
            }
            let streamEnded = isComplete || error != nil
            if let data, !data.isEmpty { self.append(data, to: connection) }
            let buffer = self.buffer(for: connection)
            if buffer.isEmpty {
                // 一条字节都没收到就断开的连接：今天也是直接回收，不回 400——
                // 对端已经走了，回它只会多一次无意义的写
                connection.cancel()
                self.dismiss(connection)
                return
            }
            if buffer.count > LocalEventHTTP.maxRequestBytes {
                self.answer(.tooLargeReply, connection: connection)
                return
            }
            switch LocalEventHTTP.parse(buffer, streamEnded: streamEnded) {
            case .needMoreData:
                // 分块到达：接着收。上一版在这里回 400「Malformed HTTP request」，
                // 等于把「TCP 把一条 POST 拆成几段」当成客户端发错了
                self.receiveNext(connection)
            case .tooLarge:
                self.answer(.tooLargeReply, connection: connection)
            case let .reply(status, body):
                self.send(connection: connection, status: status, text: body)
            case let .session(request):
                self.clearBuffer(connection)
                self.handleSession(request, connection: connection)
            case let .notify(request, payload, eventType):
                self.clearBuffer(connection)
                self.handleNotify(request, payload: payload, eventType: eventType, connection: connection)
            }
        }
    }

    /// 把一条请求的字节累到这条连接的缓冲上（上限由调用方判）
    private func append(_ data: Data, to connection: NWConnection) {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        buffers[ObjectIdentifier(connection), default: Data()].append(data)
    }

    private func buffer(for connection: NWConnection) -> Data {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        return buffers[ObjectIdentifier(connection)] ?? Data()
    }

    private func clearBuffer(_ connection: NWConnection) {
        connectionsLock.lock(); defer { connectionsLock.unlock() }
        buffers[ObjectIdentifier(connection)] = nil
    }

    /// 通用端点（含错误回脸）的统一出口：答完就收尾，缓冲跟着连接一起丢
    private func answer(_ outcome: LocalEventHTTP.Outcome, connection: NWConnection) {
        switch outcome {
        case let .reply(status, body):
            send(connection: connection, status: status, text: body)
        case .tooLarge:
            // 413 那句话只在 Core 里写一次，这里只负责把它发出去
            answer(.tooLargeReply, connection: connection)
        case .needMoreData:
            // 调用方说「还没收完」却又要直接作答：这条路径在生产里不存在，
            // 真走到了按超时回收连接（名额与缓冲一起还）
            connection.cancel()
            dismiss(connection)
        case let .session(request):
            handleSession(request, connection: connection)
        case let .notify(request, payload, eventType):
            handleNotify(request, payload: payload, eventType: eventType, connection: connection)
        }
    }

    private func handleNotify(_ request: LocalEventHTTP.Request, payload: CLINotifyRequestDTO,
                              eventType: AgentTaskEvent.EventType, connection: NWConnection) {
        let event = AgentTaskEvent(
            agentId: payload.agent,
            agentName: payload.agent,
            eventType: eventType,
            duration: 0,
            timestamp: Date(),
            pid: nil,
            message: payload.message,
            detail: payload.detail,
            // 这条路径与深链一样是「本机任意进程都能写」的入口：不带标记的话，
            // 岛内看不出是伪造的，远程外发的「外部投递一律不转发」闸门也会失效
            // （外发没有 shouldPeek 那类分级，直接就把文字送出去了）
            externallyDelivered: true
        )

        Task { @MainActor [weak self] in
            self?.engine?.postEvent(event)
        }

        let resultDTO = CLINotifyResultDTO(success: true, eventId: event.id.uuidString, message: "Event accepted")
        send(connection: connection, status: 200, text: LocalEventHTTP.json(resultDTO))
    }

    // MARK: - 可信自报入口（02 号票 / spec 第 3 节）
    //
    // 这里**只做 HTTP 形状**：路由、取值、状态码。所有判定（令牌、pid 绑定、TTL、
    // 未知档案、撤销成没成）都留在 ActivityEngine 与 SelfReportBinder 里——一旦在这里再判一次
    // 「这条可信吗」，就有了第二个答案，而两边对不上的那天就是用户看到的谎。
    // 令牌在这里只以一枚 `SelfReportCredential?` 的形式存在：那个类型的 init 是 internal，
    // 本 target 构造不出「已授权」，也写不出 `tokenAccepted: true` 这种拼写。

    private func handleSession(_ request: LocalEventHTTP.Request, connection: NWConnection) {
        // 方法白名单在 `LocalEventHTTP.route` 里判（那里回 405），走到这里必然是 POST/DELETE
        let method = request.method
        let providedToken = SelfReportHeaders.value(request.headerPart,
                                                    name: SelfReportTokenStore.headerName)
        let bodyData = Data(request.body.utf8)
        let query = SelfReportQuery.parse(from: request.fullPath)
        // 注册表、令牌、绑定判定都在引擎那一侧（@MainActor），所以整条处理走一次跳转：
        // 若把令牌校验留在本队列上先判，就得让服务端自己持有第二份令牌文件句柄
        Task { @MainActor in
            guard let engine = self.engine else {
                // 引擎没就绪是**服务端**的状态，不该报成调用方的请求有问题（503 而不是 400）
                self.answer(.noEngineReply, connection: connection)
                return
            }
            let credential = engine.authorizeSelfReport(providedToken)
            let ids = Set(engine.allProfiles.map { $0.id })
            if method == "DELETE" {
                // 没带凭证就**先不解析正文**：否则 400/`unknownAgent` 与 200/`noToken`
                // 的差集就是「这台机器装了哪些 Agent」的免费清单（POST 那边同理，见下）
                guard let credential else {
                    self.send(connection: connection, status: SelfReportWire.untrustedStatus, text: LocalEventHTTP.json(
                        CLISessionRevokeDTO(revoked: false, reason: SelfReportWire.untrustedReason)))
                    return
                }
                // 撤销不该被迫重发一遍 state
                switch SelfReportPayload.identity(bodyData, knownAgentIDs: ids, queryAgent: query.agent) {
                case .failure(let rejection):
                    // 走到这里必然带凭证（上面那条 guard 已经把无凭据的 DELETE 答完了），
                    // 所以载荷为什么被拒可以照实说
                    self.send(connection: connection,
                              status: SelfReportWire.status(for: rejection.reason),
                              text: LocalEventHTTP.json(CLISessionRevokeDTO(
                                revoked: false,
                                reason: SelfReportWire.reason(for: rejection.reason, trusted: true))))
                case .success(let id):
                    switch engine.revokeSelfReport(agentID: id.agentID, sessionID: id.sessionID,
                                                   credential: credential) {
                    case .notFound:
                        // 「没有这条记录」进枚举，人话另给：脚本 switch reason，读 message 的是人
                        self.send(connection: connection, status: 200, text: LocalEventHTTP.json(
                            CLISessionRevokeDTO(revoked: false, reason: .notFound,
                                                message: "没有这条申报（可能从来没有，也可能已被容量/保质期收走）")))
                    case .revoked:
                        self.send(connection: connection, status: 200, text: LocalEventHTTP.json(
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
                self.send(connection: connection, status: SelfReportWire.untrustedStatus, text: LocalEventHTTP.json(
                    CLISessionResultDTO(bound: false, reason: SelfReportWire.untrustedReason,
                                        message: message)))
                return
            }
            guard case .accepted(let submission) = parsed else {
                // 解析与档案判定**先于**可信度：指向不存在档案的申报，配什么令牌都不该收
                //（§3「未知 agent id 直接 400，绝不自动建档」）
                if case .rejected(let reason, let detail) = parsed {
                    self.send(connection: connection,
                              status: SelfReportWire.status(for: reason),
                              text: LocalEventHTTP.json(CLISessionResultDTO(
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
                self.send(connection: connection, status: 200, text: LocalEventHTTP.json(CLISessionResultDTO(
                    bound: true, expiresAt: record.map { $0.expiresAt.timeIntervalSince1970 })))
            case .pidMismatch(let why):
                self.send(connection: connection, status: 200, text: LocalEventHTTP.json(
                    CLISessionResultDTO(bound: false, reason: .pidMismatch, message: why)))
            case .profileGone(let why):
                self.send(connection: connection, status: SelfReportWire.rejectionStatus, text: LocalEventHTTP.json(
                    CLISessionResultDTO(bound: false, reason: .profileGone, message: why)))
            }
        }
    }

    private func send(connection: NWConnection, status: Int, text: String) {
        // 答出去就等于这条连接用完了：**先腾出名额再写**。
        // `maxLiveConnections` 只有 16，漏一处回收就是「服务用满 16 条之后把所有 notify 关在门外」——
        // 上一版靠调用方在 processHTTPPayload 之后统一 dismiss，改成异步分支后那条兜底没了，
        // 所以清理必须挂在唯一的出口上，而不是挂在每个调用点
        dismiss(connection)
        sendResponse(connection: connection, statusCode: status, body: text)
    }


    private func sendResponse(connection: NWConnection, statusCode: Int, body: String) {
        // 原因短语的表在 Core（`LocalEventHTTP.statusText`）：状态码是对外契约的一格，
        // 写在这份链接不到测试的 target 里，新增一格就会静默降级成 "Bad Request"
        let statusText = LocalEventHTTP.statusText(for: statusCode)
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

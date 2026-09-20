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
// 也就是同一局域网内任意主机都能伪造事件。现在两处都堵：能绑回环就只绑回环，
// 并在 accept 后按本地端点复核（macOS 13 上 requiredLocalEndpoint 不可用时的兜底）。

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
        let path = tokens[1]

        // 跨域预检与健康检查
        if method == "OPTIONS" {
            sendResponse(connection: connection, statusCode: 200, body: "OK")
            return
        }

        if (path == "/health" || path == "/ping") && method == "GET" {
            sendResponse(connection: connection, statusCode: 200, body: #"{"status":"ok","service":"agentisland"}"#)
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
            detail: req.detail
        )

        Task { @MainActor [weak self] in
            self?.engine?.postEvent(event)
        }

        let resultDTO = CLINotifyResultDTO(success: true, eventId: event.id.uuidString, message: "Event accepted")
        let responseData = (try? JSONEncoder().encode(resultDTO)) ?? Data()
        let responseString = String(data: responseData, encoding: .utf8) ?? #"{"success":true}"#

        sendResponse(connection: connection, statusCode: 200, body: responseString)
    }

    private func sendResponse(connection: NWConnection, statusCode: Int, body: String) {
        let statusText = statusCode == 200 ? "OK" : (statusCode == 404 ? "Not Found" : "Bad Request")
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

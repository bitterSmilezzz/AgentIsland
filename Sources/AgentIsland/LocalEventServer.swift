import Foundation
import Network
import AgentIslandCore

// MARK: - 本地事件接收端 (LocalEventServer - v0.0.77)
// 零外部依赖（基于 Apple 原生 Network.framework NWListener），仅绑定 127.0.0.1 端口 41999。
// 允许本地脚本、CI/CD 任务、第三方工具（如 git hook、curl 或 agentisland notify）
// 毫秒级直推智能体完成/需要关注事件。

public final class LocalEventServer: @unchecked Sendable {
    public static let defaultPort: UInt16 = 41999
    private var listener: NWListener?
    private let queue = DispatchQueue(label: "com.agentisland.eventserver", qos: .utility)
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
            let newListener = try NWListener(using: parameters, on: endpointPort)

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
        connection.start(queue: queue)
        receiveNext(connection)
    }

    private func receiveNext(_ connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self = self else {
                connection.cancel()
                return
            }

            if let data = data, !data.isEmpty {
                self.processHTTPPayload(data, connection: connection)
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

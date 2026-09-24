import Foundation
@testable import AgentIslandCore

// MARK: - 本机 HTTP 入口的契约（03 号票：真 socket 之外的那一半）
//
// 这些断言今天只能写在 executable target 里——而测试 runner 链接不到它，所以「什么形状的
// 请求得到哪个状态码、哪句话」此前只在 DTO 层测过。本轮把切分/路由/状态码/事件类型映射
// 全部下沉到 `LocalEventHTTP`，于是它们第一次可测。
// 注意这里**不碰 41999 端口**：真连接那一半（分块到达、只连不发、并发上限）需要跑起来的
// 服务端，写在 03 号票的 Comments 里，用 `LocalEventHTTP` 的字节级入参在本地等价复现。

enum LocalEventHTTPTests {
    private static func request(_ line: String, headers: [String] = [], body: String = "") -> Data {
        var text = line
        for header in headers { text += "\r\n" + header }
        text += "\r\n\r\n" + body
        return Data(text.utf8)
    }
    private static func status(_ outcome: LocalEventHTTP.Outcome) -> Int? {
        if case let .reply(code, _) = outcome { return code }
        return nil
    }
    private static func body(_ outcome: LocalEventHTTP.Outcome) -> String? {
        if case let .reply(_, text) = outcome { return text }
        return nil
    }
    private static func isNeedMore(_ outcome: LocalEventHTTP.Outcome) -> Bool {
        if case .needMoreData = outcome { return true }
        return false
    }
    private static func sessionRequest(_ outcome: LocalEventHTTP.Outcome) -> LocalEventHTTP.Request? {
        if case let .session(request) = outcome { return request }
        return nil
    }
    private static func notifyOutcome(_ outcome: LocalEventHTTP.Outcome)
        -> (CLINotifyRequestDTO, AgentTaskEvent.EventType)? {
        if case let .notify(_, payload, eventType) = outcome { return (payload, eventType) }
        return nil
    }

    @MainActor
    static func register() {
        // MARK: 切分与「收完没有」

        TestKit.test("HTTP: 一条完整的 GET /health 得到 200 与健康检查那句") {
            let out = LocalEventHTTP.parse(request("GET /health HTTP/1.1",
                                                   headers: ["Host: 127.0.0.1"]))
            try expectEqual(status(out), 200)
            try expectEqual(body(out), #"{"status":"ok","service":"agentisland"}"#)
        }

        TestKit.test("HTTP: 头部还没结束就答，等于把分块请求拒成畸形——必须继续收") {
            // 这是本轮修的缺陷：上一版一次 receive 切一次，切不开就回
            // 400 "Malformed HTTP request"。TCP 把一条稍大的 POST 拆成几段是常态。
            let half = Data("POST /notify HTTP/1.1\r\nContent-Length: 40\r\n".utf8)
            try expectTrue(isNeedMore(LocalEventHTTP.parse(half)),
                           "没有 CRLFCRLF 且连接没断 ⇒ 等下一块")
            // 对端就此断了：这时才允许回畸形
            if case .needMoreData = LocalEventHTTP.parse(half, streamEnded: true) {
                throw TestError(message: "流已经结束还在等，等于这条连接永远不答")
            }
            let closed = LocalEventHTTP.parse(half, streamEnded: true)
            try expectEqual(status(closed), 400)
            try expectTrue(body(closed)?.contains("Malformed") == true, "\(body(closed) ?? "nil")")
        }

        TestKit.test("HTTP: 正文分两块到达，补齐前不收，补齐后按完整请求处理") {
            let bodyText = #"{"agent":"claude","type":"attention","message":"等你确认"}"#
            let first = Data("POST /notify HTTP/1.1\r\nContent-Length: \(bodyText.utf8.count)\r\n\r\n"
                                .utf8.prefix(30))
            try expectTrue(isNeedMore(LocalEventHTTP.parse(first)),
                           "连头部都没收完（30 字节）不该作答")
            let partial = Data(("POST /notify HTTP/1.1\r\nContent-Length: \(bodyText.utf8.count)\r\n\r\n"
                                    + String(bodyText.prefix(10))).utf8)
            try expectTrue(isNeedMore(LocalEventHTTP.parse(partial)),
                           "正文只到一半（Content-Length 还没满足）不该作答")
            let whole = LocalEventHTTP.parse(request("POST /notify HTTP/1.1",
                                                     headers: ["Content-Length: \(bodyText.utf8.count)"],
                                                     body: bodyText))
            try expectNotNil(notifyOutcome(whole), "补齐之后必须当一条正常请求处理")
        }

        TestKit.test("HTTP: 多字节字符被切开时不许整包按 UTF-8 解失败回 400") {
            // 上一版把整包一次 `String(data:encoding:.utf8)`：一个汉字被 TCP 切在中间就返回 nil，
            // 于是合法的请求被拒成 "Invalid encoding"。现在头与正文分开解，且先看收完没有。
            let bodyText = #"{"agent":"claude","type":"attention","message":"等你确认下一步"}"#
            let head = "POST /notify HTTP/1.1\r\nContent-Length: \(bodyText.utf8.count)\r\n\r\n"
            let bytes = Array((head + bodyText).utf8)
            let cutHere = bytes.count - 2          // 落在最后一个汉字的中间
            try expectTrue(isNeedMore(LocalEventHTTP.parse(Data(bytes[0..<cutHere]))),
                           "半字符 + 正文不足 ⇒ 是「还没收完」，不是「编码坏了」")
        }

        TestKit.test("HTTP: 超过上限给 413 那句，而不是继续收或降级成 400") {
            let huge = request("POST /notify HTTP/1.1",
                               headers: ["Content-Length: 999999"],
                               body: String(repeating: "字", count: 30_000))
            try expectTrue(huge.count > LocalEventHTTP.maxRequestBytes, "样本要真的越过上限")
            let out = LocalEventHTTP.parse(huge)
            if case .tooLarge = out {} else {
                throw TestError(message: "越过上限必须报 tooLarge，实得 \(out)")
            }
            let reply = LocalEventHTTP.Outcome.tooLargeReply
            try expectEqual(status(reply), 413)
            try expectTrue(body(reply)?.contains("too large") == true, "\(body(reply) ?? "nil")")
        }

        TestKit.test("HTTP: 头部不是 UTF-8 才回 Invalid encoding") {
            var bytes = [UInt8]("GET /health HTTP/1.1\r\nX: ".utf8)
            bytes += [0xFF, 0xFE, 0xFF]
            bytes += [13, 10, 13, 10]
            let out = LocalEventHTTP.parse(Data(bytes))
            try expectEqual(status(out), 400)
            // 不比整串：JSONEncoder 的键序不是契约（这里曾经钉死过，换 Swift 版本就红）
            let text = try XCTRequire(body(out), "400 得带正文")
            try expectTrue(text.contains(#""message":"Invalid encoding""#), text)
            try expectTrue(text.contains(#""success":false"#), text)
        }

        // MARK: 路由与状态码

        TestKit.test("HTTP: OPTIONS 预检与 /ping 别名照旧 200") {
            try expectEqual(body(LocalEventHTTP.parse(request("OPTIONS /notify HTTP/1.1"))), "OK")
            try expectEqual(body(LocalEventHTTP.parse(request("GET /ping HTTP/1.1"))),
                            #"{"status":"ok","service":"agentisland"}"#)
        }

        TestKit.test("HTTP: GET /session 回 405 并说清只收哪两个方法") {
            let out = LocalEventHTTP.parse(request("GET /session HTTP/1.1"))
            try expectEqual(status(out), 405)
            let text = try XCTRequire(body(out), "405 得带正文")
            try expectTrue(text.contains("\"reason\":\"badMethod\""), text)
            try expectTrue(text.contains("POST") && text.contains("DELETE"), "要指名收哪两个方法：\(text)")
        }

        TestKit.test("HTTP: POST 与 DELETE 的 /session 才交给那条链，且头部与查询串原样带上") {
            // 方法大小写不敏感是今天的行为：`delete` 与 `POST` 都要认，且要归一化成大写
            for (method, want) in [("POST", "POST"), ("delete", "DELETE")] {
                let out = LocalEventHTTP.parse(request(
                    "\(method) /session?agent=claude HTTP/1.1",
                    headers: ["x-agentisland-token: abc", "Content-Length: 2"], body: "{}"))
                let req = try XCTRequire(sessionRequest(out), "\(method) /session 该走 session")
                try expectEqual(req.method, want, "\(method) 该归一化成 \(want)")
                try expectEqual(req.fullPath, "/session?agent=claude")
                try expectEqual(req.body, "{}")
                // header 名大小写混用得靠 Core 那个不区分大小写的取值函数——它决定令牌能不能被读到
                try expectEqual(SelfReportHeaders.value(req.headerPart,
                                                        name: SelfReportTokenStore.headerName), "abc")
            }
        }

        TestKit.test("HTTP: 未知路径 404，请求行残缺 400，坏 JSON 400 带 DTO 名字") {
            try expectEqual(status(LocalEventHTTP.parse(request("GET /nope HTTP/1.1"))), 404)
            let garbage = try XCTRequire(body(LocalEventHTTP.parse(request("GARBAGE"))), "要有正文")
            try expectTrue(garbage.contains(#""message":"Invalid request line""#), garbage)
            let bad = LocalEventHTTP.parse(request("POST /notify HTTP/1.1",
                                                   headers: ["Content-Length: 3"], body: "{a}"))
            try expectEqual(status(bad), 400)
            try expectTrue(body(bad)?.contains("CLINotifyRequestDTO") == true,
                           "坏正文要把**期望的形状**说给调用方：\(body(bad) ?? "nil")")
        }

        TestKit.test("外部入口: type 映射表逐格核，认不出的才落到 completed（深链与 HTTP 同一张）") {
            let table: [(String, AgentTaskEvent.EventType)] = [
                ("attention", .attention), ("confirm", .attention), ("wait", .attention),
                ("costspike", .costSpike), ("cost", .costSpike), ("budget", .costSpike),
                ("alert", .costSpike), ("done", .completed), ("", .completed),
            ]
            for (raw, want) in table {
                try expectEqual(AgentTaskEvent.externalType(from: raw), want, raw)
            }
            // 大小写：接入方写 "ATTENTION" 与 "Alert" 都是常见手滑
            try expectEqual(AgentTaskEvent.externalType(from: "ATTENTION"), .attention)
            try expectEqual(AgentTaskEvent.externalType(from: "Alert"), .costSpike)
        }

        TestKit.test("HTTP: 解码后的 /notify 载荷形状要交回服务端，别在 Core 里造事件") {
            let out = LocalEventHTTP.parse(request(
                "POST /event HTTP/1.1",
                headers: ["Content-Length: 60"],
                body: #"{"agent":"codex","type":"attention","message":"看这里","detail":"d"}"#))
            let pair = try XCTRequire(notifyOutcome(out), "该交回 notify")
            try expectEqual(pair.0.agent, "codex")
            try expectEqual(pair.0.detail, "d")
            try expectEqual(pair.1, .attention, "事件类型在 Core 那张表里定一次")
        }

        // MARK: 原因短语

        TestKit.test("HTTP: 原因短语逐码给，新增的状态码不许被降级成 Bad Request") {
            try expectEqual(LocalEventHTTP.statusText(for: 405), "Method Not Allowed")
            try expectEqual(LocalEventHTTP.statusText(for: 503), "Service Unavailable")
            // 本轮新加的那一格：上一版的表里没有它，会被 default 悄悄写成 "Bad Request"
            try expectEqual(LocalEventHTTP.statusText(for: 413), "Payload Too Large")
            try expectEqual(LocalEventHTTP.statusText(for: 499), "Client Error",
                            "认不出的 4xx 按类别给，别假装知道具体含义")
            try expectEqual(LocalEventHTTP.statusText(for: 599), "Server Error")
        }

        // MARK: 结构棘轮：服务端不许再自己写一遍契约

        TestKit.test("结构: 状态码与事件类型映射只许在 Core 那一处") {
            let serverText = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIsland/LocalEventServer.swift"))
            try expectTrue(serverText.contains("LocalEventHTTP.parse("),
                           "传输层必须把切分与路由交给 Core，别在 executable 里再解一次")
            // 错误状态码不许出现在服务端：405/503/400/404/413 全部由 Core 给出
            try expectFalse(serverText.contains("status: 4"),
                            "服务端不许自己写 4xx 状态码——那张表在 LocalEventHTTP")
            try expectFalse(serverText.contains("status: 5"),
                            "服务端不许自己写 5xx 状态码（503 走 Outcome.noEngineReply）")
            // 原因短语那张表也只在 Core：服务端不许再逐码赋值（参数名 statusCode 是合法的）
            try expectTrue(serverText.contains("LocalEventHTTP.statusText(for:"),
                           "响应行的原因短语必须问 Core")
            try expectFalse(serverText.contains("case 200:"),
                            "服务端不许还自己 switch 一遍状态码——那是第二张原因短语表")
            try expectFalse(serverText.contains("\"costspike\""),
                            "/notify 的 type 映射只许在 Core 那张表里，服务端不许再现写")
            // 同一条口径的**第二份**此前藏在深链里：`agentisland://notify?type=alert` 与
            // `POST /notify {"type":"alert"}` 各抄一遍，改一边就分叉
            let routerText = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIsland/URLSchemeRouter.swift"))
            try expectTrue(routerText.contains("AgentTaskEvent.externalType(from:"),
                           "深链的 type 必须问同一张表")
            try expectFalse(routerText.contains("\"costspike\""),
                            "深链不许自己再写一份 type 映射——两处会有一边漏改")
            try expectFalse(serverText.contains("JSONEncoder()"),
                            "DTO 变 JSON 只有一处（LocalEventHTTP.json），两处会有一边漏改")
            // 传输层唯一的自由是「没收完就别答」。这一格只能按书写形态守：
            // 服务端在 executable target 里，runner 链接不到它，行为级测试碰不到这段胶水
            let chunks = serverText.components(separatedBy: "case ")
            let needMore = try XCTRequire(chunks.first(where: { $0.hasPrefix(".needMoreData:") }),
                                          "服务端必须显式处理 needMoreData")
            try expectTrue(needMore.contains("receiveNext"),
                           "没收完要继续收，不是回头就答")
            try expectFalse(needMore.contains("send("),
                            "没收完不许作答——上一版就是在这里回了 400「Malformed HTTP request」")
            // 名额回收：`maxLiveConnections` 只有 16，漏一处 dismiss 就是「用满之后把所有 notify 关在门外」。
            // 改成异步分支后，原先「调用方在 processHTTPPayload 之后统一 dismiss」那条兜底没了，
            // 所以清理只许挂在唯一的写响应出口上
            try expectEqual(serverText.components(separatedBy: "connection.send(content:").count - 1, 1,
                            "写响应只许有一个出口，回收名额的兜底才挂得住")
            let sendChunk = String(serverText.components(separatedBy: "private func send(connection:")
                .dropFirst().first!.prefix(400))
            try expectTrue(sendChunk.contains("dismiss(connection)"),
                           "答完必须腾出 liveConnections 的名额")
        }
    }
}

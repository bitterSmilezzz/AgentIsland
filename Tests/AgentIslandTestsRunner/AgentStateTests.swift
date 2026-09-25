import Foundation
@testable import AgentIslandCore

// MARK: - App 侧状态的读端点（11 号票）
//
// 这一票治的是「CLI 另起进程，看不到 App 里的自报」——于是脚本读到 `inferred` 会以为
// 「这个 Agent 没自报过」，把**没看到**讲成**没有**。测的两件事：
// ① 读端点的形状与令牌边界；② 读不到时那句话说得准（连不上 / 被拒 / 端点不存在 / 解不出
// 是四件事，不许折成一句「读不到」）。

enum AgentStateTests {
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
    private static func body(_ outcome: LocalEventHTTP.Outcome) -> String {
        if case let .reply(_, text) = outcome { return text }
        return ""
    }
    private static func isState(_ outcome: LocalEventHTTP.Outcome) -> Bool {
        if case .state = outcome { return true }
        return false
    }

    private static let profile = AgentProfile(id: "qoder", name: "Qoder", icon: "x",
                                              bundleIDs: [], processNames: ["qoder"],
                                              sessionDirs: [], isCustom: false)

    private static func snapshot(level: ActivityLevel, provenance: AgentProvenance?,
                                 report: SelfReportRecord? = nil) -> AgentSnapshot {
        AgentSnapshot(profile: profile, level: level, processRunning: level != .offline,
                      cpuPercent: nil, installed: true, activeSessions: 1,
                      lastActivityAgo: 3, lastActivityText: "3秒前",
                      currentAction: "SENTINEL-命令正文-不许外泄",
                      provenance: provenance, selfReport: report)
    }
    private static func report(state: SelfReportState, expiresIn: TimeInterval,
                               at now: Date) -> SelfReportRecord {
        SelfReportRecord(agentID: "qoder", sessionID: "SENTINEL-会话id-不许外泄", pid: nil,
                         state: state, detail: "SENTINEL-正文-不许外泄", ask: nil,
                         receivedAt: now, ttl: expiresIn, expiresAt: now.addingTimeInterval(expiresIn),
                         expiredAt: nil)
    }

    @MainActor
    static func register() {
        // MARK: 路由

        TestKit.test("state: 只有 GET /state 进得来，别的方法回 405") {
            try expectTrue(isState(LocalEventHTTP.parse(request("GET /state HTTP/1.1"))))
            let post = LocalEventHTTP.parse(request("POST /state HTTP/1.1",
                                                    headers: ["Content-Length: 2"], body: "{}"))
            try expectEqual(status(post), 405)
            try expectTrue(body(post).contains("GET"), "405 要说清只收 GET：\(body(post))")
            // 状态码取自常量而不是服务端现写：与 /session 那格同一个出处
            try expectEqual(status(post), SelfReportWire.badMethodStatus)
        }

        TestKit.test("state: 没令牌读不到任何东西，回脸里不含注册表内容") {
            let denied = AgentStateEndpoint.deniedReply
            try expectEqual(status(denied), 403)
            let text = body(denied)
            try expectFalse(text.contains("qoder"), "拒答的那句不许带出装了谁：\(text)")
            try expectFalse(text.contains("Qoder"), "同上")
            try expectEqual(AgentStateEndpoint.noEngineReply.statusIfReply, SelfReportWire.noEngineStatus)
        }

        // MARK: 信封

        TestKit.test("state: 信封带来源、状态、冲突句与自报剩余秒数") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let rec = report(state: .attention, expiresIn: 74, at: now)
            let snap = snapshot(level: .attention, provenance: .selfReported, report: rec)
            let envelope = CLIAgentStateDTO(snapshots: [snap], now: now)
            try expectEqual(envelope.generatedBy, "app",
                            "脚本要靠这一格区分「App 的实时状态」与「CLI 自己采的样」")
            try expectEqual(envelope.agents.count, 1)
            let entry = envelope.agents[0]
            try expectEqual(entry.status, "attention")
            try expectEqual(entry.provenance, "selfReported")
            try expectEqual(entry.selfReportedState, "attention")
            try expectEqual(entry.selfReportExpiresInSeconds, 74.0)
            try expectNil(entry.conflictStatement)
        }

        TestKit.test("state: 读端点不泄露会话正文——命令、路径、会话 id 一个都不许出现") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let rec = report(state: .working, expiresIn: 60, at: now)
            let conflict = snapshot(level: .offline, provenance: .conflict, report: rec)
            let text = AgentStateEndpoint.body(snapshots: [conflict], now: now)
            for sentinel in ["SENTINEL-命令正文-不许外泄", "SENTINEL-正文-不许外泄",
                             "SENTINEL-会话id-不许外泄"] {
                try expectFalse(text.contains(sentinel), "读端点把敏感原文带出去了：\(sentinel)")
            }
            try expectTrue(text.contains("自报说工作中，进程表说离线"),
                           "冲突那句是要给的（它不含正文，只给状态词）：\(text)")
        }

        TestKit.test("state: 自报剩余时间不许是负数（过期就该走 nil 而不是「还剩 -3 秒」）") {
            let now = Date(timeIntervalSince1970: 1_700_000_000)
            let expired = SelfReportRecord(agentID: "qoder", sessionID: "s", pid: nil,
                                           state: .working, detail: nil, ask: nil,
                                           receivedAt: now.addingTimeInterval(-100), ttl: 30,
                                           expiresAt: now.addingTimeInterval(-70),
                                           expiredAt: now.addingTimeInterval(-70))
            let entry = CLIAgentStateEntryDTO(from: snapshot(level: .working, provenance: .conflict,
                                                             report: expired), now: now)
            try expectEqual(entry.selfReportExpiresInSeconds, 0.0,
                            "钳到 0：负数会印成「还剩 -70 秒」")
        }

        // MARK: 客户端怎么解读

        TestKit.test("state: classify 把 HTTP 状态码翻成结论，200 但正文坏了不能当成功") {
            try expectEqual(AgentStateClient.classify(status: nil, data: nil, transportError: true),
                            .unreachable)
            try expectEqual(AgentStateClient.classify(status: 403, data: Data(), transportError: false),
                            .denied)
            try expectEqual(AgentStateClient.classify(status: 405, data: Data(), transportError: false),
                            .unsupported)
            try expectEqual(AgentStateClient.classify(status: 200, data: Data("not json".utf8),
                                                      transportError: false), .malformed)
            let good = AgentStateEndpoint.body(snapshots: [snapshot(level: .working,
                                                                    provenance: .observed)],
                                               now: Date()).data(using: .utf8)!
            if case let .appState(env) = AgentStateClient.classify(status: 200, data: good,
                                                                   transportError: false) {
                try expectEqual(env.agents.first?.provenance, "observed")
            } else {
                throw TestError(message: "200 + 合法正文应解成 appState")
            }
        }

        TestKit.test("state: --json 失败格给枚举 reason，脚本不该去匹配中文") {
            for (outcome, want) in [(AgentStateClient.Outcome.unreachable, "unreachable"),
                                    (.denied, "noToken"), (.unsupported, "unsupported"),
                                    (.malformed, "malformed")] {
                try expectEqual(AgentStateClient.failureDTO(for: outcome).reason, want)
                try expectFalse(AgentStateClient.failureDTO(for: outcome).message.isEmpty, want)
            }
        }

        TestKit.test("state: 连不上、被拒、没这端点、引擎没就绪、解不出——是五件事") {
            let outcomes: [AgentStateClient.Outcome] = [
                .unreachable, .denied, .unsupported, .engineNotReady, .malformed,
            ]
            var lines: [String] = []
            for outcome in outcomes {
                let line = try XCTRequire(AgentStateClient.headline(for: outcome), "每种失败都得有话说")
                try expectTrue(line.count > 12, "一句话不能敷衍：\(line)")
                lines.append(line)
            }
            try expectEqual(Set(lines).count, 5, "五件事折成一句话就是把原因丢了：\(lines)")
            // 连不上这一条必须**点名它缺的是哪一格信息**。只断言「句子里出现自报两个字」是不够的——
            // 那句话后面本来就有「不会出现「自报」这一类来源」，改坏了照样绿（m85 第一版就这么幸存过）
            try expectTrue(lines[0].contains("自报记录"), lines[0])
            // 外部 review 的 P2：`state` 失败时什么都不打印，句子却写「下面这份是…」
            // ——承诺一份不会出现的输出，和把「没看到」讲成「没有」是同一类错
            for line in lines {
                try expectFalse(line.contains("下面这份"), "不许承诺不存在的输出：\(line)")
            }
        }

        TestKit.test("state: 503 是「引擎还没就绪」，不是「形状不兼容」") {
            try expectEqual(AgentStateClient.classify(status: 503, data: Data(), transportError: false),
                            .engineNotReady)
            try expectEqual(AgentStateClient.failureDTO(for: .engineNotReady).reason, "engineNotReady",
                            "等一等就有 与 要升级，脚本要能分开处理")
            let line = AgentStateClient.headline(for: .engineNotReady) ?? ""
            try expectTrue(line.contains("503") || line.contains("还没就绪"), line)
        }

        TestKit.test("state: 参数解析（在 Core，所以测得到）——缺值与错值都要出声") {
            let plain = AgentStateClient.parseArgs([])
            try expectEqual(plain.port, SelfReportWire.defaultPort)
            try expectNil(plain.error)
            try expectFalse(plain.json)
            // `--port` 是最后一个参数：以前会静默用默认端口去查一个错的端点
            let dangling = AgentStateClient.parseArgs(["--port"])
            try expectNotNil(dangling.error, "--port 后面没值必须报错")
            try expectTrue(dangling.error?.contains("--port") == true, dangling.error ?? "nil")
            let bad = AgentStateClient.parseArgs(["-p", "abc"])
            try expectNotNil(bad.error, "非数字端口必须报错")
            try expectEqual(bad.port, SelfReportWire.defaultPort, "报错时不许半改状态")
            let zero = AgentStateClient.parseArgs(["--port", "0"])
            try expectNotNil(zero.error, "0 不是合法端口")
            let ok = AgentStateClient.parseArgs(["--json", "--port", "8080"])
            try expectTrue(ok.json); try expectEqual(ok.port, 8080); try expectNil(ok.error)
        }

        // MARK: 措辞与端口的单一出口

        TestKit.test("措辞: 状态词复用 ActivityLevel，来源词四种都有名字（岛内刻意只挂两种）") {
            try expectEqual(AgentStateClient.statusLabel("working"), ActivityLevel.working.label)
            try expectEqual(AgentStateClient.statusLabel("nonsense"), "nonsense",
                            "认不出的值原样给，不编一个中文词")
            for p in AgentProvenance.allCases {
                try expectTrue(!p.explainText.isEmpty, p.rawValue)
            }
            try expectEqual(AgentStateClient.provenanceLabel("inferred"), "推断（CPU/写入兜底）")
            // 岛内短标签与 CLI 完整说法是两回事：观测/推断在岛里没有标签
            try expectNil(AgentProvenance.inferred.badgeText)
            try expectTrue(AgentProvenance.inferred.explainText.contains("推断"))
        }

        TestKit.test("结构: 端口与 /state 的形状只有一处定义") {
            let core = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIslandCore/SelfReport.swift"))
            try expectEqual(core.components(separatedBy: "public static let defaultPort").count - 1, 1)
            // 除 Core 那一行之外，Sources/ 里不许再出现 41999 这个字面量（帮助文本与注释除外）
            for rel in ["Sources/AgentIsland/LocalEventServer.swift",
                        "Sources/AgentIslandCLI/Commands/NotifyCommand.swift",
                        "Sources/AgentIslandCLI/Commands/StateCommand.swift"] {
                let text = SourceTree.codeOnly(try SourceTree.text(relativePath: rel))
                let code = text.components(separatedBy: "\n").filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
                try expectFalse(code.joined().contains("41999"),
                                "\(rel) 又自己写了一遍端口——端口只有 SelfReportWire.defaultPort 一处")
            }
            // /state 的三种回脸都在 Core：服务端只发不判
            let server = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIsland/LocalEventServer.swift"))
            try expectTrue(server.contains("AgentStateEndpoint.deniedReply"), "403 那张脸要出自 Core")
            // /state 的令牌校验写在服务端（executable target，链接不到测试），
            // 所以这条只能按**顺序**守：先 authorize，再给正文。反过来说，
            // 「把 authorize 挪到 send 之后」这种改法今天无人能拦——除非有这条断言
            let handleState = server.components(separatedBy: "private func handleState(").dropFirst().first ?? ""
            try expectTrue(handleState.contains("authorizeSelfReport"), "读端点必须过令牌")
            try expectTrue(handleState.range(of: "authorizeSelfReport")!.lowerBound
                            < handleState.range(of: "AgentStateEndpoint.body")!.lowerBound,
                           "先验令牌，再给状态正文")
            try expectTrue(handleState.range(of: "deniedReply")!.lowerBound
                            < handleState.range(of: "AgentStateEndpoint.body")!.lowerBound,
                           "拒答要在给正文之前发生")
        }
    }
}

private extension LocalEventHTTP.Outcome {
    var statusIfReply: Int? {
        if case let .reply(code, _) = self { return code }
        return nil
    }
}

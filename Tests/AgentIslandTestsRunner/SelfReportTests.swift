import Foundation
@testable import AgentIslandCore

// MARK: - 可信自报通道（02 号票：/session 协议与令牌与 TTL）
//
// 这批测试守的不是「代码能跑」，而是**信任边界的方向**：一条来历不明的申报一旦被采信，
// 就比推断更糟——它会盖掉进程表本来能证明的事实。所以每条断言都在问同一个问题：
// 「证明不了的时候，它是拒绝还是放行？」
//
// v0.0.130 的 review 直接点出一类失败：`matches()` 用 XOR 累加当「定长比较」，
// 而当时那条用例的五个反例全是「长度不同或只差一个字符」——同长度多字节差异的串
// 一个都没测。所以下面第一条就是那个反例，它同时也是这次修改的复现脚本。

enum SelfReportTests {
    @MainActor
    static func register() {
        let claude = AgentRegistry.builtin.first { $0.id == "claude" }!
        let trae = AgentRegistry.builtin.first { $0.id == "trae" }!
        let openCode = AgentRegistry.builtin.first { $0.id == "opencode" }!
        let cred = SelfReportCredential()
        // 纯目录型档案：没有进程名约束（第三方靠会话目录被发现），自报不该要求它给出可执行文件
        let dirOnly = AgentProfile(id: "dironly", name: "目录型", icon: "x", bundleIDs: [],
                                  processNames: [], sessionDirs: [])

        // MARK: 令牌：唯一那道门

        TestKit.test("令牌: 定长比较要挡住「同长度、每个字节都不同」的串") {
            // 这条用例的形状就是这次 review 的 P0：`diff ^= a[i] ^ b[i]` 是 XOR **校验和**，
            // 只要求「差异两两相消」。把真令牌每个字节 `^1`（48 个字节全不同）在旧实现下
            // 得到 `diff == 0` ⇒ 判定为正确令牌。旧的那五条反例全是单字符/变长差异，
            // 所以它永远不会红——「断言不会失败」在这里的具体形状。
            let store = tempTokenStore("constanttime")
            guard let token = store.ensure() else { throw TestError(message: "令牌没生成") }
            let bytes = Array(token.utf8)
            let flipped = String(bytes: bytes.map { $0 ^ 1 }, encoding: .ascii)
            try expectNotNil(flipped)
            try expectEqual(flipped?.count, token.count, "反例必须是同长度的，否则它测的是长度检查")
            try expectFalse(store.matches(flipped),
                            "每个字节都翻转却判定为正确 ⇒ 这不是比较，是校验和")
            // 随机同长猜测：旧实现实测 20000 次有上百次假接受（≈3%）。抽 300 次，
            // 任何一次被接受都说明不可猜测性归零
            for _ in 0..<300 {
                let guess = (0..<token.count).map { _ in
                    String(format: "%x", Int.random(in: 0...15))
                }.joined()
                try expectFalse(store.matches(guess), "随机同长串被接受：\(guess.prefix(4))…")
            }
            // 单字节差异与变长仍然要拒（旧实现这部分是对的，别一起改坏）
            try expectFalse(store.matches(String(token.dropLast())))
            try expectFalse(store.matches(token + "x"))
            try expectFalse(store.matches("x" + token.dropFirst()))
            try expectFalse(store.matches(""))
            try expectFalse(store.matches(nil))
            try expectTrue(store.matches(token))
            try expectTrue(store.matches("  " + token + "  "), "header 值两侧的空不算两个令牌")
            let masked = store.masked()
            try expectFalse(masked.contains(token), "任何输出里都不许出现完整令牌：\(masked)")
            try expectTrue(masked.contains("····"))
            cleanup(dir: store.tokenURL.deletingLastPathComponent())
        }

        TestKit.test("令牌: 凭证只由校验通过产出，且服务端构造不出来") {
            let store = tempTokenStore("authorize")
            guard let token = store.ensure() else { throw TestError(message: "令牌没生成") }
            try expectNotNil(store.authorize(token))
            try expectEqual(store.authorize(token), store.authorize(token),
                            "凭证是可比的能力值，不是一次性对象")
            try expectNil(store.authorize(String(repeating: "a", count: token.count)))
            try expectNil(store.authorize(nil), "没带 header 不是一种「差一点就通过」")
            // init 是 internal ⇒ `AgentIsland`（executable target）里没有写法能凭空造出它。
            // 这条断言看着像废话，它是下面那枚「服务端不许硬编码可信度」棘轮的类型学依据
            try expectEqual(SelfReportCredential(), cred)
            cleanup(dir: store.tokenURL.deletingLastPathComponent())
        }

        TestKit.test("令牌: 落盘即 0600，且幂等") {
            let store = tempTokenStore("mode")
            guard let first = store.ensure() else { throw TestError(message: "生成失败") }
            let attrs = try XCTRequire(try? FileManager.default.attributesOfItem(atPath: store.tokenURL.path),
                                       "令牌文件没落盘")
            let mode = (attrs[.posixPermissions] as? NSNumber)?.intValue ?? -1
            try expectEqual(mode, SelfReportTokenStore.ownerOnlyMode,
                            "它是「谁在说话」的全部依据，group/other 可读就等于把令牌发给整台机器")
            try expectEqual(store.ensure(), first, "第二个请求不该换掉令牌")
            try expectTrue(first.count >= 32, "短到能穷举的令牌等于没有：\(first.count)")
            cleanup(dir: store.tokenURL.deletingLastPathComponent())
        }

        TestKit.test("令牌: 形态不合格的原因要分得出来，不能混成「没有令牌」") {
            // 「真令牌只是权限被改宽了」与「这台机器从来没有配过」在用户侧是两件事：
            // 前者只要重配一次接入命令，后者要从头生成。混成一个 nil 的时候，
            // 现象是「昨天还好使今天全废」，而唯一的提示是响应里那句 noToken
            func hostileDir(_ contents: String, mode: Int) throws -> URL {
                let dir = FileManager.default.temporaryDirectory
                    .appendingPathComponent("agentisland-hostile-\(UUID().uuidString)", isDirectory: true)
                try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
                let url = dir.appendingPathComponent("report.token")
                try Data(contents.utf8).write(to: url)
                try FileManager.default.setAttributes([.posixPermissions: NSNumber(value: mode)],
                                                      ofItemAtPath: url.path)
                return dir
            }
            let stolen = String(repeating: "z", count: 48)
            try expectEqual(SelfReportTokenStore(directory: try hostileDir(stolen, mode: 0o600))
                                .inspect().defect, .badShape,
                            "非十六进制的串不是我们生成的形态")
            try expectEqual(SelfReportTokenStore(directory: try hostileDir(String(repeating: "b", count: 48),
                                                                           mode: 0o644))
                                .inspect().defect, .looseMode,
                            "同组读得动的令牌文件等于没有保密")
            try expectEqual(SelfReportTokenStore(directory: try hostileDir(String(repeating: "0", count: 20),
                                                                           mode: 0o600))
                                .inspect().defect, .badShape, "短到不像令牌的串按不合格处理")
            let missing = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-none-\(UUID().uuidString)", isDirectory: true)
            try expectEqual(SelfReportTokenStore(directory: missing).inspect().defect, .missing)
            // 换了属主的文件不该再被当成本机凭据：这条判据没法用真 uid 造反例，
            // 所以它被做成注入值——否则它就是「写了但永远不会红」的检查
            let ownerDir = try hostileDir(String(repeating: "a", count: 48), mode: 0o600)
            try expectEqual(SelfReportTokenStore(directory: ownerDir, ownerUID: { 88888 })
                                .inspect().defect, .foreignOwner)
            try expectNil(SelfReportTokenStore(directory: ownerDir, ownerUID: { 88888 }).read())
            try expectFalse(SelfReportTokenStore(directory: ownerDir, ownerUID: { 88888 })
                                .matches(String(repeating: "a", count: 48)))
            cleanup(dir: ownerDir)
        }

        TestKit.test("令牌: ensure 不许把新生成的令牌写穿符号链接") {
            // 本机进程可以先把 report.token 做成一条指向「它读得到的地方」的链接。
            // 实测（/tmp/symlinkprobe.swift）：createFile 会解掉链接本身再建普通文件，
            // 目标内容一字不动——所以这里钉的是**平台行为**：哪天它改成穿过链接写，
            // 这条测试就红，而不是靠代码里多一段永远观察不到的 removeItem
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-symlink-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let target = dir.appendingPathComponent("elsewhere")
            // 摆成「目标文件 0600 + 48 位十六进制」：用例失败时报出来的是「链接被读了/被写了」，
            // 而不是「目标不合格」。诚实记录一条测不出来的东西：符号链接自身 mode 实测 0755，
            // 所以 read() 那条路径上类型判据与权限判据重叠，单删类型检查不会让套件变红
            try Data(String(repeating: "0", count: 48).utf8).write(to: target)
            try FileManager.default.setAttributes([.posixPermissions: NSNumber(value: 0o600)],
                                                  ofItemAtPath: target.path)
            try FileManager.default.createSymbolicLink(at: dir.appendingPathComponent("report.token"),
                                                       withDestinationURL: target)
            let store = SelfReportTokenStore(directory: dir)
            try expectEqual(store.inspect().defect, .notRegular)
            guard let fresh = store.ensure() else { throw TestError(message: "ensure 没能替换掉链接") }
            try expectEqual(try String(contentsOf: target, encoding: .utf8), String(repeating: "0", count: 48),
                            "新令牌写进了链接的目标——攻击者只要读那个文件就拿到凭据")
            let attrs = try XCTRequire(try? FileManager.default.attributesOfItem(atPath: store.tokenURL.path),
                                       "ensure 之后 report.token 不在了")
            try expectEqual(attrs[.type] as? FileAttributeType, .typeRegular, "链接本身要被换掉，不是被穿过")
            try expectTrue(store.matches(fresh), "换掉之后这条通道要真的可用")
            cleanup(dir: dir)
        }

        TestKit.test("令牌: 悬空链接也不能把整条通道永久钉死") {
            // 指向不存在目标的链接是另一种占位：如果 ensure 在这里返回 nil，
            // 之后每一次请求都是 noToken，而错误信息会说「令牌不对」——没人查得到链接
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-dangling-\(UUID().uuidString)", isDirectory: true)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try FileManager.default.createSymbolicLink(at: dir.appendingPathComponent("report.token"),
                                                       withDestinationURL: dir.appendingPathComponent("gone"))
            let store = SelfReportTokenStore(directory: dir)
            guard let token = store.ensure() else {
                throw TestError(message: "悬空链接让 ensure 失败了：整条通道就此永远 noToken")
            }
            try expectTrue(store.matches(token), "换掉链接之后校验要立刻可用")
            cleanup(dir: dir)
        }

        TestKit.test("令牌: 目录权限带执行位，文件权限不带") {
            // 0o600 对文件是对的，对目录却是「谁都进不去」：ensure 会成功创建目录、
            // 然后 createFile 静默失败。两个模式必须分开
            try expectEqual(SelfReportTokenStore.ownerOnlyMode & 0o111, 0,
                            "令牌文件不该有执行位，也不该让同组任何人读得动")
            try expectEqual(SelfReportTokenStore.ownerOnlyDirMode, 0o700)
        }

        TestKit.test("令牌: 目录不存在时 ensure 也要能生成") {
            // 全新机器上 ~/Library/Application Support/AgentIsland 并不存在。少了这一步，
            // createFile 会默默失败、ensure 返回 nil，于是**每一次**请求都是 noToken，
            // 而接入方看到的错误信息会说「令牌不对」——没人会往「目录没建」上想
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-missing-\(UUID().uuidString)/深层/再深层", isDirectory: true)
            let store = SelfReportTokenStore(directory: dir)
            try expectFalse(FileManager.default.fileExists(atPath: dir.path))
            guard let token = store.ensure() else { throw TestError(message: "目录不存在时没能生成令牌") }
            try expectTrue(token.count >= 32)
            try expectTrue(store.matches(token), "生成完立刻校验，不该再读不到")
            cleanup(dir: dir.deletingLastPathComponent().deletingLastPathComponent())
        }

        TestKit.test("令牌: 没有令牌文件时校验一律为假，且不凭空生成") {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("agentisland-none2-\(UUID().uuidString)", isDirectory: true)
            let store = SelfReportTokenStore(directory: dir)   // 目录都不存在
            try expectFalse(store.matches("anything"))
            try expectNil(store.authorize("anything"))
            try expectFalse(FileManager.default.fileExists(atPath: store.tokenURL.path),
                            "校验路径不该有副作用——那会让一个乱发请求的进程替本机生成凭据")
        }

        // MARK: 载荷：两套形状都收

        TestKit.test("自报: 自有形状与 hook 形状解析成同一条申报") {
            let ours = json(#"{"agent":"claude","session":"s1","pid":4242,"state":"attention","ttl":30}"#)
            let hook = json(#"{"agent_id":"claude","session_id":"s1","hook_event_name":"Notification","pid":4242,"ttl":30}"#)
            let ids: Set<String> = ["claude"]
            guard case .accepted(let a) = SelfReportPayload.parse(ours, knownAgentIDs: ids),
                  case .accepted(let b) = SelfReportPayload.parse(hook, knownAgentIDs: ids) else {
                throw TestError(message: "两套形状都该被收下：\(SelfReportPayload.parse(ours, knownAgentIDs: ids))")
            }
            try expectEqual(a, b, "同一件事的两种写法必须归一化到同一条申报")
            try expectEqual(a.state, .attention)
            try expectEqual(a.pid, 4242)
        }

        TestKit.test("自报: 未知档案 id 被拒，且不自动建档") {
            let ids: Set<String> = ["claude"]
            let out = SelfReportPayload.parse(json(#"{"agent":"notreal","session":"s"}"#), knownAgentIDs: ids)
            guard case .rejected(let reason, _) = out else { throw TestError(message: "未知档案必须被拒") }
            try expectEqual(reason, .unknownAgent)
        }

        TestKit.test("自报: 数字型 agent 不算名字") {
            // `{"agent": 1}` 是调用方的 bug。收成 id "1" 会让「注册表里没有 1」这条
            // 拒收变成一次看起来正常的未知档案报错，也会让数字键的猜测进协议
            let out = SelfReportPayload.parse(json(#"{"agent":1,"session":"s"}"#), knownAgentIDs: ["1", "claude"])
            guard case .rejected(let reason, let detail) = out else { throw TestError(message: "数字 agent 不该被接受") }
            try expectEqual(reason, .malformed)
            try expectTrue(detail.contains("agent"), "消息要说清缺的是哪个字段：\(detail)")
        }

        TestKit.test("自报: 同一个值在正文与 URL 里必须同口径（都 trim）") {
            // 原先正文不 trim、URL trim：`{"agent":" claude "}` 报 unknownAgent，
            // 而 `?agent=%20claude%20` 能过。手抄配置的人撞上的是「我明明写了 claude」
            let spaced = payload(["agent": " claude ", "session": " s ", "state": "idle"])
            guard case .accepted(let s) = SelfReportPayload.parse(spaced, knownAgentIDs: ["claude"]) else {
                throw TestError(message: "正文里带空格的 agent 该和 URL 里带空格的一样被接受")
            }
            try expectEqual(s.agentID, "claude")
            try expectEqual(s.sessionID, "s",
                            "session 是登记表的键，不 trim 就会为同一个会话建两条记录")
        }

        TestKit.test("自报: pid 给了就必须读得出来，读不出来是拒绝而不是当没给") {
            // 「验不了就当没提」会把整条冒名顶替防护静默跳过，而申报方以为它生效了。
            // 这四种形态在旧实现里全部解析成 pid=nil ⇒ binder 直接 .bound
            for raw: Any in ["abc", ["42"], ["a": 1], true, 4294967298, -7, 0, "4242 "] {
                let out = SelfReportPayload.parse(
                    payload(["agent": "claude", "session": "s", "state": "attention", "pid": raw]),
                    knownAgentIDs: ["claude"])
                if raw is String, (raw as? String) == "4242 " {
                    // 带空格的四位数字串是唯一一条「能安全修好」的形态：header 与 shell
                    // 拼接常留尾空格，这种要接受而不是拒
                    guard case .accepted(let s) = out, s.pid == 4242 else {
                        throw TestError(message: "\"4242 \" 该被接受成 pid 4242，实得 \(out)")
                    }
                    continue
                }
                guard case .rejected(let reason, let detail) = out else {
                    throw TestError(message: "畸形 pid \(String(describing: raw)) 被静默当成没给：\(out)")
                }
                try expectEqual(reason, .malformed)
                try expectTrue(detail.contains("pid"), "要说清是 pid 这条字段写坏了：\(detail)")
            }
            // 越界整数不许截断成另一个 pid（4294967298 → int32 截断 = 2）
            try expectNil(SelfReportPayload.parsedPID(4294967298 as NSNumber))
            try expectEqual(SelfReportPayload.parsedPID(4242 as NSNumber), 4242)
            try expectEqual(SelfReportPayload.parsedPID("4242" as NSString), 4242)
            // 没带 pid 仍然是合法的「我不声称进程」
            guard case .accepted(let none) = SelfReportPayload.parse(
                json(#"{"agent":"claude","session":"s","state":"idle"}"#), knownAgentIDs: ["claude"]),
                  none.pid == nil else { throw TestError(message: "不带 pid 是合法形状") }
        }

        TestKit.test("自报: TTL 钳进 [15,600]，detail/ask 按上限截断") {
            func ttlOf(_ raw: Any) throws -> TimeInterval {
                let out = SelfReportPayload.parse(payload(["agent": "claude", "session": "s",
                                                           "state": "working", "ttl": raw]),
                                                  knownAgentIDs: ["claude"])
                guard case .accepted(let s) = out else { throw TestError(message: "解析失败: \(raw)") }
                return s.ttl
            }
            try expectEqual(try ttlOf(5), 15, "小于下限要抬到下限，而不是拒绝整条申报")
            try expectEqual(try ttlOf(9999), 600)
            try expectEqual(try ttlOf(45), 45)
            try expectEqual(try ttlOf("60"), 60, "hook 侧常把数字塞成字符串")
            try expectEqual(try ttlOf(-1), 90, "非正数按默认，不做「永久」——永久 = 没有退回推断那一天")

            let long = String(repeating: "字", count: 300)
            let out = SelfReportPayload.parse(payload(["agent": "claude", "session": "s", "state": "working", "detail": long]),
                                              knownAgentIDs: ["claude"])
            guard case .accepted(let s) = out else { throw TestError(message: "解析失败") }
            try expectEqual(s.detail?.count, SelfReportSubmission.detailLimit, "超长要截断而不是灌进来")
        }

        TestKit.test("自报: detail/ask 是要显示的句子，不许被标识符那套归一化改写") {
            // `name()` 的语义是「标识符」：trim + 纯空白按没给。把它套到正文上，
            // 用户可见的句子就被静默动过——`"  在等确认  "` 少了两个空格没人看得出来。
            // 这两栏**是有 UI 消费的**：未鉴权的申报会经 `SelfReportFallback.post` 变成
            // `AgentTaskEvent.message`/`.detail`，也就是岛横幅那行字与系统通知的正文
            //（第 3 轮 review 报出我上一版把这里写成「没有 UI 消费」，而正因这个错前提，
            //  「全空白顶掉另一栏」没人测——见下面「落回: 全空白不算一句话」那条）
            func accepted(_ fields: [String: Any]) throws -> SelfReportSubmission {
                let out = SelfReportPayload.parse(payload(fields), knownAgentIDs: ["claude"])
                guard case .accepted(let s) = out else { throw TestError(message: "该收下：\(fields)") }
                return s
            }
            let kept = try accepted(["agent": "claude", "session": "s", "state": "working",
                                     "detail": "  在等确认  ", "ask": " 要不要跑测试？ "])
            try expectEqual(kept.detail, "  在等确认  ", "首尾空白是内容的一部分（缩进、换行式对齐）")
            try expectEqual(kept.ask, " 要不要跑测试？ ")
            let blank = try accepted(["agent": "claude", "session": "s", "state": "working",
                                      "detail": "   ", "ask": "\u{00a0}"])
            try expectEqual(blank.detail, "   ", "纯空白该原样留着，不该被当成「没给」")
            try expectEqual(blank.ask, "\u{00a0}")
            // 非字符串仍然按没给：那是形状问题，不是内容问题
            let wrong = try accepted(["agent": "claude", "session": "s", "state": "working",
                                      "detail": 42, "ask": ["a"]])
            try expectNil(wrong.detail)
            try expectNil(wrong.ask)
            // 截断照旧生效（与 trim 是两件事）
            let long = try accepted(["agent": "claude", "session": "s", "state": "working",
                                     "detail": String(repeating: "字", count: 300)])
            try expectEqual(long.detail?.count, SelfReportSubmission.detailLimit)
        }

        TestKit.test("自报: URL 的 ?agent= 让 hook 形状接得进来，但不放松任何要求") {
            let hookBody = json(#"{"session_id":"h77","hook_event_name":"PostToolUse","cwd":"/tmp"}"#)
            guard case .accepted(let s) = SelfReportPayload.parse(hookBody, knownAgentIDs: ["qoder"],
                                                                  queryAgent: "qoder") else {
                throw TestError(message: "hook 原样 POST 加上 ?agent= 就该接住")
            }
            try expectEqual(s.agentID, "qoder")
            try expectEqual(s.state, .working)
            try expectEqual(s.sessionID, "h77")
            guard case .rejected(let r, _) = SelfReportPayload.parse(hookBody, knownAgentIDs: ["qoder"],
                                                                     queryAgent: "ghost") else {
                throw TestError(message: "URL 不是自动建档的旁路")
            }
            try expectEqual(r, .unknownAgent)
            guard case .rejected(let r2, let d) = SelfReportPayload.parse(hookBody, knownAgentIDs: ["qoder"],
                                                                          queryAgent: "   ") else {
                throw TestError(message: "空 agent 该按缺字段拒")
            }
            try expectEqual(r2, .malformed)
            try expectTrue(d.contains("agent"), d)
            guard case .accepted(let b) = SelfReportPayload.parse(
                json(#"{"agent":"claude","session":"s","state":"idle"}"#), knownAgentIDs: ["claude", "qoder"],
                queryAgent: "qoder") else { throw TestError(message: "正文的 agent 该优先") }
            try expectEqual(b.agentID, "claude")
        }

        TestKit.test("自报: 状态与文本不许从 URL 来") {
            let out = SelfReportPayload.parse(json(#"{"session_id":"s"}"#), knownAgentIDs: ["qoder"],
                                              queryAgent: "qoder")
            guard case .rejected(let r, _) = out else {
                throw TestError(message: "正文没有 state，URL 参数不该补上它")
            }
            try expectEqual(r, .unknownState,
                            "agent 可以由 URL 补，state 不行——它决定岛显示什么，只许走正文")
        }

        TestKit.test("自报: 认不出的 state 与事件名一律拒收") {
            let ids: Set<String> = ["claude"]
            for raw in [#"{"agent":"claude","session":"s","state":"busy"}"#,
                        #"{"agent":"claude","session":"s","hook_event_name":"PreCompact"}"#,
                        #"{"agent":"claude","session":"s"}"#] {
                let out = SelfReportPayload.parse(json(raw), knownAgentIDs: ids)
                guard case .rejected(let reason, _) = out else {
                    throw TestError(message: "猜一个状态比拒收更糟：\(raw)")
                }
                try expectEqual(reason, .unknownState)
            }
        }

        TestKit.test("自报: 撤销只认两个坐标，不许被迫重发一遍 state") {
            // 第一次真机接就是这里翻车的：DELETE 复用整份申报的解析，
            // 于是「撤销 live-1」回了一句 unknownState
            let body = json(#"{"agent":"qoder","session":"live-1"}"#)
            switch SelfReportPayload.identity(body, knownAgentIDs: ["qoder"], queryAgent: nil) {
            case .success(let id):
                try expectEqual(id, SelfReportIdentity(agentID: "qoder", sessionID: "live-1"))
            case .failure(let f):
                throw TestError(message: "撤销的载荷被拒了：\(f.reason) / \(f.detail)")
            }
            guard case .failure(let f) = SelfReportPayload.identity(body, knownAgentIDs: ["other"],
                                                                    queryAgent: nil),
                  f.reason == .unknownAgent else { throw TestError(message: "未知档案的撤销该被拒") }
        }

        TestKit.test("自报: 事件映射是 allowlist，未核实的档案一律不收") {
            // 01 号票：Trae 一个事件同时覆盖「等确认」与「任务完成」且载荷没带区分字段；
            // Cline / Cursor 的 `Notification` 语义本轮同样没逐字核实。
            // 写成「全局表 + 一条 deny」等于每接入一家都默认放行 Claude 的语义——
            // 凭空造出假警报的那个方向。翻成 allowlist 之后，只有核过的三家能套表。
            try expectEqual(SelfReportEventMapping.state(hookEventName: "Notification", profileID: "claude"),
                            .attention)
            try expectEqual(SelfReportEventMapping.state(hookEventName: "Stop", profileID: "codex"), .completed)
            try expectEqual(SelfReportEventMapping.state(hookEventName: "SessionStart", profileID: "qoder"), .working)
            for unverified in [trae.id, "cursor", "cline", "windsurf", "opencode", "dironly", "custom-who-knows"] {
                try expectNil(SelfReportEventMapping.state(hookEventName: "Notification", profileID: unverified),
                              "\(unverified) 的事件语义没逐字核实过，套 Claude 表就是猜")
            }
            try expectEqual(SelfReportEventMapping.state(hookEventName: "session.idle", profileID: openCode.id),
                            .idle, "小写点分名自己就把状态说了，不按档案分家")
            try expectNil(SelfReportEventMapping.state(hookEventName: "SubagentStop", profileID: "claude"),
                          "SubagentStop 的 session_id 归属没实样，映射成 completed 会把还在等子代理的父会话标成结束")
        }

        // MARK: 绑定：令牌授予可信，pid 只能否决

        TestKit.test("自报: pid 绑定要求词边界（claudex 顶不了 claude，helper 变体要放过）") {
            let bad = SelfReportBinder.bind(sub(pid: 10), credential: cred, profile: claude,
                                            probe: probe(alive: [10: "claudex"]))
            guard case .pidMismatch = bad else {
                throw TestError(message: "朴素前缀会把 claudex 认成 claude，正是冒名顶替的形状：\(bad)")
            }
            try expectEqual(SelfReportBinder.bind(sub(session: "s2", pid: 11), credential: cred,
                                                  profile: claude, probe: probe(alive: [11: "claude helper"])),
                            .bound, "Electron Helper 那一族是合法变体，词边界规则本来就为它存在")
        }

        TestKit.test("自报: 给了 pid 就必须活着") {
            let out = SelfReportBinder.bind(sub(pid: 77), credential: cred, profile: claude,
                                            probe: probe(alive: [:]))
            guard case .pidMismatch(let why) = out else { throw TestError(message: "死 pid 必须拒：\(out)") }
            try expectTrue(why.contains("77"), "原因里要带那个 pid，否则排查只能猜：\(why)")
        }

        TestKit.test("自报: 读不到可执行路径时不采信（与终止复核的保守方向相反）") {
            // `ProcessTerminator.isAlive(expectedPath:)` 在读不到路径时**当作还活着**，
            // 因为那边的失败模式是「谎报清理成功」。这边的失败模式相反：
            // 证明不了身份就不能给可信度。共用一个 libproc 出口，但保守侧不同。
            let out = SelfReportBinder.bind(sub(pid: 55), credential: cred, profile: claude,
                                            probe: SelfReportProcessProbe(isLive: { _ in true },
                                                                         executableName: { _ in nil }))
            guard case .pidMismatch = out else { throw TestError(message: "读不到路径必须拒：\(out)") }
        }

        TestKit.test("自报: 档案已经不在了 ⇒ 报 profileGone，既不盲授也不甩锅给 pid") {
            // 这条 guard 在生产里由 parse 那侧的注册表快照挡着，所以它「写了没人验过会拒绝」
            // 是上一轮的实测结论（改成盲授 .bound 全绿存活）。而 reason 混进 pidMismatch
            // 会让接入方去查自己写对的进程号——`reason` 是给脚本 switch 的枚举
            let out = SelfReportBinder.bind(sub(pid: 3), credential: cred, profile: nil,
                                            probe: probe(alive: [3: "claude"]))
            guard case .profileGone(let why) = out else {
                throw TestError(message: "档案消失不该被当成可信，也不该报成 pid 问题：\(out)")
            }
            try expectTrue(why.contains("claude"), why)
        }

        TestKit.test("自报: 无进程名约束的档案不做路径比对，也不问进程表") {
            var nameAsked = 0
            let p = SelfReportProcessProbe(isLive: { _ in true },
                                           executableName: { _ in nameAsked += 1; return nil })
            try expectEqual(SelfReportBinder.bind(sub(pid: 3), credential: cred, profile: dirOnly, probe: p),
                            .bound, "这类档案本来就匹配不出名字，拒了等于禁掉它的自报")
            try expectEqual(nameAsked, 0, "既然不比路径，就不该去读可执行文件名")
            guard case .pidMismatch = SelfReportBinder.bind(sub(pid: 3), credential: cred,
                                                            profile: dirOnly, probe: probe(alive: [:])) else {
                throw TestError(message: "档案没有进程名约束 ≠ 申报可以不带进程")
            }
        }

        // MARK: Store 与 TTL

        TestKit.test("自报: 响应带的是这条申报自己的到期时刻") {
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "长", ttl: 600), now: t0)
            store.ingest(sub(session: "短", ttl: 15), now: t0)
            try expectEqual(store.record(agentID: "claude", sessionID: "短", now: t0)?.expiresAt,
                            t0.addingTimeInterval(15))
            try expectEqual(store.record(agentID: "claude", sessionID: "长", now: t0)?.expiresAt,
                            t0.addingTimeInterval(600))
            try expectEqual(store.believable(agentID: "claude", now: t0)?.sessionID, "长",
                            "跨会话那条查询的口径没变，它就该挑最晚过期的")
            try expectNil(store.record(agentID: "claude", sessionID: "短",
                                       now: t0.addingTimeInterval(20)))
        }

        TestKit.test("自报: 同一会话续报是覆盖而不是追加") {
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "s1", state: .working), now: t0)
            store.ingest(sub(session: "s1", state: .attention), now: t0.addingTimeInterval(5))
            try expectEqual(store.records.count, 1, "两条不同时刻的同会话申报只能留一条")
            try expectEqual(store.believable(agentID: "claude", now: t0.addingTimeInterval(6))?.state, .attention)
        }

        TestKit.test("自报: TTL 到期只盖章——记录不删、证据不重复记") {
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "s1", ttl: 30), now: t0)
            try expectTrue(store.sweepExpired(now: t0.addingTimeInterval(20)).isEmpty, "没到期不该记账")
            let fresh = store.sweepExpired(now: t0.addingTimeInterval(31))
            try expectEqual(fresh.count, 1)
            try expectNil(store.believable(agentID: "claude", now: t0.addingTimeInterval(31)),
                          "过期之后不再采信")
            try expectEqual(store.records.count, 1,
                            "「不清空、不消失」是这条约定的全部意义：用户要看得见它刚才说过什么")
            try expectNotNil(store.expiredEvidence(agentID: "claude")?.expiredAt)
            try expectTrue(store.sweepExpired(now: t0.addingTimeInterval(60)).isEmpty,
                           "重复扫描不能把同一次过期记两遍")
            store.ingest(sub(session: "s1", ttl: 30), now: t0.addingTimeInterval(61))
            try expectEqual(store.believable(agentID: "claude", now: t0.addingTimeInterval(62))?.state, .working)
            try expectNil(store.expiredEvidence(agentID: "claude"), "续报之后就没有「过期」这回事了")
        }

        TestKit.test("自报: 时钟回拨不许把已过期的申报复活") {
            // 第 3 条口径的两个方向都要守住：过期不能清空（那是抹证据），
            // 也不能因为回拨又变回可信（那是凭空造出一条还活着的申报）。
            // 实测旧实现：ingest(ttl 15) → 盖章过期 → 回拨 25s ⇒ believable 又返回 attention，
            // 而这段时间没有任何新上报。NTP 阶跃与虚拟机恢复快照正是引擎自己列出的场景
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "gone", state: .attention, ttl: 15), now: t0)
            store.sweepExpired(now: t0.addingTimeInterval(20))
            try expectNil(store.believable(agentID: "claude", now: t0.addingTimeInterval(20)))
            store.reanchorIfClockRewound(now: t0.addingTimeInterval(-5))
            try expectNil(store.believable(agentID: "claude", now: t0.addingTimeInterval(-5)),
                          "重锚顺手抹掉 expiredAt 就等于让断心跳的申报重新压住推断")
            try expectNotNil(store.expiredEvidence(agentID: "claude")?.expiredAt,
                             "戳要留着：它是「它说过、后来断了」的证据")
            // 光留戳还不够：重锚的谓词一旦不看 expiredAt，那条已盖章记录的 receivedAt/expiresAt
            // 会被未来的时钟改写。它虽然仍不可信，却不再按自己真正的年龄老化——
            // 淘汰排序（`mostBelievableFirst` 比 receivedAt）与保质期都成了回拨幅度的函数
            let aged = store.records.values.first
            try expectEqual(aged?.receivedAt, t0, "已盖章的记录不参与重锚：它的年龄是证据的一部分")
            try expectEqual(aged?.expiresAt, t0.addingTimeInterval(15),
                            "把 expiresAt 推到未来，24 小时保质期就成了摆设")
        }

        TestKit.test("自报: 时钟回拨仍然重锚**未过期**的申报（不然它会永生）") {
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "s1", ttl: 15), now: t0)
            let earlier = t0.addingTimeInterval(-600)   // NTP 阶跃 / 虚拟机恢复快照
            store.reanchorIfClockRewound(now: earlier)
            store.sweepExpired(now: earlier.addingTimeInterval(16))
            try expectEqual(store.records.values.first?.expiredAt, earlier.addingTimeInterval(16),
                            "回拨后 expiresAt 仍落在未来 ⇒ 这条申报再也不会退回推断")
            try expectEqual(store.records.values.first?.receivedAt, earlier)
        }

        TestKit.test("自报: 写入之后不许留下「已过 TTL 却没盖章」的记录") {
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(session: "早到", ttl: 15), now: t0)
            store.ingest(sub(session: "晚到", ttl: 600), now: t0.addingTimeInterval(100))
            try expectEqual(store.records.values.filter { $0.expiredAt != nil }.count, 1,
                            "第二条申报进来时，第一条的过期就该已经被盖上了")
        }

        TestKit.test("自报: 一个说话者的洪水不能挤掉别人的活记录") {
            // 旧实现的淘汰键只有 receivedAt，且 64 个坑全档案共用：实测灌 64 条
            // `qoder/flood*` 之后，刚写进来的 `claude/real`（ttl 600）直接没了，
            // 而卡片只会安静地退回推断——「没看到」被讲成「没有」
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(agent: "claude", session: "real", state: .attention, ttl: 600), now: t0)
            for i in 0..<200 {
                store.ingest(sub(agent: "qoder", session: "flood\(i)", ttl: 600),
                             now: t0.addingTimeInterval(Double(i)))
            }
            try expectNotNil(store.record(agentID: "claude", sessionID: "real", now: t0),
                             "别的档案的申报不能把正在跑的会话挤出去")
            try expectTrue(store.records.values.filter { $0.agentID == "qoder" }.count
                            <= SelfReportStore.perAgentCapacity,
                           "洪水最多占掉**它自己**那几个坑")
            try expectTrue(store.records.count <= SelfReportStore.capacity, "总量仍然封顶")
            // 24 小时保质期后，全部过期证据要能退场（封顶不等于膨胀换个名字）
            for i in 0..<SelfReportStore.perAgentCapacity {
                store.ingest(sub(agent: "qoder", session: "later\(i)", ttl: 15),
                             now: t0.addingTimeInterval(SelfReportStore.evidenceRetention + 600))
            }
            try expectTrue(store.records.count <= SelfReportStore.capacity)
        }

        TestKit.test("自报: 可信窗口内的记录优先于旧证据被保留") {
            // 淘汰键**只**按 receivedAt 保新的时候，最老的那条可信记录第一批出局——
            // 而它恰恰是唯一还在续报窗口内、正在被卡片引用的那条。分层：先可信，再比新旧
            var store = SelfReportStore()
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            store.ingest(sub(agent: "claude", session: "live", state: .attention, ttl: 600), now: t0)
            // 洪水要摊到多个档案上，否则每档案封顶会先把它管住，测不到总量那一层
            for i in 0..<100 {
                store.ingest(sub(agent: "agent\(i % 20)", session: "s\(i)", ttl: 15),
                             now: t0.addingTimeInterval(Double(i)))
            }
            let now = t0.addingTimeInterval(40)   // claude/live 仍在窗口内，那 100 条已过 TTL
            try expectNotNil(store.record(agentID: "claude", sessionID: "live", now: now),
                             "比它们都新却已过期的证据，不该把还在续报的会话挤掉")
            try expectEqual(store.records.count, SelfReportStore.capacity, "总量仍然封顶")
        }

        // MARK: 协议形状（搬进 Core 才被测得到）
        //
        // 票 02 的 Done-when 是「curl 三条命令分别得到 accepted / pidMismatch / noToken」——
        // 那一面原先全在 executable target 里，测试 runner 链接不到，等于零守卫。
        // 两个纯函数搬过来之后，剩下的只有「发什么状态码」。

        TestKit.test("协议: ?agent= 的解析形状") {
            func agentOf(_ url: String) -> String? { SelfReportQuery.parse(from: url).agent }
            try expectNil(agentOf("/session"))
            try expectEqual(agentOf("/session?agent=claude"), "claude")
            try expectEqual(agentOf("/session?agent=claude&x=1"), "claude")
            try expectEqual(agentOf("/session?x=1&agent=claude"), "claude")
            try expectEqual(agentOf("/session?agent=cl%61ude"), "claude", "百分号编码要解")
            try expectEqual(agentOf("/session?agent=%20claude%20"), "claude")
            try expectNil(agentOf("/session?state=attention&session=s"), "只认 agent 这一个键")
            try expectNil(agentOf("/session?AGENT=claude"), "键大小写敏感：URL 是协议的键面，不猜")
            try expectEqual(agentOf("/session?agent="), "", "空值交给载荷层报「缺 agent」")
            // `"pid": null` 是「我明确不声称进程」，与「没带这个键」同义，不该被拒
            guard case .accepted(let explicitNil) = SelfReportPayload.parse(
                json(#"{"agent":"claude","session":"s","state":"idle","pid":null}"#),
                knownAgentIDs: ["claude"]), explicitNil.pid == nil else {
                throw TestError(message: "显式 null 的 pid 该按「不声称进程」收下")
            }
            try expectNil(agentOf("/session?agent=a=b"), "值里再带等号不是我们的形状")
            try expectEqual(agentOf("/session?agent=claude#frag"), "claude",
                            "片段要摘掉，否则回给调用方一个莫须有的 unknownAgent")
        }

        TestKit.test("协议: header 取值大小写不敏感，值里允许冒号") {
            let block = "POST /session?agent=claude HTTP/1.1\r\n"
                + "HOST: 127.0.0.1:41999\r\n"
                + "x-agentisland-token: abc123\r\n"
                + "Content-Type: application/json"
            try expectEqual(SelfReportHeaders.value(block, name: SelfReportTokenStore.headerName), "abc123",
                            "HTTP header 名按规范就是大小写不敏感的")
            try expectEqual(SelfReportHeaders.value(block, name: "content-type"), "application/json")
            try expectNil(SelfReportHeaders.value(block, name: "X-Absent"))
            // 值里带冒号时只能切第一刀：令牌是十六进制，但别的 header 会带时间戳
            try expectEqual(SelfReportHeaders.value("POST / HTTP/1.1\r\nX-When: a:b:c", name: "X-When"), "a:b:c")
        }

        TestKit.test("协议: 撤销结果的 reason 只能是枚举，人话进 message") {
            // `reason` 是给脚本 switch 的一格，上一轮它同时装过 "noToken" 与「没有这条记录」
            let revoke = try JSONDecoder().decode(CLISessionRevokeDTO.self,
                                                  from: Data(#"{"revoked":false,"reason":"notFound"}"#.utf8))
            try expectEqual(revoke.reason, .notFound)
            let encode = String(data: (try? JSONEncoder().encode(
                CLISessionResultDTO(bound: false, reason: .pidMismatch, message: "pid 9 不在进程表里"))) ?? Data(),
                encoding: .utf8) ?? ""
            try expectTrue(encode.contains("\"reason\":\"pidMismatch\""), encode)
            for r in SelfReportReason.allCases {
                try expectFalse(r.rawValue.contains(" "), "枚举值里不许出现句子：\(r.rawValue)")
                try expectFalse(r.rawValue.contains("未") || r.rawValue.contains("没有"), "\(r.rawValue)")
            }
        }

        TestKit.test("协议: 未鉴权的请求拿不到注册表的任何信息，也没人替它说谎") {
            // 两条口径同一条链路：① 无凭据 ⇒ reason/status 恒定（否则依次试 claude / zzz
            // 就枚举出这台机器装了哪些 Agent）；② 话要说真——`working` 根本不转事件，
            // 回一句「已按未采信事件投递」就是把没送讲成送了
            for r in [SelfReportRejection.malformed, .unknownAgent, .unknownState] {
                try expectEqual(SelfReportWire.reason(for: r, trusted: false), .noToken, "\(r)")
                // 未鉴权那条只许有 200 这一个来源：常量。表里不再另设一个「未鉴权分支」
                try expectEqual(SelfReportWire.untrustedStatus, 200, "\(r)")
                try expectEqual(SelfReportWire.reason(for: r, trusted: true),
                                SelfReportReason(rawValue: r.rawValue))
                try expectEqual(SelfReportWire.status(for: r), 400)
            }
            try expectTrue(SelfReportWire.untrustedMessage(posted: true, state: .attention)
                            .contains("已按未采信事件投递"))
            for persistent in [SelfReportState.working, .idle] {
                let msg = SelfReportWire.untrustedMessage(posted: false, state: persistent)
                try expectFalse(msg.contains("已按未采信事件投递"),
                                "没投递却写「已投递」：\(msg)")
                try expectTrue(msg.contains(persistent.rawValue), "要指名是哪条状态不转事件：\(msg)")
            }
        }

        // MARK: 引擎侧

        func engine(_ profiles: [AgentProfile], probe: SelfReportProcessProbe,
                   tokens: SelfReportTokenStore? = nil) -> ActivityEngine {
            ActivityEngine(profiles: profiles,
                           processMonitor: FakeProcessProvider(processNames: [], bundleIDs: []),
                           fileMonitor: FakeFileActivityProvider(writes: [:]),
                           installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }),
                           selfReportTokens: tokens ?? SelfReportTokenStore(
                               directory: FileManager.default.temporaryDirectory
                                   .appendingPathComponent("agentisland-engine-\(UUID().uuidString)")),
                           selfReportProbe: probe)
        }

        TestKit.test("引擎: 只有过令牌的申报会写进登记表") {
            let store = tempTokenStore("engine")
            guard let token = store.ensure() else { throw TestError(message: "没生成令牌") }
            let e = engine([claude], probe: probe(alive: [4242: "claude"]), tokens: store)
            _ = e.bindSelfReport(sub(pid: 4242, state: .attention),
                                 credential: try XCTRequire(e.authorizeSelfReport(token), "有效令牌却没产出凭证"))
            try expectEqual(e.selfReports.records.count, 1)
            _ = e.bindSelfReport(sub(session: "s2", pid: 999), credential: cred)
            try expectEqual(e.selfReports.records.count, 1, "pid 对不上不该留下一条可信记录")
            try expectNil(e.authorizeSelfReport("ffffffff"), "错串不产出凭证")
            try expectNil(e.authorizeSelfReport(nil))
            try expectEqual(e.believableSelfReport(agentID: "claude")?.state, .attention,
                            "没有令牌的请求连**调用**绑定的资格都没有（凭证非可选）")
            cleanup(dir: store.tokenURL.deletingLastPathComponent())
        }

        TestKit.test("引擎: 撤销要出示凭证，且没有的记录报 notFound 不报 revoked") {
            let e = engine([claude], probe: probe(alive: [4242: "claude"]))
            try expectEqual(e.revokeSelfReport(agentID: "claude", sessionID: "s1", credential: cred),
                            .notFound, "没有这条申报时不许谎报「已撤销」")
            _ = e.bindSelfReport(sub(state: .attention), credential: cred)
            try expectEqual(e.revokeSelfReport(agentID: "claude", sessionID: "s1", credential: cred), .revoked)
            try expectEqual(e.selfReports.records.count, 0)
        }

        TestKit.test("引擎: 采样那一拍把到期盖章做掉，且不清空") {
            let t0 = Date(timeIntervalSince1970: 1_700_000_000)
            let e = engine([claude], probe: probe(alive: [7: "claude"]))
            _ = e.bindSelfReport(sub(pid: 7, ttl: 20), credential: cred, now: t0)
            _ = e.sample(now: t0.addingTimeInterval(10))
            try expectEqual(e.selfReports.records.values.first?.expiredAt, nil)
            _ = e.sample(now: t0.addingTimeInterval(25))
            try expectNotNil(e.selfReports.records.values.first?.expiredAt)
            try expectEqual(e.selfReports.records.count, 1)
        }

        TestKit.test("引擎: 档案**删除**才回收自报；停用、终止、进程消失都不") {
            // 第一版挂在 `retainTracking` 上，而那条路径由设置页的可逆开关与安装扫描的
            // 自动重放驱动：实测「把 claude 关掉再打开」就永久删掉了 24 小时保质期内的证据。
            // 内存上界本来就由 capacity + evidenceRetention 给出，这一行只贡献丢证据的路径
            let custom = AgentProfile(id: "mine", name: "自定义", icon: "x", bundleIDs: [],
                                      processNames: ["mine"], sessionDirs: [], isCustom: true)
            let e = engine([claude, custom], probe: probe(alive: [7: "mine"]))
            _ = e.bindSelfReport(sub(agent: "mine", pid: 7), credential: cred)
            try expectEqual(e.selfReports.records.count, 1)
            e.setEnabled([claude.id])                      // 可逆的停用
            _ = e.sample(now: Date().addingTimeInterval(1))
            try expectEqual(e.selfReports.records.count, 1, "停用之后再打开，昨天那条申报该还在")
            e.setEnabled([claude.id, custom.id])
            e.removeCustomProfile("mine")                  // 不可逆的删除
            try expectEqual(e.selfReports.records.count, 0, "档案不在了，那条申报没有对撞对象")
        }

        TestKit.test("落回: 无令牌只投未采信事件，持续状态不投") {
            let e = engine([claude], probe: probe(alive: [:]))
            try expectEqual(SelfReportFallback.eventKind(for: .attention), .attention)
            try expectEqual(SelfReportFallback.eventKind(for: .completed), .completed)
            try expectNil(SelfReportFallback.eventKind(for: .working),
                          "工作态是持续状态，转成事件就是每续报一次响一次")
            try expectFalse(SelfReportFallback.post(sub(state: .working), to: e))
            try expectTrue(SelfReportFallback.post(sub(state: .completed), to: e))
            try expectEqual(e.eventHistory.first?.externallyDelivered, true,
                            "落回的事件必须带未采信标记，否则远程外发的闸门就白设了")
        }

        TestKit.test("落回: 全空白不算一句话，但有内容就不许动它一个字") {
            // 上一轮把 `detail`/`ask` 从 `name()`（trim + 纯空白按没给）换成只认字符串的 `text()`，
            // 于是 `{"ask":"   "}` 从「没给」变成「给了一行空白」。而落回时 `message` 是
            // `ask ?? detail` 这条 **nil 合并**链——空串与非空串一样「有值」，于是那一行空白
            // 顶掉了真正有内容的 `detail`；岛的 `summaryText` 只挡 `isEmpty`，
            // 系统通知更是 `message ?? ...` 直连。结果：未鉴权的一条申报能把横幅与通知刷成空白
            let e = engine([claude], probe: probe(alive: [:]))
            try expectTrue(SelfReportFallback.post(
                sub(state: .attention, detail: "在等确认", ask: "   "), to: e))
            try expectEqual(e.eventHistory.first?.message, "在等确认",
                            "全空白的 ask 不许顶掉有内容的 detail")
            try expectTrue(SelfReportFallback.post(
                sub(session: "s2", state: .completed, detail: "  \n  "), to: e))
            try expectNil(e.eventHistory.first?.message, "全是空白 ⇒ 没有这句话，让默认文案顶上")
            try expectNil(e.eventHistory.first?.detail, "「原因」展开器不许展开成一片空白")
            // 反过来：只要有一点点内容，就一个字都不许多动
            try expectTrue(SelfReportFallback.post(
                sub(session: "s3", state: .attention, detail: "  在等确认  ",
                    ask: " 要不要跑测试？ "), to: e))
            try expectEqual(e.eventHistory.first?.message, " 要不要跑测试？ ",
                            "保真：显示层只判「有没有话说」，不改写要说的话")
            try expectEqual(e.eventHistory.first?.detail, "  在等确认  ")
        }

        // MARK: 结构棘轮
        //
        // 只钉**类型挡不住**的那几条：可信度判定的位置、清理入口的边界、既有通道不许被换掉。
        // 上一轮那枚「不许出现 `tokenAccepted: true` 字面量」的棘轮已经由 `SelfReportCredential`
        // 接管（构造不出来就是编译错误），所以它连同它的假阳性一起删掉了。

        TestKit.test("结构: 可信自报只有一条写入路径，可信度只有一个出口") {
            let engineText = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIslandCore/ActivityEngine.swift"))
            let serverText = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIsland/LocalEventServer.swift"))
            let reportText = SourceTree.codeOnly(
                try SourceTree.text(relativePath: "Sources/AgentIslandCore/SelfReport.swift"))
            try expectEqual(engineText.components(separatedBy: "selfReports.ingest(").count - 1, 1,
                            "第二个写入点就是第二个「什么算可信自报」的出口")
            let bindRange = engineText.range(of: "func bindSelfReport")!.lowerBound
                ..< engineText.range(of: "func authorizeSelfReport")!.lowerBound
            try expectTrue(engineText[bindRange].contains("SelfReportBinder.bind"),
                           "绑定判定必须在 bindSelfReport 里，别挪到调用方")
            // 可信度的**唯一**产出点是 authorizeSelfReport → store.authorize
            try expectEqual(engineText.components(separatedBy: "SelfReportCredential()").count - 1, 0,
                            "引擎不许直接造凭证，凭证只能由令牌校验产出")
            try expectEqual(reportText.components(separatedBy: "SelfReportCredential()").count - 1, 1,
                            "全仓只有一处产出凭证（`authorize`），多一处就多一个出口")
            try expectFalse(serverText.contains("SelfReportCredential("),
                            "服务端不许构造凭证（它连 init 都看不到），只许转手")
            try expectFalse(serverText.contains("ingest("),
                            "服务端只该做 HTTP 形状，判可信的权力留在引擎")
            try expectFalse(serverText.contains("SelfReportTokenStore("),
                            "服务端不许自己再开一份令牌 store——读取方只许是引擎那一份")
            try expectTrue(serverText.contains("SelfReportTokenStore.headerName"),
                           "/session 必须从 header 取令牌，且 header 名来自唯一常量")
            try expectEqual(serverText.components(separatedBy: "engine.bindSelfReport(").count - 1, 1,
                            "第二个绑定调用点会绕过令牌判断")
            // 落回通道不能只在测试里活着：§3 那句「无令牌 ⇒ 落回 externallyDelivered」
            // 要真的挂在 noToken 响应那条路上
            try expectTrue(serverText.contains("SelfReportFallback.post("),
                           "无令牌路径必须把申报落到既有的未采信事件通道，而不是丢进字典了事")
            // 「没投递就说没投递」与「无凭据不透露载荷」这两条口径要真的挂在线上：
            // 它们住在 Core（可测），但服务端把它们换回硬编码字面量时行为仍然一样错
            try expectTrue(serverText.contains("SelfReportWire.untrustedMessage("),
                           "未采信响应的话术必须出自 Core 那个分叉函数，不许在服务端现写")
            try expectTrue(serverText.contains("SelfReportWire.reason(for:"),
                           "reason 的翻译只许走那张表")
            // 未鉴权那条回脸原先在服务端写死 `status: 200` + `reason: .noToken`，与表值相同
            // 却是第二处书写——改表的人看不见它，两边就此悄悄分叉。
            // 断言要**数得出两侧**：只断言「出现过一次」等于放开 DELETE 那一侧
            // （实测把 DELETE 改回 `status: 200` 而 reason 仍走常量时，`contains` 仍然绿）
            try expectEqual(serverText.components(separatedBy: "SelfReportWire.untrustedStatus").count - 1, 2,
                            "未鉴权的状态码只能由 SelfReportWire 那一个常量给出，且 POST/DELETE 两侧都走它")
            try expectEqual(serverText.components(separatedBy: "SelfReportWire.untrustedReason").count - 1, 2,
                            "noToken 要出自 SelfReportWire.untrustedReason，两侧都是")
            // 这一条比 `reason: .noToken` 更狠：`SelfReportReason.noToken`、换行的写法、
            // 服务端自己再开一个 `let mine = SelfReportReason.noToken` 都照样命中
            try expectFalse(serverText.contains(".noToken"),
                            "服务端不许出现 noToken 这个标识符——它只在 Core 那张表里写一次")
            // 400 也不许在服务端落地：绑定结果（profileGone）与载荷被拒共用 `rejectionStatus`，
            // 少这一条就会留下「改表的人看不见服务端那处 400」
            try expectFalse(serverText.contains("status: 400"),
                            "「不采信 ⇒ 400」只许写一次，服务端取 SelfReportWire.rejectionStatus")
            try expectFalse(serverText.contains("reason: .unknownAgent"),
                            "服务端不许自己把 unknownAgent 塞进响应——它只在表里翻译一次")
            try expectTrue(serverText.contains("SelfReportFallback.post(submission, to: engine)"),
                           "落回要带**这条**申报，不是另造一条空事件")
            // 既有通道一字不改：/notify 与 /event 的分支还在，且仍带未采信标记
            try expectTrue(serverText.contains(#"path == "/notify" || path == "/event""#),
                           "/session 是加上去的，不是把 /notify 换掉的")
            try expectTrue(serverText.contains("externallyDelivered: true"))
            // resetTracking / resetAllTracking / retainTracking 是「推断计时器」的清理入口。
            // 自报被卷进去的那天，用户会发现终止或**停用**一个 Agent 同时也抹掉了
            // 「它刚才说它在等确认」这条证据。按函数体切，且切的是剥完注释的正文——
            // 上一轮那版切法会把 `resetAllTracking` 的文档注释一起切进来，
            // 于是在注释里写一句合规的口径说明就能把它弄红
            for (from, to) in [("func resetTracking", "func resetAllTracking"),
                               ("func resetAllTracking", "func retainTracking"),
                               ("func retainTracking", "func bindSelfReport")] {
                let body = engineText[engineText.range(of: from)!.lowerBound
                                      ..< engineText.range(of: to)!.lowerBound]
                try expectFalse(body.contains("selfReports"),
                                "\(from) 里出现了 selfReports——清理边界被改动了")
            }
        }
    }
}

// MARK: - 这批用例共用的构造器
//
// 放在文件作用域而不是 `register()` 里面：局部函数**不继承**外层 `@MainActor`，
// 而它们要被 actor 内的闭包调用——照原来的写法会撞出一堆隔离诊断，读起来像 bug 而不是笔误。

private func json(_ s: String) -> Data { Data(s.utf8) }

/// 带变量的载荷一律走字典：raw string 里嵌 `"(数字)"` 会让转义读起来费劲，
/// 而这批用例有一半在测各种类型的字段值
private func payload(_ fields: [String: Any]) -> Data {
    Data((try? JSONSerialization.data(withJSONObject: fields)) ?? Data())
}

private func sub(agent: String = "claude", session: String = "s1", pid: Int32? = nil,
                 state: SelfReportState = .working, ttl: TimeInterval = 90,
                 detail: String? = nil, ask: String? = nil) -> SelfReportSubmission {
    SelfReportSubmission(agentID: agent, sessionID: session, pid: pid, state: state,
                         detail: detail, ask: ask, ttl: ttl)
}

private func probe(alive: [Int32: String]) -> SelfReportProcessProbe {
    SelfReportProcessProbe(isLive: { alive[$0] != nil }, executableName: { alive[$0] })
}

private func tempTokenStore(_ label: String) -> SelfReportTokenStore {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("agentisland-token-\(label)-\(UUID().uuidString)", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return SelfReportTokenStore(directory: dir)
}

/// 临时目录清理。写成函数而不是每个用例末尾一个 `defer`：`defer` 放在测试闭包的
/// **最后一行**时它的作用域就是整个闭包，编译器会直接告警「always executes immediately」，
/// 而这一批用例里有五条是那样写的
private func cleanup(dir: URL) {
    try? FileManager.default.removeItem(at: dir)
}

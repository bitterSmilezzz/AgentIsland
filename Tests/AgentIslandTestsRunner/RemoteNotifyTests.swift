import Foundation
@testable import AgentIslandCore

// MARK: - 远程通知（外发通道）
//
// 全部离线可测：SMTP 走脚本化的假服务器，HTTP 走注入的假传输。
// 真实端到端只能靠设置页的「发送测试」在用户机器上验，这里不假装验过。

/// 脚本化 SMTP 服务器：按队列吐出回复，并把客户端写来的每一行记下来。
/// 只由测试在主线程串行驱动（同 FakeTokenUsageProvider 的做法）
final class FakeSMTPSession: SMTPSessionIO, @unchecked Sendable {
    var script: [String]
    private(set) var written: [String] = []
    var tlsUpgrades = 0
    var closed = false

    init(script: [String]) { self.script = script }

    func readLine(timeout: TimeInterval) async -> String? { script.isEmpty ? nil : script.removeFirst() }
    func writeLine(_ text: String) async -> Bool { written.append(text); return true }
    func upgradeTLS() async -> Bool { tlsUpgrades += 1; return true }
    func close() { closed = true }
}

/// 假 HTTP 传输：记录请求、按预设返回结果
final class RecordingTransport: RemoteTransport, @unchecked Sendable {
    private(set) var requests: [RenderedRequest] = []
    var result: OutboundOutcome = .delivered
    func perform(_ request: RenderedRequest) async -> OutboundOutcome {
        requests.append(request)
        return result
    }
}

enum RemoteNotifyTests {
    @MainActor
    static func register() {
        // MARK: SMTP 协议序列

        TestKit.test("SMTP: 465 隐式 TLS 的完整命令序列与点填充") {
            let io = FakeSMTPSession(script: [
                "220 smtp.qq.com ESMTP",
                "250-AUTH LOGIN PLAIN", "250 OK",
                "334 VXNlcm5hbWU6", "334 UGFzc3dvcmQ6", "235 auth ok",
                "250 mail ok", "250 rcpt ok", "354 go ahead", "250 queued",
                    ])
            let target = SMTPTarget(host: "smtp.qq.com", port: 465, user: "me@qq.com",
                password: "authcode123", from: "me@qq.com", to: "me@qq.com")
            let outcome = try awaitOnMain {
                await SMTPClient.run(io: io, target: target, subject: "Dim 任务完成",
                    textBody: "第一行\n.第二行以点开头")
            }
            try expectEqual(outcome, .delivered)
            // 隐式 TLS 下不该出现 STARTTLS
            try expectFalse(io.written.contains("STARTTLS"), "465 已经是 TLS，再发 STARTTLS 是协议错误")
            try expectEqual(io.written.first, "EHLO \(SMTPHello.hostName)")
            try expectTrue(io.written.contains("MAIL FROM:<me@qq.com>"), "信封发件人必须来自配置")
            try expectTrue(io.written.contains("RCPT TO:<me@qq.com>"), "信封收件人必须来自配置")
            try expectTrue(io.written.contains("DATA"), "缺 DATA 服务器不会收正文")
            try expectEqual(io.written.last, "QUIT")
            // 授权码走 base64（不是明文），且确实是那两个值
            try expectTrue(io.written.contains(Data("me@qq.com".utf8).base64EncodedString()),
                "用户名必须以 base64 应答 334 挑战")
            try expectTrue(io.written.contains(Data("authcode123".utf8).base64EncodedString()),
                "密码必须以 base64 应答 334 挑战")
            // 行首的点是 SMTP 的邮件结束符，必须加倍
            try expectTrue(io.written.contains("..第二行以点开头"),
                "dot-stuffing 缺失会让正文被截断，且看起来像发送成功")
            try expectTrue(io.closed, "结束后必须关连接")
        }

        TestKit.test("SMTP: 服务器拒绝认证时如实失败，不谎报送达") {
            let io = FakeSMTPSession(script: [
                "220 hi", "250 OK", "334 x", "334 y", "535 auth failed",
                    ])
            let target = SMTPTarget(host: "h", port: 465, user: "u", password: "p", from: "u", to: "t")
            let outcome = try awaitOnMain {
                await SMTPClient.run(io: io, target: target, subject: "s", textBody: "b")
            }
            if case .failed(let reason) = outcome {
                try expectTrue(reason.contains("535"), "失败原因要带服务器回复码，否则用户无从排查：\(reason)")
            } else {
                throw TestError(message: "认证失败必须报 .failed，实得 \(String(describing: outcome))")
            }
            try expectFalse(io.written.contains("MAIL FROM:<u>"), "认证没过时不得继续发信")
            // 每条失败路径都必须关：留着的是一个活的 TLS 会话 + 已解密的授权码
            try expectTrue(io.closed, "认证失败后连接必须关掉")
            // 失败也必须关连接：走 SMTP 的通道每次失败都攒一条活 socket，
            // 长跑下会攒出几十个
            try expectTrue(io.closed, "失败路径同样要关连接")
        }

        TestKit.test("SMTP: 服务器静默（无响应）时报超时而不是卡死") {
            let io = FakeSMTPSession(script: [])   // 一行都不回
            let target = SMTPTarget(host: "h", port: 465, user: "u", password: "p", from: "u", to: "t")
            let started = Date()
            let outcome = try awaitOnMain(5) {
                await SMTPClient.run(io: io, target: target, subject: "s", textBody: "b", timeout: 0.2)
            }
            try expectTrue(Date().timeIntervalSince(started) < 3.0, "必须靠超时收敛，不能无限等")
            try expectTrue(io.closed, "超时退出也要关连接，否则每事件漏一个会话")
            if case .failed = outcome { } else {
                throw TestError(message: "无响应必须报失败，实得 \(String(describing: outcome))")
            }
        }

        TestKit.test("SMTP: 邮件头按 RFC 822 与 dot-stuffing 组好") {
            let target = SMTPTarget(host: "h", port: 465, user: "u", password: "p", from: "a@b.c", to: "d@e.f")
            let now = Date(timeIntervalSince1970: 1_800_000_000)
            let lines = SMTPMessage.lines(subject: "任务完成", body: "x\r\n.y\rz",
                target: target, now: now)
            try expectEqual(lines.first, "From: a@b.c")
            try expectTrue(lines.contains("To: d@e.f"), "收件人必须写进信头")
            try expectTrue(lines.contains(where: { $0.hasPrefix("Subject: =?UTF-8?B?") }),
                "中文主题要用 RFC 2047 编码，裸 UTF-8 会被多数服务器丢弃")
            try expectTrue(lines.contains("Date: \(RFC822Date.string(from: now))"), "Date 头必须是 RFC 822 格式")
            // \r\n 与孤立 \r 都要归一化，再各自点填充
            try expectTrue(lines.contains("x") && lines.contains("..y") && lines.contains("z"),
                "换行归一化 + 逐行点填充：\(lines)")
            try expectEqual(lines.last, ".")
        }

        TestKit.test("SMTP: 25/587 在配置层就被拒绝（Network.framework 不支持原地升级 TLS）") {
            var config = RemoteChannelConfig()
            config.smtpHost = "smtp.163.com"; config.smtpUser = "a@163.com"
            config.smtpTo = "a@163.com";  config.smtpPort = 25
            let missing = RemoteChannelKind.smtpEmail.missingField(config: config, hasSecret: true)
            try expectTrue(missing?.contains("465") == true,
                "非 465 端口要在设置页就提示，而不是每次发送都失败；实得 \(missing ?? "nil")")
            config.smtpPort = 465
            try expectNil(RemoteChannelKind.smtpEmail.missingField(config: config, hasSecret: true))
            // hasSecret 必须真的参与判据：恒 true 时「没存授权码」会被放行到发送阶段，
            // 用户看到的是每次连不上，而不是一句「去存钥匙串」
            let noKey = RemoteChannelKind.smtpEmail.missingField(config: config, hasSecret: false)
            try expectTrue(noKey?.contains("授权码") == true, "缺授权码要单独报出来：\(noKey ?? "nil")")
            var custom = RemoteChannelConfig()
            custom.urlTemplate = "https://x/{key}.send"
            try expectNotNil(RemoteChannelKind.customHTTP.missingField(config: custom, hasSecret: false),
                "有 {key} 占位却没密钥属于配齐失败")
            custom.urlTemplate = "https://x/y"
            custom.bodyTemplate = "title={title}&body={body}"
            try expectNil(RemoteChannelKind.customHTTP.missingField(config: custom, hasSecret: false),
                          "模板里没有占位，就不该要求密钥")
        }

        // MARK: 最小外发内容

        TestKit.test("隐私: 默认正文不含动作与消息原文，勾选后才进得去") {
            let inputs = RemoteNotifier.Inputs(agentName: "Qoder", kind: .attention, seconds: 95,
                actionDetail: "运行: swift build --target SecretTarget",
                message: "读取 /Users/x/private/notes.md")
            let off = RemoteNotifier.render(inputs: inputs, config: RemoteChannelConfig(),
                kind: .customHTTP)
            try expectFalse(off.body.contains("swift build"), "未勾选时命令内容不得离开本机")
            try expectFalse(off.body.contains("private"), "未勾选时消息原文不得离开本机")
            try expectTrue(off.title.contains("Qoder") && off.body.contains("等待你确认"),
                "最小内容也要够定位：Agent 名 + 状态")
            // 「刚发生的事」写成「N 分钟前」是谎报；只有 completed 才带用时
            let finished = RemoteNotifier.render(
                inputs: .init(agentName: "Qoder", kind: .completed, seconds: 95),
                config: RemoteChannelConfig(), kind: .customHTTP)
            try expectTrue(finished.body.contains("用时 1分"), "完成类要带用时：\(finished.body)")
            try expectTrue(off.body.contains("刚刚"), "非完成类不该编一个时长出来：\(off.body)")

            var config = RemoteChannelConfig()
            config.includeActionDetail = true
            let on = RemoteNotifier.render(inputs: inputs, config: config, kind: .customHTTP)
            try expectTrue(on.body.contains("swift build"), "勾选后应带上动作")
            try expectEqual(on.urgent, true, "等待确认属高优先级，完成不是")
            let done = RemoteNotifier.render(inputs: .init(agentName: "Dim", kind: .completed, seconds: 3),
                config: config, kind: .customHTTP)
            try expectEqual(done.urgent, false, "任务完成不该按高优先级送")
        }

        TestKit.test("时长文案: 秒/分/小时/天四档且不吃负数") {
            try expectEqual(DurationText.short(7), "7秒")
            try expectEqual(DurationText.short(95), "1分")
            try expectEqual(DurationText.short(7_200), "2小时")
            try expectEqual(DurationText.short(90_000), "1天")
            try expectEqual(DurationText.short(-5), "0秒")
        }

        TestKit.test("预览与掩码: 授权码、token、含 key 的 URL 一律打码") {
            var config = RemoteChannelConfig()
            config.urlTemplate = "https://sctapi.ftqq.com/{key}.send?token=abc123def456"
            config.bodyTemplate = "title={title}&desp={body}"
            config.useJSONBody = false
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in "SCT9999PRIVATEKEY" })
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 10)
            let preview = notifier.preview(inputs: inputs, kind: .customHTTP, config: config)
            try expectFalse(preview.requestSummary.contains("SCT9999PRIVATEKEY"),
                "预览泄漏密钥：\(preview.requestSummary)")
            try expectFalse(preview.requestSummary.contains("abc123def456"), "query 里的 token 必须打码")
            try expectTrue(preview.requestSummary.contains("•"), "打码后要看得出打过码")
            try expectFalse(preview.body.contains("SCT"), "正文本身也不该混进密钥")

            // 真实发送走的是明文，且密钥确实替换进去了
            _ = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .customHTTP, config: config,
                    policy: RemoteNotifyPolicy(masterEnabled: true))
            }
            let sent = try XCTRequire(transport.requests.first, "未发出请求")
            try expectTrue(sent.url.contains("SCT9999PRIVATEKEY"), "实际请求应带真实密钥")
            try expectFalse(sent.url.contains("{key}"), "占位符必须被替换掉")
            try expectTrue(sent.body.contains("title=Dim"), "bodyTemplate 要真的被替换")
            try expectFalse(sent.body.contains("{body}"), "body 占位符未替换说明模板没走完")
            try expectEqual(sent.headers.first?.value, "application/x-www-form-urlencoded")
        }

        TestKit.test("掩码工具: query 密钥、主机名即密钥、短值全打码") {
            let m = RemoteSecret.masked("abcdefghij")
            try expectTrue(m.hasPrefix("ab") && m.hasSuffix("ij") && m.contains("••"),
                "掩码要留住首尾各 2 位供辨认，中间全部遮掉：\(m)")
            try expectEqual(m.count, 10, "打码不该改变长度，否则用户无法核对是不是那条密钥")
            try expectFalse(RemoteSecret.masked("abcdefghij").contains("cdefg"), "中间必须遮")
            try expectEqual(RemoteSecret.masked("abc"), "•••", "短值没有可留的首尾，全打码")

            let query = RemoteSecret.maskedURL("https://h/x?access_token=zztop1secret99xx&y=1")
            try expectFalse(query.contains("top1secret"), "access_token 中段必须打码：\(query)")
            try expectTrue(query.contains("y=1"), "其余参数不该被顺手改掉：\(query)")
            let form = RemoteSecret.maskedURL("https://h/x?key=abcdefghijklmnop&y=1")
            try expectFalse(form.contains("bcdefghijklm"), "key= 形式必须打码：\(form)")
            let host = RemoteSecret.maskedURL("https://SCT1234567890abcdefghijklmno.send/api")
            try expectFalse(host.contains("1234567890abcdefghijkl"),
                "整段主机名就是密钥的形态必须打码：\(host)")
            try expectTrue(host.contains(".send/api"), "打码后要看得出协议形状：\(host)")
            try expectTrue(RemoteSecret.maskedURL("https://sctapi.ftqq.com/x").contains("sctapi.ftqq.com"),
                "正常域名不该被打码，否则预览认不出在往哪发")
            try expectTrue(RemoteSecret.maskedURL("https://ntfy.sh/my-topic").contains("my-topic"),
                "普通地址不该被动过")
        }

        // MARK: ntfy 请求形状（按官方文档）

        TestKit.test("ntfy: POST 到 <服务器>/<主题>，标题与优先级走 X-Title / X-Priority") {
            var config = RemoteChannelConfig()
            config.topicOrURL = "my-agent-island-topic"
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            _ = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Codex", kind: .costSpike, seconds: 5),
                    kind: .ntfy, config: config,
                    policy: RemoteNotifyPolicy(masterEnabled: true))
            }
            let req = try XCTRequire(transport.requests.first, "未发出请求")
            try expectEqual(req.url, "https://ntfy.sh/my-agent-island-topic")
            try expectEqual(req.method, "POST")
            let title = try XCTRequire(req.headers.first(where: { $0.name == "X-Title" })?.value)
            try expectEqual(title, WireText.headerValue("Codex · 消耗告警"))
            try expectEqual(title.removingPercentEncoding, "Codex · 消耗告警",
                            "头字段必须 ASCII 且可解码回原文（非 ASCII 会被 URLSession 静默丢掉）")
            try expectEqual(req.headers.first(where: { $0.name == "X-Priority" })?.value, "4",
                "告警要用 high(4)")

            _ = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Codex", kind: .completed, seconds: 5),
                    kind: .ntfy, config: config,
                    policy: RemoteNotifyPolicy(masterEnabled: true))
            }
            try expectEqual(transport.requests.last?.headers.first(where: { $0.name == "X-Priority" })?.value, "3",
                "普通完成用 default(3)")

            // 自建服务器：填完整地址时不再拼主题
            var selfHosted = config
            selfHosted.topicOrURL = "https://ntfy.mine.example/island"
            _ = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Codex", kind: .attention, seconds: 5),
                    kind: .ntfy, config: selfHosted,
                    policy: RemoteNotifyPolicy(masterEnabled: true))
            }
            try expectEqual(transport.requests.last?.url, "https://ntfy.mine.example/island",
                "填完整地址时不该再追加一段主题")
        }

        // MARK: 策略：开关、类型、节流、静默

        TestKit.test("策略: 总开关与分类开关关掉时一个字节都不出本机") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 1)

            let off = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config,
                    policy: RemoteNotifyPolicy(masterEnabled: false))
            }
            try expectEqual(off, .suppressed(reason: "总开关未开"))
            let typeOff = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config,
                    policy: RemoteNotifyPolicy(masterEnabled: true, sendCompleted: false))
            }
            if case .suppressed(let r) = typeOff {
                try expectTrue(r.contains("类"), "应报「该类事件已关闭」，实得 \(r)")
            } else {
                throw TestError(message: "分类开关未生效：\(String(describing: typeOff))")
            }
            // 被策略挡下时绝不能碰到网络（上面三次都该是这个结果）
            try expectTrue(transport.requests.isEmpty, "被策略挡下时绝不能碰到网络")
        }

        TestKit.test("策略: 节流只挡同类事件，失败不占用节流窗口") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let policy = RemoteNotifyPolicy(masterEnabled: true, throttleSeconds: 600)
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 1)

            let first = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy)
            }
            let second = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(first, .delivered)
            if case .suppressed(let reason) = second {
                try expectTrue(reason.contains("节流"), "第二次应被节流挡住，实得 \(reason)")
            } else {
                throw TestError(message: "节流未生效：\(String(describing: second))")
            }
            // 不同事件类型是另一条节流线（一次任务里「等待确认」不该被「完成」挤掉）
            let other = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Dim", kind: .attention, seconds: 1),
                    kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(other, .delivered, "节流按「Agent+事件类型」分键")

            // 失败不记节流时间戳：否则一次抖动会吞掉后面一整段通知
            var config2 = config
            config2.topicOrURL = "another"
            let attnInputs = RemoteNotifier.Inputs(agentName: "Other", kind: .completed, seconds: 1)
            transport.result = .failed(reason: "服务端返回 500")
            let failed = try awaitOnMain {
                await notifier.deliver(inputs: attnInputs, kind: .ntfy, config: config2, policy: policy)
            }
            try expectEqual(failed, .failed(reason: "服务端返回 500"))
            transport.result = .delivered
            let afterFailure = try awaitOnMain {
                await notifier.deliver(inputs: attnInputs, kind: .ntfy, config: config2, policy: policy)
            }
            try expectEqual(afterFailure, .delivered, "上一次失败不该挡住这次重试")
        }

        TestKit.test("策略: 配置缺失优先于节流，别让「根本没配好」看起来像被节流") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 1)
            let policy = RemoteNotifyPolicy(masterEnabled: true, throttleSeconds: 600)
            // 先成功发一次，占住这条「Dim|completed」的节流窗口
            let ok = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(ok, .delivered)
            // 之后配置坏掉（主题被清空）：第二次必须说「缺主题」，不能说「节流命中」——
            // 后者会把用户引向等一等的死路，而真实原因是根本没配好
            config.topicOrURL = ""
            let second = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy)
            }
            if case .notConfigured(let reason) = second {
                try expectTrue(reason.contains("主题"), "要指出缺什么，实得 \(reason)")
            } else {
                throw TestError(message: "未配置必须报 notConfigured，实得 \(String(describing: second))")
            }
            try expectEqual(transport.requests.count, 1, "配置坏了的那一次不该碰网络")
        }

        TestKit.test("策略: 密钥缺失时报未配置而不是发出带 {key} 的废请求") {
            let transport = RecordingTransport()
            var config = RemoteChannelConfig()
            config.urlTemplate = "https://sctapi.ftqq.com/{key}.send"
            config.bodyTemplate = "title={title}&desp={body}"
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 1)
            let policy = RemoteNotifyPolicy(masterEnabled: true)

            let noKey = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            let out = try awaitOnMain {
                await noKey.deliver(inputs: inputs, kind: .customHTTP, config: config, policy: policy)
            }
            if case .notConfigured(let reason) = out {
                try expectTrue(reason.contains("钥匙串"), "要指明去钥匙串存，实得 \(reason)")
            } else {
                throw TestError(message: "缺密钥应报 notConfigured，实得 \(String(describing: out))")
            }
            try expectTrue(transport.requests.isEmpty, "带着未替换的 {key} 发出去等于白发")

            // SMTP 同理：没授权码就别去连服务器
            var mail = RemoteChannelConfig()
            mail.smtpHost = "smtp.qq.com"; mail.smtpUser = "a@qq.com"
            mail.smtpTo = "a@qq.com"
            let mailOut = try awaitOnMain {
                await noKey.deliver(inputs: inputs, kind: .smtpEmail, config: mail, policy: policy)
            }
            if case .notConfigured = mailOut { } else {
                throw TestError(message: "SMTP 缺授权码应报 notConfigured，实得 \(String(describing: mailOut))")
            }
        }

        TestKit.test("静默时段: 跨零点、不跨零点、未配置三种情形都判对") {
            // 与 inQuietHours 的默认 calendar 取同一个（Calendar.current），否则测试构造
            // 用的时区与被测代码读的时区可能不是一回事
            let cal = Calendar.current
            func at(_ h: Int, _ m: Int) throws -> Date {
                guard let d = cal.date(from: DateComponents(year: 2026, month: 9, day: 20,
                    hour: h, minute: m)) else {
                    throw TestError(message: "测试时刻 \(h):\(m) 构造失败")
                }
                return d
            }
            let wrap = RemoteNotifyPolicy(masterEnabled: true, quietStart: "22:00", quietEnd: "07:30")
            try expectTrue(wrap.inQuietHours(at(23, 30)), "23:30 应在 22:00–07:30 内")
            try expectTrue(wrap.inQuietHours(at(3, 0)), "03:00 应算跨零点区间内")
            try expectFalse(wrap.inQuietHours(at(12, 0)), "12:00 不该静默")
            try expectFalse(wrap.inQuietHours(at(21, 59)), "起点前一分钟不该静默")
            try expectTrue(wrap.inQuietHours(at(22, 0)), "起点当刻该静默")
            try expectFalse(wrap.inQuietHours(at(7, 30)), "终点当刻不静默（左闭右开）")

            let day = RemoteNotifyPolicy(masterEnabled: true, quietStart: "09:00", quietEnd: "17:00")
            try expectTrue(day.inQuietHours(at(10, 0)))
            try expectFalse(day.inQuietHours(at(18, 0)))
            try expectFalse(RemoteNotifyPolicy(masterEnabled: true).inQuietHours(at(3, 0)),
                "没配静默时段却全天静默 = 通知功能悄悄失效")
            try expectFalse(RemoteNotifyPolicy(masterEnabled: true, quietStart: "08:00", quietEnd: "08:00")
            .inQuietHours(at(8, 30)), "起止相同视为不静默，否则等于全天吞掉")
        }

        TestKit.test("策略归一化: 写坏的静默时段退化成「不静默」，节流秒数钳进区间") {
            var policy = RemoteNotifyPolicy(throttleSeconds: 99_999, quietStart: "25:99", quietEnd: "07:00")
            var normalized = policy.normalized()
            try expectEqual(normalized.throttleSeconds, RemoteNotifyPolicy.normalizedThrottle.upperBound)
            try expectTrue(normalized.quietStart.isEmpty && normalized.quietEnd.isEmpty,
                "非法时间必须整段作废；只清一半会造成「全天静默」这种查不出来的失效")
            policy = RemoteNotifyPolicy(throttleSeconds: -10)
            normalized = policy.normalized()
            try expectEqual(normalized.throttleSeconds, RemoteNotifyPolicy.normalizedThrottle.lowerBound,
                "节流为 0 会让一次任务的连发事件全部外送")
            policy = RemoteNotifyPolicy(quietStart: "22:00", quietEnd: "")
            try expectTrue(policy.normalized().quietStart.isEmpty, "只有一端有值的半截区间应整段作废")
        }

        TestKit.test("发送测试: 绕过节流与静默时段，但不绕过配置缺失") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in "k" })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let policy = RemoteNotifyPolicy(masterEnabled: true, throttleSeconds: 3600,
                                            quietStart: "22:00", quietEnd: "06:00")
            // 固定时刻（不取 Date()）：静默窗口的判定必须是被测代码给的，不是测试跑到的时刻
            let cal = Calendar.current
            guard let morning = cal.date(from: DateComponents(year: 2026, month: 9, day: 20, hour: 10)),
                  let night = cal.date(from: DateComponents(year: 2026, month: 9, day: 20, hour: 23, minute: 30)) else {
                throw TestError(message: "测试时刻构造失败")
            }
            // testDeliver 内部用的是 agentName "AgentIsland" + .attention：
            // 先用同一条键占掉节流窗口，才能证明「测试」真的绕过了节流
            let probeInputs = RemoteNotifier.Inputs(agentName: "AgentIsland", kind: .attention, seconds: 0)
            let first = try awaitOnMain {
                await notifier.deliver(inputs: probeInputs, kind: .ntfy, config: config, policy: policy,
                                       now: morning)
            }
            try expectEqual(first, .delivered)
            let blocked = try awaitOnMain {
                await notifier.deliver(inputs: probeInputs, kind: .ntfy, config: config, policy: policy,
                                       now: morning.addingTimeInterval(1))
            }
            if case .suppressed = blocked { } else {
                throw TestError(message: "节流应挡住常规事件，实得 \(String(describing: blocked))")
            }
            // 「发送测试」是用户按下按钮要看通道通不通：既绕节流也绕静默
            let tested = try awaitOnMain {
                await notifier.testDeliver(kind: .ntfy, config: config, policy: policy, now: night)
            }
            try expectEqual(tested, .delivered, "测试按钮被策略挡住就等于没有测试功能")
            try expectEqual(transport.requests.count, 2)
            // 但总开关仍然管用：它是隐私闸门，按一次测试就能出本机就等于开关不可信
            let offPolicy = RemoteNotifyPolicy(masterEnabled: false)
            let gated = try awaitOnMain {
                await notifier.testDeliver(kind: .ntfy, config: config, policy: offPolicy, now: night)
            }
            if case .suppressed(let reason) = gated {
                try expectTrue(reason.contains("总开关"), "实得 \(reason)")
            } else {
                throw TestError(message: "总开关关闭时测试也不该送出，实得 \(String(describing: gated))")
            }
            // 事件类型开关也不绕过：关掉「等待你确认」的人不该被测试按钮代发一条
            let typeOff = RemoteNotifyPolicy(masterEnabled: true, sendAttention: false)
            let typed = try awaitOnMain {
                await notifier.testDeliver(kind: .ntfy, config: config, policy: typeOff, now: morning)
            }
            if case .suppressed(let reason) = typed {
                try expectTrue(reason.contains("类"), "要说明是哪一类事件被关了，实得 \(reason)")
            } else {
                throw TestError(message: "类型开关不该被测试按钮绕过，实得 \(String(describing: typed))")
            }
            // 但配置缺失仍要如实报，不能因为「是测试」就假装成功
            var broken = config
            broken.topicOrURL = ""
            let unconfigured = try awaitOnMain {
                await notifier.testDeliver(kind: .ntfy, config: broken, policy: policy)
            }
            if case .notConfigured = unconfigured { } else {
                throw TestError(message: "缺主题时应报未配置，实得 \(String(describing: unconfigured))")
            }
            try expectEqual(transport.requests.count, 2, "未配置时不该碰到网络")
            // 测试结果按时间倒序进历史（最新在前），设置页「最近外发」读的就是它
            try expectEqual(notifier.recentAttempts.first?.shortText, "未配置：缺主题名或服务器地址")
            try expectEqual(notifier.recentAttempts.count, 6, "含被挡下与被开关拦住的每一次都要留痕")
        }

        // MARK: 上线前的文本整形

        TestKit.test("线格式: 中文标题编成 ASCII 头值，且能解码回原文") {
            let raw = "Qoder · 等待你确认"
            let encoded = WireText.headerValue(raw)
            try expectTrue(encoded.allSatisfy { $0.isASCII },
                           "HTTP 头值含非 ASCII 时 URLSession 会静默丢弃后半段（实测只剩「Qoder · 」）")
            try expectEqual(encoded.removingPercentEncoding, raw, "编码后要能还原，接收端才会显示原文")
            try expectTrue(!encoded.contains(" "), "空格编成 %20 而不是留原字符：\(encoded)")
            try expectEqual(WireText.headerValue("plain-Title_1"), "plain-Title_1",
                            "本来就安全的值不该被动过")
            // 头字段就算接收端不解码，正文也必须自带 Agent 名——否则中文在链路上丢了就没人知道
            let msg = RemoteNotifier.render(inputs: .init(agentName: "Qoder", kind: .attention, seconds: 0),
                                            config: RemoteChannelConfig(), kind: .ntfy)
            try expectTrue(msg.body.contains("Qoder"), "正文首行要含 Agent 名：\(msg.body)")
        }

        TestKit.test("线格式: SMTP 地址里的换行不会变成第二条命令") {
            let evil = "a@b.c\r\nRCPT TO:<victim@evil.com>"
            let line = WireText.smtpLine(evil)
            try expectFalse(line.contains("\r") || line.contains("\n"),
                            "一行里留裸换行 = 往 SMTP 会话里注入一条命令：\(line)")
            try expectTrue(line.contains("RCPT TO:<victim@evil.com>"),
                           "折行而不是丢弃，用户能看出自己填错了什么")
            let target = SMTPTarget(host: "h", port: 465, user: "u", password: "p",
                                    from: evil, to: evil)
            let io = FakeSMTPSession(script: [
                "220 hi", "250 OK", "334 u", "334 p", "235 ok",
                "250 mail", "250 rcpt", "354 go", "250 queued",
            ])
            let outcome = try awaitOnMain {
                await SMTPClient.run(io: io, target: target, subject: "s", textBody: "b")
            }
            try expectEqual(outcome, .delivered)
            try expectEqual(io.written.filter { $0.hasPrefix("RCPT TO:") }.count, 1,
                            "被注入的话这里会出现两条 RCPT")
            try expectEqual(io.written.filter { $0.contains("\n") || $0.contains("\r") }.count, 0,
                            "writeLine 的内容里不该有裸换行")
        }

        TestKit.test("真实 socket: 连不上的地址必须在超时内返回 false，而不是永久挂住") {
            // 实测过：127.0.0.1 上一个没人监听的端口会让 NWConnection 停在 .waiting 反复
            // 重试路径，状态永远走不到 .ready/.failed——只在状态回调里恢复续体的写法
            // 会把「发送测试」按钮永久卡在「正在送出…」。这条用真 socket，不走假 IO。
            let sock = SMTPSocketConnection(host: "127.0.0.1", port: 1, implicitTLS: true)
            let started = Date()
            let ok = try awaitOnMain(25) { await sock.connect(timeout: 1.5) }
            try expectFalse(ok, "连不上却报成功就是谎报")
            try expectTrue(Date().timeIntervalSince(started) < 12,
                           "必须在超时内收敛，实得 \(Int(Date().timeIntervalSince(started)))s")
            sock.close()
            // 非隐式 TLS 端口在 socket 层再兜一道（配置层已拦，这里防绕过）
            let plain = SMTPSocketConnection(host: "127.0.0.1", port: 1, implicitTLS: false)
            let ok2 = try awaitOnMain(10) { await plain.connect() }
            try expectFalse(ok2, "不支持原地升级 TLS，不该去连")
        }

        TestKit.test("策略: 只在无人时外发——判据来自调用点给的在场信号") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let inputs = RemoteNotifier.Inputs(agentName: "Dim", kind: .completed, seconds: 1)
            let policy = RemoteNotifyPolicy(masterEnabled: true, onlyWhenAway: true)

            let present = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy,
                                       away: false)
            }
            if case .suppressed(let reason) = present {
                try expectTrue(reason.contains("有人"), "要说明是谁在场挡下的，实得 \(reason)")
            } else {
                throw TestError(message: "有人在机器前不该外发，实得 \(String(describing: present))")
            }
            try expectTrue(transport.requests.isEmpty, "被这条挡下时绝不能碰网络")
            let awayOutcome = try awaitOnMain {
                await notifier.deliver(inputs: inputs, kind: .ntfy, config: config, policy: policy,
                                       away: true)
            }
            try expectEqual(awayOutcome, .delivered)
            // 开关默认关：关了就是「无条件照发」，不能把在场当理由吞掉通知。
            // 换一个 Agent 名：上一条已经占住了「Dim|completed」的节流窗口
            let off = RemoteNotifyPolicy(masterEnabled: true)
            try expectFalse(off.onlyWhenAway, "默认必须关：这是新增行为，默认开会让老用户的通知少发")
            let ignored = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Dim2", kind: .completed, seconds: 1),
                                       kind: .ntfy, config: config, policy: off, away: false)
            }
            try expectEqual(ignored, .delivered)
            // 「发送测试」按按下按钮就要立刻看到结果的语义走，和节流/静默一样绕过
            let tested = try awaitOnMain {
                await notifier.testDeliver(kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(tested, .delivered, "测试按钮被「有人在机器前」挡住等于没有测试功能")
        }

        TestKit.test("线格式: 正文里的 & # % + 不会被当成查询分隔符") {
            // 勾选「附带最后一条动作」后正文是真命令，`a && curl evil` 这类文本
            // 必须整体成为一个字段的值，而不是在接收端被切成两个参数
            let tricky = "运行: git commit -m a && curl 'http://x/y?a=1#z' 100%"
            let query = WireText.queryValue(tricky)
            try expectFalse(query.contains("&"), "查询串里不许出现裸 &：\(query)")
            try expectFalse(query.contains("#"), "裸 # 会被当成片段起点：\(query)")
            try expectFalse(query.contains("+"), "裸 + 会被解成空格：\(query)")
            try expectEqual(query.removingPercentEncoding, tricky, "必须能原样还原")
            let form = WireText.formValue("a b")
            try expectEqual(form, "a+b", "表单按 x-www-form-urlencoded 用 + 表示空格")
            // 头值是另一套规则：那里 & # % + 本来就允许出现，不该被过度编码
            try expectEqual(WireText.headerValue("a&b#c"), "a&b#c")
            let raw = "他说\"好\" 尾随一个反斜杠 \\ 换行\n下一行 \u{0001}"
            let json = WireText.jsonString(raw)
            try expectTrue(json.hasPrefix("\"") && json.hasSuffix("\""), "JSON 字面量要带引号")
            try expectTrue(json.contains("\\\""), "裸引号必须转义，否则整条 JSON 非法：\(json)")
            try expectTrue(json.contains("\\\\"), "反斜杠自身要转义：\(json)")
            try expectTrue(json.contains("\\n") && !json.contains("\n"),
                           "正文里的换行不能真的断行：\(json)")
            try expectTrue(json.contains("\\u0001"), "控制字符要转义：\(json)")
            // 转义完必须还能解回去，否则是「看着安全、其实对方收到一堆乱码」
            let back = try XCTRequire(
                JSONSerialization.jsonObject(with: Data("{\"v\":\(json)}".utf8)) as? [String: String])
            try expectEqual(back["v"], raw, "JSON 转义要可逆")
        }

        TestKit.test("SMTP: 多行回复要读到结束，否则后面的读取全部错位") {
            // QQ / 163 的 250 常见多行形态。只读一行的话，剩下的行会留在流里，
            // 之后每一次 expect 都读到上一条的尾巴——症状是「莫名其妙的回复码」
            let io = FakeSMTPSession(script: [
                "220-smtp.qq.com 准备就绪", "220 OK",
                "250-SIZE 104857600", "250 HELP",
                "334 VXNlcm5hbWU6", "334 UGFzc3dvcmQ6", "235 ok",
                "250-继续", "250 好", "250 rcpt", "354 go", "250 queued",
            ])
            let target = SMTPTarget(host: "smtp.qq.com", port: 465, user: "u@qq.com",
                                    password: "p", from: "u@qq.com", to: "t@qq.com")
            let outcome = try awaitOnMain {
                await SMTPClient.run(io: io, target: target, subject: "s", textBody: "b")
            }
            try expectEqual(outcome, .delivered, "多行回复没读完就会在后面的步骤上判成失败")
        }

        TestKit.test("配置层: ntfy 少贴协议头不被送进公网；customHTTP 必须自带字段名") {
            var ntfy = RemoteChannelConfig()
            // 这正是危险形态：不带 :// 的自建域名会被拼到 https://ntfy.sh/ 下面
            ntfy.topicOrURL = "ntfy.mine.local/island"
            let missing = RemoteChannelKind.ntfy.missingField(config: ntfy, hasSecret: true)
            try expectTrue(missing?.contains("http://") == true,
                           "少协议头要提示，不能静默把内容发到公网：\(missing ?? "nil")")
            ntfy.topicOrURL = "https://ntfy.mine.local/island"
            try expectNil(RemoteChannelKind.ntfy.missingField(config: ntfy, hasSecret: true))
            ntfy.topicOrURL = "我的主题"
            try expectNotNil(RemoteChannelKind.ntfy.missingField(config: ntfy, hasSecret: true),
                             "非 ASCII 主题名到不了 ntfy")

            var custom = RemoteChannelConfig()
            custom.urlTemplate = "https://hooks.example.com/notify"
            try expectTrue(RemoteChannelKind.customHTTP.missingField(config: custom, hasSecret: true)?
                .contains("请求体模板") == true,
                "本仓不预置未核实的字段名，所以模板必须用户自己填")
            custom.bodyTemplate = "title={title}&body={body}"
            try expectNil(RemoteChannelKind.customHTTP.missingField(config: custom, hasSecret: true))
        }

        TestKit.test("掩码与警告: 路径段里的凭据要遮；明文 http 与粘进模板的密钥要警告") {
            let path = "https://api.example.com/notify/0123456789abcdef0123456789abcdef"
            let masked = RemoteSecret.maskedURL(path)
            try expectFalse(masked.contains("0123456789abcdef0123456789abcdef"),
                            "密钥在路径段里的形态也要遮：\(masked)")
            try expectTrue(masked.contains("/notify/"), "遮完仍要看得出地址形状：\(masked)")
            // 普通 ntfy 主题不能被顺手遮掉，否则预览认不出在往哪发
            try expectTrue(RemoteSecret.maskedURL("https://ntfy.sh/my-agent-island-topic")
                .contains("my-agent-island-topic"))

            var http = RemoteChannelConfig()
            http.topicOrURL = "http://ntfy.lan/island"
            try expectNotNil(RemoteChannelKind.ntfy.insecureEndpoint(config: http),
                             "明文 http 会把密钥与内容明文送出，必须单独警告")
            http.topicOrURL = "https://ntfy.sh/island"
            try expectNil(RemoteChannelKind.ntfy.insecureEndpoint(config: http))

            var pasted = RemoteChannelConfig()
            pasted.urlTemplate = "https://sctapi.ftqq.com/SCT1234567890abcdefghij.send"
            try expectNotNil(RemoteChannelKind.customHTTP.plaintextSecretInTemplate(config: pasted),
                             "用户把密钥直接粘进地址时要说破：它会明文落盘")
            pasted.urlTemplate = "https://sctapi.ftqq.com/{key}.send"
            try expectNil(RemoteChannelKind.customHTTP.plaintextSecretInTemplate(config: pasted),
                          "用 {key} 占位的正常写法不该报警")
        }

        TestKit.test("节流按来源分键: 换一个显示名不能绕开窗口") {
            let transport = RecordingTransport()
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            var config = RemoteChannelConfig()
            config.topicOrURL = "topic"
            let policy = RemoteNotifyPolicy(masterEnabled: true, throttleSeconds: 600)
            // 外部投递路径的 agentName 是调用方随便填的：键必须用 id，
            // 否则每次换个名字就等于绕过节流（无上限地往外发）
            let first = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Dim", agentId: "claude",
                                                     kind: .completed, seconds: 1),
                                       kind: .ntfy, config: config, policy: policy)
            }
            let second = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "完全不同的名字", agentId: "claude",
                                                     kind: .completed, seconds: 1),
                                       kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(first, .delivered)
            if case .suppressed(let reason) = second {
                try expectTrue(reason.contains("节流"), "实得 \(reason)")
            } else {
                throw TestError(message: "同 id 换显示名绕开了节流：\(String(describing: second))")
            }
            // 另一个 id 是另一条窗口（两个 Agent 同时完成不该互相吞掉）
            let other = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Qoder", agentId: "qoder",
                                                     kind: .completed, seconds: 1),
                                       kind: .ntfy, config: config, policy: policy)
            }
            try expectEqual(other, .delivered)
        }

        TestKit.test("配置解码: 以后加字段不会让已配好的通道整条清空") {
            let defaults = TestDefaults.suite("remote-decode")
            // 只写一半字段（等价于「旧版本存档」），读回来必须拿到其余默认值而不是整条 nil
            let json = "{\"topicOrURL\":\"half-written\"}"
            defaults.set(Data(json.utf8), forKey: RemoteNotifyStore.configKey(.ntfy))
            let back = RemoteNotifyStore.loadConfig(for: .ntfy, defaults: defaults)
            try expectEqual(back.topicOrURL, "half-written", "缺键要能补，否则升级即清空用户配置")
            try expectTrue(back.useJSONBody, "缺键的布尔值回落到默认而不是 false")
            // 手改 plist 写出的坏端口在读盘时收口
            // 类型写错的整条解不开 → 回落默认
            let malformed = "{\"smtpHost\":\"h\",\"smtpPort\":\"25\",\"smtpUser\":\"u\",\"smtpTo\":\"t\"}"
            defaults.set(Data(malformed.utf8), forKey: RemoteNotifyStore.configKey(.smtpEmail))
            // 单个字段类型写错只作废那一个字段，其余保住（见 RemoteChannelConfig 的解码注释）
            try expectEqual(RemoteNotifyStore.loadConfig(for: .smtpEmail, defaults: defaults).smtpHost, "h",
                            "一处笔误不该放大成「全部重填」")
            try expectEqual(RemoteNotifyStore.loadConfig(for: .smtpEmail, defaults: defaults).smtpPort, 465,
                            "写坏的端口这一项要回落默认")
            // 能解开的坏值（0 / 越界）要在读盘时收口，否则每次都连向一个不存在的端口
            let bad = "{\"smtpHost\":\"h\",\"smtpPort\":0,\"smtpUser\":\"u\",\"smtpTo\":\"t\"}"
            defaults.set(Data(bad.utf8), forKey: RemoteNotifyStore.configKey(.smtpEmail))
            let portFixed = RemoteNotifyStore.loadConfig(for: .smtpEmail, defaults: defaults)
            try expectEqual(portFixed.smtpPort, 465, "越界端口不该原样带进发送路径")
            try expectEqual(portFixed.smtpHost, "h", "其余字段要保住")
        }

        TestKit.test("结果记账: 最近外发结果有界可查，供设置页显示成败") {
            let transport = RecordingTransport()
            transport.result = .failed(reason: "服务端返回 429")
            var config = RemoteChannelConfig()
            config.topicOrURL = "t"
            let notifier = RemoteNotifier(transport: transport, secretReader: { _ in nil })
            _ = try awaitOnMain {
                await notifier.deliver(inputs: .init(agentName: "Dim", kind: .completed, seconds: 1),
                                   kind: .ntfy, config: config,
                                   policy: RemoteNotifyPolicy(masterEnabled: true))
            }
            try expectEqual(notifier.recentAttempts.count, 1)
            try expectEqual(notifier.recentAttempts.first?.shortText, "失败：服务端返回 429")
            try expectEqual(notifier.recentAttempts.first?.title, "Dim · 任务完成")

            for i in 0..<40 {
                _ = try awaitOnMain {
                    await notifier.deliver(inputs: .init(agentName: "A\(i)", kind: .completed, seconds: 1),
                        kind: .ntfy, config: config,
                        policy: RemoteNotifyPolicy(masterEnabled: true))
                }
            }
            try expectEqual(notifier.recentAttempts.count, 20, "长时运行下历史必须有上界")
        }

        // MARK: 设置页落盘

        TestKit.test("结构: 带 delegate 的 URLSession 不许配 completionHandler 版 dataTask") {
            // 真实回归：为了拦重定向给 session 加了 delegate，同时沿用
            // `dataTask(with:) { ... }`——实测每次都是 NSURLError -999 且请求根本没出门，
            // 而离线测试全绿（它们注入的是假传输，从不走 HTTPTransport）
            let url = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/AgentIslandCore/RemoteTransport.swift")
            let text = try XCTRequire(try? String(contentsOf: url, encoding: .utf8),
                                      "读不到 RemoteTransport.swift，这条断言就成了永真")
            try expectTrue(text.contains("URLSession(configuration:"),
                           "前提变了（不再用 delegate session），这条断言就该跟着改，而不是静默通过")
            let offenders = text.components(separatedBy: "\n").enumerated()
                .filter { $0.element.contains("dataTask(with:") && !$0.element.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
                .map { "\($0.offset + 1): \($0.element.trimmingCharacters(in: .whitespaces))" }
            try expectTrue(offenders.isEmpty,
                           "设了 delegate 就必须用 async 的 data(for:)，否则 completionHandler 永不回调\n        \(offenders)")
            try expectTrue(text.contains("session.data(for:"), "必须确实在用 async 版请求 API")
        }

        TestKit.test("落盘: 策略与每个通道各自一份配置，存了能读回且互不覆盖") {
            let defaults = TestDefaults.suite("remote")
            var policy = RemoteNotifyPolicy(masterEnabled: true, throttleSeconds: 300,
                                            quietStart: "23:00", quietEnd: "07:00")
            policy.onlyWhenAway = true
            RemoteNotifyStore.save(policy, defaults: defaults)
            try expectEqual(RemoteNotifyStore.loadPolicy(defaults: defaults), policy)

            var ntfy = RemoteChannelConfig(); ntfy.topicOrURL = "topic-a"
            var mail = RemoteChannelConfig()
            mail.smtpHost = "smtp.qq.com"; mail.smtpUser = "a@qq.com"; mail.smtpTo = "b@qq.com"
            RemoteNotifyStore.save(ntfy, for: .ntfy, defaults: defaults)
            RemoteNotifyStore.save(mail, for: .smtpEmail, defaults: defaults)
            let backNtfy = RemoteNotifyStore.loadConfig(for: .ntfy, defaults: defaults)
            let backMail = RemoteNotifyStore.loadConfig(for: .smtpEmail, defaults: defaults)
            try expectEqual(backNtfy.topicOrURL, "topic-a", "ntfy 配置读回")
            try expectEqual(backMail.smtpHost, "smtp.qq.com", "SMTP 配置读回")
            try expectEqual(backNtfy.smtpHost, "", "两个通道共用一份存储会让切回来时字段串味")
            try expectEqual(backMail.topicOrURL, "", "同上，反方向")

            RemoteNotifyStore.save(.smtpEmail, defaults: defaults)
            try expectEqual(RemoteNotifyStore.loadKind(defaults: defaults), RemoteChannelKind.smtpEmail)
            // 没写过的通道读回默认值，而不是崩溃或读到别人的
            try expectEqual(RemoteNotifyStore.loadConfig(for: .customHTTP, defaults: defaults),
                            RemoteChannelConfig())

            // 读到坏数据要退化成默认值 + 归一化，而不是把通知功能整个弄瘫
            defaults.set(Data([0x00, 0x01]), forKey: RemoteNotifyStore.policyKey)
            try expectEqual(RemoteNotifyStore.loadPolicy(defaults: defaults), RemoteNotifyPolicy(),
                            "解不开的 JSON 回落默认（总开关关）：宁可不发，也不要把配好的通道当成还在生效")
            // 故意只写老字段（没有 onlyWhenAway）：合成的 Decodable 要求 JSON 含全部键，
            // 那样每加一个字段就会让老存档整条解不开，而解不开的回落是「总开关关闭」——
            // 用户看到的是通知悄悄不再外发。缺键必须按默认值补上。
            defaults.set(Data(#"{"masterEnabled":true,"sendCompleted":true,"sendAttention":true,"sendCostSpike":true,"throttleSeconds":999999,"quietStart":"99:99","quietEnd":"07:00"}"#.utf8),
                         forKey: RemoteNotifyStore.policyKey)
            policy = RemoteNotifyStore.loadPolicy(defaults: defaults)
            try expectFalse(policy.onlyWhenAway, "缺字段要按默认值解出来，而不是整条解不开")
            try expectTrue(policy.masterEnabled, "同一条存档里已有的字段不能跟着一起丢")
            try expectEqual(policy.throttleSeconds, RemoteNotifyPolicy.normalizedThrottle.upperBound,
                            "读盘路径也必须过归一化")
            try expectTrue(policy.quietStart.isEmpty, "坏时段读盘后应作废")
            defaults.removeObject(forKey: RemoteNotifyStore.kindKey)
            try expectEqual(RemoteNotifyStore.loadKind(defaults: defaults), RemoteChannelKind.ntfy,
                            "未知/缺失的通道名要有确定回落")

            // 钥匙串条目名必须按通道分开：共用一条时，配好邮箱再配微信，
            // 发送时会拿授权码去当 SendKey——两边都失败且看不出现在串了
            let names = Set(RemoteChannelKind.allCases.map(\.defaultSecretName))
            try expectEqual(names.count, RemoteChannelKind.allCases.count, "三条通道各一个钥匙串条目名")
            try expectTrue(RemoteChannelKind.allCases.allSatisfy { $0.defaultSecretName.hasPrefix("remote.") },
                           "条目名要带前缀，便于在钥匙串里认出是谁写的")
        }

        TestKit.test("结构: 岛内转发事件到远程前必须挡住外部投递事件") {
            // 深链与 /notify 端点让本机任意进程能造出一条事件。岛内看得见「外部投递」
            // 标记，手机上看不到——所以这类事件一律不外送，否则任何本地进程都能
            // 往用户邮箱/手机里灌任意文字。这条测试是那道闸门的绊线。
            let root = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/AgentIsland")
            let files = (try? FileManager.default.contentsOfDirectory(at: root,
                                                                     includingPropertiesForKeys: nil))
                .map { $0.filter { $0.pathExtension == "swift" } } ?? []
            try expectFalse(files.isEmpty, "扫描不到 UI 源码，这条结构测试就永远是绿的")
            var callers = 0
            for url in files {
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { continue }
                let lines = text.components(separatedBy: "\n")
                let callLines = lines.enumerated().filter { $0.element.contains(".deliver(inputs:") }
                guard !callLines.isEmpty else { continue }
                callers += callLines.count
                for (idx, _) in callLines {
                    // 只要求「有一行提到 externallyDelivered」会被取反写法的假通过骗过去，
                    // 这里钉住否定形式：闸门语义就是「外部投递的一律不外送」
                    let guards = lines.enumerated()
                        .filter { $0.element.contains("!event.externallyDelivered") }
                        .map(\.offset)
                    try expectTrue(guards.contains { $0 < idx },
                                   "\(url.lastPathComponent):\(idx + 1) 外发前没有「外部投递一律不转发」的闸门")
                    // 同一条链路上的第二个闸门：`away` 有默认值，调用点漏传既不报错也不红，
                    // 表现是「永远按有人在场 → 远程通知再也不发」。钉住调用点必须显式给值。
                    let callWindow = lines[idx...min(idx + 4, lines.count - 1)].joined(separator: "\n")
                    try expectTrue(callWindow.contains("away:"),
                                   "\(url.lastPathComponent):\(idx + 1) 外发没有把「是否无人」传进策略判定")
                }
            }
            try expectTrue(callers > 0, "没有任何外发调用点，说明整条链路没接上")
        }

        TestKit.test("结构: 外部入口造事件时必须当场打上下外部标记") {
            // 上一条钉的是消费侧（外发前有没有闸门），这条钉产生侧。
            // 只查消费侧不够：`/notify` 构造事件时漏传 externallyDelivered，
            // 事件就会以「引擎自己判定的」身份通过闸门——标记必须写在造事件的那一处
            let dir = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/AgentIsland")
            let entryFiles = ["LocalEventServer.swift", "URLSchemeRouter.swift"]
            var total = 0
            for name in entryFiles {
                let text = try XCTRequire(
                    try? String(contentsOf: dir.appendingPathComponent(name), encoding: .utf8),
                    "读不到 \(name)，这条结构测试就成了永真")
                for piece in text.components(separatedBy: "AgentTaskEvent(").dropFirst() {
                    total += 1
                    // 构造点之后的一小段里必须出现「= true」形式的标记
                    let head = String(piece.prefix(600))
                    try expectTrue(head.contains("externallyDelivered: true"),
                                   "\(name) 里有一处事件构造点没打上下外部标记：\(head.prefix(120))")
                }
            }
            try expectTrue(total >= 2, "两个外部入口都该有构造点（实得 \(total)），否则扫描本身就是空的")
        }
    }
}

/// 测试用的强制解包：失败时给出可读信息（本仓无 XCTest，等价于 XCTUnwrap）
func XCTRequire<T>(_ value: T?, _ message: String = "值为 nil") throws -> T {
    guard let value else { throw TestError(message: message) }
    return value
}

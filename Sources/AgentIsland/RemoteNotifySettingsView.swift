import AgentIslandCore
import SwiftUI

// MARK: - 远程通知设置页
//
// 三个通道共用一张表：选通道 → 填非密钥字段 → 密钥单独进钥匙串 →
// 先看「发送预览」再按「发送测试」。预览与真发走同一段渲染代码，
// 差别只在密钥是掩码还是原值——所以预览看到的内容就是实际会离开本机的内容。

/// 页面状态。配置与策略改动即时落盘（与 @AppStorage 的观感一致），
/// 密钥值**从不**进入这个对象的生命周期之外：存进钥匙串后立刻清空输入框。
@MainActor
final class RemoteNotifyModel: ObservableObject {
    @Published var policy: RemoteNotifyPolicy { didSet { savePolicy() } }
    @Published var kind: RemoteChannelKind { didSet { saveKind() } }
    @Published var config: RemoteChannelConfig { didSet { saveConfig() } }

    /// 密钥输入框（一次性：写入成功后清空）
    @Published var secretInput = ""
    @Published var secretStored = false
    @Published var secretMessage: String?
    @Published var secretRejected = false
    @Published var previewText: String?
    @Published var testResult: String?
    @Published var testFailed = false
    @Published var testing = false
    /// 外发历史快照：notifier 内部记账不是 @Published，按这里的节奏刷新
    @Published private(set) var history: [OutboundAttempt] = []

    let notifier: RemoteNotifier

    init(notifier: RemoteNotifier) {
        self.notifier = notifier
        // init 里直接赋值不触发 didSet，不会在加载阶段把原值再写回去
        let k = RemoteNotifyStore.loadKind()
        self.kind = k
        self.policy = RemoteNotifyStore.loadPolicy()
        self.config = RemoteNotifyStore.loadConfig(for: k)
        self.secretStored = RemoteSecret.exists(k.defaultSecretName)
        self.history = notifier.recentAttempts
        self.presenceNow = ScreenPresence.signals
    }

    private func savePolicy() { RemoteNotifyStore.save(policy) }
    private func saveKind() {
        RemoteNotifyStore.save(kind)
        // 每个通道各自一份配置，切回来时字段还在——但界面必须跟着换，
        // 否则会用上一个通道的字段去校验新通道（NTFY 的主题名跑去给 SMTP 当主机）
        config = RemoteNotifyStore.loadConfig(for: kind)
        secretStored = RemoteSecret.exists(kind.defaultSecretName)
        secretInput = ""
        secretMessage = nil
        previewText = nil
        testResult = nil
    }
    private func saveConfig() { RemoteNotifyStore.save(config, for: kind) }

    /// 当前通道的配齐缺口；nil 表示可以试发
    var missing: String? {
        kind.missingField(config: config, hasSecret: secretStored)
    }

    /// 明文端点与「地址里直接粘了密钥」两条独立警告。与配齐检查分开，
    /// 否则「能发」和「不该这么发」会互相稀释成一句模糊的红字
    var warnings: [String] {
        [kind.insecureEndpoint(config: config),
         kind.plaintextSecretInTemplate(config: config)].compactMap { $0 }
    }

    /// 界面上填的静默时段与**实际生效**的那份不一致时（写坏的时:分整段作废），
    /// 必须说出来：让用户对着一个看起来开着、其实没开的时间窗毫无察觉，
    /// 与本仓反复修的「以为开了其实没生效」是同一类失效
    var quietHoursIgnored: Bool {
        policy.quietStart != policy.normalized().quietStart
            || policy.quietEnd != policy.normalized().quietEnd
    }

    /// 当前在场信号 + 能不能取到。开关的效果必须在页面上看得见，
    /// 否则「只在无人时发送」是一个只能靠猜有没有生效的开关
    @Published private(set) var presenceNow: PresenceSignals = .unavailable

    /// 岛内事件与测试都会往 notifier 的历史里写，但那份记账不是 @Published；
    /// 页面开着时定时取一次，否则用户只能靠改字段触发 body 重算才能看到结果
    func refresh() {
        history = notifier.recentAttempts
        presenceNow = ScreenPresence.signals
    }

    var needsSecret: Bool {
        switch kind {
        case .ntfy: return false
        case .customHTTP: return RemoteSecret.containsPlaceholder(config.urlTemplate)
        case .smtpEmail: return true
        }
    }

    func storeSecret() {
        let value = secretInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else {
            secretRejected = true
            secretMessage = "先粘贴密钥，再点存入钥匙串"
            return
        }
        switch RemoteSecret.write(value, for: kind.defaultSecretName) {
        case .ok:
            secretStored = true
            secretInput = ""
            secretRejected = false
            secretMessage = "已存入钥匙串（条目 \(kind.defaultSecretName)）"
        case .refused(let reason):
            secretStored = RemoteSecret.exists(kind.defaultSecretName)
            secretRejected = true
            secretMessage = "钥匙串拒绝写入：\(reason)"
        }
    }

    func forgetSecret() {
        RemoteSecret.delete(kind.defaultSecretName)
        secretStored = false
        secretInput = ""
        secretRejected = false
        secretMessage = "已从钥匙串删除"
    }

    /// 预览用一条示例事件渲染：正文与请求形状就是真实外发时会用的那一份，
    /// 只有密钥替换成掩码
    func renderPreview() {
        let sample = RemoteNotifier.Inputs(agentName: "Qoder", kind: .attention, seconds: 0,
                                          actionDetail: "读取 Sources/…（示例）",
                                          message: "示例消息正文（示例）")
        let p = notifier.preview(inputs: sample, kind: kind, config: config)
        previewText = "\(p.title)\n\(p.body)\n—— 请求 ——\n\(p.requestSummary)"
    }

    func runTest() {
        testing = true
        testResult = "正在送出…"
        testFailed = false
        let kind = self.kind
        let config = self.config
        let policy = self.policy
        let notifier = self.notifier
        Task { @MainActor in
            let outcome = await notifier.testDeliver(kind: kind, config: config, policy: policy)
            self.testing = false
            self.history = notifier.recentAttempts
        self.presenceNow = ScreenPresence.signals
            switch outcome {
            case .delivered:
                self.testResult = "已送达（对方服务器已接受）"
                self.testFailed = false
            case .notConfigured(let reason):
                self.testResult = "未配置：\(reason)"
                self.testFailed = true
            case .failed(let reason):
                self.testResult = "送出失败：\(reason)"
                self.testFailed = true
            case .suppressed(let reason):
                // 「发送测试」只绕过「什么时候打扰用户」那三条（节流/静默/在场）；
                // 走到这里说明总开关或该类事件的开关没开——这两条是故意不绕的
                self.testResult = "被策略挡下：\(reason)"
                self.testFailed = true
            }
        }
    }
}

// MARK: - 视图

struct RemoteNotifySettingsView: View {
    @StateObject private var model: RemoteNotifyModel

    init(notifier: RemoteNotifier) {
        _model = StateObject(wrappedValue: RemoteNotifyModel(notifier: notifier))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            switchCard
            fieldsCard
            if model.needsSecret { secretCard }
            policyCard
            verifyCard
        }
        .onAppear { model.refresh() }
        // 岛内每拍都可能往外发并写历史，而那份记账不是 ObservableObject 的 @Published：
        // 页面开着时定时拉一次，否则用户要动一下字段才看得到新记录
        .onReceive(Timer.publish(every: 5, on: .main, in: .common).autoconnect()) { _ in
            model.refresh()
        }
    }

    // MARK: 通道选择

    private var switchCard: some View {
        SettingsCard(title: "远程通知") {
            Toggle(isOn: $model.policy.masterEnabled) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("启用外发（内容会离开这台 Mac）")
                        .font(Theme.bodyFont(12, weight: .medium))
                    Text("默认只送「哪个 Agent + 什么状态」，不含命令内容、路径与消息原文。"
                         + "关掉后不产生任何外发请求。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
                }
            }
            .toggleStyle(.switch)

            Picker("通道", selection: $model.kind) {
                ForEach(RemoteChannelKind.allCases, id: \.self) { k in
                    Text(k.label).tag(k)
                }
            }
            .pickerStyle(.radioGroup)
            .horizontalRadioGroupLayout()
            .disabled(!model.policy.masterEnabled)
        }
        .opacity(model.policy.masterEnabled ? 1 : 0.55)
    }

    // MARK: 通道字段

    @ViewBuilder private var fieldsCard: some View {
        SettingsCard(title: model.kind.label) {
            switch model.kind {
            case .ntfy:
                field("主题名或自建服务器地址", text: $model.config.topicOrURL,
                      placeholder: "my-agent-topic 或 https://ntfy.mine.local/island")
                help("手机装 ntfy 后订阅同名主题即可收到；公开服务器 ntfy.sh 上主题名近似口令，"
                     + "换成自建地址更稳妥。")
            case .customHTTP:
                field("地址模板", text: $model.config.urlTemplate,
                      placeholder: "https://…/{key}.send?title={title}&desp={body}")
                field("请求体模板（必填）", text: $model.config.bodyTemplate,
                      placeholder: "title={title}&desp={body}")
                Toggle("请求体用 JSON（关闭则用表单编码）", isOn: $model.config.useJSONBody)
                    .font(Theme.bodyFont(12))
                help("微信侧（Server酱 / PushPlus 等）与企微、钉钉、飞书机器人的端点与字段各家不同，"
                     + "这里不预置：把你那家控制台给出的地址整段粘进来，密钥部分写成 {key}，"
                     + "由本 App 在发送时从钥匙串替换。标题用 {title}、正文用 {body}。")
            case .smtpEmail:
                field("SMTP 服务器", text: $model.config.smtpHost, placeholder: "smtp.qq.com")
                field("端口", text: portBinding, placeholder: "465")
                field("发信账号", text: $model.config.smtpUser, placeholder: "you@qq.com")
                field("收件地址", text: $model.config.smtpTo, placeholder: "you@qq.com")
                help("只支持 465（隐式 TLS）。25/587 的 STARTTLS 需要「在已建立的 TCP 上原地升级」，"
                     + "Network.framework 不提供，所以填了会被拦住并说明原因。"
                     + "QQ / 163 邮箱需在邮箱设置里生成「授权码」，用它而不是登录密码。")
            }
            Toggle(isOn: $model.config.includeActionDetail) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("附带最后一条动作与消息原文")
                        .font(Theme.bodyFont(12, weight: .medium))
                    Text("会把命令内容与文件路径送出这台机器，默认关闭。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.dangerRed)
                }
            }
            .toggleStyle(.switch)
            if let missing = model.missing {
                Label(missing, systemImage: "exclamationmark.triangle.fill")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.warningOrange)
            }
            ForEach(model.warnings, id: \.self) { warning in
                Label(warning, systemImage: "lock.open")
                    .font(Theme.bodyFont(11))
                    .foregroundColor(Theme.dangerRed)
            }
        }
        .opacity(model.policy.masterEnabled ? 1 : 0.55)
    }

    /// 端口用字符串编辑再转 Int：`TextField(value:)` 在非数字输入下会把 0 静默写回配置，
    /// 用户以为填了 465 实际存成 0，每次发送都连不上
    private var portBinding: Binding<String> {
        Binding(get: { String(model.config.smtpPort) }, set: { text in
            let digits = text.filter(\.isNumber)
            if let v = Int(digits), (1...65_535).contains(v) { model.config.smtpPort = v }
        })
    }

    // MARK: 密钥

    private var secretCard: some View {
        SettingsCard(title: "密钥（存钥匙串）") {
            HStack(spacing: 8) {
                SecureField("粘贴 SendKey / token / 邮箱授权码", text: $model.secretInput)
                    .textFieldStyle(.roundedBorder)
                    .font(Theme.bodyFont(12))
                Button("存入钥匙串") { model.storeSecret() }
                    .disabled(model.secretInput.isEmpty)
                Button("删除") { model.forgetSecret() }
                    .disabled(!model.secretStored)
            }
            HStack(spacing: 6) {
                Circle()
                    .fill(model.secretStored ? Theme.statusWorking : Theme.warningOrange)
                    .frame(width: 7, height: 7)
                Text(model.secretStored
                     ? "已存在钥匙串条目 \(model.kind.defaultSecretName)（界面与日志只显示掩码）"
                     : "尚未存入钥匙串条目 \(model.kind.defaultSecretName)")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.inkMuted48)
            }
            if let msg = model.secretMessage {
                Text(msg)
                    .font(Theme.bodyFont(10))
                    // 按「密钥写入被拒」上色而不是「测试失败」：钥匙串拒写是本 App
                    // ad-hoc 签名下的高频失效，显示成灰色次要文本就等于没提示
                    .foregroundColor(model.secretRejected ? Theme.dangerRed : Theme.inkMuted48)
            }
            help("密钥不进配置文件：UserDefaults 是明文 plist，会被备份与同步带走。")
        }
        .opacity(model.policy.masterEnabled ? 1 : 0.55)
    }

    // MARK: 何时发

    private var policyCard: some View {
        SettingsCard(title: "何时发送") {
            Toggle("任务完成", isOn: $model.policy.sendCompleted)
            Toggle("等待你确认", isOn: $model.policy.sendAttention)
            Toggle("消耗与熔断告警", isOn: $model.policy.sendCostSpike)
            Toggle(isOn: $model.policy.onlyWhenAway) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("只在人不在机器前时发送")
                        .font(Theme.bodyFont(12, weight: .medium))
                    Text("三条信号任一成立即算离开：屏幕锁定、显示器睡眠、"
                         + "键盘鼠标无输入超过下面的阈值。"
                         + "用远程桌面连着时前两条永远不会成立，只有无输入这条管用。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
                }
            }
            .toggleStyle(.switch)
            Stepper(value: $model.policy.awayIdleSeconds, in: 30...3600, step: 30) {
                Text("无输入满 \(model.policy.awayIdleSeconds) 秒算离开")
                    .font(Theme.bodyFont(12))
            }
            // 实时判定：让用户当场看出这条判据在这台机器上到底成不成立
            HStack(spacing: 6) {
                Circle()
                    .fill(model.presenceNow.idleSeconds == nil ? Theme.warningOrange
                                                                : Theme.statusWorking)
                    .frame(width: 7, height: 7)
                Text(presenceLine)
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.inkMuted48)
            }
            Stepper(value: $model.policy.throttleSeconds, in: 15...3600, step: 15) {
                Text("同一 Agent 同类事件 \(model.policy.throttleSeconds) 秒内只发一次")
                    .font(Theme.bodyFont(12))
            }
            HStack(spacing: 8) {
                field("静默开始（时:分，留空即不静默）", text: $model.policy.quietStart, placeholder: "22:00")
                field("静默结束", text: $model.policy.quietEnd, placeholder: "07:30")
            }
            if model.quietHoursIgnored {
                Text("这一段时间窗无效，实际按「不静默」处理（宁可多发，也不整天吞掉通知）")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.dangerRed)
            }
            help("静默时段与「岛内通知策略」互相独立：这里管的是送不送出去，"
                 + "完全静默岛内时外发照常。写坏的时段会整段作废（宁可多发也不全天吞掉）。")
        }
        .opacity(model.policy.masterEnabled ? 1 : 0.55)
    }

    /// 当前判定的一行说明。取不到输入时长要说破——那种情况下这条开关只能靠锁屏/熄屏，
    /// 而远程桌面连着时那两个永远为 false，等于开了不管用
    private var presenceLine: String {
        let p = model.presenceNow
        guard let idle = p.idleSeconds else {
            return "当前判定：取不到键盘鼠标输入时长（这台机器上这条判据没有数据，"
                + "只认锁屏与显示器睡眠）"
        }
        let away = model.policy.isAway(p)
        return "当前判定：\(away ? "已离开" : "有人在机器前")"
            + "（距上次输入 \(Int(idle)) 秒；锁屏 \(p.screenLocked ? "是" : "否")、"
            + "显示器睡眠 \(p.displayAsleep ? "是" : "否")）"
    }

    // MARK: 验证

    private var verifyCard: some View {
        SettingsCard(title: "验证") {
            HStack(spacing: 8) {
                // 按钮名点明「不送出」：这个页面里另一颗按钮是真的往外发，
                // 两个都叫「发送…」会让人按错
                Button("生成预览（不送出）") { model.renderPreview() }
                Button(model.testing ? "发送测试中…" : "发送测试（真的送出）") { model.runTest() }
                    // 总开关是这个功能的隐私闸门：关着时连测试也不外发，
                    // 否则「关掉后一个字节都不出本机」这句承诺就不成立
                    .disabled(model.testing || !model.policy.masterEnabled)
            }
            if !model.policy.masterEnabled {
                Text("总开关关闭时「发送测试」不可用：开关的作用就是不产生任何外发请求。")
                    .font(Theme.bodyFont(10))
                    .foregroundColor(Theme.inkMuted48)
            }
            if let result = model.testResult {
                Text(result)
                    .font(Theme.bodyFont(11))
                    .foregroundColor(model.testFailed ? Theme.dangerRed : Theme.statusWorking)
                if !model.testFailed {
                    Text("「已送达」只表示对方服务器接受了这条请求；多数中转服务即使内部失败"
                         + "也回 200，最终有没有到手机仍以你自己的 App/邮箱为准。")
                        .font(Theme.bodyFont(10))
                        .foregroundColor(Theme.inkMuted48)
                }
            }
            if let preview = model.previewText {
                ScrollView {
                    Text(preview)
                        .font(Theme.monoFont(10))
                        .foregroundColor(Theme.inkMuted80)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 150)
            }
            let attempts = model.history
            if !attempts.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    Text("最近外发").font(Theme.bodyFont(10, weight: .semibold))
                        .foregroundColor(Theme.inkMuted48)
                    ForEach(attempts.prefix(5), id: \.self) { attempt in
                        HStack(spacing: 8) {
                            Text(TimeFormat.hourAndMinute.string(from: attempt.at))
                                .font(Theme.monoDigitFont(10))
                                .foregroundColor(Theme.inkMuted48)
                            Text(attempt.title).font(Theme.bodyFont(11))
                                .foregroundColor(Theme.inkMuted80)
                            Text(attempt.shortText).font(Theme.bodyFont(11))
                                .foregroundColor(attempt.outcome.isDelivered
                                                 ? Theme.statusWorking : Theme.warningOrange)
                            Spacer(minLength: 0)
                        }
                    }
                }
            }
            help("「发送测试」会真的送出一条消息到你在上方配置的地址，并绕过节流与静默时段。"
                 + "预览里的密钥是掩码，实发时才会替换成原值。\n"
                 + "自动外发失败后隔 5 秒再试一次（一次网络抖动不该丢掉唯一能叫醒你的提醒），"
                 + "「最近外发」会写明是重试后送达还是重试仍未送达。"
                 + "「发送测试」不重试：它给的就是这一次的真实结果。")
        }
        .opacity(model.policy.masterEnabled ? 1 : 0.55)
    }

    // MARK: 小部件

    private func field(_ title: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(Theme.bodyFont(11, weight: .semibold))
                .foregroundColor(Theme.inkMuted80)
            TextField(placeholder, text: text)
                .textFieldStyle(.roundedBorder)
                .font(Theme.bodyFont(12))
        }
    }

    private func help(_ text: String) -> some View {
        Text(text)
            .font(Theme.bodyFont(10))
            .foregroundColor(Theme.inkMuted48)
            .fixedSize(horizontal: false, vertical: true)
    }
}

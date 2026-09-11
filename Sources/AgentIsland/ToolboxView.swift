import AgentIslandCore
import SwiftUI

// MARK: - 智能体维护工作台快捷视图 (ToolboxView · v1.7.7)

struct ToolboxView: View {
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController

    @State private var anomalies: [AgentAnomaly] = []
    @State private var isScanning = false
    @State private var confirmingCleanAll = false
    @State private var cleaningPid: Int32? = nil
    /// 单条清理确认态的自动复位任务（可取消）
    @State private var singleConfirmTask: Task<Void, Never>?
    /// 「一键清理」确认态的自动复位任务（可取消）：此前进入确认态后永不复位
    @State private var cleanAllConfirmTask: Task<Void, Never>?
    /// 清理失败提示（terminate 无权限/进程已消失时不能说「已清理」）
    @State private var cleanFeedback: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(
                title: "智能体维护工作台",
                subtitle: "孤儿/死锁/内存泄漏扫描",
                onBack: { controller.route = .list },
                controller: controller
            )

            DarkDivider()

            GeometryReader { geo in
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(alignment: .leading, spacing: 10) {
                        // 顶部状态指标卡
                        metricsCard

                        if let cleanFeedback {
                            failureBanner(cleanFeedback)
                        }

                        if isScanning {
                            CenteredSpinner()
                                .frame(maxWidth: .infinity, minHeight: 120)
                        } else if anomalies.isEmpty {
                            healthyStateView
                        } else {
                            anomaliesListView
                        }
                    }
                    .padding(.horizontal, Theme.pageMargin)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, minHeight: max(geo.size.height - 20, 0))
                }
            }

            // 底部一键清理操作栏
            if !anomalies.isEmpty {
                DarkDivider()
                bottomActionBar
            }
        }
        .cardShell(dockEdge: controller.dockEdge, controller: controller)
        .onAppear {
            runScan()
        }
    }

    // MARK: 顶部指标卡
    private var metricsCard: some View {
        HStack(spacing: 0) {
            overviewCell("异常项", "\(anomalies.count)",
                         color: anomalies.isEmpty ? Theme.statusWorking : Theme.dangerRed,
                         help: "本次扫描发现的孤儿进程 / 假死死锁 / 内存超限条目数")
            Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
            let totalMem = anomalies.reduce(UInt64(0)) { $0 + $1.memoryBytes }
            let mb = Double(totalMem) / (1024 * 1024)
            let memStr = mb >= 1024 ? String(format: "%.1fG", mb / 1024.0) : "\(Int(mb))M"
            overviewCell("可回收", totalMem > 0 ? memStr : "0M",
                         color: totalMem > 0 ? Theme.warningOrange : Theme.onDark,
                         help: "异常进程当前占用的物理内存合计")
            Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
            // 第三格改为真实口径：此前是「健康度 = max(10, 100 − 异常数×20)%」，
            // 系数纯属编造（4 个异常就报 20%，无任何依据）。改为展示监控规模，
            // 让「异常项」有个可解释的分母。
            overviewCell("监控项", "\(engine.snapshots.count)",
                         color: Theme.onDark,
                         help: "当前纳入监控的智能体数量（异常项占比的实际分母）")
        }
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous).fill(Theme.cardFill))
    }

    private func overviewCell(_ label: String, _ value: String, color: Color, help: String) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(Theme.monoFont(14, weight: .bold))
                .foregroundColor(color)
            Text(label)
                .font(Theme.bodyFont(9))
                .foregroundColor(Theme.onDarkFaint)
        }
        .frame(maxWidth: .infinity)
        .help(help)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label) \(value)")
    }

    /// 清理失败提示条：清理动作不再「无条件成功」，失败要看得见
    private func failureBanner(_ text: String) -> some View {
        HStack(spacing: 4) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 9))
            Text(text)
                .font(Theme.bodyFont(9, weight: .medium))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .foregroundColor(Theme.dangerRed)
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(RoundedRectangle(cornerRadius: Theme.radiusSm, style: .continuous)
            .fill(Theme.dangerRed.opacity(0.12)))
        .accessibilityElement(children: .combine)
    }

    // MARK: 全健康状态
    private var healthyStateView: some View {
        VStack(spacing: 8) {
            Spacer(minLength: 16)
            Image(systemName: "checkmark.shield.fill")
                .font(.system(size: 28))
                .foregroundColor(Theme.statusWorking)
            Text("所有智能体运行健康")
                .font(Theme.bodyFont(12, weight: .semibold))
                .foregroundColor(Theme.onDark)
            Text("未发现孤儿进程、假死死锁或内存泄露")
                .font(Theme.bodyFont(10))
                .foregroundColor(Theme.onDarkFaint)
            Button {
                runScan()
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 9, weight: .semibold))
                    Text("重新扫描")
                        .font(Theme.bodyFont(10))
                }
                .foregroundColor(Theme.onDark)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Capsule().fill(Theme.chipFill))
            }
            .buttonStyle(.plain)
            .padding(.top, 4)
            Spacer(minLength: 16)
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: 异常列表
    private var anomaliesListView: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("待维护进程 (\(anomalies.count))")
                    .font(Theme.bodyFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDarkFaint)
                Spacer()
                Button {
                    runScan()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 9))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .buttonStyle(.plain)
                .help("重新扫描")
                .accessibilityLabel("重新扫描异常进程")
            }

            ForEach(anomalies) { item in
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 4) {
                            Text(item.agentName)
                                .font(Theme.bodyFont(11, weight: .bold))
                                .foregroundColor(Theme.onDark)
                                // 名称过长会把 PID/类型徽标挤出可见区
                                .lineLimit(1)
                                .truncationMode(.tail)
                                .help("\(item.agentName) · \(item.commandPath)")
                            Text("PID: \(item.pid)")
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDarkFaint)
                            Spacer()
                            anomalyTag(item.anomalyType)
                        }

                        Text(item.reason)
                            .font(Theme.bodyFont(9))
                            .foregroundColor(Theme.onDarkMuted)
                            .lineLimit(2)

                        HStack(spacing: 6) {
                            if item.cpuPercent > 0.1 {
                                Text(String(format: "CPU %.1f%%", item.cpuPercent))
                                    .font(Theme.monoFont(8))
                                    .foregroundColor(Theme.warningOrange)
                            }
                            if item.memoryBytes > 0 {
                                Text("内存 \(item.memoryText)")
                                    .font(Theme.monoFont(8))
                                    .foregroundColor(Theme.onDarkFaint)
                            }
                            Text("PPID: \(item.ppid)")
                                .font(Theme.monoFont(8))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }

                    // 单条清理同样需要二次确认：杀的是整棵进程树，误触代价是用户编辑器退出
                    if cleaningPid == item.pid {
                        Button {
                            cleanSingle(item)
                            cleaningPid = nil
                        } label: {
                            Text("确认?")
                                .font(Theme.bodyFont(9, weight: .bold))
                                .foregroundColor(.white)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 3)
                                .background(Capsule().fill(Theme.dangerRed))
                        }
                        .buttonStyle(.plain)
                        .help("再次点击终止 PID \(item.pid) 及其子进程树")
                        .accessibilityLabel("确认清理 \(item.agentName) PID \(item.pid)")
                        .onAppear {
                            singleConfirmTask?.cancel()
                            singleConfirmTask = Task { @MainActor in
                                try? await Task.sleep(nanoseconds: 3_000_000_000)
                                guard !Task.isCancelled else { return }
                                cleaningPid = nil
                            }
                        }
                    } else {
                        Button {
                            cleaningPid = item.pid
                        } label: {
                            Image(systemName: "trash.circle")
                                .font(.system(size: 15))
                                .foregroundColor(Theme.dangerRed.opacity(0.85))
                        }
                        .buttonStyle(.plain)
                        .help("安全终止该异常进程（需二次确认）")
                        .accessibilityLabel("清理 \(item.agentName) PID \(item.pid)")
                    }
                }
                .padding(8)
                .background(RoundedRectangle(cornerRadius: Theme.radiusSm).fill(Theme.cardFill))
            }
        }
    }

    private func anomalyTag(_ type: AgentAnomaly.AnomalyType) -> some View {
        let text: String
        let color: Color
        switch type {
        case .orphan:
            text = "孤儿进程"
            color = Theme.warningOrange
        case .hung:
            text = "疑似死锁"
            color = Theme.dangerRed
        case .overweight:
            text = "内存超限"
            color = Theme.statusIdle
        }
        return Text(text)
            .font(Theme.bodyFont(8, weight: .bold))
            .foregroundColor(color)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(Capsule().fill(color.opacity(0.18)))
    }

    // MARK: 底部操作栏
    private var bottomActionBar: some View {
        HStack {
            Text("建议一键释放失联与僵死资源")
                .font(Theme.bodyFont(10))
                .foregroundColor(Theme.onDarkFaint)

            Spacer()

            if confirmingCleanAll {
                Button {
                    cleanAll()
                    confirmingCleanAll = false
                } label: {
                    Text("确认清理?")
                        .font(Theme.bodyFont(10, weight: .bold))
                        .foregroundColor(.white)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                        .background(Capsule().fill(Theme.dangerRed))
                }
                .buttonStyle(.plain)
                .help("再次点击确认终止全部 \(anomalies.count) 个异常进程（不可撤销）")
                .accessibilityLabel("确认清理全部异常进程")
                .onAppear {
                    // 与单条清理一致：3 秒内不确认就自动复位，
                    // 否则误触进入确认态后会一直停在「确认清理?」，下一次误触即真清理
                    cleanAllConfirmTask?.cancel()
                    cleanAllConfirmTask = Task { @MainActor in
                        try? await Task.sleep(nanoseconds: 3_000_000_000)
                        guard !Task.isCancelled else { return }
                        confirmingCleanAll = false
                    }
                }
            } else {
                Button {
                    confirmingCleanAll = true
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "sparkles")
                            .font(.system(size: 9))
                        Text("一键安全清理")
                            .font(Theme.bodyFont(10, weight: .semibold))
                    }
                    .foregroundColor(.white)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
                    .background(Capsule().fill(Theme.actionBlue))
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        .padding(.vertical, 8)
    }

    // MARK: 操作

    private func runScan() {
        cleanFeedback = nil
        scan(showSpinner: true)
    }

    /// 扫描异常进程。
    /// - Parameter showSpinner: 清理后的复核扫描传 false —— 复核期间必须继续显示原列表，
    ///   否则用户看不到「条目是否真的消失」，等于用 loading 掩盖清理结果。
    private func scan(showSpinner: Bool, completion: (([AgentAnomaly]) -> Void)? = nil) {
        if showSpinner { isScanning = true }
        let hungIDs = Set(engine.snapshots.filter { $0.isHung }.map { $0.profile.id })
        let profiles = engine.allProfiles
        let cleaner = engine.cleaner
        // NSWorkspace 必须主线程访问（ProcessProviding 契约）：先在主线程抓 bundle 集合，
        // 再进后台做快照与匹配。此前整段丢到后台队列，违反线程契约。
        let bundleIDs = ProcessProvider().runningBundleIDs()
        DispatchQueue.global(qos: .userInitiated).async {
            let found = cleaner?.scanAnomalies(profiles: profiles, hungAgentIDs: hungIDs,
                                               runningBundleIDs: bundleIDs) ?? []
            DispatchQueue.main.async {
                self.anomalies = found
                self.isScanning = false
                completion?(found)
            }
        }
    }

    /// 单条清理。此前无条件 `anomalies.removeAll { $0.id == item.id }`：
    /// `ProcessTerminator` 在无权限或进程已消失时会失败，条目却照样消失，
    /// 用户看到「列表空了」就以为清理成功（引擎侧还会配一条「已安全清理」事件）。
    /// 现在改为「发出终止请求 → 等待信号生效 → 用真实重扫结果刷新列表」，
    /// 仍在运行的条目会原样回到列表并给出失败提示。
    private func cleanSingle(_ item: AgentAnomaly) {
        cleanFeedback = nil
        engine.cleanAnomalies([item])
        verifyCleanup { remaining in
            guard let stillAlive = remaining.first(where: { $0.pid == item.pid }) else { return }
            self.cleanFeedback = "未能终止 \(stillAlive.agentName)（PID \(item.pid)）："
                + "进程仍在运行，可能需要更高权限，可从活动监视器处理"
            self.anomalies = remaining
        }
    }

    private func cleanAll() {
        cleanFeedback = nil
        let toClean = anomalies
        engine.cleanAnomalies(toClean)
        // 与单条清理同一口径：不再直接清空列表并返回（那等于替引擎宣告成功）。
        // 复核后确认全部消失才回主卡；有残留就留在工作台并提示。
        verifyCleanup { remaining in
            if remaining.isEmpty {
                self.controller.route = .list
            } else {
                self.anomalies = remaining
                self.cleanFeedback = "有 \(remaining.count) 个进程未能终止，已保留在列表中"
            }
        }
    }

    /// 清理结果复核：terminate 先发 SIGTERM、300ms 后补发 SIGKILL（且 GUI App 走
    /// terminate 通知），因此要留出缓冲再取快照，避免把「正在退出」误判成失败。
    private func verifyCleanup(_ handle: @escaping ([AgentAnomaly]) -> Void) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
            self.scan(showSpinner: false) { found in
                handle(found)
            }
        }
    }
}

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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            DetailHeader(
                title: "智能体维护工作台",
                subtitle: "孤儿/死锁/内存泄漏扫描",
                onBack: { controller.route = .list },
                onMoved: { controller.dragMoved(translation: $0) },
                onEnded: { controller.dragEnded() }
            )

            DarkDivider()

            GeometryReader { geo in
                ScrollView(.vertical, showsIndicators: true) {
                    VStack(alignment: .leading, spacing: 10) {
                        // 顶部状态指标卡
                        metricsCard

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
            overviewCell("异常项", "\(anomalies.count)", color: anomalies.isEmpty ? Theme.statusWorking : Theme.dangerRed)
            Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
            let totalMem = anomalies.reduce(UInt64(0)) { $0 + $1.memoryBytes }
            let mb = Double(totalMem) / (1024 * 1024)
            let memStr = mb >= 1024 ? String(format: "%.1fG", mb / 1024.0) : "\(Int(mb))M"
            overviewCell("可回收", totalMem > 0 ? memStr : "0M", color: totalMem > 0 ? Theme.warningOrange : Theme.onDark)
            Rectangle().fill(Theme.onDark.opacity(0.10)).frame(width: 1, height: 26)
            overviewCell("健康度", anomalies.isEmpty ? "100%" : "\(max(10, 100 - anomalies.count * 20))%", color: anomalies.isEmpty ? Theme.statusWorking : Theme.warningOrange)
        }
        .padding(.vertical, 8)
        .background(RoundedRectangle(cornerRadius: Theme.radiusMd, style: .continuous).fill(Theme.cardFill))
    }

    private func overviewCell(_ label: String, _ value: String, color: Color) -> some View {
        VStack(spacing: 2) {
            Text(value)
                .font(Theme.monoFont(14, weight: .bold))
                .foregroundColor(color)
            Text(label)
                .font(Theme.bodyFont(9))
                .foregroundColor(Theme.onDarkFaint)
        }
        .frame(maxWidth: .infinity)
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
            }

            ForEach(anomalies) { item in
                HStack(spacing: 8) {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 4) {
                            Text(item.agentName)
                                .font(Theme.bodyFont(11, weight: .bold))
                                .foregroundColor(Theme.onDark)
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
        isScanning = true
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
            }
        }
    }

    private func cleanSingle(_ item: AgentAnomaly) {
        engine.cleanAnomalies([item])
        anomalies.removeAll { $0.id == item.id }
    }

    private func cleanAll() {
        let toClean = anomalies
        engine.cleanAnomalies(toClean)
        anomalies.removeAll()
        controller.route = .list
    }
}

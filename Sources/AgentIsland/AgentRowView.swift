import AgentIslandCore
import SwiftUI

// MARK: - Agent 行（主列表行视图 + 行内快捷操作）

// MARK: - Agent 行

struct AgentRowView: View {
    let snapshot: AgentSnapshot
    @ObservedObject var engine: ActivityEngine
    @ObservedObject var controller: IslandPanelController
    @State private var confirmingKill = false
    @State private var showingTooltip = false
    /// 直达失败反馈：activate 返回 false 时短暂切换图标（ssh/tmux 启动的 CLI 无可激活窗口）
    @State private var activateFailed = false
    /// 关闭 tooltip 的延迟任务（可取消）：鼠标从环移向 popover 的途中会先触发
    /// onHover(false)，若立即关闭则 popover 里的按钮永远点不到。
    @State private var tooltipCloseTask: Task<Void, Never>?
    /// 终止确认态的自动复位任务（可取消）
    @State private var confirmResetTask: Task<Void, Never>?

    /// Token 徽标文本："1.23M" 或 "1.23M $0.42"
    static func tokenBadge(_ usage: TokenUsage) -> String {
        let tokens = TokenUsage.compact(usage.tokens24h)
        let cost = TokenUsage.cost(usage.cost24h)
        return cost.isEmpty ? tokens : "\(tokens) \(cost)"
    }

    /// 是否显示实时动作横条（与内边距共用同一判定，避免 2pt 行高漂移）
    private var hasActionBar: Bool {
        snapshot.level == .working && !(snapshot.currentAction ?? "").isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                AgentRingView(snapshot: snapshot, size: 26)
                    .onHover { h in
                        // 延迟关闭：给用户时间把鼠标从 26×26 的环移到 popover 上，
                        // 否则 popover 里的「终止 / 直达窗口」永远来不及点。
                        tooltipCloseTask?.cancel()
                        if h {
                            showingTooltip = true
                        } else {
                            tooltipCloseTask = Task { @MainActor in
                                try? await Task.sleep(nanoseconds: 400_000_000)
                                guard !Task.isCancelled else { return }
                                showingTooltip = false
                            }
                        }
                    }
                    .popover(isPresented: $showingTooltip, arrowEdge: controller.dockEdge == .top ? .bottom : .leading) {
                        AgentHoverTooltipCard(snapshot: snapshot, engine: engine, controller: controller)
                            // 鼠标进入 popover 时取消关闭任务，让按钮可点
                            .onHover { inside in
                                if inside { tooltipCloseTask?.cancel() }
                            }
                    }

                VStack(alignment: .leading, spacing: 1) {
                    Text(snapshot.profile.name)
                        .font(Theme.bodyFont(12.5, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                        .lineLimit(1)
                        .help(snapshot.profile.name)

                    if snapshot.level != .working || snapshot.currentAction == nil {
                        if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                            Text(Self.tokenBadge(usage))
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDark.opacity(0.75))
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Capsule().fill(Theme.chipFill))
                        } else {
                            Text(snapshot.lastActivityText)
                                .font(Theme.bodyFont(9.5))
                                .foregroundColor(Theme.onDarkFaint)
                        }
                    }
                }

                Spacer(minLength: 4)

                // 内存占用与健康状态指示（v1.7.6）
                if snapshot.processRunning && snapshot.memoryBytes > 0 {
                    Text(snapshot.memoryText)
                        .font(Theme.monoFont(9))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(RoundedRectangle(cornerRadius: 4).fill(Theme.chipFill))
                        .help("物理内存驻留集 (RSS): \(snapshot.memoryText)")
                }

                if snapshot.isHung {
                    HStack(spacing: 2) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: 8))
                        Text("疑似卡死")
                            .font(Theme.bodyFont(9, weight: .bold))
                    }
                    .foregroundColor(Theme.dangerRed)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2.5)
                    .background(Capsule().fill(Theme.dangerRed.opacity(0.18)))
                    .help("检测到进程持续异常高负荷且缺乏会话响应，疑似处于死循环或线程死锁状态")
                } else {
                    Text(snapshot.level.label)
                        .font(Theme.bodyFont(10, weight: .semibold))
                        .foregroundColor(snapshot.level.color)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2.5)
                        .background(Capsule().fill(snapshot.level.color.opacity(0.16)))
                }

                if snapshot.processRunning {
                    HStack(spacing: 3) {
                        // 确认态仅在仍处于工作时有效：若 3 秒内 Agent 已转 idle，
                        // 继续显示红色「终止?」会诱导用户终止一个已空闲的进程
                        if confirmingKill && snapshot.level == .working {
                            Button {
                                engine.terminateAgent(pid: snapshot.pid, agentId: snapshot.profile.id)
                                confirmingKill = false
                            } label: {
                                Text("终止?")
                                    .font(Theme.bodyFont(9, weight: .bold))
                                    .foregroundColor(.white)
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 2)
                                    .background(Capsule().fill(Theme.dangerRed.opacity(0.9)))
                            }
                            .buttonStyle(.plain)
                            .help("再次点击立即强制终止该 Agent 进程")
                            .accessibilityLabel("确认终止 \(snapshot.profile.name)")
                            .onAppear {
                                // Task 可随视图销毁取消；DispatchQueue 版本会在行消失后
                                // 继续向失效的 @State 写值
                                confirmResetTask?.cancel()
                                confirmResetTask = Task { @MainActor in
                                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                                    guard !Task.isCancelled else { return }
                                    confirmingKill = false
                                }
                            }
                        } else if snapshot.level == .working {
                            Button {
                                confirmingKill = true
                            } label: {
                                Image(systemName: "xmark.circle")
                                    .font(.system(size: 11, weight: .medium))
                                    .foregroundColor(Theme.dangerRed.opacity(0.85))
                                    .padding(4)
                                    .background(Circle().fill(Theme.dangerRed.opacity(0.15)))
                            }
                            .buttonStyle(.plain)
                            .help("一键终止逃生舱：关闭该正在运行的 Agent 及其子任务")
                            .accessibilityLabel("终止 \(snapshot.profile.name)")
                        }

                        Button {
                            controller.openLiveStream(agentId: snapshot.profile.id)
                        } label: {
                            Image(systemName: "terminal")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(Theme.onDark.opacity(0.75))
                                .padding(4)
                                .background(Circle().fill(Theme.chipFill))
                        }
                        .buttonStyle(.plain)
                        .help("查看 \(snapshot.profile.name) 实时事件与输出流水")
                        .accessibilityLabel("查看 \(snapshot.profile.name) 实时流水")

                        Button {
                            if !AppActivator.activate(pid: snapshot.pid, bundleIDs: snapshot.profile.bundleIDs) {
                                activateFailed = true
                                Task { @MainActor in
                                    try? await Task.sleep(nanoseconds: 1_500_000_000)
                                    activateFailed = false
                                }
                            }
                        } label: {
                            Image(systemName: activateFailed ? "exclamationmark.triangle" : "arrow.up.forward.app")
                                .font(.system(size: 10, weight: .medium))
                                .foregroundColor(activateFailed ? Theme.warningOrange : Theme.onDark.opacity(0.75))
                                .padding(4)
                                .background(Circle().fill(Theme.chipFill))
                        }
                        .buttonStyle(.plain)
                        .help(activateFailed ? "未找到可激活的窗口（CLI 经 ssh/tmux 启动时无窗口可带）" : "置顶并激活该智能体窗口/终端")
                        .accessibilityLabel("直达 \(snapshot.profile.name) 窗口")
                    }
                }
            }

            // 第二行：工作状态下的专属实时动作横条（全宽展示，彻底根治截断问题）
            if hasActionBar, let action = snapshot.currentAction {
                HStack(spacing: 5) {
                    Image(systemName: "terminal.fill")
                        .font(.system(size: 8))
                        .foregroundColor(Theme.statusWorking)
                    Text(action)
                        .font(Theme.monoFont(9.5))
                        .foregroundColor(Theme.statusWorking)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 0)
                    // 工作态也保留 token 徽标：正在消耗的 Agent 恰是最需要关注的，
                    // 此前它只在非工作态显示，工作中反而看不到用量
                    if let usage = snapshot.tokenUsage, usage.tokens24h > 0 {
                        Text(Self.tokenBadge(usage))
                            .font(Theme.monoFont(8))
                            .foregroundColor(Theme.statusWorking.opacity(0.85))
                            .lineLimit(1)
                            .help("24h \(TokenUsage.compact(usage.tokens24h)) token")
                    }
                }
                .padding(.horizontal, 7)
                .padding(.vertical, 3)
                .background(
                    RoundedRectangle(cornerRadius: 4, style: .continuous)
                        .fill(Theme.statusWorking.opacity(0.10))
                )
                .padding(.leading, 34) // 与 Agent 名称对齐
                .help(action)
            }
        }
        .padding(.horizontal, Theme.pageMargin)
        // 与横条显示条件严格一致：空字符串动作不显示横条，也不应多出 2pt 内边距
        .padding(.vertical, hasActionBar ? 5 : 4)
        .hoverRowBackground(cornerRadius: Theme.radiusSm, idleFill: .clear)
        .onTapGesture {
            // 点行进 agent 详情页（原 Finder 跳转移入详情页会话列表）
            controller.route = .agentDetail(snapshot.profile.id)
        }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(snapshot.profile.name)，\(snapshot.level.label)，点按查看详情")
        .accessibilityHint(snapshot.processRunning ? "行内含终止 / 流水 / 直达三个快捷按钮" : "点按查看详情")
    }
}

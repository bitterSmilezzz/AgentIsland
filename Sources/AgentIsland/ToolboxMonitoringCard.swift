import AgentIslandCore
import SwiftUI

// MARK: - 监控可信度自查卡 (v0.0.88)
//
// 把 CLI `agentisland doctor` 的同一套结论搬进岛内。刻意只做两件事：读引擎已经在算的
// 快照、调用同一个 `AgentObservability`——不重新采样，也不在这里再写一份判定。
// 两处口径一旦各写各的，「终端说这个 Agent 的待机不可信、岛上却显示一切正常」
// 这种分裂迟早会回来，而那张卡片的可信度正是这个功能存在的全部理由。
extension ToolboxView {

    /// 结论不可信的 Agent（未安装的不算问题：本来就不该期待它有状态）
    var flaggedForMonitoring: [(snapshot: AgentSnapshot, verdict: AgentObservability.Verdict)] {
        engine.snapshots.compactMap { snap in
            let verdict = AgentObservability.evaluate(snapshot: snap)
            guard !verdict.isTrustworthy, verdict.code != .notInstalled else { return nil }
            return (snap, verdict)
        }
    }

    var monitoringHealthCard: some View {
        let flagged = flaggedForMonitoring
        return VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 8) {
                Image(systemName: "questionmark.circle.magnifyingglass")
                    .font(.system(size: 11))
                    .foregroundColor(flagged.isEmpty ? Theme.sydedockEmerald : Theme.warningOrange)
                Text("监控可信度自查")
                    .font(Theme.bodyFont(10, weight: .semibold))
                    .foregroundColor(Theme.onDark)
                Spacer()
                Text(flagged.isEmpty ? "全部可信" : "\(flagged.count) 项存疑")
                    .font(Theme.bodyFont(9.5, weight: .medium))
                    .foregroundColor(flagged.isEmpty ? Theme.sydedockEmerald : Theme.warningOrange)
            }

            if flagged.isEmpty {
                // 空态必须说清「查过了，没问题」，而不是留一片空白让人以为没数据
                Text("已核对全部在跑与已装智能体：每条状态结论都有可读的会话或进程证据支撑。")
                    .font(Theme.bodyFont(9.5))
                    .foregroundColor(Theme.onDarkMuted)
            } else {
                ForEach(flagged, id: \.snapshot.id) { item in
                    Button {
                        controller.openAgentDetail(item.snapshot.profile.id, from: .toolbox)
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text("\(item.snapshot.profile.emoji) \(item.snapshot.profile.name)")
                                    .font(Theme.bodyFont(10, weight: .medium))
                                    .foregroundColor(Theme.onDark)
                                Text(item.verdict.summary)
                                    .font(Theme.bodyFont(9.5, weight: .semibold))
                                    .foregroundColor(item.verdict.code == .blindSessionSource
                                                     ? Theme.dangerRed : Theme.warningOrange)
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 8))
                                    .foregroundColor(Theme.onDarkMuted)
                            }
                            Text(item.verdict.evidence.first ?? "")
                                .font(Theme.bodyFont(9))
                                .foregroundColor(Theme.onDarkMuted)
                                .readableSingleLine(fullText: item.verdict.evidence.first ?? "")
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("查看 \(item.snapshot.profile.name) 的详情")
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .subtleCardStyle()
    }
}

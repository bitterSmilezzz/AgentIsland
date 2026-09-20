import AgentIslandCore
import SwiftUI

// MARK: - 派生子进程与工具调用树视图 (v0.0.74)

struct ProcessTreeView: View {
    let report: ProcessTreeReport
    @State private var isExpanded = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            // 折叠/展开标题栏
            Button {
                withAnimation(.easeInOut(duration: 0.2)) {
                    isExpanded.toggle()
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "square.stack.3d.down.right.fill")
                        .font(.system(size: 10))
                        .foregroundColor(Theme.sydedockCyan)
                    Text("派生子进程与工具树 (\(report.subprocessCount))")
                        .font(Theme.bodyFont(10, weight: .semibold))
                        .foregroundColor(Theme.onDark)
                    Spacer()
                    if report.subprocessCount > 0 {
                        HStack(spacing: 4) {
                            Text(report.totalSubprocessCpuText)
                                .font(Theme.monoFont(9, weight: .semibold))
                                .foregroundColor(report.totalSubprocessCpu > 20 ? Theme.sydedockAmber : Theme.onDarkMuted)
                            Text("·")
                                .foregroundColor(Theme.onDarkFaint)
                            Text(report.totalSubprocessMemoryText)
                                .font(Theme.monoFont(9))
                                .foregroundColor(Theme.onDarkMuted)
                        }
                    }
                    Image(systemName: isExpanded ? "chevron.up" : "chevron.down")
                        .font(Theme.badgeFont(.semibold))
                        .foregroundColor(Theme.onDarkFaint)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(
                    RoundedRectangle(cornerRadius: Theme.radiusSm)
                        .fill(Theme.chipFill)
                        .overlay(
                            RoundedRectangle(cornerRadius: Theme.radiusSm)
                                .strokeBorder(colorScheme == .light ? Ramp.slate200 : Color.clear, lineWidth: 0.5)
                        )
                )
            }
            .buttonStyle(.plain)

            if isExpanded {
                if report.nodes.isEmpty {
                    Text("当前未探查到活跃的子进程或工具命令")
                        .font(Theme.bodyFont(9.5))
                        .foregroundColor(Theme.onDarkFaint)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 4)
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(flattenedNodes) { item in
                            nodeRow(item.node, level: item.level)
                        }
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 6)
                    .background(
                        RoundedRectangle(cornerRadius: Theme.radiusSm)
                            .fill(Theme.obsidianCardFill)
                    )
                    // 层级用缩进竖条、工具类型用图标表达——两者 VoiceOver 都读不出来，
                    // 这里把「谁挂在谁下面、谁在吃资源」折成一句播报（与趋势图同一标准）
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(treeAccessibilitySummary)
                }
            }
        }
    }

    /// 进程树口语化摘要：节点数、层深、合计资源与占用居前的节点
    private var treeAccessibilitySummary: String {
        let nodes = flattenedNodes
        guard !nodes.isEmpty else { return "当前未探查到活跃的子进程或工具命令" }
        let deepest = (nodes.map(\.level).max() ?? 0) + 1
        let top = nodes.sorted { $0.node.cpuPercent > $1.node.cpuPercent }
            .prefix(3)
            .map { item -> String in
                var text = "\(item.node.name) PID \(item.node.pid)"
                if item.node.cpuPercent > 0.1 { text += " CPU \(item.node.cpuText)" }
                if item.node.memoryBytes > 0 { text += " 内存 \(item.node.memoryText)" }
                return text + "（第 \(item.level + 1) 层）"
            }
            .joined(separator: "；")
        return "派生子进程与工具树共 \(nodes.count) 个节点、最深 \(deepest) 层，合计 CPU \(report.totalSubprocessCpuText)、内存 \(report.totalSubprocessMemoryText)。占用居前：\(top)"
    }

    private struct FlattenedProcessNode: Identifiable {
        var id: Int32 { node.pid }
        let node: ProcessTreeNode
        let level: Int
    }

    private var flattenedNodes: [FlattenedProcessNode] {
        var list: [FlattenedProcessNode] = []
        func appendRecursively(_ node: ProcessTreeNode, level: Int) {
            list.append(FlattenedProcessNode(node: node, level: level))
            for child in node.children {
                appendRecursively(child, level: level + 1)
            }
        }
        for root in report.nodes {
            appendRecursively(root, level: 0)
        }
        return list
    }

    @ViewBuilder
    private func nodeRow(_ node: ProcessTreeNode, level: Int) -> some View {
        HStack(spacing: 6) {
            if level > 0 {
                HStack(spacing: 2) {
                    ForEach(0..<level, id: \.self) { _ in
                        Rectangle()
                            .fill(Theme.onDark.opacity(0.15))
                            .frame(width: 1, height: 12)
                            .padding(.horizontal, 3)
                    }
                }
            }

            Image(systemName: toolIcon(for: node.name))
                .font(.system(size: 9))
                .foregroundColor(Theme.sydedockCyan.opacity(0.85))

            Text(node.name)
                .font(Theme.monoFont(9.5, weight: .medium))
                .foregroundColor(Theme.onDark)
                .lineLimit(1)

            Text("[PID: \(node.pid)]")
                .font(Theme.monoFont(8.5))
                .foregroundColor(Theme.onDarkFaint)

            Spacer()

            if node.cpuPercent > 0.1 {
                Text(node.cpuText)
                    .font(Theme.monoDigitFont(8.5, weight: .semibold))
                    .foregroundColor(node.cpuPercent >= 20 ? Theme.dangerRed : Theme.onDarkMuted)
            }

            if node.memoryBytes > 0 {
                Text(node.memoryText)
                    .font(Theme.monoDigitFont(8.5))
                    .foregroundColor(Theme.onDarkFaint)
            }
        }
        .help(node.path)
    }

    private func toolIcon(for name: String) -> String {
        let n = name.lowercased()
        if n.contains("node") || n.contains("python") || n.contains("ruby") {
            return "chevron.left.forwardslash.chevron.right"
        }
        if n.contains("git") {
            return "arrow.triangle.branch"
        }
        if n.contains("zsh") || n.contains("bash") || n.contains("sh") {
            return "terminal"
        }
        if n.contains("rg") || n.contains("grep") || n.contains("find") {
            return "magnifyingglass"
        }
        return "gearshape"
    }
}

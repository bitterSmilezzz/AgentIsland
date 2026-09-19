import Foundation

// MARK: - 智能体审计报告导出器 (v0.0.74)

public enum AuditReportExporter {

    private static let dateFormatter: DateFormatter = {
        let df = DateFormatter()
        df.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return df
    }()

    private static let fileDateFormatter: DateFormatter = {
        let df = DateFormatter()
        df.dateFormat = "yyyyMMdd_HHmmss"
        return df
    }()

    public static func defaultFilename(extension ext: String) -> String {
        "AgentIsland_Audit_\(fileDateFormatter.string(from: Date())).\(ext)"
    }

    /// 生成结构化 Markdown 审计报告
    public static func generateMarkdown(
        snapshots: [AgentSnapshot],
        history: [AgentTaskEvent] = [],
        now: Date = Date()
    ) -> String {
        var md = ""
        md += "# AgentIsland 智能体运维与 Token 消耗审计报告\n\n"
        md += "- **生成时间**：\(dateFormatter.string(from: now))\n"
        md += "- **监控智能体总数**：\(snapshots.count) 个\n"

        let runningCount = snapshots.filter { $0.processRunning }.count
        let workingCount = snapshots.filter { $0.level == .working }.count
        let totalTokens24h = snapshots.reduce(0) { $0 + ($1.tokenUsage?.tokens24h ?? 0) }
        let totalCost24h = snapshots.reduce(0.0) { $0 + ($1.tokenUsage?.cost24h ?? 0.0) }

        md += "- **在线运行中**：\(runningCount) 个 (其中 \(workingCount) 个工作中)\n"
        md += "- **24h Token 消耗**：\(TokenUsage.compact(totalTokens24h)) tokens"
        if totalCost24h > 0 {
            md += " (\(TokenUsage.cost(totalCost24h)))"
        }
        md += "\n\n---\n\n"

        // 1. 智能体健康与资源状态
        md += "## 1. 智能体健康与系统负载\n\n"
        md += "| 智能体 | 状态 | 进程 PID | CPU | 物理内存 | 健康评分 | 评级 | 诊断建议 |\n"
        md += "| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n"

        for snap in snapshots {
            let report = AgentHealthEvaluator.evaluate(snapshot: snap)
            let pidStr = snap.pid.map { String($0) } ?? "—"
            let cpuStr = String(format: "%.1f%%", snap.cpuPercent)
            let memStr = snap.memoryBytes > 0 ? snap.memoryText : "—"
            let gradeStr: String = {
                switch report.grade {
                case .healthy: return "健康"
                case .attention: return "需关注"
                case .warning: return "预警"
                case .critical: return "危急"
                }
            }()

            md += "| \(snap.profile.name) | \(snap.level.label) | \(pidStr) | \(cpuStr) | \(memStr) | \(report.score) | \(gradeStr) | \(report.suggestion) |\n"
        }
        // 会话源不可读的 Agent 单独列出：健康评分只看进程/CPU/内存，读不到会话库时
        // 报告里只会是一片「待机」，等于把「解析器坏了」伪装成「智能体闲着」。
        let blindSources = snapshots.compactMap { snap -> String? in
            guard let health = snap.sessionProbeHealth else { return nil }
            return "- **\(snap.profile.name)**：\(health.diagnosticText)"
        }
        if !blindSources.isEmpty {
            md += "\n### 会话源不可读（下述智能体的「待机」只代表没有信号）\n\n"
            md += blindSources.joined(separator: "\n") + "\n"
        }
        md += "\n"

        // 2. Token 消耗统计
        md += "## 2. Token 与成本消耗总览\n\n"
        md += "| 智能体 | 24h Token | 24h 成本 | 累计 Token | 累计成本 |\n"
        md += "| :--- | :--- | :--- | :--- | :--- |\n"

        for snap in snapshots {
            let u = snap.tokenUsage
            let tok24 = u.map { TokenUsage.compact($0.tokens24h) } ?? "0"
            let cost24 = (u?.cost24h ?? 0) > 0 ? TokenUsage.cost(u!.cost24h) : "—"
            let tokTot = u.map { TokenUsage.compact($0.tokensTotal) } ?? "0"
            let costTot = (u?.costTotal ?? 0) > 0 ? TokenUsage.cost(u!.costTotal) : "—"

            md += "| \(snap.profile.name) | \(tok24) | \(cost24) | \(tokTot) | \(costTot) |\n"
        }
        md += "\n"

        // 3. 近期任务与告警事件流水
        if !history.isEmpty {
            md += "## 3. 近期生命周期与告警事件流水\n\n"
            md += "| 时间 | 智能体 | 类型 | 耗时 | 摘要说明 |\n"
            md += "| :--- | :--- | :--- | :--- | :--- |\n"

            for ev in history.prefix(20) {
                let timeStr = dateFormatter.string(from: ev.timestamp)
                let durStr = ev.duration > 0 ? AgentTaskEvent.durationText(ev.duration) : "—"
                let typeStr: String = {
                    switch ev.eventType {
                    case .completed: return "任务完成"
                    case .attention: return "待确认"
                    case .costSpike: return "熔断告警"
                    }
                }()
                let msg = ev.summaryText.replacingOccurrences(of: "|", with: "\\|")
                md += "| \(timeStr) | \(ev.agentName) | \(typeStr) | \(durStr) | \(msg) |\n"
            }
            md += "\n"
        }

        md += "> 本报告由 AgentIsland 自动生成并导出。\n"
        return md
    }

    /// 生成标准 CSV 格式报表
    public static func generateCSV(snapshots: [AgentSnapshot], now: Date = Date()) -> String {
        var csv = "Timestamp,AgentID,AgentName,Level,PID,CPU_Percent,Memory_Bytes,HealthScore,Grade,Tokens_24h,Cost_24h,Tokens_Total,Cost_Total\n"
        let timeStr = dateFormatter.string(from: now)

        for snap in snapshots {
            let report = AgentHealthEvaluator.evaluate(snapshot: snap)
            let pidStr = snap.pid.map { String($0) } ?? ""
            let u = snap.tokenUsage

            let row = [
                escapeCSV(timeStr),
                escapeCSV(snap.profile.id),
                escapeCSV(snap.profile.name),
                escapeCSV(snap.level.rawValue),
                pidStr,
                String(format: "%.1f", snap.cpuPercent),
                String(snap.memoryBytes),
                String(report.score),
                escapeCSV(report.grade.rawValue),
                String(u?.tokens24h ?? 0),
                String(format: "%.4f", u?.cost24h ?? 0.0),
                String(u?.tokensTotal ?? 0),
                String(format: "%.4f", u?.costTotal ?? 0.0)
            ].joined(separator: ",")

            csv += row + "\n"
        }

        return csv
    }

    private static func escapeCSV(_ str: String) -> String {
        if str.contains(",") || str.contains("\"") || str.contains("\n") {
            return "\"\(str.replacingOccurrences(of: "\"", with: "\"\""))\""
        }
        return str
    }

    /// 生成 Raycast Extension 命令定义清单 (v0.0.78)
    public static func generateRaycastManifest(snapshots: [AgentSnapshot]) -> String {
        var commands: [[String: String]] = [
            [
                "name": "toggle",
                "title": "Toggle AgentIsland",
                "description": "展开或收起灵动岛监控面板",
                "url": "agentisland://toggle"
            ],
            [
                "name": "analytics",
                "title": "Token Analytics",
                "description": "打开 Token 用量与成本预测分析",
                "url": "agentisland://analytics"
            ],
            [
                "name": "toolbox",
                "title": "Agent Workbench & Diagnostics",
                "description": "打开维护工作台与死锁排查",
                "url": "agentisland://toolbox"
            ],
            [
                "name": "clean",
                "title": "Clean Orphan & Hung Agents",
                "description": "一键安全清理挂起死锁与孤儿后台进程",
                "url": "agentisland://clean"
            ],
            [
                "name": "export",
                "title": "Export Audit Report",
                "description": "导出 Markdown 运维审计报告至剪贴板",
                "url": "agentisland://export"
            ]
        ]

        for s in snapshots where s.installed || s.processRunning {
            commands.append([
                "name": "agent-\(s.id)",
                "title": "Inspect \(s.profile.name)",
                "description": "直达 \(s.profile.name) 运行态详情与会话",
                "url": "agentisland://agent?id=\(s.id)"
            ])
        }

        let dict: [String: Any] = [
            "name": "AgentIsland Raycast Commands",
            "version": AppVersion.string,
            "commands": commands
        ]

        if let data = try? JSONSerialization.data(withJSONObject: dict, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]),
           let str = String(data: data, encoding: .utf8) {
            return str
        }
        return "{}"
    }
}

import Foundation

// MARK: - 智能体审计报告导出器 (v0.0.74)

public enum AuditReportExporter {
    /// Markdown 表格单元转义。此前只有 `summaryText` 过了 `|`，`agentName` 与所有字段
    /// 的**换行**都不处理——而 `agentisland://notify?...message=x%0A%7C...` 里
    /// `URLComponents.queryItems` 会把 %0A 解成真换行，于是攻击者能往用户粘进工单/群聊
    /// 的报告里自造行与小节（`export` 还会把这份报告写进剪贴板）
    static func cell(_ text: String) -> String {
        // 只处理会破坏表格结构的字符：`|` 分列、换行分行。反斜杠不转义——
        // Markdown 单元里的 `\` 不是列分隔符，转它只会让文本变样
        text.replacingOccurrences(of: "|", with: "\\|")
            .replacingOccurrences(of: "\r", with: " ")
            .replacingOccurrences(of: "\n", with: " ")
    }


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
    /// - Parameter grandTotal: 面板汇总栏那份跨源总量。**给了就必须用它做头条数字**：
    ///   `snapshots` 只覆盖当前在册条目（离线但仍有用量的工具、被宿主去重掉的档案都不在
    ///   那份列表里），逐条相加会比面板少一大截。用户拿着报告对不上岛，怀疑的是面板。
    public static func generateMarkdown(
        snapshots: [AgentSnapshot],
        history: [AgentTaskEvent] = [],
        grandTotal: TokenUsage? = nil,
        now: Date = Date()
    ) -> String {
        var md = ""
        md += "# AgentIsland 智能体运维与 Token 消耗审计报告\n\n"
        md += "- **生成时间**：\(dateFormatter.string(from: now))\n"
        md += "- **监控智能体总数**：\(snapshots.count) 个\n"

        let runningCount = snapshots.filter { $0.processRunning }.count
        let workingCount = snapshots.filter { $0.level == .working }.count
        let listedTokens24h = snapshots.reduce(0) { $0 + ($1.tokenUsage?.tokens24h ?? 0) }
        let listedCost24h = snapshots.reduce(0.0) { $0 + ($1.tokenUsage?.cost24h ?? 0.0) }
        let totalTokens24h = grandTotal?.tokens24h ?? listedTokens24h
        let totalCost24h = grandTotal?.cost24h ?? listedCost24h

        md += "- **在线运行中**：\(runningCount) 个 (其中 \(workingCount) 个工作中)\n"
        md += "- **24h Token 消耗**：\(TokenUsage.compact(totalTokens24h)) tokens"
        if totalCost24h > 0 {
            md += " (\(TokenUsage.cost(totalCost24h)))"
        }
        md += "\n"
        // 两份口径都摆出来，而不是悄悄选一个：跨源总量与逐条列表之和不等时，
        // 差额是「离线但有历史」「被宿主去重」这类在册范围差异，读报告的人需要知道
        if let grandTotal, grandTotal.tokens24h != listedTokens24h {
            md += "  ·  口径：全部数据源总量（与面板汇总栏同源）；"
                + "下表逐条相加为 \(TokenUsage.compact(listedTokens24h)) tokens，"
                + "差额来自离线但仍有用量记录、以及与宿主合并的内嵌组件\n"
        }
        md += "\n---\n\n"

        // 1. 智能体健康与资源状态
        md += "## 1. 智能体健康与系统负载\n\n"
        md += "| 智能体 | 状态 | 进程 PID | CPU | 物理内存 | 健康评分 | 评级 | 诊断建议 |\n"
        md += "| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n"

        for snap in snapshots {
            let report = AgentHealthEvaluator.evaluate(snapshot: snap)
            let pidStr = snap.pid.map { String($0) } ?? "—"
            let cpuStr = String(format: "%.1f%%", snap.cpuPercent)
            let memStr = snap.memoryBytes > 0 ? snap.memoryText : "—"
            // 评级直接用 HealthGrade 的措辞：这里曾自己 switch 出「需关注 / 预警」，
            // 而 status --json 与详情卡说「需留意 / 异常」——同一个评级两份话。
            let gradeStr = report.grade.rawValue

            md += "| \(cell(snap.profile.name)) | \(snap.level.label) | \(pidStr) | \(cpuStr) | \(memStr) | \(report.score) | \(gradeStr) | \(cell(report.suggestion)) |\n"
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

            md += "| \(cell(snap.profile.name)) | \(tok24) | \(cost24) | \(tokTot) | \(costTot) |\n"
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
                let msg = cell(ev.summaryText)
                md += "| \(timeStr) | \(cell(ev.agentName)) | \(typeStr) | \(durStr) | \(msg) |\n"
            }
            md += "\n"
        }

        md += "> 本报告由 AgentIsland 自动生成并导出。\n"
        return md
    }

    /// 生成标准 CSV 格式报表
    public static func generateCSV(snapshots: [AgentSnapshot], now: Date = Date()) -> String {
        var csv = "Timestamp,AgentID,AgentName,Level,PID,CPU_Percent,Memory_Bytes,HealthScore,Grade,Tokens_24h,Cost_24h,Tokens_Total,Cost_Total,Observability,ObservationEvidence\n"
        let timeStr = dateFormatter.string(from: now)

        for snap in snapshots {
            let report = AgentHealthEvaluator.evaluate(snapshot: snap)
            let verdict = AgentObservability.evaluate(snapshot: snap)
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
                String(format: "%.4f", u?.costTotal ?? 0.0),
                escapeCSV(verdict.code.rawValue),
                escapeCSV(verdict.evidence.first ?? "")
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

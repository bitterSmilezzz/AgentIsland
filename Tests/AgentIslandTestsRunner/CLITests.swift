import Foundation
@testable import AgentIslandCore

// MARK: - CLI 工具与格式化单元测试 (v0.0.76)

enum CLITests {
    @MainActor
    static func register() {
        TestKit.test("CLI: ANSI 终端颜色与无 TTY 纯文本回退") {
            let originalTTY = CLIColor.isTTY
            defer { CLIColor.isTTY = originalTTY }

            CLIColor.isTTY = true
            let colored = CLIColor.red("error")
            try expectTrue(colored.contains("\u{001B}[31m"), "TTY 环境应包含 ANSI 红色转义序列")
            try expectTrue(colored.contains("error"), "应包含原文字符串")

            CLIColor.isTTY = false
            let plain = CLIColor.red("error")
            try expectEqual(plain, "error", "非 TTY 环境应退回纯文本")
        }

        TestKit.test("CLI: CLITable 表格排版与字符宽度对齐") {
            let originalTTY = CLIColor.isTTY
            defer { CLIColor.isTTY = originalTTY }
            CLIColor.isTTY = false

            var table = CLITable(columns: [
                CLITable.Column("名称", minWidth: 6),
                CLITable.Column("数值", minWidth: 6, alignRight: true)
            ])
            table.addRow(["Alpha", "100"])
            table.addRow(["Beta", "20"])

            let rendered = table.render()
            try expectTrue(rendered.contains("名称"), "表头应包含列名")
            try expectTrue(rendered.contains("Alpha"), "表格应包含第一行")
            try expectTrue(rendered.contains("Beta"), "表格应包含第二行")
            try expectTrue(rendered.contains("100"), "表格应包含右对齐数值")
        }

        TestKit.test("CLI: 状态与用量 DTO 序列化往返") {
            let dummyProfile = AgentRegistry.builtin[0]
            let snapshot = AgentSnapshot(
                profile: dummyProfile,
                level: .working,
                processRunning: true,
                cpuPercent: 12.5,
                installed: true,
                activeSessions: 2,
                lastActivityAgo: 45,
                lastActivityText: "正在写入代码",
                tokenUsage: TokenUsage(tokens24h: 12500, tokensTotal: 50000, cost24h: 0.25, costTotal: 1.0),
                pid: 12345,
                currentAction: "swift build",
                memoryBytes: 150_000_000,
                isHung: false
            )

            let dto = CLIAgentStatusDTO(from: snapshot)
            try expectEqual(dto.id, snapshot.id, "DTO id 应一致")
            try expectEqual(dto.name, dummyProfile.name, "DTO name 应一致")
            try expectEqual(dto.status, "working", "状态应为 working")
            try expectEqual(dto.pid, 12345, "PID 应匹配")
            try expectEqual(dto.tokens24h, 12500, "24h token 应匹配")
            try expectEqual(dto.cost24h, 0.25, "24h cost 应匹配")

            let encoder = JSONEncoder()
            let data = try encoder.encode(dto)
            let decoder = JSONDecoder()
            let decoded = try decoder.decode(CLIAgentStatusDTO.self, from: data)
            try expectEqual(decoded.id, dto.id, "JSON 编解码往返后 id 一致")
            try expectEqual(decoded.tokens24h, dto.tokens24h, "JSON 编解码往返后 tokens24h 一致")
            // 往返断言两边一起动，改键名它永远绿；对外契约只能按字面键名钉
            let json = String(data: data, encoding: .utf8) ?? ""
            for key in ["\"id\"", "\"status\"", "\"pid\"", "\"tokens24h\"", "\"cost24h\""] {
                try expectTrue(json.contains(key), "对外 JSON 键名漂移：缺 \(key)（Raycast/CSV 下游按键名取数）")
            }
        }

        TestKit.test("CLI: 异常诊断与清理 DTO 往返") {
            let anomaly = AgentAnomaly(
                id: "dim-hung-100",
                pid: 100,
                ppid: 1,
                agentName: "DimAgent",
                profileId: "dim",
                commandPath: "/usr/local/bin/dim",
                cpuPercent: 95.0,
                memoryBytes: 500_000_000,
                anomalyType: .hung,
                reason: "持续卡顿死锁",
                batchCleanable: true
            )

            let dto = CLIAnomalyDTO(from: anomaly)
            try expectEqual(dto.pid, 100, "PID 匹配")
            try expectEqual(dto.type, "hung", "类型为 hung")
            try expectTrue(dto.batchCleanable, "可批量清理")

            let cleanResult = CLICleanResultDTO(
                success: true,
                killedPids: [100],
                freedMemoryBytes: 500_000_000,
                freedMemoryFormatted: "476M",
                dryRun: false
            )
            let data = try JSONEncoder().encode(cleanResult)
            let decodedResult = try JSONDecoder().decode(CLICleanResultDTO.self, from: data)
            try expectEqual(decodedResult.killedPids, [100], "Killed PIDs 往返一致")
            try expectFalse(decodedResult.dryRun, "dryRun 标志一致")
            let json = String(data: data, encoding: .utf8) ?? ""
            for key in ["\"killedPids\"", "\"dryRun\""] {
                try expectTrue(json.contains(key), "清理结果 JSON 键名漂移：缺 \(key)")
            }
        }

        TestKit.test("CLI: Notify DTO 序列化与解析往返") {
            let notifyReq = CLINotifyRequestDTO(
                agent: "antigravity",
                type: "completed",
                message: "构建与单元测试通过",
                detail: "265 个测试全部通过"
            )
            let data = try JSONEncoder().encode(notifyReq)
            let decoded = try JSONDecoder().decode(CLINotifyRequestDTO.self, from: data)
            try expectEqual(decoded.agent, "antigravity", "智能体名称一致")
            try expectEqual(decoded.type, "completed", "类型一致")
            try expectEqual(decoded.message, "构建与单元测试通过", "消息一致")
            try expectEqual(decoded.detail, "265 个测试全部通过", "详情一致")

            let notifyRes = CLINotifyResultDTO(success: true, eventId: "evt-123", message: "Accepted")
            let resData = try JSONEncoder().encode(notifyRes)
            let decodedRes = try JSONDecoder().decode(CLINotifyResultDTO.self, from: resData)
            try expectTrue(decodedRes.success, "结果成功标志一致")
            try expectEqual(decodedRes.eventId, "evt-123", "事件 ID 一致")
        }

        TestKit.test("CLI: Token 预算报告 DTO 字段与往返") {
            let report = CLITokenReportDTO(
                tokens24h: 500_000,
                cost24h: 5.0,
                tokensTotal: 1_000_000,
                costTotal: 10.0,
                projectedMonthEndTokens: 15_000_000,
                projectedMonthEndCost: 150.0,
                budgetExhaustionDay: 18,
                forecastSummary: "按当前消耗速率预计第 18 天超额",
                dailyBudget: 1_000_000,
                budgetRatio: 0.5,
                budgetStatus: "正常",
                agents: [:]
            )
            let data = try JSONEncoder().encode(report)
            let decoded = try JSONDecoder().decode(CLITokenReportDTO.self, from: data)
            try expectEqual(decoded.dailyBudget, 1_000_000, "预算上限一致")
            let reportJSON = String(data: data, encoding: .utf8) ?? ""
            for key in ["\"dailyBudget\"", "\"budgetRatio\"", "\"budgetStatus\""] {
                try expectTrue(reportJSON.contains(key), "Token 报表 JSON 键名漂移：缺 \(key)")
            }
            try expectEqual(decoded.budgetRatio, 0.5, "预算比例一致")
            try expectEqual(decoded.budgetStatus, "正常", "预算状态一致")
            try expectEqual(decoded.budgetExhaustionDay, 18, "超额天数一致")
        }

        TestKit.test("CLI: CSV 与 Raycast 导出格式生成") {
            let dummyProfile = AgentRegistry.builtin[0]
            let snapshot = AgentSnapshot(
                profile: dummyProfile,
                level: .working,
                processRunning: true,
                cpuPercent: 15.0,
                installed: true,
                activeSessions: 1,
                lastActivityAgo: 30,
                lastActivityText: "正在运行",
                tokenUsage: TokenUsage(tokens24h: 5000, tokensTotal: 10000, cost24h: 0.1, costTotal: 0.2),
                pid: 1234,
                currentAction: "test",
                memoryBytes: 100_000_000,
                isHung: false
            )

            let csv = AuditReportExporter.generateCSV(snapshots: [snapshot])
            try expectTrue(csv.contains("Timestamp,AgentID,AgentName"), "CSV 包含表头")
            try expectTrue(csv.contains("dim"), "CSV 包含 agent id")
            try expectTrue(csv.contains("1234"), "CSV 包含 PID")

            let raycast = AuditReportExporter.generateRaycastManifest(snapshots: [snapshot])
            try expectTrue(raycast.contains("AgentIsland Raycast Commands"), "Raycast 清单包含标题")
            try expectTrue(raycast.contains("agentisland://toggle"), "Raycast 清单包含 toggle 协议")
            try expectTrue(raycast.contains("agentisland://agent?id=dim"), "Raycast 清单包含 agent 直达协议")
        }

        TestKit.test("可观测性: 四类结论互斥，且「读不到」绝不与「闲着」混为一谈") {
            let profile = AgentRegistry.builtin[0]
            func snap(processRunning: Bool, installed: Bool, sessions: Int = 0,
                      tokens: Int = 0, tokensTotal: Int? = nil, level: ActivityLevel = .idle,
                      health: SessionProbeHealth? = nil) -> AgentSnapshot {
                // 24h 与累计默认取**不同**值：此前两者恒等，把生产里的
                // `tokenUsage?.tokensTotal` 改成 `tokens24h` 也照样全绿
                let total = tokensTotal ?? tokens * 7
                return AgentSnapshot(profile: profile, level: level, processRunning: processRunning,
                              cpuPercent: 0, installed: installed, activeSessions: sessions,
                              lastActivityAgo: nil, lastActivityText: "—",
                              tokenUsage: tokens > 0 || total > 0
                                  ? TokenUsage(tokens24h: tokens, tokensTotal: total,
                                               cost24h: 0, costTotal: 0) : nil,
                              sessionProbeHealth: health)
            }
            let blind = SessionProbeHealth(failure: .prepareFailed, path: "/tmp/x.db")
            // 无会话、24h 为零但累计有量：口径必须认「累计」，判成可信
            let totalOnly = snap(processRunning: true, installed: true, sessions: 0,
                                 tokens: 0, tokensTotal: 999)
            try expectEqual(AgentObservability.evaluate(snapshot: totalOnly).code, .observed,
                            "tokensTotal>0 就是「本地明细读到了」；读错字段会降级成无本地明细")

            // 表驱动：(名称, 输入, 期望 code)
            let cases: [(String, AgentSnapshot, AgentObservability.Code)] = [
                ("未安装且不在跑", snap(processRunning: false, installed: false), .notInstalled),
                ("已装但进程不在 → 离线可信", snap(processRunning: false, installed: true), .observed),
                ("在跑但会话库坏了 → 待机不可信",
                 snap(processRunning: true, installed: true, sessions: 3, tokens: 999, health: blind),
                 .blindSessionSource),
                ("在跑但无任何本地明细", snap(processRunning: true, installed: true), .noLocalData),
                ("在跑且有活跃会话", snap(processRunning: true, installed: true, sessions: 2), .observed),
                ("在跑但有用量账本", snap(processRunning: true, installed: true, tokens: 50), .observed),
                // 会话强语义本身就是「源是通的」的证据：等级为待确认/已完成/工作中时，
                // 哪怕活跃会话数为 0 也不能判成「无本地明细」（实测 Antigravity 会自相矛盾）
                ("待确认但会话数为 0",
                 snap(processRunning: true, installed: true, level: .attention), .observed),
                ("工作中但会话数为 0",
                 snap(processRunning: true, installed: true, level: .working), .observed),
                ("已完成且无明细",
                 snap(processRunning: true, installed: true, level: .completed), .observed),
            ]
            // 「未接入明细源」与「接入却读不到」必须分家：前者是设计如此，后者才需要排查。
            // 取一个既无会话库也无 token 目录的档案来断言（内置集里 ChatGPT 正是这种）
            let unwiredProfile = AgentRegistry.builtin.first {
                $0.sessionDatabase == nil && $0.tokenRoots.isEmpty
            }
            if let unwiredProfile {
                let verdict = AgentObservability.evaluate(snapshot: AgentSnapshot(
                    profile: unwiredProfile, level: .idle, processRunning: true, cpuPercent: 0,
                    installed: true, activeSessions: 0, lastActivityAgo: nil, lastActivityText: "—"))
                try expectEqual(verdict.code, .sourceNotWired, "\(unwiredProfile.id) 未接入明细源")
                try expectTrue(verdict.evidence[0].contains("不代表它没在工作"),
                               "未接入的依据必须明确它不是故障")
            }
            // 登记了源的档案在同样「什么都读不到」时必须落到 noLocalData
            let wiredProfile = AgentRegistry.builtin.first { $0.hasLocalDetailSource }
            if let wiredProfile {
                let verdict = AgentObservability.evaluate(snapshot: AgentSnapshot(
                    profile: wiredProfile, level: .idle, processRunning: true, cpuPercent: 0,
                    installed: true, activeSessions: 0, lastActivityAgo: nil, lastActivityText: "—"))
                try expectEqual(verdict.code, .noLocalData, "\(wiredProfile.id) 登记了源却读不到")
                try expectTrue(verdict.evidence[0].contains("同步刷新用量源后仍无记录"),
                               "依据必须说明「已经取过了」，而不是含糊的没赶上")
            }
            for (label, snapshot, expected) in cases {
                let verdict = AgentObservability.evaluate(snapshot: snapshot)
                try expectEqual(verdict.code, expected, label)
                try expectEqual(verdict.isTrustworthy, expected == .observed, "\(label)：可信标记")
                try expectFalse(verdict.evidence.isEmpty, "\(label)：结论必须带依据")
            }
            // 「读不到」的依据文案必须自己说清边界，否则用户仍会把它当待机
            let blindVerdict = AgentObservability.evaluate(
                snapshot: snap(processRunning: true, installed: true, health: blind))
            try expectTrue(blindVerdict.evidence[0].contains("不代表智能体真的空闲"),
                           "失明依据必须沿用 SessionProbeHealth 的原始措辞")
        }

        TestKit.test("CLI: doctor 与状态 DTO 携带可观测性结论与依据") {
            let profile = AgentRegistry.builtin[0]
            let snapshot = AgentSnapshot(profile: profile, level: .idle, processRunning: true,
                                         cpuPercent: 0, installed: true, activeSessions: 0,
                                         lastActivityAgo: nil, lastActivityText: "—",
                                         sessionProbeHealth: SessionProbeHealth(
                                            failure: .unreadableDB, path: "/tmp/blind.db"))
            let doctor = CLIAgentDoctorDTO(from: snapshot)
            try expectEqual(doctor.observability, "blindSessionSource")
            try expectTrue(doctor.evidence.first?.contains("/tmp/blind.db") == true,
                           "JSON 里要能定位到是哪个源读不到")

            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode([CLIAgentStatusDTO(from: snapshot)])
            let back = try JSONDecoder().decode([CLIAgentStatusDTO].self, from: data)
            try expectEqual(back.first?.observability, "blindSessionSource", "状态 DTO 往返不得丢结论")
            try expectEqual(back.first?.observabilityEvidence.isEmpty, false, "状态 DTO 也要带上依据")
            try expectTrue((back.first?.healthScore ?? -1) >= 0, "状态 DTO 应携带稳定性评分")

            let csv = AuditReportExporter.generateCSV(snapshots: [snapshot])
            let header = csv.split(separator: "\n").first.map(String.init) ?? ""
            try expectTrue(header.hasPrefix("Timestamp,AgentID,AgentName,Level"),
                           "既有 CSV 列序不得变动（下游脚本按列位解析）")
            try expectTrue(header.contains("Observability,ObservationEvidence"), "CSV 需追加可观测性两列")
        }
    }
}

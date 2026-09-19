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
    }
}

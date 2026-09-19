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
    }
}

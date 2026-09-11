import Foundation
@testable import AgentIslandCore

// MARK: - memoryText 三处实现的边界
//
// 「内存文案」曾在三个类型上各写一份且口径不一致（`AgentSnapshot` 有 `<1M` 分支，
// `AgentAnomaly`/`CleanResult` 走 `Int(mb)` 截断显示「0M」）。现已收敛到
// `MemoryFormat.text(_:)` 唯一实现，下面的用例改为断言**三处一致**。

@MainActor
enum MemoryTextTests {

    private static let kb: UInt64 = 1024
    private static let mb: UInt64 = 1024 * 1024

    private static func snapshotText(_ bytes: UInt64) -> String {
        AgentSnapshot(
            profile: AgentProfile(id: "mem-agent", name: "Mem Agent", icon: "terminal",
                                  bundleIDs: [], processNames: ["memcli"], sessionDirs: []),
            level: .working, processRunning: true, cpuPercent: 1, installed: true,
            activeSessions: 0, lastActivityAgo: 1, lastActivityText: "刚刚",
            memoryBytes: bytes
        ).memoryText
    }

    private static func anomalyText(_ bytes: UInt64) -> String {
        AgentAnomaly(id: "overweight-1", pid: 1, ppid: 1, agentName: "Mem Agent",
                     profileId: "mem-agent", commandPath: "/opt/fake/bin/memcli",
                     cpuPercent: 0, memoryBytes: bytes, anomalyType: .overweight,
                     reason: "fixture").memoryText
    }

    private static func cleanResultText(_ bytes: UInt64) -> String {
        CleanResult(terminatedCount: 1, reclaimedMemoryBytes: bytes).reclaimedMemoryText
    }

    static func register() {

        TestKit.test("内存文案: 0 字节三处均为占位符（无数据）") {
            try expectEqual(snapshotText(0), "—", "AgentSnapshot 用占位符表示无数据")
            try expectEqual(anomalyText(0), "—", "AgentAnomaly 与快照口径一致")
            try expectEqual(cleanResultText(0), "—", "CleanResult 与快照口径一致")
        }

        TestKit.test("内存文案: 1 字节与 1MB-1 三处均走 <1M 分支（不得显示 0M）") {
            for bytes: UInt64 in [1, kb, 512 * kb, mb - 1] {
                try expectEqual(snapshotText(bytes), "<1M", "\(bytes) 字节快照应为 <1M")
                try expectEqual(anomalyText(bytes), "<1M", "\(bytes) 字节异常应为 <1M")
                try expectEqual(cleanResultText(bytes), "<1M", "\(bytes) 字节清理结果应为 <1M")
                try expectFalse(snapshotText(bytes).hasPrefix("0"), "不得以 0 开头（0M 易被误读为无占用）")
            }
        }

        TestKit.test("内存文案: 1MB / 1023MB / 1024MB / 1.5GB 三处一致") {
            try expectEqual(snapshotText(mb), "1M", "1MB")
            try expectEqual(anomalyText(mb), "1M", "1MB")
            try expectEqual(cleanResultText(mb), "1M", "1MB")

            let mb1023 = 1023 * mb
            try expectEqual(snapshotText(mb1023), "1023M", "1023MB 未达 GB 分界")
            try expectEqual(anomalyText(mb1023), "1023M", "1023MB")
            try expectEqual(cleanResultText(mb1023), "1023M", "1023MB")

            let mb1024 = 1024 * mb
            try expectEqual(snapshotText(mb1024), "1.0G", "1024MB 恰好跨到 GB 分支")
            try expectEqual(anomalyText(mb1024), "1.0G", "1024MB")
            try expectEqual(cleanResultText(mb1024), "1.0G", "1024MB")

            let gb15: UInt64 = 1536 * mb
            try expectEqual(snapshotText(gb15), "1.5G", "1.5GB")
            try expectEqual(anomalyText(gb15), "1.5G", "1.5GB")
            try expectEqual(cleanResultText(gb15), "1.5G", "1.5GB")
        }

        TestKit.test("内存文案: 三处实现完全一致（收敛后的一致性守护）") {
            // 该用例守护「唯一实现」这一契约：任何一处再被单独改写都会在此失败
            for bytes: UInt64 in [0, 1, kb, 512 * kb, mb - 1, mb, 1023 * mb, 1024 * mb, 1536 * mb, 10 * 1024 * mb] {
                let snap = snapshotText(bytes)
                try expectEqual(anomalyText(bytes), snap, "\(bytes) 字节：异常文案应与快照一致")
                try expectEqual(cleanResultText(bytes), snap, "\(bytes) 字节：清理结果应与快照一致")
            }
        }
    }
}

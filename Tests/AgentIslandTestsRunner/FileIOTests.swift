import Foundation
@testable import AgentIslandCore

@MainActor
enum FileIOTests {
    static func register() {
        TestKit.test("日志尾读：中文字符截断不影响后续完整记录") {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: file) }
            for (budget, read) in readers {
                // fileSize - budget == 1：从「中」的第二个 UTF-8 字节开始。
                let content = "中" + String(repeating: "x", count: budget - 6) + "\nOK\n"
                try Data(content.utf8).write(to: file)
                try expectEqual(read(file, 1), ["OK"], "字节预算 \(budget)")
            }
        }
        TestKit.test("日志尾读：丢弃预算截断的首行，不伪造半条记录") {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: file) }
            for (budget, read) in readers {
                try Data((String(repeating: "x", count: budget) + "\nOK\n").utf8).write(to: file)
                try expectTrue(read(file, 10) == ["OK"], "只保留完整行")
            }
        }
        TestKit.test("日志尾读：保留文件起点、行边界及无换行末条") {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: file) }
            for (budget, read) in readers {
                try Data("  首条\r\n\n末条".utf8).write(to: file)
                try expectEqual(read(file, 10), ["首条", "末条"])
                let finalLine = String(repeating: "x", count: budget)
                try Data(("old\n" + finalLine).utf8).write(to: file)
                try expectEqual(read(file, 1), [finalLine], "恰好从行边界开始")
            }
        }
        TestKit.test("日志尾读：空文件、缺失文件和非法数量返回空") {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: file) }
            for (_, read) in readers { try expectTrue(read(file, 10).isEmpty) }
            try Data().write(to: file)
            for (_, read) in readers { try expectTrue(read(file, 10).isEmpty) }
            try Data("OK".utf8).write(to: file)
            for (_, read) in readers {
                try expectTrue(read(file, 0).isEmpty)
                try expectTrue(read(file, -1).isEmpty)
            }
        }
        TestKit.test("日志尾读：末行不完整 UTF-8 不遮蔽前面的完整行") {
            let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: file) }
            try (Data("OK\n".utf8) + Data([0xe4, 0xb8])).write(to: file)
            for (_, read) in readers { try expectEqual(read(file, 10), ["OK"]) }
        }
        TestKit.test("文件缓存：窗口修改后下一次扫描采用新口径") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let session = root.appendingPathComponent("session")
            try FileManager.default.createDirectory(at: session, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let old = Date().addingTimeInterval(-120)
            try FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: session.path)
            try FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: root.path)
            let monitor = FileActivityMonitor(scanMinInterval: 3600)
            monitor.watch(dirs: [root.path])
            monitor.scanSync()
            try expectEqual(monitor.activeSessionCounts(for: [root.path])[root.path], 1)
            monitor.setActiveSessionWindow(10)
            monitor.scanSync()
            try expectEqual(monitor.activeSessionCounts(for: [root.path])[root.path], 0, "窗口变更绕过旧节流与快跳过")
            monitor.setActiveSessionWindow(600)
            monitor.scanSync()
            try expectEqual(monitor.activeSessionCounts(for: [root.path])[root.path], 1, "窗口扩大同样重算")
        }
        TestKit.test("文件缓存：新增监控目录无需等待旧扫描节流") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let monitor = FileActivityMonitor(scanMinInterval: 3600)
            monitor.scanSync()
            monitor.replaceWatchedDirs([root.path])
            monitor.scanSync()
            try expectTrue(monitor.lastWriteDates(for: [root.path])[root.path] != nil)
            monitor.replaceWatchedDirs([])
            try expectTrue(monitor.lastWriteDates(for: [root.path]).isEmpty)
        }
        TestKit.test("文件活动：编辑历史与附件缓存不触发工作态") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let history = root.appendingPathComponent("session/file-history")
            let blobs = root.appendingPathComponent("session/blobs/files")
            try FileManager.default.createDirectory(at: history, withIntermediateDirectories: true)
            try FileManager.default.createDirectory(at: blobs, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }

            let old = Date().addingTimeInterval(-300)
            try? FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: root.path)
            try? FileManager.default.setAttributes([.modificationDate: old], ofItemAtPath: root.appendingPathComponent("session").path)
            try Data("history".utf8).write(to: history.appendingPathComponent("edit@v1"))
            try Data("blob".utf8).write(to: blobs.appendingPathComponent("attachment"))

            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 6, window: 60, now: Date())
            try expectTrue(result.newest != nil && Date().timeIntervalSince(result.newest!) > 120,
                           "file-history/blobs 的新写入不应被计入最近活动")
            try expectEqual(result.activeSessions, 0, "缓存子树不应计为活跃会话")
        }
    }

    private static let readers: [(Int, (URL, Int) -> [String])] = [
        (16_384, { AgentActionInspector.readLastLines(from: $0, maxLines: $1) }),
        (65_536, { AgentLogStreamer.readLastLines(from: $0, maxLines: $1) }),
    ]
}

import Foundation
@testable import AgentIslandCore

@MainActor
enum FileIOTests {
    static func register() {
        TestKit.test("目录扫描: 整棵没看完 ≠ 看完了但没有信号文件") {
            // 清零的语义是「这个目录里的产物被清理了」；枚举器建不起来是「没看到」。
            // 两者混同的现场：一个正在写文件的 Agent 因为一次读不了目录被判成待机。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                                           ofItemAtPath: root.path)
                    try? FileManager.default.removeItem(at: root) }
            let now = Date()
            try Data("x".utf8).write(to: root.appendingPathComponent("session.jsonl"))
            let readable = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4,
                                                        window: 60, now: now)
            try expectFalse(readable.scanFailed, "可读目录不该报扫描失败")
            try expectNotNil(readable.newest, "可读目录该看到写入信号")

            // 只给执行位：目录能 stat（父级要遍历）却不能枚举 ⇒ 枚举器建不起来
            try FileManager.default.setAttributes([.posixPermissions: 0o100],
                                                  ofItemAtPath: root.path)
            let blocked = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4,
                                                       window: 60, now: now)
            try expectTrue(blocked.scanFailed, "没有读权限的目录必须回吐「整棵没看完」")
            try expectNil(blocked.newest, "没看完就是没有结论")

            try FileManager.default.setAttributes([.posixPermissions: 0o755],
                                                  ofItemAtPath: root.path)
            let empty = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: empty, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: empty) }
            let scannedEmpty = FileActivityMonitor.scanTree(in: empty.path, maxDepth: 4,
                                                            window: 60, now: now)
            try expectFalse(scannedEmpty.scanFailed,
                            "扫完但没有信号文件是正常终态，不该被算成失败")
        }

        TestKit.test("目录扫描: 读不到的目录保留上一份活动，不清零") {
            // scanTree 回吐 scanFailed 只是前半段；调用方必须据此走「缺失宽限」而不是
            // 「扫完了没信号」的清零分支——否则岛上的措辞从「工作中」变成「待机」。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755],
                                                           ofItemAtPath: root.path)
                    try? FileManager.default.removeItem(at: root) }
            let session = root.appendingPathComponent("s1")
            try FileManager.default.createDirectory(at: session, withIntermediateDirectories: true)
            try Data("x".utf8).write(to: session.appendingPathComponent("transcript.jsonl"))

            let monitor = FileActivityMonitor(maxDepth: 4, scanMinInterval: 0)
            monitor.setWorkingWindow(60)
            monitor.watch(dirs: [root.path])
            monitor.scanSync()
            let before = monitor.lastWriteDates(for: [root.path])[root.path]
            try expectNotNil(before, "前置：首扫要看到写入信号")
            let countBefore = monitor.activeSessionCounts(for: [root.path])[root.path]
            try expectEqual(countBefore, 1, "前置：s1 里刚写过 ⇒ 活跃会话数 1")

            // 关键一步：必须让第二拍**真的去做深层扫描**。根 mtime 没变 + 目录不在
            // runningDirs 里 ⇒ 走「跳过深层递归」那条快路，缓存被原样返回，这条用例
            // 就永远不可能失败（前两版都栽在这里，变异验证才抓得出来）。
            // 新上线的目录会把 lastFullScans 置空 ⇒ 强制全量深搜。
            try Data("y".utf8).write(to: session.appendingPathComponent("more.jsonl"))
            try FileManager.default.setAttributes([.posixPermissions: 0o100],
                                                  ofItemAtPath: root.path)
            monitor.setRunningDirs([root.path])
            monitor.scanSync()
            let after = monitor.lastWriteDates(for: [root.path])[root.path]
            try expectEqual(after, before,
                            "整棵没看完时不得把活动清零（那是「没看到」，不是「没在干活」）")
            // 同一份快照里的两个字段必须同口径。此前日期走「保留」、计数走「清零」，
            // 于是「刚刚还在写」与「0 个会话」同时成立，而 activeSessions==0 会被
            // AgentObservability 翻成「无本地明细」，`status` / `doctor` 的 JSON 跟着印 0。
            let countAfter = monitor.activeSessionCounts(for: [root.path])[root.path]
            try expectEqual(countAfter, countBefore,
                            "会话数不许比活动日期更武断：没看到就是没看到")
        }

        TestKit.test("目录扫描: 连续缺失到终态时，宽限保留的会话数也要收尾") {
            // 上一条修的是「读不到就清零」；这条守另一端：保留不能永久冻住。
            // 目录真的没了（连续 3 趟 stat 失败），活动与计数必须一起退场，
            // 否则岛上会永远挂着最后一次看到的会话数——那时「没看到」已经攒成「永远看不到」。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let session = root.appendingPathComponent("s1")
            try FileManager.default.createDirectory(at: session, withIntermediateDirectories: true)
            try Data("x".utf8).write(to: session.appendingPathComponent("transcript.jsonl"))

            let monitor = FileActivityMonitor(maxDepth: 4, scanMinInterval: 0)
            monitor.setWorkingWindow(60)
            monitor.watch(dirs: [root.path])
            monitor.setRunningDirs([root.path])
            monitor.scanSync()
            try expectEqual(monitor.activeSessionCounts(for: [root.path])[root.path], 1,
                            "前置：首扫看到 1 个活跃会话")

            try FileManager.default.removeItem(at: root)
            monitor.scanSync()
            monitor.scanSync()
            try expectEqual(monitor.activeSessionCounts(for: [root.path])[root.path], 1,
                            "第 1、2 趟缺失仍在宽限期内，不许提前清零")
            monitor.scanSync()
            try expectNil(monitor.lastWriteDates(for: [root.path])[root.path], "终态：活动清零")
            try expectNil(monitor.activeSessionCounts(for: [root.path])[root.path],
                          "终态：会话数跟着活动一起清零，不是留在旧值上")
        }

        TestKit.test("尾读: 打不开的文件回吐 unreadable，空文件回吐「真的没有」") {
            // 两者都回 [] 的话，岛显示待机 + doctor 说「结论可信」，而文件明明在那里读不出
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: root) }
            let locked = root.appendingPathComponent("locked.jsonl")
            try "content\n".write(to: locked, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: locked.path)
            defer { try? FileManager.default.setAttributes([.posixPermissions: 0o644],
                                                           ofItemAtPath: locked.path) }
            let blocked = LogTailReader.readChecked(from: locked, maxLines: 8, maxBytes: 4096)
            try expectTrue(blocked.unreadable, "权限拒绝必须是「没看到」，不是「没有」")
            try expectTrue(blocked.lines.isEmpty, "读不到就是没有行")

            let empty = root.appendingPathComponent("empty.jsonl")
            try "".write(to: empty, atomically: true, encoding: .utf8)
            let blank = LogTailReader.readChecked(from: empty, maxLines: 8, maxBytes: 4096)
            try expectFalse(blank.unreadable, "空文件是「真的没有内容」，不该报失败")

            let good = root.appendingPathComponent("good.jsonl")
            try "a\nb\n".write(to: good, atomically: true, encoding: .utf8)
            let read = LogTailReader.readChecked(from: good, maxLines: 8, maxBytes: 4096)
            try expectFalse(read.unreadable, "正常读取不报失败")
            try expectEqual(read.lines, ["a", "b"], "行内容照旧")
            try expectEqual(LogTailReader.read(from: good, maxLines: 8, maxBytes: 4096).count, 2,
                            "旧签名行为不变")
        }

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
            try Data("{}".utf8).write(to: root.appendingPathComponent("s1.json"))
            let monitor = FileActivityMonitor(scanMinInterval: 3600)
            monitor.scanSync()
            monitor.replaceWatchedDirs([root.path])
            monitor.scanSync()
            try expectTrue(monitor.lastWriteDates(for: [root.path])[root.path] != nil)
            try expectEqual(monitor.latestActivityFiles(for: [root.path])[root.path]?.lastPathComponent,
                            "s1.json", "状态语义检查应复用扫描器定位的最新文件")
            monitor.replaceWatchedDirs([])
            try expectTrue(monitor.lastWriteDates(for: [root.path]).isEmpty)
            try expectTrue(monitor.latestActivityFiles(for: [root.path]).isEmpty,
                           "移除监控目录时不得残留旧会话文件路由")
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
            // 非忽略的信号文件（旧 mtime）：F2 语义下 newest 仅由信号文件聚合，
            // 断言「新写入的 file-history/blobs 未被计入」因此仍可判定
            try Data("{}".utf8).write(to: root.appendingPathComponent("session").appendingPathComponent("a.jsonl"))
            try? FileManager.default.setAttributes([.modificationDate: old],
                                                   ofItemAtPath: root.appendingPathComponent("session").appendingPathComponent("a.jsonl").path)
            // 写入 a.jsonl 会刷新 session 目录 mtime——夹具语义是「最后真实工作在 5
            // 分钟前，其后只有噪声写入」，故目录 mtime 也要回拨旧值
            try? FileManager.default.setAttributes([.modificationDate: old],
                                                   ofItemAtPath: root.appendingPathComponent("session").path)

            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 6, window: 60, now: Date())
            try expectTrue(result.newest != nil && Date().timeIntervalSince(result.newest!) > 120,
                           "file-history/blobs 的新写入不应被计入最近活动")
            // session-a.jsonl（旧 mtime）超出 window → 不计活跃；新写入的 file-history/blobs
            // 在忽略子树内不计。此前该夹具无信号文件、目录 mtime 是唯一信号（F2 后改由文件聚合）
            try expectEqual(result.activeSessions, 0,
                            "仅缓包子树时不得计活跃会话（实际 \(result.activeSessions)）")
        }

        TestKit.test("忽略集: 深层嵌套忽略目录（node_modules/deep/pkg）仍被剪枝（basename 判定等价性）") {
            // isIgnoredActivityPath 从 pathComponents 全组件匹配改为 basename 匹配：
            // 依赖「忽略目录自身条目处已被 skipDescendants 剪枝」——本用例锁死该等价性。
            // 注意必须用非隐藏目录（.git 会被 .skipsHiddenFiles 直接跳过，判别力不足）
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let deep = root.appendingPathComponent("session/proj/node_modules/deep/pkg")
            try FileManager.default.createDirectory(at: deep, withIntermediateDirectories: true)
            try Data("packdata".utf8).write(to: deep.appendingPathComponent("index.js"))
            // index.js mtime 置于「未来」：若剪枝失效，它将成为 newest（未来时间戳），
            // 断言即可判别；剪枝生效时 newest 只能是 real.jsonl（现在时间）
            try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(300)],
                                                  ofItemAtPath: deep.appendingPathComponent("index.js").path)

            let fresh = root.appendingPathComponent("session/real.jsonl")
            try Data("{}".utf8).write(to: fresh)

            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 6, window: 60, now: Date())
            try expectTrue(result.newest != nil, "有 fresh 写入必须产出 newest")
            try expectTrue(result.newest! <= Date().addingTimeInterval(5),
                            "剪枝生效时 newest 只能是 real.jsonl（未来 mtime 的 index.js 若被计入即等价性破坏）")
        }
    }

    private static let readers: [(Int, (URL, Int) -> [String])] = [
        (16_384, { AgentActionInspector.readLastLines(from: $0, maxLines: $1) }),
        (65_536, { AgentLogStreamer.readLastLines(from: $0, maxLines: $1) }),
    ]

    // MARK: - 会话树 newestFile（防循环 + 预算，共享实现）

    /// 构造临时会话树并注入符号链接循环：root/link → root（枚举器若无防护将无限递归）
    private static func makeLoopedTree() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root.appendingPathComponent("session"),
                                                withIntermediateDirectories: true)
        try Data("{}".utf8).write(to: root.appendingPathComponent("session/a.jsonl"))
        try Data("{}".utf8).write(to: root.appendingPathComponent("session/b.jsonl"))
        try FileManager.default.createSymbolicLink(
            at: root.appendingPathComponent("loop"),
            withDestinationURL: root)
        return root
    }

    private static func newestMtime(_ root: URL) throws -> Date {
        let a = try root.appendingPathComponent("session/a.jsonl").resourceValues(forKeys: [.contentModificationDateKey])
        return a.contentModificationDate!
    }

    static func registerTreeTests() {
        TestKit.test("文件监控: 噪声文件写入不得经父目录 mtime 传播进 newest（R33/F2）") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let session = root.appendingPathComponent("session-a")
            try FileManager.default.createDirectory(at: session, withIntermediateDirectories: true)
            // 产物文件：旧 mtime（真实的最近工作信号）
            let artifact = session.appendingPathComponent("artifact.jsonl")
            try Data("{}".utf8).write(to: artifact)
            let old = Date().addingTimeInterval(-1800)
            try FileManager.default.setAttributes([.modificationDate: old],
                                                  ofItemAtPath: artifact.path)
            // 噪声文件：刚刚写入（会刷新 session-a 目录 mtime）
            try Data("lock".utf8).write(to: session.appendingPathComponent(".task.lock"))

            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4, window: 60, now: Date())
            try expectTrue(result.newest != nil, "产物文件存在必须有 newest")
            let newest = result.newest!
            try expectTrue(abs(Date().timeIntervalSince(newest) - 1800) < 30,
                            "newest 必须是产物文件的旧 mtime，不得被噪声文件的父目录 mtime 抬到 now（实际 \(newest)）")
        }

        TestKit.test("文件监控: 监控目录永久删除后幽灵活动终态清零（R33/F3）") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root.appendingPathComponent("tasks"), withIntermediateDirectories: true)
            try Data("{}".utf8).write(to: root.appendingPathComponent("tasks/s1.json"))
            let watchDir = root.appendingPathComponent("tasks").path
            let monitor = FileActivityMonitor(scanMinInterval: 0)
            monitor.watch(dirs: [watchDir])
            monitor.scanSync()
            try expectTrue(monitor.lastWriteDates(for: [watchDir])[watchDir] != nil, "前置：活动已缓存")

            // 永久删除目录
            try FileManager.default.removeItem(at: root.appendingPathComponent("tasks"))
            for _ in 0..<3 { monitor.scanSync() }
            try expectNil(monitor.lastWriteDates(for: [watchDir])[watchDir],
                          "连续 3 趟缺失必须终态清零（幽灵活动不得残留）")
        }

        TestKit.test("文件监控: 配置变更后的首扫绕过节流（R33/F4）") {
            let base = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            let oldDir = base.appendingPathComponent("old"); let newDir = base.appendingPathComponent("new")
            try FileManager.default.createDirectory(at: oldDir, withIntermediateDirectories: true)
            try FileManager.default.createDirectory(at: newDir, withIntermediateDirectories: true)
            try Data("{}".utf8).write(to: oldDir.appendingPathComponent("a.json"))
            defer { try? FileManager.default.removeItem(at: base) }
            let monitor = FileActivityMonitor(scanMinInterval: 3600)   // 节流拉满，专测豁免路径
            monitor.watch(dirs: [oldDir.path])
            monitor.scanSync()
            try expectTrue(monitor.lastWriteDates(for: [oldDir.path])[oldDir.path] != nil, "前置：旧目录已扫")

            // 配置变更：切到新目录（invalidateScan → 代际前进）并写入新产物
            monitor.replaceWatchedDirs([newDir.path])
            try Data("{}".utf8).write(to: newDir.appendingPathComponent("b.json"))
            monitor.scanSync()
            try expectTrue(monitor.lastWriteDates(for: [newDir.path])[newDir.path] != nil,
                            "配置变更后的首扫必须绕过节流立即落地（此前空白到引擎下一拍）")
        }

        TestKit.test("会话树: newestFile 命中预算内最新文件且不被符号链接循环挂死") {
            let root = try makeLoopedTree()
            defer { try? FileManager.default.removeItem(at: root) }
            let start = Date()
            let found = LogTailReader.newestFile(in: root, maxAge: 3600)
            try expectTrue(found != nil, "存在常规文件必须命中")
            try expectTrue(["a.jsonl", "b.jsonl"].contains(found!.lastPathComponent),
                            "命中同 mtime 相邻写入的两个文件之一即可（枚举序不定）")
            try expectTrue(Date().timeIntervalSince(start) < 5.0,
                            "含循环的树必须在秒级内返回（无防护会无限枚举）")
        }

        TestKit.test("会话树: newestFile 遵守 maxAge 与条目预算") {
            let root = try makeLoopedTree()
            defer { try? FileManager.default.removeItem(at: root) }
            let newest = try newestMtime(root)
            // 两个文件全部回拨到 maxAge 之外
            for name in ["a.jsonl", "b.jsonl"] {
                try FileManager.default.setAttributes([.modificationDate: newest.addingTimeInterval(-7200)],
                                                      ofItemAtPath: root.appendingPathComponent("session/\(name)").path)
            }
            try expectNil(LogTailReader.newestFile(in: root, maxAge: 3600),
                          "全部文件早于 maxAge 时返回 nil")
            try expectTrue(LogTailReader.newestFile(in: root, maxAge: 3600 + 7201) != nil,
                            "放宽 maxAge 后应命中回拨后的文件")
            // 预算=2：会话目录下 2 个文件 + 1 个链接条目，预算足够命中；预算=1 可能命中
            // 任一首条——只断言「正常返回或 nil」，绝不断言挂死或崩溃
            let budgeted = LogTailReader.newestFile(in: root, maxAge: 3600 + 7201, maxEntries: 1)
            try expectTrue(budgeted == nil || budgeted!.lastPathComponent == "a.jsonl",
                            "预算截断应安全返回")
        }

        // R37：实测噪声清单——每个名字都对应「应用仅被打开」时真实发生的写入
        // （Antigravity 20 分钟 36 次、ChatGPT 的 Sparkle appcast、SQLite -shm 周期触碰）。
        // 清单与实现同一口径：这里断言必须为「噪声文件」，防实现侧被误删/改写。
        TestKit.test("文件监控: 浏览器内核状态文件与应用账号文件都算噪声（R37）") {
            let noiseNames = [
                "Network Persistent State", "DevToolsActivePort", "DIPS", "DIPS-wal",
                "SharedStorage", "SharedStorage-wal", "Trust Tokens", "Trust Tokens-journal",
                "SingletonLock", "SingletonCookie", "SingletonSocket",
                "Preferences", "Secure Preferences", "Local State",
                "Cookies", "Cookies-journal", "History", "Web Data", "Login Data",
                "TransportSecurity", "NetworkActionPredictor", "Quota Manager",
                "First Run", "Last Version", "Variations", "BrowserMetrics-spare.pma",
                "oauth_credentials.json", "app_storage.json",
                "production-appcast-bootstrap.json",
                "ed845c00-a13c-47a8-9929-f32ec0e06745.db-shm",
            ]
            for name in noiseNames {
                let url = URL(fileURLWithPath: "/tmp/\(name)")
                try expectTrue(FileActivityMonitor.isHeartbeatOrNoiseFile(url),
                                "\(name) 必须判为噪声文件")
            }
            // 反例：任务产物绝不能被噪声规则误吞
            for name in ["transcript.jsonl", "state.json", "session.sqlite", "history.jsonl",
                         "ed845c00-a13c-47a8-9929-f32ec0e06745.db-wal", "messages.json"] {
                let url = URL(fileURLWithPath: "/tmp/\(name)")
                try expectTrue(!FileActivityMonitor.isHeartbeatOrNoiseFile(url),
                                "\(name) 是任务产物，不得判为噪声")
            }
        }

        TestKit.test("文件监控: -shm 触碰与浏览器缓存子树不得顶起 newest（R37）") {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let fm = FileManager.default
            let old = Date().addingTimeInterval(-7200)

            // 会话库 + 真实任务产物（旧 mtime，代表最后一次真实工作）
            let conversations = root.appendingPathComponent("conversations")
            try fm.createDirectory(at: conversations, withIntermediateDirectories: true)
            for name in ["c1.db", "c1.db-wal", "transcript.jsonl"] {
                let f = conversations.appendingPathComponent(name)
                try Data("{}".utf8).write(to: f)
                try fm.setAttributes([.modificationDate: old], ofItemAtPath: f.path)
            }

            // 空闲触碰：-shm 被周期性刷新（刚刚）。
            // 目录 mtime 回拨到旧值：真实场景是「触碰已存在文件」，不会改动父目录
            // mtime（只有新建/删除条目才会），这里还原该时序以免把创建动作算进断言
            try Data(String(repeating: "\0", count: 32).utf8)
                .write(to: conversations.appendingPathComponent("c1.db-shm"))
            try fm.setAttributes([.modificationDate: old], ofItemAtPath: conversations.path)

            // 浏览器内核用户数据子树：Cache / Code Cache / Session Storage 全新鲜
            for rel in ["Cache/Cache_Data/data_1", "Code Cache/js/1", "Session Storage/000003.log",
                        "GPUCache/data_1", "Local Storage/leveldb/CURRENT", "blob_storage/1"] {
                let f = root.appendingPathComponent(rel)
                try fm.createDirectory(at: f.deletingLastPathComponent(), withIntermediateDirectories: true)
                try Data("x".utf8).write(to: f)
            }

            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4, window: 60, now: Date())
            try expectTrue(result.newest != nil, "旧会话产物存在必须有 newest")
            let newest = result.newest!
            try expectTrue(Date().timeIntervalSince(newest) > 3600,
                            "newest 必须停在真实产物的旧 mtime：-shm 与缓存子树写入不得算活动"
                            + "（实际距今 \(Int(Date().timeIntervalSince(newest)))s）")
            try expectEqual(result.activeSessions, 0,
                            "缓存子目录不得被计成活跃会话（此前 Antigravity 空闲时虚报 8 个）")
        }

        TestKit.test("文件监控: 深度上限层的目录照常参与判定，只是不再下探（steps/<n>/output.txt 夹具）") {
            // R38/P1 剪枝的等价性证明。scanTree 现在对 level == maxDepth 的目录直接
            // skipDescendants（实测 ~/.gemini/antigravity/brain 19,490 趟 → 10,756 趟、
            // 218ms → 74ms）。这一层今天本来就读不到任何东西：子项落在 maxDepth+1，
            // 一律被深度守卫丢弃——所以剪的是「 opendir 一趟空转」，不是任何判定输入。
            // 夹具同时锁死两头：
            // · level-4 的 steps/<n> 目录**自身**的 mtime 仍是 activeTops 的合法输入
            //   （`level > top` 分支）→ 谁要是按目录名把 steps 子树整个剪掉，本用例即红；
            // · level-5 的 output.txt 从来不可见 → 谁要是放开深度守卫，newest 会被它抬走。
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at: root) }
            let fm = FileManager.default
            let old = Date().addingTimeInterval(-3000)
            let fresh = Date().addingTimeInterval(-5)
            let future = Date().addingTimeInterval(600)
            // 真实产物：level-2 文件，旧 mtime —— newest 只能是它
            let artifact = root.appendingPathComponent("sess-a/notes.jsonl")
            try fm.createDirectory(at: artifact.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("{}".utf8).write(to: artifact)
            // Antigravity 形态：sess-a(1) / .system_generated(2) / steps(3) / <n>(4) / output.txt(5)
            for (step, mtime) in [("1", fresh), ("2", old)] {
                let dir = root.appendingPathComponent("sess-a/.system_generated/steps/\(step)")
                try fm.createDirectory(at: dir, withIntermediateDirectories: true)
                let payload = dir.appendingPathComponent("output.txt")
                try Data("payload".utf8).write(to: payload)
                // 深层载荷写成「未来 mtime」：一旦被枚举到就会顶掉 newest，剪枝失效即刻露馅
                try fm.setAttributes([.modificationDate: future], ofItemAtPath: payload.path)
                try fm.setAttributes([.modificationDate: mtime], ofItemAtPath: dir.path)
            }
            // 建目录/写文件都会刷新祖先目录 mtime，统一回拨：唯一的「新鲜事物」只有 steps/1
            for path in [root.path, root.appendingPathComponent("sess-a").path,
                         root.appendingPathComponent("sess-a/.system_generated").path,
                         root.appendingPathComponent("sess-a/.system_generated/steps").path,
                         artifact.path] {
                try fm.setAttributes([.modificationDate: old], ofItemAtPath: path)
            }

            let now = Date()
            let result = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4, window: 60, now: now)
            try expectEqual(result.activeSessions, 1,
                            "steps/1 目录自身的新鲜 mtime 必须仍把 sess-a 记成活跃会话（level-4 目录参与判定）")
            try expectTrue(result.newest != nil && abs(result.newest!.timeIntervalSince(old)) < 2,
                           "newest 只能是 level-2 的 notes.jsonl，level-5 的 output.txt 从来不可见（实际 \(String(describing: result.newest))）")
            try expectEqual(result.newestFile?.lastPathComponent, "notes.jsonl")
            // 窗口收到 1s：steps/1 距今 5s → 不再活跃。这一支证明 activeSessions 真的由
            // 那层目录的 mtime 决定，而不是夹具里某处恒真的巧合
            let narrow = FileActivityMonitor.scanTree(in: root.path, maxDepth: 4, window: 1, now: now)
            try expectEqual(narrow.activeSessions, 0, "窗口收紧后唯一的新鲜源退出")
            try expectEqual(narrow.newestFile?.lastPathComponent, "notes.jsonl", "newest 与窗口无关")
        }
    }
}

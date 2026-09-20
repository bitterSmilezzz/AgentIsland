import Foundation
import AppKit
@testable import AgentIslandCore

// MARK: - 多智能体深审回归锁（v0.0.93）
//
// 五个并行审计各报回一批问题。下面每一条都是**逐条复核确认为真**之后的回归锁：
// 复核方式是把修复改回去跑一遍，测试必须变红（做过变异验证的才留在这里）。
// 审计报回但经复核为假的（例如「无锁字典」其实有 NSLock）不进这里。

/// 记录 snapshot() 调用次数的假进程表提供者
/// ——「详情页每次渲染都另开一次全表扫描」这类问题只能靠计数断言钉住
final class CountingProcessProvider: ProcessProviding, @unchecked Sendable {
    private let lock = NSLock()
    private var calls = 0
    private var mainThreadCalls = 0
    private let names: Set<String>

    init(names: Set<String>) { self.names = names }

    var snapshotCalls: Int {
        lock.lock(); defer { lock.unlock() }
        return calls
    }

    /// 落在主线程上的全表扫描次数（后台采样路径断言用）
    var mainThreadSnapshotCalls: Int {
        lock.lock(); defer { lock.unlock() }
        return mainThreadCalls
    }

    func snapshot() -> ProcessSnapshot {
        lock.lock()
        calls += 1
        if Thread.isMainThread { mainThreadCalls += 1 }
        lock.unlock()
        return ProcessSnapshot(entries: names.map {
            ProcessSnapshot.Entry(pid: 777, path: "/Applications/\($0).app/Contents/MacOS/\($0)",
                                  basename: $0.lowercased(), cpuPercent: 0, rssBytes: 1_000_000)
        })
    }

    func runningBundleIDs() -> Set<String> { [] }
}

enum HardeningTests {
    @MainActor
    static func register() {
        // MARK: 1. shell / AppleScript 转义（注入面）

        TestKit.test("ShellQuoting: 单引号词逐值锁定（含注入样本与空格路径）") {
            try expectEqual(ShellQuoting.shellWord("/tmp/proj"), #"'/tmp/proj'"#)
            // 空格路径在修复前连 `cd` 都会失败（词被拆开）
            try expectEqual(ShellQuoting.shellWord("/tmp/my proj"), #"'/tmp/my proj'"#)
            try expectEqual(ShellQuoting.shellWord("/tmp/a'b"), #"'/tmp/a'\''b'"#)
            try expectEqual(ShellQuoting.shellWord("/tmp\"; touch /tmp/pwned; echo \""),
                            ##"'/tmp"; touch /tmp/pwned; echo "'"##)
            try expectEqual(ShellQuoting.shellWord("$(id)`x`&&|;"), #"'$(id)`x`&&|;'"#)

            // 结构性不变式：结果永远是**一个**词——内部不得出现裸单引号
            for hostile in ["/x; rm -rf /", "/x && curl evil", "/x | tee", "/x $(id)",
                            "/x `id`", "/x > /etc/y", "/x*?", "/x\ny"] {
                let word = ShellQuoting.shellWord(hostile)
                try expectTrue(word.hasPrefix("'") && word.hasSuffix("'") && word.count >= 2,
                               "未整体加引号：\(word)")
                try expectFalse(String(word.dropFirst().dropLast()).contains("'"),
                                "词内出现未转义单引号，shell 会在此闭合词：\(word)")
            }
        }

        TestKit.test("ShellQuoting: AppleScript 字面量转义与两层组合顺序") {
            try expectEqual(ShellQuoting.appleScriptLiteral("a\"b"), "a\\\"b")
            try expectEqual(ShellQuoting.appleScriptLiteral("a\\b"), "a\\\\b")
            try expectEqual(ShellQuoting.appleScriptLiteral("a\nb"), "a\\nb")
            try expectEqual(ShellQuoting.appleScriptLiteral("a\tb"), "a\\tb")
            try expectEqual(ShellQuoting.appleScriptLiteral("正常路径/中文"), "正常路径/中文")
            // 顺序必须是 shell 词在前、AppleScript 转义在后：
            // '\'' 里的反斜杠若先被吞，shell 就会在引号外收到一个裸引号
            try expectEqual(ShellQuoting.appleScriptLiteral(ShellQuoting.shellWord("/x'y")),
                            #"'/x'\\''y'"#)
        }

        TestKit.test("结构: 任何 shell / AppleScript 出口都必须过 ShellQuoting") {
            // 两处 sink 的成因都是「只转义了双引号」。新增第三处时这条会拦住裸插值
            var offenders: [String] = []
            for (name, text) in try requireSources() {
                for (index, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                    let trimmed = line.trimmingCharacters(in: .whitespaces)
                    guard trimmed.contains("do script") || trimmed.contains("do shell script") else { continue }
                    if trimmed.hasPrefix("//") || trimmed.hasPrefix("///") { continue }   // 注释里提到不算
                    if !trimmed.contains("ShellQuoting") {
                        offenders.append("\(name):\(index + 1) \(trimmed.prefix(70))")
                    }
                }
            }
            try expectTrue(offenders.isEmpty,
                           "裸插值进 shell/AppleScript（外部库里的目录可携带 ; | $()）："
                           + offenders.joined(separator: " ; "))
        }

        TestKit.test("ShellQuoting: 拼出的 AppleScript 必须真能编译（含控制字符路径）") {
            // NSAppleScript(source:) 编译失败返回 nil，而两个调用点都是 `?` / `if let` 静默跳过
            // ——脚本编译不过的症状不是报错，而是「按钮按了没反应」。
            // 第一版把 0x00–0x1F 之外的控制字符写成 \u{1b}，实测 AppleScript 并不认这个转义
            for hostile in ["/tmp/ok", "/tmp/a'b", "/tmp/a\\b", "/tmp/a\"b", "/tmp/a\nb",
                            "/tmp/a\tb", "/tmp/a\u{1b}b", "/tmp/a\u{0c}b", "/tmp/中文 🎉",
                            "/tmp/x; rm -rf /", "/tmp/$(id)", "-leading-dash"] {
                let cmd = "cd -- " + ShellQuoting.shellWord(hostile)
                let source = "tell application \"Terminal\"\nactivate\n"
                    + "do script \"\(ShellQuoting.appleScriptLiteral(cmd))\"\nend tell"
                try expectTrue(NSAppleScript(source: source) != nil,
                               "脚本编译失败：路径 \(hostile.debugDescription)")
            }
        }

        // MARK: 2. SQLite 绑定生命周期

        TestKit.test("结构: sqlite 文本绑定必须用 TRANSIENT 析构器") {
            // 传 nil（= SQLITE_STATIC）时 SQLite 不复制：Swift 桥出的 C 缓冲区只在 bind
            // 那一行有效，而 step 在其后——读到的是已回收内存，轻则查错会话重则崩溃
            var offenders: [String] = []
            for (name, text) in try requireSources() {
                for (index, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
                    let trimmed = line.trimmingCharacters(in: .whitespaces)
                    guard trimmed.contains("sqlite3_bind_text("), !trimmed.hasPrefix("//") else { continue }
                    if !trimmed.contains("transientDestructor") {
                        offenders.append("\(name):\(index + 1) \(trimmed.prefix(70))")
                    }
                }
            }
            try expectTrue(offenders.isEmpty,
                           "sqlite3_bind_text 未用 ReadonlyDB.transientDestructor："
                           + offenders.joined(separator: " ; "))
        }

        // MARK: 3. 外部时间戳饱和

        TestKit.test("Cline 时间戳: 越界/Inf/NaN 不得在采样拍 trap") {
            // ts 直接来自第三方 ui_messages.json；Int64(1e30) 是运行时 trap，
            // 触发一次就每 2s 崩一回（岛直接消失），而该函数跑在 @MainActor 采样链上
            for hostile: Double in [1e30, -1e30, .infinity, -.infinity, .nan, -9.2e18, 1e19] {
                let messages: [[String: Any]] = [
                    ["type": "ask", "ask": "command", "text": "ls -la", "ts": hostile]
                ]
                let signal = AgentSessionInspector.detectClineOrRoo(messages: messages, fileAge: 0)
                guard case .attention(let request)? = signal else {
                    throw TestError(message: "ts=\(hostile) 应识别为待确认，实际 \(String(describing: signal))")
                }
                try expectTrue(request.fingerprint.hasPrefix("cline-"),
                               "指纹格式漂移：\(request.fingerprint)")
                let digits = request.fingerprint.dropFirst("cline-".count)
                try expectTrue(Int(digits) != nil, "指纹里的时间戳不是可解析整数：\(digits)")
                try expectTrue(abs(Int(digits) ?? 0) <= SafeNumber.magnitudeCeiling,
                               "ts=\(hostile) 未被饱和钳制，仍会 trap：\(digits)")
            }
            // 正常毫秒时间戳必须原样保留（钳制不能顺手吃掉真值）
            let real: [[String: Any]] = [["type": "ask", "ask": "command", "text": "ls",
                                          "ts": 1_750_000_000_000.0]]
            if case .attention(let request)? = AgentSessionInspector.detectClineOrRoo(messages: real, fileAge: 0) {
                try expectEqual(request.fingerprint, "cline-1750000000000")
            } else {
                throw TestError(message: "正常 ts 也应识别为待确认")
            }
        }

        // MARK: 4. 休眠/唤醒联动

        TestKit.test("休眠唤醒: 唤醒后必须重新拉起 token 轮询") {
            let token = FakeTokenUsageProvider()
            let engine = EngineTests.makeEngine(processNames: ["DimAgent"], writes: [:],
                                                tokenMonitor: token)
            engine.start()
            engine.setPresentationActive(true)
            let startedBefore = token.startCount
            try expectTrue(startedBefore >= 1, "前置不成立：展开态未启动轮询，本测试将失去意义")

            engine.handleSystemSleep()
            try expectEqual(token.pauseCount, 1, "休眠应暂停轮询")

            engine.handleSystemWake()
            try expectTrue(token.startCount > startedBefore,
                           "唤醒后没有重新 start()：handleSystemSleep 只 pause 未复位 "
                           + "tokenPollingStarted，startTokenPollingIfNeeded 被 guard 挡回，"
                           + "展开态合盖再开盖后面板 Token 数字永久停更")
            engine.stop()
        }

        // MARK: 5. 详情页进程树不再另开全表扫描

        TestKit.test("进程树: 复用上一拍的进程表，body 不再触发全表 sysctl") {
            let provider = CountingProcessProvider(names: ["DimAgent"])
            let engine = EngineTests.makeEngine(processNames: [], writes: [:],
                                                processMonitor: provider)
            _ = engine.sample(now: Date())
            let afterSample = provider.snapshotCalls
            try expectTrue(afterSample >= 1, "前置不成立：采样未取进程表")

            let report = engine.inspectProcessTree(agentId: "dim")
            // 先确认树非空，否则下面的「调用次数不变」会因为函数根本没活干而假绿
            try expectTrue(report != nil, "前置不成立：进程树为空，计数断言将失去意义")
            _ = engine.inspectProcessTree(agentId: "dim")
            _ = engine.inspectProcessTree(agentId: "dim")
            try expectEqual(provider.snapshotCalls, afterSample,
                            "inspectProcessTree 又当场扫全表了：详情页 body 每拍要付两次 "
                            + "sysctl(KERN_PROC_ALL)（≈600 条 kinfo_proc）+ 每 pid 一次 proc_pid_rusage，"
                            + "全落主线程并与采样拍互相消费 CPU 差分窗口")
        }

        // MARK: 6. 只读库线程契约

        TestKit.test("结构: 后台队列不得走跨 SQL 持锁的缓存连接") {
            // withConnection 的锁跨整段 SQL 持有（不可重入，且保证 invalidate 不关正在用的句柄）。
            // 实时流水在后台队列刷新，若共用它，一次 500 行扫描就能堵住主线程那一拍
            let all = try requireSources()
            guard let (_, text) = all.first(where: { $0.name == "AgentLogStreamer.swift" }) else {
                throw TestError(message: "没扫到 AgentLogStreamer.swift，断言无从执行")
            }
            let cached = text.components(separatedBy: "ReadonlyDB.withConnection(").count - 1
            try expectEqual(cached, 0,
                            "流水页（后台队列）又用回了缓存连接：与主线程采样共用一把跨 SQL 的锁")
            let dedicated = text.components(separatedBy: "ReadonlyDB.withDedicatedConnection(").count - 1
            try expectTrue(dedicated >= 5, "流水源应各走一条专用连接（实得 \(dedicated) 处）")
        }
    }

    // MARK: - 源码扫描（与「债务棘轮」同一手法：从测试文件位置回溯到 Sources）

    /// 源码清单。取不到就抛——结构类断言在「扫了个空」时静默通过，等于给自己发假绿证
    private static func requireSources() throws -> [(name: String, text: String)] {
        let found = repoSwiftSources()
        try expectTrue(found.count > 20,
                       "未能定位仓库 Sources 目录（只扫到 \(found.count) 个文件），"
                       + "本测试的「通过」没有意义")
        return found
    }

    private static func repoSwiftSources() -> [(name: String, text: String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // Tests/AgentIslandTestsRunner
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // repo root
            .appendingPathComponent("Sources")
        guard let en = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil) else { return [] }
        return en.compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
            .compactMap { url in
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { return nil }
                return (url.lastPathComponent, text)
            }
    }
}

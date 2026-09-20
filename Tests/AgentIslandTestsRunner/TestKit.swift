import Foundation
 import AgentIslandCore

// MARK: - 极简测试框架（无 Xcode 环境，零依赖）

struct TestCase {
    let name: String
    let body: @MainActor () throws -> Void
}

enum TestKit {
    static var tests: [TestCase] = []
    static var passed = 0
    static var failed = 0
    static var failures: [String] = []

    @MainActor
    static func test(_ name: String, _ body: @escaping @MainActor () throws -> Void) {
        tests.append(TestCase(name: name, body: body))
    }

    @MainActor
    static func runAll() -> Int32 {
        print("AgentIsland 测试套件 — \(tests.count) 个用例\n")
        for t in tests {
            do {
                try t.body()
                passed += 1
                print("  ✅ \(t.name)")
            } catch let error as TestError {
                failed += 1
                failures.append("\(t.name): \(error.message)")
                print("  ❌ \(t.name) — \(error.message)")
            } catch {
                failed += 1
                failures.append("\(t.name): \(error)")
                print("  ❌ \(t.name) — \(error)")
            }
        }
        print("\n结果: \(passed) 通过, \(failed) 失败")
        if failed > 0 {
            print("失败明细:")
            for f in failures {
                print("  - \(f)")
            }
        }
        return failed == 0 ? 0 : 1
    }
}

struct TestError: Error, CustomStringConvertible {
    let message: String
    var description: String { message }
}

// MARK: - 断言

func expect(_ condition: Bool, _ message: String = "断言失败") throws {
    if !condition {
        throw TestError(message: message)
    }
}

func expectEqual<T: Equatable>(_ actual: T, _ expected: T, _ label: String = "") throws {
    if actual != expected {
        throw TestError(message: "\(label) 期望 [\(expected)] 实际 [\(actual)]")
    }
}

func expectTrue(_ value: Bool, _ label: String = "") throws {
    if !value {
        throw TestError(message: "\(label) 期望 true")
    }
}

func expectFalse(_ value: Bool, _ label: String = "") throws {
    if value {
        throw TestError(message: "\(label) 期望 false")
    }
}

func expectNil<T>(_ value: T?, _ label: String = "") throws {
    if value != nil {
        throw TestError(message: "\(label) 期望 nil")
    }
}

func expectNotNil<T>(_ value: T?, _ label: String = "") throws {
    if value == nil {
        throw TestError(message: "\(label) 期望非 nil")
    }
}

// MARK: - async 桥（runner 的测试体是同步闭包，本仓无 XCTest 的 async 支持）

/// 在同步测试体内执行一段 @MainActor async 逻辑并取回结果。
/// 机制与套件里既有的「回调 + 小步 RunLoop」同构：Task 派发到 MainActor，
/// 本函数在 main run loop 上等它跑完（真异步 IO 也照常推进）。
/// 注意 body 内的 throw 穿不出 Task 边界，断言一律写在调用方。
@MainActor
func awaitOnMain<T: Sendable>(_ timeout: TimeInterval = 15,
                              _ body: @escaping @MainActor () async -> T) throws -> T {
    let cell = MainCell<T>()
    Task { @MainActor in cell.value = await body() }
    let deadline = Date().addingTimeInterval(timeout)
    while cell.value == nil {
        if Date() >= deadline {
            throw TestError(message: "async 测试体在 \(timeout)s 内未完成")
        }
        RunLoop.main.run(until: Date().addingTimeInterval(0.005))
    }
    guard let value = cell.value else { throw TestError(message: "async 测试体结果为 nil") }
    return value
}

/// awaitOnMain 的结果盒子：泛型函数内不能嵌套类型，故放文件作用域。
/// 值只在 MainActor 上串行读写，@unchecked 表的是「同 actor 内可变」而非真并发。
final class MainCell<T>: @unchecked Sendable {
    var value: T?
    init() {}
}

// MARK: - Token 监控假实现（引擎 token 速率链路测试用）

/// 可变 usage 的假 token 监控（引用语义）：测试先改 usage 再触发采样。
/// 仅由测试在主线程串行驱动，无并发。
final class FakeTokenUsageProvider: TokenUsagePolling, TokenUsageQuerying {
    var usage: [String: TokenUsage] = [:]
    var grandTotal: TokenUsage = TokenUsage()
    var onRefresh: (@MainActor () -> Void)?

    /// 生命周期调用计数（休眠/唤醒联动断言用）
    private(set) var startCount = 0
    private(set) var pauseCount = 0
    private(set) var stopCount = 0

    private(set) var refreshAsyncCount = 0
    private(set) var refreshSyncCount = 0

    func start(interval: TimeInterval) { startCount += 1 }
    func stop() { stopCount += 1 }
    func pause() { pauseCount += 1 }
    func refreshAsync() { refreshAsyncCount += 1 }
    func refreshSync() { refreshSyncCount += 1 }
    func modelBreakdown(agentId: String, completion: @escaping @MainActor ([ModelUsage]) -> Void) {
        Task { @MainActor in completion([]) }
    }
    func sessions(agentId: String, modelId: String, completion: @escaping @MainActor ([SessionUsage]) -> Void) {
        Task { @MainActor in completion([]) }
    }
}

// MARK: - 测试 UserDefaults 套件管理（R16 plist 泄漏根治）

/// 测试专用套件（登记制）：12 处散点 `UserDefaults(suiteName:)` + 各自
/// `removePersistentDomain` 的旧模式只解除注册——cfprefs 缓存会把域重建为
/// plist 文件，~/Library/Preferences 逐次累积（实测 1300+ 个）。
/// 现在统一登记，runAll 末尾 removePersistentDomain + removeItem 双保险，
/// 并自守护断言零残留。
enum TestDefaults {
    private static let lock = NSLock()
    /// 本 run 登记的套件名（清理后保留名单供泄漏断言）
    private static var names: [String] = []

    /// 创建登记制测试套件（UUID 隔离，用例间零共享）
    static func suite(_ label: String) -> UserDefaults {
        lock.lock(); defer { lock.unlock() }
        let name = "agentisland-test-\(label)-\(UUID().uuidString)"
        names.append(name)
        return UserDefaults(suiteName: name)!
    }

    /// runAll 末尾调用：移除全部登记套件并删除残留 plist（幂等）
    static func cleanupAll() {
        lock.lock(); defer { lock.unlock() }
        let fm = FileManager.default
        for name in names {
            if let d = UserDefaults(suiteName: name) {
                d.removePersistentDomain(forName: name)
            }
            let plist = fm.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Preferences/\(name).plist")
            try? fm.removeItem(at: plist)
        }
    }

    /// 自守护数据：登记套件中仍残留 plist 文件的数量（应在清理后为 0）
    static var leakedFiles: Int {
        lock.lock(); defer { lock.unlock() }
        let fm = FileManager.default
        return names.filter { name in
            fm.fileExists(atPath: fm.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Preferences/\(name).plist").path)
        }.count
    }
}

/// snapshot() 故意慢的假提供者：用来构造「后台采样还在飞、此时 stop()」的竞态窗口
final class SlowProcessProvider: ProcessProviding, @unchecked Sendable {
    private let names: Set<String>
    private let delay: TimeInterval

    init(names: Set<String>, delay: TimeInterval) {
        self.names = names
        self.delay = delay
    }

    func snapshot() -> ProcessSnapshot {
        Thread.sleep(forTimeInterval: delay)   // 后台队列，阻塞以撑开竞态窗口
        return ProcessSnapshot(entries: names.map {
            ProcessSnapshot.Entry(pid: 777, path: "/Applications/\($0).app/Contents/MacOS/\($0)",
                                  basename: $0.lowercased(), cpuPercent: 0, rssBytes: 1_000_000)
        })
    }

    func runningBundleIDs() -> Set<String> { [] }
}

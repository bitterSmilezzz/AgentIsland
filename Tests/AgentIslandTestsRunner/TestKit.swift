import Foundation

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

func expectNil<T>(_ value: T?, _ label: String = "") throws {
    if value != nil {
        throw TestError(message: "\(label) 期望 nil")
    }
}

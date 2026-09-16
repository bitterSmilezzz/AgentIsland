import Foundation
@testable import AgentIslandCore

// MARK: - DeepSeek Harness (DSH) 状态跟踪与长思考防误报回归测试

@MainActor
enum DSHTrackingTests {
    static let dsh = AgentRegistry.builtin.first { $0.id == "dsh" }!

    static func register() {
        TestKit.test("DSH投影缓存: 活跃轮次解析为 active 并携带执行动作与步数") {
            let tempDir = NSTemporaryDirectory() + "dsh-test-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: tempDir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: tempDir) }

            let jsonPath = "\(tempDir)/session-abc.json"
            let json = """
            {
              "record": {
                "identity": { "createdAt": 1789562930898 },
                "rows": {
                  "title": { "val": "重构状态跟踪引擎" },
                  "turnBoundary": { "val": { "openTurnStartSeq": 4, "lastTurn": 1 } },
                  "sessionStats": { "val": { "turns": 1, "steps": 36, "lastTurn": 1, "openStep": { "turn": 1, "step": 37 } } }
                }
              }
            }
            """
            try json.data(using: .utf8)!.write(to: URL(fileURLWithPath: jsonPath))

            let signal = AgentSessionInspector.inspectDSHSession(baseDir: tempDir, now: Date())
            guard case let .active(fingerprint, action)? = signal else {
                throw TestError(message: "活跃轮次必须返回 .active 强语义信号")
            }
            try expectEqual(fingerprint, "dsh-session-abc-turn1")
            try expectEqual(action, "执行中: 重构状态跟踪引擎 (第 37 步)")
        }

        TestKit.test("DSH投影缓存: 轮次结束解析为 completed 且保留步数指纹") {
            let tempDir = NSTemporaryDirectory() + "dsh-test-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: tempDir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: tempDir) }

            let jsonPath = "\(tempDir)/session-done.json"
            let json = """
            {
              "record": {
                "identity": { "createdAt": 1789562930898 },
                "rows": {
                  "title": { "val": "归档所有旧会话" },
                  "turnBoundary": { "val": { "openTurnStartSeq": null, "lastTurn": 1 } },
                  "sessionStats": { "val": { "turns": 1, "steps": 51, "lastTurn": 1, "openStep": null } }
                }
              }
            }
            """
            try json.data(using: .utf8)!.write(to: URL(fileURLWithPath: jsonPath))

            let signal = AgentSessionInspector.inspectDSHSession(baseDir: tempDir, now: Date())
            guard case let .completed(fingerprint)? = signal else {
                throw TestError(message: "结束轮次必须返回 .completed 信号")
            }
            try expectEqual(fingerprint, "dsh-session-done-t1-s51")
        }

        TestKit.test("DSH投影缓存: 审批请求准确解析为 attention") {
            let tempDir = NSTemporaryDirectory() + "dsh-test-\(UUID().uuidString)"
            try FileManager.default.createDirectory(atPath: tempDir, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(atPath: tempDir) }

            let jsonPath = "\(tempDir)/session-approval.json"
            let json = """
            {
              "record": {
                "identity": { "createdAt": 1789562930898 },
                "rows": {
                  "approval": { "val": { "id": "req-999", "toolName": "bash" } },
                  "turnBoundary": { "val": { "openTurnStartSeq": 5, "lastTurn": 1 } }
                }
              }
            }
            """
            try json.data(using: .utf8)!.write(to: URL(fileURLWithPath: jsonPath))

            let signal = AgentSessionInspector.inspectDSHSession(baseDir: tempDir, now: Date())
            guard case let .attention(req)? = signal else {
                throw TestError(message: "有审批请求时必须返回 .attention 信号")
            }
            try expectEqual(req.fingerprint, "req-999")
            try expectEqual(req.message, "等待你批准执行: bash")
        }

        TestKit.test("DSH事件解析: JSONL turn/end 与 approval/asked 协议扩展兼容") {
            let stepStart = #"{"type":"step/start","data":{"turn":1,"step":1}}"#
            let toolCall = #"{"type":"tool/call","data":{"name":"bash"}}"#
            let turnEnd = #"{"type":"turn/end","data":{"turn":1,"reason":{"kind":"completed"}}}"#
            let approvalAsked = #"{"type":"approval/asked","data":{"id":"appr-42","toolName":"exec"}}"#
            let approvalDecided = #"{"type":"approval/decided","data":{"id":"appr-42","outcome":"allowed-once"}}"#

            try expectNil(AgentSessionInspector.detect(lines: [stepStart, toolCall]),
                          "执行中步骤与工具调用不是完成也不是等待用户")

            guard case let .completed(fingerprint)? = AgentSessionInspector.detect(lines: [stepStart, turnEnd]) else {
                throw TestError(message: "turn/end 必须解析为完成态")
            }
            try expectFalse(fingerprint.isEmpty)

            guard case let .attention(req)? = AgentSessionInspector.detect(lines: [stepStart, approvalAsked]) else {
                throw TestError(message: "approval/asked 必须解析为等待审批")
            }
            try expectEqual(req.fingerprint, "appr-42")

            try expectNil(AgentSessionInspector.detect(lines: [stepStart, approvalAsked, approvalDecided]),
                          "approval/decided 之后等待审批必须解除")
        }

        TestKit.test("引擎: DSH 在长思考/无写入期间稳定保持 working 且绝不提前报完成") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.dsh/sessions"
            let files = FakeFileActivityProvider(writes: [dir: start])
            let engine = ActivityEngine(
                profiles: [dsh],
                config: EngineConfig(workingWindow: 60, minWorkingHold: 10),
                processMonitor: FakeProcessProvider(
                    processNames: ["dsh"],
                    bundleIDs: [],
                    entries: [ProcessSnapshot.Entry(pid: 57941, path: "/Users/user/workspace/deepseek-harness/dsh", basename: "dsh", cpuPercent: 0, rssBytes: 100_000_000)]
                ),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            var signal: AgentSessionSignal? = .active(fingerprint: "dsh-turn-1", action: "执行中: 长思考任务 (第 12 步)")
            engine.inspectSessionHook = { _, _, _ in signal }
            engine.inspectActionHook = { _, _, _, _ in nil }

            // 1. 刚进入工作
            let snap1 = engine.sample(now: start)
            try expectEqual(snap1.first?.level, .working)
            try expectEqual(snap1.first?.currentAction, "执行中: 长思考任务 (第 12 步)")
            try expectTrue(engine.anyWorking)

            // 2. 过去 90 秒（大模型深度思考中，无磁盘写入，CPU 空闲）
            // 写入时间停留在 90 秒前（远超 workingWindow 60 秒）
            files.writes = [dir: start]
            let deepThinkingTime = start.addingTimeInterval(90)
            let snap2 = engine.sample(now: deepThinkingTime)
            try expectEqual(snap2.first?.level, .working, "语义处于 active 状态时，即使 90s 无文件写入也必须保持 working")
            try expectNil(engine.latestEvent, "长思考中途严禁提前发送完成通知！")

            // 3. 过去 120 秒，模型思考完毕并正式输出 turn/end 完成检查点
            signal = .completed(fingerprint: "dsh-turn-1-done")
            let completeTime = start.addingTimeInterval(120)
            let snap3 = engine.sample(now: completeTime)
            try expectEqual(snap3.first?.level, .completed, "收到 completed 信号时转入完成态")
            try expectEqual(engine.latestEvent?.eventType, .completed, "真正完成时必须发送完成通知")
            try expectEqual(engine.latestEvent?.agentId, "dsh")

            // 4. 下一拍保持 completed，不重复通知
            engine.clearLatestEvent()
            let snap4 = engine.sample(now: completeTime.addingTimeInterval(2))
            try expectEqual(snap4.first?.level, .completed)
            try expectNil(engine.latestEvent, "同一完成标记不得重复发送通知")
        }
    }
}

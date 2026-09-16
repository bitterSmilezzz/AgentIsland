import Foundation
@testable import AgentIslandCore

// MARK: - Antigravity 状态跟踪与待机/工作中状态区分回归测试

@MainActor
enum AntigravityTrackingTests {
    static let antigravity = AgentRegistry.builtin.first { $0.id == "antigravity" }!

    static func register() {
        TestKit.test("Antigravity日志解析: 活跃工具调用解析为 active 并清洗动作") {
            let line1 = #"{"step_index":100,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"build app"}"#
            let line2 = #"{"step_index":101,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"toolAction":"Running swift build","CommandLine":"swift build"}}]}"#

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1, line2], fileAge: 10)
            guard case let .active(fingerprint, action)? = signal else {
                throw TestError(message: "活跃工具调用必须返回 .active 信号")
            }
            try expectEqual(fingerprint, "antigravity-step-101")
            try expectEqual(action, "运行: swift build")
        }

        TestKit.test("Antigravity日志解析: 思考规划中解析为 active") {
            let line1 = #"{"step_index":100,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"fix bugs"}"#
            let line2 = #"{"step_index":101,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","thinking":"Thinking deeply..."}"#

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1, line2], fileAge: 5)
            guard case let .active(fingerprint, action)? = signal else {
                throw TestError(message: "思考中必须返回 .active 信号")
            }
            try expectEqual(fingerprint, "antigravity-step-101")
            try expectEqual(action, "思考规划中")
        }

        TestKit.test("Antigravity日志解析: ask_question 弹窗提问解析为 attention") {
            let line1 = "{\"step_index\":105,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"ask_question\",\"args\":{\"questions\":\"[{\\\"question\\\":\\\"请选择重构方案：\\\"}]\"}}]}"

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1], fileAge: 10)
            guard case let .attention(req)? = signal else {
                throw TestError(message: "未答复的提问必须解析为 .attention")
            }
            try expectEqual(req.fingerprint, "antigravity-step-105")
            try expectEqual(req.message, "请选择重构方案：")
        }

        TestKit.test("Antigravity日志解析: 轮次完成 15 分钟内为 completed，超时自然转为待机 (nil)") {
            let lineDone = "{\"step_index\":120,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"content\":\"### 修复汇报\\n已全部修复完成。\"}"

            // 1. 完成 2 分钟内
            let signalRecent = AgentSessionInspector.detectAntigravitySession(lines: [lineDone], fileAge: 120)
            guard case let .completed(fingerprint)? = signalRecent else {
                throw TestError(message: "刚完成必须返回 .completed 信号")
            }
            try expectEqual(fingerprint, "antigravity-step-120")

            // 2. 完成超过 15 分钟 (900秒)
            let signalIdle = AgentSessionInspector.detectAntigravitySession(lines: [lineDone], fileAge: 901)
            try expectNil(signalIdle, "超过 15 分钟的已完成应自然转为待机 (nil)")
        }

        TestKit.test("Antigravity动作文案: 原生工具名称本土化汉化与 GENERIC 结果处理") {
            try expectEqual(AgentActionInspector.cleanAntigravityAction("run_command"), "执行终端命令")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("view_file"), "查看文件")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("replace_file_content"), "编辑文件代码")
            try expectEqual(AgentActionInspector.cleanAntigravityAction("grep_search"), "搜索代码正则")

            let line1 = "{\"step_index\":201,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"view_file\",\"args\":{\"toolAction\":\"Viewing SettingsView.swift\"}}]}"
            let line2 = "{\"step_index\":202,\"source\":\"MODEL\",\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"file contents...\"}"
            let signal = AgentSessionInspector.detectAntigravitySession(lines: [line1, line2], fileAge: 2)
            guard case let .active(_, action)? = signal else {
                throw TestError(message: "GENERIC 输出阶段必须保持 .active 状态")
            }
            try expectEqual(action, "查看: SettingsView.swift (处理中)")
        }

        TestKit.test("Antigravity进程匹配: 排除渲染器与辅助进程，仅匹配主应用") {
            let snapshot = ProcessSnapshot(entries: [
                ProcessSnapshot.Entry(
                    pid: 35910,
                    path: "/Applications/Antigravity.app/Contents/MacOS/Antigravity",
                    basename: "antigravity",
                    cpuPercent: 1.5,
                    rssBytes: 100_000_000
                ),
                ProcessSnapshot.Entry(
                    pid: 35927,
                    path: "/Applications/Antigravity.app/Contents/Frameworks/Antigravity Helper (Renderer).app/Contents/MacOS/Antigravity Helper (Renderer)",
                    basename: "antigravity helper (renderer)",
                    cpuPercent: 25.0,
                    rssBytes: 300_000_000
                ),
                ProcessSnapshot.Entry(
                    pid: 35925,
                    path: "/Applications/Antigravity.app/Contents/Resources/bin/language_server",
                    basename: "language_server",
                    cpuPercent: 12.0,
                    rssBytes: 200_000_000
                )
            ])
            let matcher = ProcessMatcher(
                snapshot: snapshot,
                runningBundleIDs: ["com.google.antigravity"]
            )
            let matched = matcher.matchingEntries(for: antigravity)
            // 必须只匹配主窗口进程，排除了 Helper (Renderer) 与 language_server
            try expectEqual(matched.count, 1)
            try expectEqual(matched.first?.pid, 35910)
            try expectEqual(matched.first?.cpuPercent, 1.5)
        }

        TestKit.test("引擎: Antigravity 空闲时正确判定为 IDLE，不被高 CPU 假阳性污染") {
            let start = Date()
            let dir = FileManager.default.homeDirectoryForCurrentUser.path + "/.gemini/antigravity/brain"
            let files = FakeFileActivityProvider(writes: [dir: start.addingTimeInterval(-600)]) // 10分钟前写入
            let engine = ActivityEngine(
                profiles: [antigravity],
                config: EngineConfig(workingWindow: 60, minWorkingHold: 10),
                processMonitor: FakeProcessProvider(
                    processNames: ["antigravity"],
                    bundleIDs: ["com.google.antigravity"],
                    entries: [
                        ProcessSnapshot.Entry(
                            pid: 35910,
                            path: "/Applications/Antigravity.app/Contents/MacOS/Antigravity",
                            basename: "antigravity",
                            cpuPercent: 1.5,
                            rssBytes: 100_000_000
                        )
                    ]
                ),
                fileMonitor: files,
                installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] })
            )
            // 无活跃会话信号
            engine.inspectSessionHook = { _, _, _ in nil }
            engine.inspectActionHook = { _, _, _, _ in nil }

            let snap = engine.sample(now: start)
            try expectEqual(snap.first?.level, .idle, "空闲且低 CPU 的主进程必须判定为 IDLE")
            try expectNil(snap.first?.currentAction, "空闲态下不得显示任何残留动作")
            try expectFalse(engine.anyWorking)
        }
    }
}

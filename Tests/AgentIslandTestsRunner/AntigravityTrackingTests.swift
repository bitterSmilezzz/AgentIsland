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

        TestKit.test("Antigravity日志解析: 已答复的 ask_question（后有 GENERIC）不得触发 attention 误报") {
            // 模拟真实场景：ask_question 后紧跟 GENERIC（用户已答复），再有后续工具调用
            let lineAsk = "{\"step_index\":105,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"ask_question\",\"args\":{\"questions\":\"[{\\\"question\\\":\\\"请选择重构方案：\\\"}]\"}}]}"
            let lineGeneric = "{\"step_index\":106,\"source\":\"TOOL\",\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"user selected option 1\"}"
            let lineWork = "{\"step_index\":107,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"run_command\",\"args\":{\"toolAction\":\"Building app\",\"CommandLine\":\"swift build\"}}]}"

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [lineAsk, lineGeneric, lineWork], fileAge: 10)
            // 最新 step 是 run_command，应为 active，不是 attention
            guard case .active? = signal else {
                throw TestError(message: "已答复的 ask_question 不得再触发 attention；应为 active（run_command 进行中）")
            }
        }

        TestKit.test("Antigravity会话探测: 统一入口 inspect 优先分派至 Antigravity 专有探测器，防止通用检测器误报") {
            let antigravity = AgentRegistry.builtin.first { $0.id == "antigravity" }!
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            let brainDir = URL(fileURLWithPath: "\(home)/.gemini/antigravity/brain")
            let subdirs = (try? FileManager.default.contentsOfDirectory(at: brainDir, includingPropertiesForKeys: [.contentModificationDateKey], options: [.skipsHiddenFiles])) ?? []

            var files: [URL] = []
            for sub in subdirs {
                let logFile = sub.appendingPathComponent(".system_generated/logs/transcript.jsonl")
                if FileManager.default.fileExists(atPath: logFile.path) {
                    files.append(logFile)
                }
            }

            let now = Date()
            let signalInspect = AgentSessionInspector.inspect(profile: antigravity, activityFiles: files, now: now)
            let signalDedicated = AgentSessionInspector.inspectAntigravitySession(now: now)
            
            // inspect 必须与专有解析器行为完全一致，且绝不得出现虚假的 attention 误报
            switch (signalInspect, signalDedicated) {
            case (.none, .none):
                break
            case let (.active(f1, _), .active(f2, _)):
                try expectEqual(f1, f2, "inspect 与专有探测器的活跃指纹必须一致")
            case let (.completed(f1), .completed(f2)):
                try expectEqual(f1, f2, "inspect 与专有探测器的完成指纹必须一致")
            case let (.attention(r1), .attention(r2)):
                try expectEqual(r1.fingerprint, r2.fingerprint, "inspect 与专有探测器的提问指纹必须一致")
            default:
                throw TestError(message: "inspect 派发结果 (\(String(describing: signalInspect))) 与专有解析器 (\(String(describing: signalDedicated))) 不一致")
            }
        }

        TestKit.test("Antigravity日志解析: 历史 ask_question 后仍有后续工作，最终完成后为 completed") {
            // ask_question → GENERIC → 后续工作 → 最终完成
            let lineAsk = "{\"step_index\":105,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"ask_question\",\"args\":{\"questions\":\"[{\\\"question\\\":\\\"选择方案？\\\"}]\"}}]}"
            let lineGeneric = "{\"step_index\":106,\"source\":\"TOOL\",\"type\":\"GENERIC\",\"status\":\"DONE\",\"content\":\"answered\"}"
            let lineDone = "{\"step_index\":120,\"source\":\"MODEL\",\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"content\":\"已完成全部改造。\"}"

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [lineAsk, lineGeneric, lineDone], fileAge: 30)
            guard case .completed? = signal else {
                throw TestError(message: "ask_question 已答复且最终有 content 回答，应解析为 completed")
            }
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

        TestKit.test("Antigravity后台任务: 启动后台任务后 PLANNER_RESPONSE 不得报 completed，保持 active 态") {
            let lineStart = #"{"step_index":200,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"arch -arm64 swift build"}}]}"#
            let lineBg = #"{"step_index":201,"source":"MODEL","type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: conv123/task-400\nTask Description: arch -arm64 swift build\nTask logs are available at: task-400.log"}"#
            let lineTurnEnd = #"{"step_index":202,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"已在后台启动编译任务，正在等待完成…"}"#

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [lineStart, lineBg, lineTurnEnd], fileAge: 10)
            guard case let .active(fingerprint, action)? = signal else {
                throw TestError(message: "后台任务运行中，PLANNER_RESPONSE 绝不得报 completed，必须返回 .active")
            }
            try expectEqual(fingerprint, "antigravity-bg-task-400-202")
            try expectEqual(action, "后台任务: swift build")
        }

        TestKit.test("Antigravity后台任务: 后台任务完成后，模型最终回答正常转换为 completed") {
            let lineStart = #"{"step_index":200,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"CommandLine":"arch -arm64 swift build"}}]}"#
            let lineBg = #"{"step_index":201,"source":"MODEL","type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: conv123/task-400\nTask Description: arch -arm64 swift build\nTask logs are available at: task-400.log"}"#
            let lineTurnEnd = #"{"step_index":202,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"已在后台启动编译任务，正在等待完成…"}"#
            let lineFinishMsg = #"{"step_index":203,"source":"SYSTEM","type":"SYSTEM_MESSAGE","status":"DONE","content":"[Message] sender=conv123/task-400 content=Task id \"conv123/task-400\" finished with result:\nBuild complete!"}"#
            let lineFinalDone = #"{"step_index":204,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"编译通过！所有单元测试全部通过。"}"#

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [lineStart, lineBg, lineTurnEnd, lineFinishMsg, lineFinalDone], fileAge: 10)
            guard case let .completed(fingerprint)? = signal else {
                throw TestError(message: "后台任务完成后，模型最终汇报必须正常返回 .completed")
            }
            try expectEqual(fingerprint, "antigravity-step-204")
        }

        TestKit.test("Antigravity后台任务: 多个后台任务时，只有全部完成才 completed") {
            let lineBg1 = #"{"step_index":301,"source":"MODEL","type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: conv123/task-1\nTask Description: arch -arm64 swift build"}"#
            let lineBg2 = #"{"step_index":302,"source":"MODEL","type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: conv123/task-2\nTask Description: Timer: 10s, Prompt: Check status"}"#
            let lineTurnEnd = #"{"step_index":303,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"等待任务…"}"#
            // 只有 task-2 (计时器) 完成
            let lineTimerDone = #"{"step_index":304,"source":"SYSTEM","type":"SYSTEM_MESSAGE","status":"DONE","content":"Task id \"conv123/task-2\" finished with result: ok"}"#
            let lineCheckAgain = #"{"step_index":305,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"编译仍在继续…"}"#

            // task-1 仍未完成，应保持 active
            let signalPartial = AgentSessionInspector.detectAntigravitySession(lines: [lineBg1, lineBg2, lineTurnEnd, lineTimerDone, lineCheckAgain], fileAge: 10)
            guard case let .active(fp, act)? = signalPartial else {
                throw TestError(message: "task-1 仍在进行中，必须返回 .active")
            }
            try expectEqual(fp, "antigravity-bg-task-1-305")
            try expectEqual(act, "后台任务: swift build")

            // task-1 也完成
            let lineBuildDone = #"{"step_index":306,"source":"SYSTEM","type":"SYSTEM_MESSAGE","status":"DONE","content":"Task id \"conv123/task-1\" finished with result: success"}"#
            let lineAllDone = #"{"step_index":307,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"全部构建完成。"}"#

            let signalAll = AgentSessionInspector.detectAntigravitySession(lines: [lineBg1, lineBg2, lineTurnEnd, lineTimerDone, lineCheckAgain, lineBuildDone, lineAllDone], fileAge: 10)
            guard case let .completed(fingerprint)? = signalAll else {
                throw TestError(message: "全部后台任务交付后，必须返回 .completed")
            }
            try expectEqual(fingerprint, "antigravity-step-307")
        }

        TestKit.test("Antigravity后台任务: 任务被取消或被 kill 后解除阻塞") {
            let lineBg = #"{"step_index":401,"source":"MODEL","type":"GENERIC","status":"RUNNING","content":"Tool is running as a background task with task id: conv123/task-999\nTask Description: long running job"}"#
            let lineCancel = #"{"step_index":402,"source":"MODEL","type":"GENERIC","status":"DONE","content":"Task \"conv123/task-999\" cancelled."}"#
            let lineDone = #"{"step_index":403,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"已取消任务。"}"#

            let signal = AgentSessionInspector.detectAntigravitySession(lines: [lineBg, lineCancel, lineDone], fileAge: 10)
            guard case let .completed(fingerprint)? = signal else {
                throw TestError(message: "任务取消后，模型回答应返回 .completed")
            }
            try expectEqual(fingerprint, "antigravity-step-403")
        }

        TestKit.test("Antigravity后台任务: 动作文案规范化与环境变量清洗") {
            let act1 = AgentSessionInspector.formatAntigravityBackgroundTaskAction("SDKROOT=/SDK arch -arm64 swift build")
            try expectEqual(act1, "后台任务: swift build")

            let act2 = AgentSessionInspector.formatAntigravityBackgroundTaskAction("Timer: 10s, Prompt: Check status")
            try expectEqual(act2, "后台定时中 (10s)")

            let act3 = AgentSessionInspector.formatAntigravityBackgroundTaskAction("grep -rn \"foo\" .")
            try expectEqual(act3, "后台任务: grep -rn \"foo\" .")
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

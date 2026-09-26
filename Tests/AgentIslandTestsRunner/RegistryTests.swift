import Foundation
@testable import AgentIslandCore

// MARK: - 注册表 / 动态集合 测试（第二轮审查补：addCustom/removeCustom/setEnabled/
// discoverCLI 幂等、持久化往返、全关持久化）

@MainActor
enum RegistryTests {

    static func register() {
        TestKit.test("注册表: 用户关闭自定义 Agent 后不被强制重新启用") {
            // 回归：resolvedEnabled 曾把「不在 knownAgents 且 defaultEnabled」的条目
            // 当作版本升级新增项补入启用集，导致用户刚关掉的自定义 Agent 被静默重开
            let suite = TestDefaults.suite("registry-1")

            let custom = AgentProfile(id: "custom-qa", name: "QA", icon: "terminal",
                                      bundleIDs: [], processNames: ["qa-agent"],
                                      sessionDirs: [], isCustom: true)
            let builtinDim = AgentRegistry.builtin.first { $0.id == "dim" }!

            // 首次求解：初始化默认启用集
            _ = EnabledAgentStore.resolvedEnabled(registry: [builtinDim, custom], defaults: suite)
            // 用户主动关闭自定义条目
            EnabledAgentStore.save([builtinDim.id], to: suite)
            // 再次求解（等价于重开设置窗口）：自定义条目不得被补回
            let resolved = EnabledAgentStore.resolvedEnabled(registry: [builtinDim, custom], defaults: suite)
            try expectTrue(!resolved.contains("custom-qa"),
                           "用户关闭的自定义 Agent 不应被自动重新启用，实际: \(resolved)")
            try expectTrue(resolved.contains("dim"), "内置条目应保持启用")
        }

        TestKit.test("注册表: 内置 Agent 仍按宿主过滤后展示（设置页与主列表同口径）") {
            // ChatGPT 已装、codex 无独立安装 → 内置列表不应含 codex（与 --probe 一致）
            let filtered = AgentRegistry.filteredBuiltin(installedCLIs: [],
                                                         installedBundles: ["com.openai.codex"])
            try expectTrue(!filtered.contains { $0.id == "codex" }, "内嵌 Codex 不应出现在内置列表")
            try expectTrue(filtered.contains { $0.id == "chatgpt" }, "宿主 ChatGPT 应保留")
            // 自动发现也不应把被过滤的组件以 cli- 形式重新引入
            let discovered = AgentRegistry.discoverCLIProfiles(installedCLIs: ["codex"],
                                                               installedBundles: ["com.openai.codex"])
            try expectTrue(!discovered.contains { $0.processNames.contains("codex") },
                           "被宿主过滤的 CLI 不应被自动发现重复引入")
        }

        TestKit.test("注册表: discoverCLIProfiles 幂等去重") {
            let installed: Set<String> = ["foo-cli", "dim", "gemini"]   // dim 为内置已含，不应重复发现
            let a = AgentRegistry.discoverCLIProfiles(installedCLIs: installed)
            let b = AgentRegistry.discoverCLIProfiles(installedCLIs: installed)
            try expectEqual(Set(a.map(\.id)), Set(b.map(\.id)), "两次发现应一致")
            try expectEqual(a.count, Set(a.map(\.id)).count, "id 不应重复")
            // 内置集已含的 CLI 不应出现在自动发现里
            let existing = Set(AgentRegistry.builtin.flatMap { $0.processNames.map { $0.lowercased() } })
            try expectTrue(!a.contains { existing.contains($0.processNames.first?.lowercased() ?? "") },
                           "内置 CLI 不应重复发现")
            try expectTrue(a.contains { $0.id == "cli-foo-cli" } && a.contains { $0.id == "cli-gemini" },
                           "已安装未内置 CLI 应被发现")
        }

        TestKit.test("注册表: 自定义 profile 持久化往返") {
            let p = AgentProfile(id: "custom-qa", name: "QA Agent", icon: "terminal",
                                 bundleIDs: [], processNames: ["qa-agent"],
                                 sessionDirs: ["/tmp/qa-sessions"], isCustom: true)
            let suite = TestDefaults.suite("registry-roundtrip")
            AgentRegistry.saveCustomProfiles([p], defaults: suite)
            let loaded = AgentRegistry.loadCustomProfiles(defaults: suite)
            try expectEqual(loaded.first?.id, p.id, "往返后 id 一致")
            try expectEqual(loaded.first?.processNames, p.processNames, "进程名一致")
            try expectEqual(loaded.first?.isCustom, true, "isCustom 保留")
        }

        TestKit.test("注册表: fullRegistry 含内置+自定义（自动发现不炸）") {
            let p = AgentProfile(id: "custom-tmp", name: "TMP", icon: "terminal",
                                 bundleIDs: [], processNames: ["tmp-agent"],
                                 sessionDirs: [], isCustom: true)
            let suite = TestDefaults.suite("registry-full")
            AgentRegistry.saveCustomProfiles([p], defaults: suite)
            let full = AgentRegistry.fullRegistry(installedCLIs: [], defaults: suite)
            try expectTrue(full.contains { $0.id == "dim" }, "含内置 dim")
            try expectTrue(full.contains { $0.id == "custom-tmp" }, "含自定义")
            // 引擎的 guard 防重复
            let engine = ActivityEngine(profiles: full, config: EngineConfig(),
                                        processMonitor: FakeProcessProvider(processNames: [], bundleIDs: []),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]),
                                        installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }))
            engine.addCustomProfile(p)
            let count = engine.allProfiles.filter { $0.id == "custom-tmp" }.count
            try expectEqual(count, 1, "重复添加应被引擎 guard 拒绝")
            engine.removeCustomProfile("custom-tmp")
            try expectTrue(!engine.allProfiles.contains { $0.id == "custom-tmp" }, "移除后不存在")
        }

        TestKit.test("引擎: setEnabled 过滤启停集合") {
            let engine = ActivityEngine(profiles: AgentRegistry.builtin, config: EngineConfig(),
                                        processMonitor: FakeProcessProvider(processNames: ["DimAgent"], bundleIDs: []),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]),
                                        installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }))
            engine.setEnabled(["dim"])
            try expectEqual(engine.allProfiles.map(\.id), ["dim"], "只保留启用项")
        }

        TestKit.test("引擎: setEnabled 从全量注册表过滤（关→再开不丢失监控）") {
            // 回归：之前 setEnabled 只从当前已缩水列表过滤，
            // 「关闭后再开启」的 agent 本会话内永久丢失监控
            let engine = ActivityEngine(profiles: AgentRegistry.builtin, config: EngineConfig(),
                                        processMonitor: FakeProcessProvider(processNames: ["DimAgent", "Codex"], bundleIDs: []),
                                        fileMonitor: FakeFileActivityProvider(writes: [:]),
                                        installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }))
            engine.setEnabled(["dim"])                      // 只启用 dim
            try expectEqual(engine.allProfiles.map(\.id), ["dim"], "只保留启用项")
            engine.setEnabled(["dim", "claude"])            // 再开启 claude
            let ids = engine.allProfiles.map(\.id)
            try expectTrue(ids.contains("dim"), "dim 应保留")
            try expectTrue(ids.contains("claude"), "claude 应能加回（修复后从 fullRegistry 取源）")
            try expectEqual(Set(ids).count, ids.count, "无重复")
        }

        TestKit.test("持久化: 全关（空数组）不被回退为默认") {
            // 「用户主动全关」与「从没配过」必须是两个不同的读取结果，否则一关就复活。
            // 走 store 的读写路径：上一版直接拿 JSONEncoder/Decoder 对拍空数组，那是
            // 一句恒成立的往返；而且它写的是 **UserDefaults.standard** 并随手 removeObject，
            // 跑一次测试就把开发机上真实的启停集合清掉了。
            let suite = TestDefaults.suite("registry-empty-enabled")
            try expectNil(EnabledAgentStore.load(from: suite), "没写过 = 无记录（由调用方按默认处理）")
            EnabledAgentStore.save([], to: suite)
            try expectEqual(EnabledAgentStore.load(from: suite) ?? ["sentinel"], [],
                            "空集合要能原样读回来，不能被当成「清除记录」")
            let resolved = EnabledAgentStore.resolvedEnabled(registry: AgentRegistry.builtin,
                                                             defaults: suite, readOnly: true)
            try expectTrue(resolved.isEmpty, "全关时求解结果仍为空，不得被 defaultEnabled 复活")
        }

        TestKit.test("引擎: bundle 命中但进程名未匹配 → 标记运行且 CPU 未测（[pid:-1] 占位）") {
            // 回归：sampleCore 单趟 matchingEntries 后 running/cpu 语义等价性
            //（测试缺口）——bundleHit 无名字匹配应返回占位条目
            let provider = FakeProcessProvider(processNames: ["some-other-app"],
                                               bundleIDs: ["com.dimcode.app"])
            let engine = ActivityEngine(profiles: AgentRegistry.builtin.filter { $0.id == "dim" },
                                        config: EngineConfig(),
                                        processMonitor: provider,
                                        fileMonitor: FakeFileActivityProvider(writes: [:]),
                                        installedApps: InstalledAppsCache(scanCLIs: { [] }, scanBundles: { [] }))
            let snaps = engine.sample(now: Date())
            let dim = snaps.first { $0.id == "dim" }
            try expectTrue(dim?.processRunning == true, "bundle 运行即 processRunning=true")
            // 占位条目的 0 是「没看着这个进程」，不是「这个进程不占 CPU」，所以不公布成实测值。
            // 第二拍才有 CPU 差分窗口——用它排除「首拍未测」这一层，钉住的就是占位这一条
            let second = engine.sample(now: Date().addingTimeInterval(2)).first { $0.id == "dim" }
            try expectEqual(second?.cpuPercent, nil, "无进程名匹配时 CPU 是「没测」而不是 0")
        }

        TestKit.test("注册表: 自定义档案解码容错（坏 JSON/空数据回落空集）") {
            let suite = TestDefaults.suite("registry-decode")
            suite.set(Data("not json".utf8), forKey: SettingKey.customAgents)
            try expectEqual(AgentRegistry.loadCustomProfiles(defaults: suite).count, 0, "坏 JSON 回落空集")
            suite.set(Data(), forKey: SettingKey.customAgents)
            try expectEqual(AgentRegistry.loadCustomProfiles(defaults: suite).count, 0, "空数据回落空集")
        }

        TestKit.test("注册表: 坏元素只丢自身，其余抢救回来（防永久丢失覆写）") {
            let suite = TestDefaults.suite("registry-tolerant")
            let good = AgentProfile(id: "custom-ok", name: "OK", icon: "terminal",
                                    bundleIDs: [], processNames: ["ok-agent"],
                                    sessionDirs: [], isCustom: true)
            // 构造混合存档：好元素 + name 类型漂移的坏元素（旧版 Schema 演进形态）
            let goodJSON = String(data: try JSONEncoder().encode([good]), encoding: .utf8)!
            let bad = "{\"id\":\"broken-bad\",\"name\":123,\"icon\":\"x\"}"
            // 构造混合存档：好元素 + name 类型漂移的坏元素（旧版 Schema 演进形态）
            let mixed = "[\(goodJSON.dropFirst().dropLast()),\(bad)]"
            suite.set(Data(mixed.utf8), forKey: SettingKey.customAgents)
            let loaded = AgentRegistry.loadCustomProfiles(defaults: suite)
            try expectEqual(loaded.count, 1, "坏元素丢弃，好元素必须救回（实际 \(loaded.count)）")
            try expectEqual(loaded.first?.id, "custom-ok", "抢救回的应是好元素")
        }

        TestKit.test("注册表: Antigravity 只监控会话数据，不含浏览器内核用户数据目录（R37）") {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            let antigravity = AgentRegistry.builtin.first { $0.id == "antigravity" }!
            try expectTrue(antigravity.sessionDirs.contains(home + "/.gemini/antigravity/conversations"),
                            "会话库目录必须保留")
            try expectTrue(antigravity.sessionDirs.contains(home + "/.gemini/antigravity/brain"),
                            "brain 目录必须保留")
            try expectFalse(antigravity.sessionDirs.contains { $0.lowercased().contains("application support/antigravity") },
                            "内核用户数据目录不得进监控：空闲缓存/账号写入会把应用顶成 working")

            // 通用哨兵：任何档案都不得把浏览器内核内部目录直接当会话目录
            let noiseDirNames: Set<String> = ["cache", "code cache", "gpucache", "local storage",
                                              "session storage", "crashpad", "blob_storage"]
            for profile in AgentRegistry.builtin {
                for dir in profile.sessionDirs {
                    let last = (dir as NSString).lastPathComponent.lowercased()
                    try expectFalse(noiseDirNames.contains(last),
                                    "\(profile.id) 把内核目录当会话目录：\(dir)")
                }
            }
        }

        TestKit.test("注册表: Windsurf 与 Aider 内置档案完整性") {
            let windsurf = AgentRegistry.builtin.first { $0.id == "windsurf" }
            try expectTrue(windsurf != nil, "Windsurf 必须存在于内置档案中")
            try expectTrue(windsurf?.bundleIDs.contains("com.exafunction.windsurf") == true, "Windsurf 必须包含 bundle ID")
            try expectTrue(windsurf?.processNames.contains("Windsurf") == true, "Windsurf 必须包含进程名")

            let aider = AgentRegistry.builtin.first { $0.id == "aider" }
            try expectTrue(aider != nil, "Aider 必须存在于内置档案中")
            try expectTrue(aider?.processNames.contains("aider") == true, "Aider 必须包含进程名")
        }

        TestKit.test("注册表: ZCode 保留 checkpoints 并接入模型 IO，排除 CLI 日志") {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            let zcode = AgentRegistry.builtin.first { $0.id == "zcode" }!
            try expectEqual(zcode.sessionDirs, [home + "/.zcode/v2/checkpoints",
                                                home + "/.zcode/cli/rollout"])
            try expectFalse(zcode.sessionDirs.contains { $0.hasSuffix("/.zcode/cli/log") },
                            "运行日志更新不应单独判为 Agent 工作")
        }

        TestKit.test("注册表: Cline, Roo Code, Continue 与 Goose 内置档案完整性") {
            let cline = AgentRegistry.builtin.first { $0.id == "cline" }
            try expectTrue(cline != nil, "Cline 必须存在于内置档案中")
            try expectTrue(cline?.processNames.contains("cline") == true, "Cline 必须包含进程名")

            let roo = AgentRegistry.builtin.first { $0.id == "roo-code" }
            try expectTrue(roo != nil, "Roo Code 必须存在于内置档案中")
            try expectTrue(roo?.processNames.contains("roo") == true, "Roo Code 必须包含进程名")

            let cont = AgentRegistry.builtin.first { $0.id == "continue" }
            try expectTrue(cont != nil, "Continue 必须存在于内置档案中")
            try expectTrue(cont?.processNames.contains("continue-core") == true, "Continue 必须包含进程名")

            let goose = AgentRegistry.builtin.first { $0.id == "goose" }
            try expectTrue(goose != nil, "Goose 必须存在于内置档案中")
            try expectTrue(goose?.processNames.contains("goose") == true, "Goose 必须包含进程名")
        }

        TestKit.test("注册表自洽: 声明的会话方言、只读库与 token 采集根必须能在同一份档案里定位到") {
            // 方言/库位置/采集根都是档案数据，解析器与用量页不再按 id 猜路径。一旦注册表里
            // 声明了专有方言却没给出对应目录，探测会静默返回「无信号」（表现为该 Agent
            // 永远只显示待机）；tokenRoots 漏声明则是该工具的用量静默消失。
            for profile in AgentRegistry.builtin {
                switch profile.sessionDialect {
                case .genericTail, .clineTasks:
                    break
                case .qoderTranscript:
                    // Qoder 的会话定位是「projects/<slug>/*.jsonl」两层枚举，
                    // 没有可静态推导的子目录名，只能要求 sessionDirs 给出 projects 根
                    try expectTrue(profile.sessionDirs.contains { $0.hasSuffix("/projects") },
                                   "\(profile.id) 声明了 qoderTranscript 方言但 sessionDirs 不指向 projects 根")
                case .antigravityBrain:
                    try expectTrue(AgentSessionInspector.antigravityBrainDir(in: profile.sessionDirs) != nil,
                                   "\(profile.id) 声明了 antigravityBrain 方言但 sessionDirs 里没有 brain 目录")
                case .dshProjection:
                    try expectTrue(AgentSessionInspector.dshProjectionDir(in: profile.sessionDirs) != nil,
                                   "\(profile.id) 声明了 dshProjection 方言但 sessionDirs 里没有投影缓存目录")
                }
                if let db = profile.sessionDatabase {
                    try expectTrue(db.path.hasPrefix("/"), "\(profile.id) 的会话库必须是绝对路径: \(db.path)")
                    if db.schema == .statusIndex {
                        try expectTrue(db.statusSQL?.isEmpty == false, "\(profile.id) 的 statusIndex 方言缺少查询语句")
                    }
                }
            }

            // token 采集根：家目录下的绝对路径（相对路径会让索引静默扫不到任何文件）
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            for profile in AgentRegistry.builtin {
                for root in profile.tokenRoots {
                    try expectTrue(root.hasPrefix(home + "/"),
                                   "\(profile.id) 的 tokenRoots 必须是家目录下的绝对路径: \(root)")
                }
            }

            // 现网构造（TokenUsageMonitor.init / 会话目录定位）按 id 取档案读这些声明：
            // 条目或声明一旦被删，该工具的用量会静默变空而没有任何报错，故逐个钉死。
            // 期望值写死在此处是有意的——档案是唯一事实源，本测试是它不被悄悄改动的哨兵。
            let expectedTokenRoots: [String: [String]] = [
                "dim": ["\(home)/.dimcode/v2/data/sessions"],
                "codex": ["\(home)/.codex/sessions"],
                "claude": ["\(home)/.claude/sessions", "\(home)/.claude/projects"],
                "workbuddy": ["\(home)/.workbuddy/projects"],
                "workbuddy-ai": ["\(home)/.workbuddy-ai/projects"],
            ]
            for (id, roots) in expectedTokenRoots {
                let profile = AgentRegistry.builtin.first { $0.id == id }
                try expectTrue(profile != nil, "内置档案 \(id) 不能删：用量页按 id 取 tokenRoots")
                try expectEqual(profile?.tokenRoots, roots, "\(id) 的 tokenRoots 声明漂移")
            }
            // 反向哨兵：没列进上表的档案不得声明采集根（否则新增 Agent 的用量来源不会被
            // 用量页构造读到，需要显式把它加进 structuredSources）
            let declared = Set(AgentRegistry.builtin.filter { !$0.tokenRoots.isEmpty }.map(\.id))
            try expectEqual(declared, Set(expectedTokenRoots.keys),
                            "声明了 tokenRoots 的档案集合与用量页读取的集合不一致")

            // 落 SQLite 的 Agent：库位置必须是档案里的绝对路径
            for id in ["dim", "opencode", "mimocode", "zcode", "workbuddy", "workbuddy-ai"] {
                let profile = AgentRegistry.builtin.first { $0.id == id }
                try expectTrue(profile != nil, "内置档案 \(id) 不能删")
                try expectTrue(profile?.sessionDatabase != nil, "\(id) 依赖会话库，档案必须声明 sessionDatabase")
                try expectTrue(profile?.sessionDatabase?.path.hasPrefix("/") == true,
                               "\(id) 的会话库必须是绝对路径")
            }
        }

        TestKit.test("注册表: Xiaomi MiMo 按 OpenCode 方言的 fork 登记") {
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            let mimo = AgentRegistry.builtin.first { $0.id == "mimocode" }
            try expectNotNil(mimo, "Xiaomi MiMo（MiMo Code）必须在内置档案里")
            try expectEqual(mimo?.name, "Xiaomi MiMo")
            try expectEqual(mimo?.bundleIDs, ["com.xiaomi.mimo.desktop"], "安装探测与运行判定都按 bundle id")
            try expectTrue(mimo?.processNames.contains("Xiaomi MiMo") == true, "主进程名")
            try expectEqual(mimo?.sessionDirs, [home + "/.local/share/mimocode"],
                            "只监控引擎数据根；Electron 用户数据目录空闲时也在写，会把「开着窗」报成「在干活」")
            try expectEqual(mimo?.sessionDatabase?.path, home + "/.local/share/mimocode/mimocode.db")
            try expectEqual(mimo?.sessionDatabase?.schema, .openCode,
                            "表结构与 OpenCode 同形（session/message/part），状态与动作都靠这条声明复用")
        }

        TestKit.test("进程匹配: Xiaomi MiMo 记主进程与引擎进程，Electron helper 与崩溃上报除外") {
            // 前四条就是本机实测的进程树（主进程 + GPU + 渲染 + crashpad）；第五条是按
            // 注册表线索放的引擎进程形态——CPU 跨条目求和，所以「该算进来的」漏了会
            // 表现为「明明在跑任务，岛上一路 0% CPU」
            let entries = [
                ProcessSnapshot.Entry(pid: 26_762,
                                      path: "/Applications/Xiaomi MiMo.app/Contents/MacOS/Xiaomi MiMo",
                                      basename: "xiaomi mimo", cpuPercent: 3.2, rssBytes: 400_000_000),
                ProcessSnapshot.Entry(pid: 26_868,
                                      path: "/Applications/Xiaomi MiMo.app/Contents/Frameworks/Xiaomi MiMo Helper.app/Contents/MacOS/Xiaomi MiMo Helper",
                                      basename: "xiaomi mimo helper", cpuPercent: 22.0, rssBytes: 180_000_000),
                ProcessSnapshot.Entry(pid: 26_890,
                                      path: "/Applications/Xiaomi MiMo.app/Contents/Frameworks/Xiaomi MiMo Helper (Renderer).app/Contents/MacOS/Xiaomi MiMo Helper (Renderer)",
                                      basename: "xiaomi mimo helper (renderer)", cpuPercent: 31.0, rssBytes: 260_000_000),
                ProcessSnapshot.Entry(pid: 26_864,
                                      path: "/Applications/Xiaomi MiMo.app/Contents/Frameworks/Electron Framework.framework/Helpers/chrome_crashpad_handler",
                                      basename: "chrome_crashpad_handler", cpuPercent: 0.0, rssBytes: 12_000_000),
                ProcessSnapshot.Entry(pid: 27_001,
                                      path: "/Applications/Xiaomi MiMo.app/Contents/Resources/mimocode/bin/mimocode",
                                      basename: "mimocode", cpuPercent: 40.0, rssBytes: 220_000_000, ppid: 26_762),
            ]
            let matcher = ProcessMatcher(snapshot: ProcessSnapshot(entries: entries),
                                         runningBundleIDs: ["com.xiaomi.mimo.desktop"])
            let matched = matcher.matchingEntries(for: AgentRegistry.builtin.first { $0.id == "mimocode" }!)
            try expectEqual(Set(matched.map(\.pid)), [26_762, 27_001],
                            "主进程与引擎进程各记一次；helper 与 crashpad 不得进（实得 \(matched.map(\.pid))）")
        }

        TestKit.test("结构: OpenCode 方言的 Agent 按档案 schema 路由，不再按 id 逐个列") {
            // 状态、动作、实时流水三处以前各自按 id 列一遍；接一个同表结构的 fork（小米 MiMo）
            // 就会漏掉其中几处，表现为「有这个卡片但没有当前动作、流水一片空白」而没有任何报错
            for path in ["Sources/AgentIslandCore/AgentActionInspector.swift",
                         "Sources/AgentIslandCore/AgentLogStreamer.swift"] {
                let text = SourceTree.codeOnly(try SourceTree.text(relativePath: path))
                try expectFalse(text.contains("case \"opencode\""),
                                "\(path) 又回到按 id 硬分派：新的 OpenCode 方言 fork 会静默没有动作/流水")
                try expectTrue(text.contains("schema == .openCode"),
                               "\(path) 的 OpenCode 方言路由必须建立在档案声明的 schema 上")
            }
        }
    }
}

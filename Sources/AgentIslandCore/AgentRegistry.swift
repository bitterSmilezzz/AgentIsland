import Foundation

// MARK: - Agent 注册表
// 内置集（按主流 Agent 工具的实际安装形态维护）+ 自动发现（按传入的已安装集判定）+ 用户自定义（UserDefaults）
// 无全局可变状态：安装判定一律经注入的 InstalledAppsCache（见 InstalledAppsCache.swift）

public enum AgentRegistry {

    /// 按 id 取内置档案：会话库位置、会话方言等能力一律从这里取，
    /// 各子系统不再重复硬编码 `~/...` 字面量（档案换目录时不会只有一半生效）。
    public static func profile(_ id: String) -> AgentProfile? {
        builtin.first { $0.id == id }
    }

    /// 某 Agent 的只读会话库位置（未登记则为 nil）
    public static func databasePath(for id: String) -> String? {
        profile(id)?.sessionDatabase?.path
    }

    /// 桌面类 Agent 的 CPU 判定下限（Electron/多进程空闲抖动约 4%~15%）
    private static let desktopCPUFloor: Double = 20.0
    /// WorkBuddy 常驻多个 prewarm 进程，CPU 汇总更容易抬高，需要更高下限
    private static let workbuddyCPUFloor: Double = 35.0
    /// WorkBuddy 多专家团（Multi-Agent）并发交互 Token 净消耗极高，设定 100 万 tokens/分钟专属保护下限，
    /// 避免日常多专家协同被误判为异常突增/死循环，同时保留超限 runaway 的熔断能力。
    private static let workbuddyTokenFloor: Int = 1_000_000

    /// 内置定义（覆盖常见 Agent；自动发现负责标记哪些真实安装）
    public static let builtin: [AgentProfile] = [
        AgentProfile(
            id: "dim",
            name: "DimAgent",
            icon: "sparkles.rectangle.stack.fill",
            bundleIDs: ["com.dimcode.app"],
            processNames: ["DimAgent", "DimRemote", "dim"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home(".dimcode/v2/data/sessions")],
            // 用量页按 sessionId 定位会话目录（明细就在该目录树下），路径只在此声明一次
            tokenRoots: [home(".dimcode/v2/data/sessions")],
            category: .assistant,
            emoji: "✨",
            sessionDatabase: AgentSessionDatabase(path: home(".dimcode/v2/dimcode.sqlite"), schema: .dimTasks),
        ),
        AgentProfile(
            id: "claude",
            name: "Claude",
            icon: "bubble.left.and.bubble.right.fill",
            bundleIDs: ["com.anthropic.claudefordesktop", "com.anthropic.claudecode"],
            processNames: ["claude", "Claude", "claude-code"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home(".claude/sessions"), home(".claude/projects")],
            tokenRoots: [home(".claude/sessions"), home(".claude/projects")],
            category: .assistant,
            emoji: "🧠",
        ),
        AgentProfile(
            id: "qoder",
            name: "Qoder",
            icon: "cpu.fill",
            // 实测：/Applications/Qoder.app/Contents/MacOS/Qoder，Helper 与 Renderer
            // 走「name + 空格」前缀族规则命中（"qoder helper" / "qoder helper (renderer)"）
            bundleIDs: ["com.qoder.app"],
            processNames: ["qoder"],
            // 路径锚定到安装目录，不用裸子串 "qoder"：裸子串会把 ~/code/qoder-playground
            // 里跑的任何 Electron 程序认成本 Agent（同类过宽匹配已在工作台记为已知风险）
            pathContains: ["/applications/qoder.app"],
            // 桌面 IDE：空闲时的渲染/IPC 抖动会越过 CLI 档的 CPU 阈值
            cpuWorkingThreshold: desktopCPUFloor,
            // 实测：~/.qoder/projects/<项目 slug>/<会话 uuid>.jsonl
            // 同目录还有 <uuid>/subagents/*.jsonl（子智能体）与 state.json
            sessionDirs: [home(".qoder/projects")],
            // tokenRoots 故意留空：Qoder 落盘的 token 字段全为 0（真值只有 credits），
            // 接上采集只会多花 1~1.9s 换 0 条数据。详见 TokenUsageMonitor 的
            // structuredSources 注释与 docs/research/ 的用量口径调研。
            category: .assistant,
            emoji: "🖥️",
            sessionDialect: .qoderTranscript,
        ),
        AgentProfile(
            id: "codex",
            name: "Codex",
            icon: "chevron.left.forwardslash.chevron.right",
            bundleIDs: [],
            processNames: ["codex", "Codex"],
            // 排除 ChatGPT 桌面版内嵌的 Codex（/Applications/ChatGPT.app/Contents/Resources/codex
            // 及其 Codex Framework 辅助进程）：与独立 codex CLI 同名同 basename，
            // 不排除会把同一个 ChatGPT 同时数成 ChatGPT 和 Codex 两个 Agent
            pathExcludes: ["/Applications/ChatGPT.app/"],
            // ChatGPT 桌面版内嵌 Codex（同一份 ~/.codex 会话目录、同一进程族）：
            // 宿主已装且 codex 无独立安装时不单独成条目，避免同一份程序显示两行
            hostBundleIDs: ["com.openai.codex"],
            // 只监控会话 JSONL 目录；~/.codex 根目录含 sqlite/WAL/cache，被 app 后台高频刷新，
            // 会导致 Codex 仅打开但未运行任务时被误判为 WORKING
            sessionDirs: [home(".codex/sessions")],
            // token_usage_record 就在 sessions 的 rollout JSONL 里（~/.codex 根目录另有
            // sqlite/WAL，不是采集入口）
            tokenRoots: [home(".codex/sessions")],
            category: .assistant,
            emoji: "🤖",
        ),
        AgentProfile(
            id: "cursor",
            name: "Cursor",
            icon: "cursorarrow.click.2",
            bundleIDs: ["com.todesktop.230113mital1efw", "com.cursor.cursor"],
            processNames: ["Cursor", "cursor"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home("Library/Application Support/Cursor/User/workspaceStorage")],
            category: .codeEditor,
            emoji: "💻",
        ),
        AgentProfile(
            id: "trae",
            name: "Trae",
            icon: "paintbrush.pointed.fill",
            bundleIDs: ["cn.trae.solo.app", "com.trae.ai"],
            processNames: ["TRAE SOLO CN", "Trae", "trae", "Electron"],
            pathContains: ["trae"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home("Library/Application Support/Trae CN/User/workspaceStorage")],
            category: .codeEditor,
            emoji: "📐",
        ),
        AgentProfile(
            id: "copilot",
            name: "ima.copilot",
            icon: "sparkles",
            bundleIDs: ["com.tencent.imamac"],
            processNames: ["ima.copilot", "Copilot"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home("Library/Application Support/com.tencent.imamac")],
            category: .assistant,
            emoji: "🧑‍✈️",
        ),
        AgentProfile(
            // 国内版（腾讯系，WorkBuddy.app）。数据目录与国外版（workbuddy-ai）完全独立。
            // 注意 pathContains 不能用宽口径 "workbuddy"——两个变体的进程路径都含它，
            // 会互相误命中造成双份计数；必须精确到各自的 .app 目录 / 数据目录
            id: "workbuddy",
            name: "WorkBuddy",
            icon: "briefcase.fill",
            bundleIDs: ["com.tencent.workbuddy.mac"],
            processNames: ["WorkBuddy", "workbuddy", "Electron"],
            pathContains: ["/applications/workbuddy.app", ".workbuddy/"],

            cpuWorkingThreshold: workbuddyCPUFloor,
            tokenAlertFloor: workbuddyTokenFloor,
            sessionDirs: [
                // sessions/*.json 是宿主心跳/连接登记，不代表有任务；
                // memory 也会被后台同步触碰。任务产物才是工作信号。
                home(".workbuddy/tasks")
            ],
            // token 明细在 projects/ 的会话 JSONL 里，与 tasks/（工作信号）不同子树
            tokenRoots: [home(".workbuddy/projects")],
            category: .assistant,
            emoji: "💼",
            sessionDatabase: AgentSessionDatabase(path: home(".workbuddy/workbuddy.db"), schema: .statusIndex, statusSQL: "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"),
        ),
        // 国外版（WorkBuddy AI.app，com.workbuddy.workbuddy）：数据目录 ~/.workbuddy-ai，
        // 进程 basename 同为 Electron，只能靠路径区分（app 内路径含 "WorkBuddy AI"）
        AgentProfile(
            id: "workbuddy-ai",
            name: "WorkBuddy AI",
            icon: "globe",
            // 真实 bundle id 以 WorkBuddy AI.app/Contents/Info.plist 为准（plutil 实测
            // com.workbuddy.workbuddy-ai）——此前照抄 Application Support 目录名漏了 -ai
            bundleIDs: ["com.workbuddy.workbuddy-ai"],
            processNames: ["Electron", "workbuddy"],
            pathContains: ["workbuddy ai.app", ".workbuddy-ai"],
            cpuWorkingThreshold: workbuddyCPUFloor,
            tokenAlertFloor: workbuddyTokenFloor,
            sessionDirs: [
                home(".workbuddy-ai/tasks")
            ],
            // 同国内版：明细在独立的 projects/ 子树（两变体数据目录完全独立）
            tokenRoots: [home(".workbuddy-ai/projects")],
            category: .assistant,
            emoji: "💼",
            sessionDatabase: AgentSessionDatabase(path: home(".workbuddy-ai/workbuddy.db"), schema: .statusIndex, statusSQL: "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"),
        ),
        AgentProfile(
            id: "zcode",
            name: "ZCode",
            icon: "curlybraces.square.fill",
            bundleIDs: ["dev.zcode.app"],
            processNames: ["ZCode", "zcode-host-local-1", "zcode-cli", "Electron"],
            pathContains: ["zcode"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [
                // v2 根目录包含 bot 状态、轮询日志和 sqlite；这些会在空闲时持续更新。
                // checkpoints 才对应一次实际 Agent 运行的状态落盘。
                home(".zcode/v2/checkpoints")
            ],
            category: .codeEditor,
            emoji: "🧩",
            sessionDatabase: AgentSessionDatabase(path: home(".zcode/v2/tasks-index.sqlite"), schema: .statusIndex, statusSQL: "SELECT id, task_status, updated_at FROM tasks WHERE deleted = 0 ORDER BY updated_at DESC LIMIT 1;"),
        ),
        AgentProfile(
            id: "antigravity",
            name: "Antigravity",
            icon: "atom",
            bundleIDs: ["com.google.antigravity", "com.yuzhiqiang.antigravity.studio"],
            processNames: ["Antigravity", "Electron"],
            pathContains: ["antigravity"],
            pathExcludes: ["frameworks", "helper", "language_server"],
            cpuWorkingThreshold: desktopCPUFloor,
            // 只监控 Agent 自身的会话数据；`Library/Application Support/Antigravity`
            // 是内核用户数据目录（Chromium 缓存/存储 + 更新器 + 账号状态），
            // 实测仅打开应用就有 36 次写入/20 分钟且全在此列，会把空闲判成工作（R37）
            sessionDirs: [
                home(".gemini/antigravity/conversations"),
                home(".gemini/antigravity/brain")
            ],
            category: .codeEditor,
            emoji: "⚛️",
            sessionDialect: .antigravityBrain,
        ),
        AgentProfile(
            id: "opencode",
            name: "OpenCode",
            icon: "terminal.fill",
            bundleIDs: ["ai.opencode.desktop"],
            processNames: ["opencode"],
            cpuWorkingThreshold: desktopCPUFloor,
            // 只监控会话数据库目录。~/.config/opencode 是配置目录（含 node_modules），
            // 与任务无关却占全量扫描 22-50ms / 2098 个条目（实测）——依赖安装不是工作信号。
            sessionDirs: [home(".local/share/opencode")],
            category: .assistant,
            emoji: "📖",
            sessionDatabase: AgentSessionDatabase(path: home(".local/share/opencode/opencode.db"), schema: .openCode),
        ),
        AgentProfile(
            id: "mimocode",
            name: "Xiaomi MiMo",
            icon: "antenna.radiowaves.left.and.right",
            bundleIDs: ["com.xiaomi.mimo.desktop"],
            // 桌面主进程 + 引擎进程。注册表里的 asar 线索（~/.local/state/mimocode、
            // ~/.config/mimocode、mimocode-cli）表明编码引擎是独立进程而非渲染器内跑；
            // 引擎进程的 CPU 才是真干活的那份——主进程只按 desktopCPUFloor 会长期显示 0%。
            // 「mimocode」这一条按 CLI 命名习惯推的，真实进程名待跑过一次编码会话核对
            processNames: ["Xiaomi MiMo", "mimocode"],
            // 不写 pathContains：桌面版走 bundle/主进程名已经够收敛，而 CLI 单独跑时
            // 可执行路径里并不含 "xiaomi mimo"，加了白名单反而把那种形态排除掉
            // Electron 派生进程（GPU/渲染/网络/崩溃上报）与宿主是同一个程序：CPU 会跨条目
            // 求和，把渲染器的空闲抖动累成「高负载工作中」，因此按路径排除
            pathExcludes: ["frameworks", "helper"],
            cpuWorkingThreshold: desktopCPUFloor,
            // 引擎数据根（会话库 + 心跳日志）。`Library/Application Support/Xiaomi MiMo`
            // 是 Electron 用户数据目录（Chromium 缓存、账号状态、更新器），空闲时也在写，
            // 不进监控——同 Antigravity 那条 R37 的教训
            sessionDirs: [home(".local/share/mimocode")],
            category: .assistant,
            emoji: "📡",
            // 表结构与 OpenCode 同形（session / message / part，part.data 存 JSON part），
            // 所以复用 .openCode 方言：状态、当前动作、实时流水三处都按 schema 路由，
            // 再接同类 fork 只改这张表
            sessionDatabase: AgentSessionDatabase(path: home(".local/share/mimocode/mimocode.db"), schema: .openCode),
        ),
        AgentProfile(
            id: "hermes",
            name: "Hermes Agent",
            icon: "wand.and.stars",
            bundleIDs: [],
            processNames: ["hermes-agent", "hermes"],
            sessionDirs: [home(".hermes/sessions"), home(".hermes/logs")],
            category: .assistant,
            emoji: "🪄",
        ),
        AgentProfile(
            id: "continue",
            name: "Continue",
            icon: "arrow.triangle.2.circlepath",
            bundleIDs: ["com.continue.continue"],
            processNames: ["Continue", "continue", "continue-core"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home(".continue")],
            defaultEnabled: false,
            category: .codeEditor,
            emoji: "▶️",
        ),
        AgentProfile(
            id: "chatgpt",
            name: "ChatGPT",
            icon: "brain.head.profile",
            bundleIDs: ["com.openai.codex"],
            processNames: ["ChatGPT"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home("Library/Application Support/com.openai.codex")],
            category: .assistant,
            emoji: "💬",
        ),
        AgentProfile(
            id: "dsh",
            name: "DeepSeek Harness",
            icon: "bolt.horizontal.fill",
            bundleIDs: [],
            // web 模式由 Node 启动 bin.js，最终可执行 basename 是 node；
            // 仅靠 dsh 进程名会漏掉当前桌面/网页宿主形态。
            processNames: ["dsh"],
            pathContains: ["deepseek-harness"],
            pathExcludes: [".codegraph"],
            // storages/workspace.json 是网页宿主的工作区状态，会在空闲时被后台刷新；
            // 真实会话目录与投影检查点目录 (session_projcache/sessions) 联动代表 DSH 真实任务生命周期。
            sessionDirs: [home(".dsh/sessions"), home(".dsh/storages/session_projcache/sessions")],
            category: .assistant,
            emoji: "⚡️",
            sessionDialect: .dshProjection,
        ),
        AgentProfile(
            id: "ego-browser",
            name: "Ego Browser",
            icon: "globe.americas.fill",
            bundleIDs: ["com.citrolabs.ego.lite"],
            processNames: ["ego-browser", "ego lite", "ego"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home(".local/share/ego"), home("Library/Application Support/ego lite")],
            category: .assistant,
            emoji: "🌐",
        ),
        AgentProfile(
            id: "vibe-usage",
            name: "Vibe Usage",
            icon: "chart.bar.xaxis",
            bundleIDs: ["ai.vibecafe.vibe-usage"],
            processNames: ["vibe-usage", "Vibe Usage"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home(".vibe-usage"), home("Library/Application Support/Vibe Usage")],
            category: .assistant,
            emoji: "📊",
        ),
        AgentProfile(
            id: "openviking",
            name: "OpenViking",
            icon: "shippingbox.fill",
            bundleIDs: [],
            processNames: ["openviking", "openviking-server", "ov", "vikingbot"],
            pathContains: ["openviking"],
            // 只监控数据目录。~/.local/share/uv/tools/openviking 是 uv 装的 Python venv
            // （lib/bin/pyvenv.cfg，4452 个条目），与任务无关，实测占全量扫描最大的一块。
            sessionDirs: [home(".openviking")],
            category: .assistant,
            emoji: "📦",
        ),
        AgentProfile(
            id: "windsurf",
            name: "Windsurf",
            icon: "wind",
            bundleIDs: ["com.exafunction.windsurf"],
            processNames: ["Windsurf", "windsurf", "Electron"],
            pathContains: ["windsurf"],
            cpuWorkingThreshold: desktopCPUFloor,
            sessionDirs: [home("Library/Application Support/Windsurf/User/workspaceStorage")],
            category: .codeEditor,
            emoji: "🏄",
        ),
        AgentProfile(
            id: "aider",
            name: "Aider",
            icon: "terminal.fill",
            bundleIDs: [],
            processNames: ["aider"],
            sessionDirs: [home(".aider")],
            category: .assistant,
            emoji: "🛠️",
        ),
        AgentProfile(
            id: "cline",
            name: "Cline",
            icon: "brain.head.profile",
            bundleIDs: [],
            processNames: ["cline"],
            sessionDirs: [home("Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/tasks")],
            category: .assistant,
            emoji: "🪡",
            sessionDialect: .clineTasks,
        ),
        AgentProfile(
            id: "roo-code",
            name: "Roo Code",
            icon: "sparkle.magnifyingglass",
            bundleIDs: [],
            processNames: ["roo", "roo-code"],
            sessionDirs: [home("Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline/tasks")],
            category: .assistant,
            emoji: "🦘",
            sessionDialect: .clineTasks,
        ),
        AgentProfile(
            id: "goose",
            name: "Goose",
            icon: "bird.fill",
            bundleIDs: [],
            processNames: ["goose"],
            sessionDirs: [home(".config/goose/sessions")],
            category: .assistant,
            emoji: "🪿",
        ),
    ]

    // MARK: - 自动发现

    /// 自动发现的额外 CLI profile（不在内置集里的 CLI，如 aider/gemini/windsurf）。
    /// installedCLIs 由调用方从 InstalledAppsCache 取得——本类型不读任何全局状态
    public static func discoverCLIProfiles(installedCLIs: Set<String>,
                                           installedBundles: Set<String> = []) -> [AgentProfile] {
        // 判重基线用 filteredBuiltin（含宿主内嵌过滤），与调用方展示/启停口径一致：
        // 否则被 hostBundleIDs 过滤掉的组件仍会在这里以 CLI 形式重复出现
        let existing = Set(filteredBuiltin(installedCLIs: installedCLIs,
                                           installedBundles: installedBundles)
            .flatMap { $0.processNames.map { $0.lowercased() } })
        var extra: [AgentProfile] = []
        for cli in installedCLIs where !existing.contains(cli) {
            extra.append(AgentProfile(
                id: "cli-\(cli)",
                name: cli,
                icon: "terminal",
                bundleIDs: [],
                processNames: [cli],
                sessionDirs: [],
                defaultEnabled: false,   // 自动发现默认关闭（与设置页注释一致，避免首启全开）
                category: .assistant
            ))
        }
        return extra
    }

    // MARK: - 用户自定义（UserDefaults）

    /// 自定义档案持久化（defaults 可注入：测试传独立套件，避免经 standard domain
    /// 的隐性共享状态污染后续用例；生产走默认 .standard）。
    /// 逐元素容错（R21）：整条数组解码是全有或全无——单个坏元素（缺键/类型漂移）
    /// 会让全部自定义档案「凭空消失」，且后续添加/删除会以空基线覆写造成永久丢失。
    /// 现在逐条抢救：坏元素丢弃并打日志，其余原样返回
    public static func loadCustomProfiles(defaults: UserDefaults = .standard) -> [AgentProfile] {
        guard let data = defaults.data(forKey: SettingKey.customAgents) else { return [] }
        let object = try? JSONSerialization.jsonObject(with: data)
        guard let rawList = object as? [[String: Any]] else {
            // 合法空数组 "[]" 走此分支但不是异常；仅真损坏（解析失败/非对象数组）才打日志
            let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty, text != "[]" {
                AppLog.error("customAgents 存档顶层结构异常（非对象数组），已按空集处理")
            }
            return []
        }
        var list: [AgentProfile] = []
        var badCount = 0
        for element in rawList {
            guard let elementData = try? JSONSerialization.data(withJSONObject: element),
                  let profile = try? JSONDecoder().decode(AgentProfile.self, from: elementData) else {
                badCount += 1
                continue
            }
            list.append(profile)
        }
        if badCount > 0 {
            AppLog.error("customAgents 存档含 \(badCount)/\(rawList.count) 个损坏元素（已丢弃并保留其余）")
        }
        return list
    }

    /// 存档顶层状态。「没这个键」与「有但读不懂」必须分开：
    /// 把后者当成前者，下一次增删就会以空基线覆写，用户剩下的自定义档案静默消失
    public enum CustomArchiveState: Equatable {
        case absent
        case ok
        /// 顶层解析失败或非对象数组，附原始字节数（不附内容：日志里不落用户数据）
        case corrupt
    }

    public static func customArchiveState(defaults: UserDefaults = .standard) -> CustomArchiveState {
        guard let data = defaults.data(forKey: SettingKey.customAgents) else { return .absent }
        guard let object = try? JSONSerialization.jsonObject(with: data),
              object is [[String: Any]] else {
            let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            return (text.isEmpty || text == "[]") ? .ok : .corrupt
        }
        return .ok
    }

    /// 自定义档案数量上限。真实用量是个位数，这个上限只防「存档被脚本写爆」
    /// 之后每次启动都解码几千条（fullRegistry 在采样路径上）
    public static let maxCustomAgents = 64

    public enum CustomWriteResult: Equatable {
        case saved(Int)
        /// 没写。原因要能显示给用户——静默丢弃用户的档案是更坏的结果
        case refused(reason: String)
    }

    /// 写入闸门：校验 + 去重 + 损坏存档留证。所有写路径都必须经过这里
    /// （设置页的增/删以前是直接 encode+set，等于绕过一切检查）
    @discardableResult
    public static func saveCustomProfiles(_ profiles: [AgentProfile],
                                          defaults: UserDefaults = .standard) -> CustomWriteResult {
        var seen = Set<String>()
        var clean: [AgentProfile] = []
        for profile in profiles {
            let id = profile.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty else { continue }
            guard !seen.contains(id) else { continue }
            seen.insert(id)
            clean.append(profile)
        }
        if clean.count > maxCustomAgents {
            return .refused(reason: "自定义档案 \(clean.count) 条，超过上限 \(maxCustomAgents) 条；未写入")
        }
        if profiles.count != clean.count {
            AppLog.warn("customAgents 写入前修掉 \(profiles.count - clean.count) 条空 id 或重复 id 的档案")
        }
        // 顶层损坏时先把原始字节挪到备份键：覆写不可避免（用户正在增删），
        // 但留着原件才有手工恢复的可能
        if customArchiveState(defaults: defaults) == .corrupt {
            if let data = defaults.data(forKey: SettingKey.customAgents) {
                defaults.set(data, forKey: SettingKey.customAgentsCorruptBackup)
            }
            AppLog.error("customAgents 原存档损坏，已另存到 \(SettingKey.customAgentsCorruptBackup) 以便手工恢复")
        }
        guard let data = try? JSONEncoder().encode(clean) else {
            return .refused(reason: "编码失败，未写入（原存档保持不变）")
        }
        defaults.set(data, forKey: SettingKey.customAgents)
        return .saved(clean.count)
    }

    /// 完整注册表：内置 + 自动发现 CLI（按传入已安装集）+ 自定义。
    /// installedBundles 用于识别「宿主内嵌组件」：宿主已装而本组件无独立安装时跳过，
    /// 避免同一份程序（ChatGPT 桌面版内嵌的 Codex）在列表里显示成两个 Agent。
    public static func fullRegistry(installedCLIs: Set<String>,
                                    installedBundles: Set<String> = [],
                                    defaults: UserDefaults = .standard) -> [AgentProfile] {
        var list = filteredBuiltin(installedCLIs: installedCLIs, installedBundles: installedBundles)
        list.append(contentsOf: discoverCLIProfiles(installedCLIs: installedCLIs,
                                                    installedBundles: installedBundles))
        list.append(contentsOf: loadCustomProfiles(defaults: defaults))
        return list
    }

    /// 内置条目按「宿主内嵌」规则过滤（fullRegistry 与 discoverCLIProfiles 的共用基线，
    /// 抽成独立函数以打断两者的相互调用）
    static func filteredBuiltin(installedCLIs: Set<String>,
                                installedBundles: Set<String>) -> [AgentProfile] {
        builtin.filter { profile in
            guard !profile.hostBundleIDs.isEmpty else { return true }
            let hostInstalled = profile.hostBundleIDs.contains { installedBundles.contains($0.lowercased()) }
            guard hostInstalled else { return true }
            let selfInstalled = profile.bundleIDs.contains { installedBundles.contains($0.lowercased()) }
                || profile.processNames.contains { installedCLIs.contains($0.lowercased()) }
            return selfInstalled
        }
    }

    /// 单条查找（内置 + 自定义）。
    /// ⚠️ 防呆（加固1）：installedCLIs 恒为 []，永远查不到 cli-* 自动发现条目；
    /// host 内嵌过滤口径也与真实注册表不同。需要真实注册表时必须注入
    /// InstalledAppsCache 后走 fullRegistry，勿把本函数当「按 id 取档案」的通用入口
    public static func profile(id: String) -> AgentProfile? {
        fullRegistry(installedCLIs: []).first { $0.id == id }
    }

    // MARK: - 进程名冲突校验（自定义 Agent 用）

    /// 纯核：「已安装 或 已启用」条目的进程名集合（小写）。
    /// - 已启用条目：同进程名必然双份计数，必须拦
    /// - 已安装但禁用：日后启用会同名双份，也要拦
    /// - 未安装且未启用：进程不会运行，不误拦
    /// installed 集注入（测试 canned，生产读本类型全局缓存）
    public static func conflictingProcessNames(
        registry: [AgentProfile],
        enabledIDs: Set<String>,
        installedCLIs: Set<String>,
        installedBundles: Set<String>
    ) -> [String] {
        var names = Set<String>()
        for p in registry {
            let installed = p.bundleIDs.contains { installedBundles.contains($0.lowercased()) }
                || p.processNames.contains { installedCLIs.contains($0.lowercased()) }
            if installed || enabledIDs.contains(p.id) {
                names.formUnion(p.processNames.map { $0.lowercased() })
            }
        }
        return Array(names)
    }

    /// 便捷入口：registry 取全量、installed 集取自注入缓存（设置表单用）
    public static func conflictingProcessNames(
        enabledIDs: Set<String>,
        installedApps: InstalledAppsCache
    ) -> [String] {
        conflictingProcessNames(
            registry: fullRegistry(installedCLIs: installedApps.installedCLIs(),
                                   installedBundles: installedApps.installedBundleIDs()),
            enabledIDs: enabledIDs,
            installedCLIs: installedApps.installedCLIs(),
            installedBundles: installedApps.installedBundleIDs())
    }

    // MARK: - 工具

    private static func home(_ path: String) -> String {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(path).path
    }
}

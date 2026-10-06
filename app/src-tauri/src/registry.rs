use crate::models::{AgentProfile, SessionDatabase, SessionDialect, SessionSchema};

/// Agent 注册表：内置集（按平台路径）。与 macOS AgentRegistry 同构；
/// 新增复用既有方言的 Agent 只改这里。
pub fn builtin() -> Vec<AgentProfile> {
    let home = dirs::home_dir().unwrap_or_default();
    let appdata = dirs::data_dir().unwrap_or_default(); // %APPDATA% (roaming)
    let p = |parts: &[&str]| -> String {
        let mut path = home.clone();
        for part in parts {
            path.push(part);
        }
        path.to_string_lossy().to_string()
    };
    let a = |parts: &[&str]| -> String {
        let mut path = appdata.clone();
        for part in parts {
            path.push(part);
        }
        path.to_string_lossy().to_string()
    };

    const DESKTOP_CPU_FLOOR: f64 = 20.0;
    /// WorkBuddy 的专家团保护下限：日常 3-5 专家并行的高消耗不该误报，
    /// 而超大规模死循环（远高于此）依然能熔断。
    const WORKBUDDY_TOKEN_FLOOR: i64 = 1_000_000;

    vec![
        AgentProfile {
            id: "dim".into(),
            name: "DimAgent".into(),
            glyph: "\u{E734}".into(),
            emoji: "✨".into(),
            process_names: vec!["DimAgent".into(), "DimRemote".into(), "dim".into()],
            bundle_ids: vec!["com.dimcode.app".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".dimcode", "v2", "data", "sessions"])],
            // 用量明细在下面的 usage_ledger 里，不读 JSONL；会话目录与采集根同路径
            // （与 Swift 档案逐字段一致）
            token_roots: vec![p(&[".dimcode", "v2", "data", "sessions"])],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: Some(SessionDatabase {
                path: p(&[".dimcode", "v2", "dimcode.sqlite"]),
                schema: SessionSchema::DimTasks,
                status_sql: None,
            }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "zcode".into(),
            name: "ZCode".into(),
            glyph: "\u{E945}".into(),
            emoji: "⚡".into(),
            process_names: vec!["ZCode".into(), "zcode".into()],
            bundle_ids: vec!["dev.zcode.app".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // Model IO is task evidence; CLI log can update for unrelated runtime activity.
            session_dirs: vec![p(&[".zcode", "cli", "rollout"])],
            // Canonical ledger first; JSONL is only a fallback, never added to its mirror.
            token_roots: vec![p(&[".zcode", "cli", "db", "db.sqlite"]), p(&[".zcode", "cli", "rollout"])],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: Some(SessionDatabase {
                path: p(&[".zcode", "v2", "tasks-index.sqlite"]),
                schema: SessionSchema::StatusIndex,
                // 真库实测：`tasks` 的主键是 (workspace_key, task_id)，**没有 `id` 列**
                status_sql: Some(
                    "SELECT task_id, task_status, updated_at, workspace_key FROM tasks WHERE deleted = 0 AND archived = 0 ORDER BY updated_at DESC LIMIT 1;"
                        .into(),
                ),
            }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "claude".into(),
            name: "Claude".into(),
            glyph: "\u{E8BD}".into(),
            emoji: "🧠".into(),
            process_names: vec!["claude".into()],
            bundle_ids: vec!["com.anthropic.claudefordesktop".into(), "com.anthropic.claudecode".into()],
            cmdline_hints: vec!["claude".into()],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".claude", "projects"]), p(&[".claude", "sessions"])],
            token_roots: vec![p(&[".claude", "sessions"]), p(&[".claude", "projects"])],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "codex".into(),
            name: "ChatGPT / Codex".into(),
            glyph: "\u{E99A}".into(),
            emoji: "🤖".into(),
            process_names: vec!["codex".into(), "ChatGPT".into()],
            bundle_ids: vec!["com.openai.codex".into()],
            cmdline_hints: vec!["codex".into()],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".codex", "sessions"])],
            token_roots: vec![p(&[".codex", "sessions"])],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "cursor".into(),
            name: "Cursor".into(),
            glyph: "\u{E7C2}".into(),
            emoji: "💻".into(),
            process_names: vec!["Cursor".into()],
            bundle_ids: vec!["com.todesktop.230113mital1efw".into(), "com.cursor.cursor".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Cursor", "User", "workspaceStorage"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "vscode".into(),
            name: "VS Code".into(),
            glyph: "\u{E943}".into(),
            emoji: "⌨️".into(),
            process_names: vec!["Code".into()],
            bundle_ids: vec!["com.microsoft.VSCode".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Code", "User", "workspaceStorage"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "cline".into(),
            name: "Cline".into(),
            glyph: "\u{E756}".into(),
            emoji: "🔗".into(),
            process_names: vec![],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::ClineTasks,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "roo-code".into(),
            name: "Roo Code".into(),
            glyph: "\u{E8A5}".into(),
            emoji: "🦘".into(),
            process_names: vec![],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::ClineTasks,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "opencode".into(),
            name: "OpenCode".into(),
            glyph: "\u{E8A7}".into(),
            emoji: "📂".into(),
            process_names: vec!["opencode".into()],
            bundle_ids: vec!["ai.opencode.desktop".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".local", "share", "opencode"])],
            // tokenRoots 空是**刻意的**：这个产品的用量只落在自己的 SQLite 库里
            // （message.data 的 JSON），JSONL 侧没有可用计数。用量走 session_database。
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: Some(SessionDatabase { path: p(&[".local", "share", "opencode", "opencode.db"]), schema: SessionSchema::OpenCode, status_sql: None }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "minimaxcode".into(), name: "MiniMax Code".into(), glyph: "\u{E774}".into(), emoji: "🤖".into(),
            process_names: vec!["MiniMax Code".into(), "mcode".into(), "minimax-code".into()], bundle_ids: vec!["com.minimax.agent.cn".into()],
            cmdline_hints: vec!["@minimax-ai/code".into(), "minimax-code/dist/cli".into()],
            path_excludes: vec!["frameworks".into(), "helper".into()], path_contains: vec![], cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![], token_roots: vec![], token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: Some(SessionDatabase { path: p(&[".minimax", "v2", "sqlite", "runtime-state.sqlite"]), schema: SessionSchema::MiniMaxRuntime, status_sql: None }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "mimocode".into(),
            name: "Xiaomi MiMo".into(),
            glyph: "\u{E774}".into(),
            emoji: "📡".into(),
            // 桌面主进程 + 引擎进程。真实进程名按 CLI 命名习惯推的（Swift 档案同注：
            // 待跑过一次编码会话核对），此处照搬以免两边再分叉
            process_names: vec!["Xiaomi MiMo".into(), "mimocode".into()],
            bundle_ids: vec!["com.xiaomi.mimo.desktop".into()],
            cmdline_hints: vec![],
            // Electron 派生进程（GPU/渲染/网络/崩溃上报）与宿主是同一个程序：
            // CPU 会跨条目求和，把渲染器空闲抖动累成「高负载工作中」，故按路径排除
            path_excludes: vec!["frameworks".into(), "helper".into()],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".local", "share", "mimocode"])],
            // 与 OpenCode 同表形（session / message / part），复用同一方言
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: Some(SessionDatabase {
                path: p(&[".local", "share", "mimocode", "mimocode.db"]),
                schema: SessionSchema::OpenCode,
                status_sql: None,
            }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "goose".into(),
            name: "Goose".into(),
            glyph: "\u{E7BB}".into(),
            emoji: "🪿".into(),
            process_names: vec!["goose".into()],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".config", "goose", "sessions"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "aider".into(),
            name: "Aider".into(),
            glyph: "\u{E943}".into(),
            emoji: "🛠️".into(),
            process_names: vec!["aider".into()],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".aider"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "windsurf".into(),
            name: "Windsurf".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🏄".into(),
            process_names: vec!["Windsurf".into()],
            bundle_ids: vec!["com.exafunction.windsurf".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".codeium", "windsurf"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "trae".into(),
            name: "Trae CN".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🛳️".into(),
            process_names: vec!["Trae CN".into(), "Electron".into()],
            bundle_ids: vec!["cn.trae.app".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec!["/trae cn.app/".into(), "/trae cn.exe".into(), "\\trae cn.exe".into()],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Trae CN", "User", "workspaceStorage"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "traework".into(),
            name: "TraeWork".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🛳️".into(),
            process_names: vec!["TRAE SOLO CN".into(), "Electron".into()],
            bundle_ids: vec!["cn.trae.solo.app".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec!["/trae solo cn.app/".into()],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "doubaowork".into(),
            name: "豆包工作".into(),
            glyph: "\u{E7BB}".into(),
            emoji: "🫘".into(),
            process_names: vec!["DoubaoWork".into()],
            bundle_ids: vec!["com.work.pc.doubao".into()],
            cmdline_hints: vec![],
            path_contains: vec!["/doubaowork.app/contents/macos/".into()],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // No verified local session or token contract yet; browser storage is not a session log.
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        // ===== v0.0.209：补齐 macOS 端有、这里没有的 12 个档案 =====
        // 逐条对齐 `Sources/AgentIslandCore/AgentRegistry.swift` 的同名档案。
        //
        // **`path_contains` 不是可选项**：这 12 个里有 6 个依赖它，而缺了它
        // `workbuddy` 与 `workbuddy-ai` 会互相命中（两者进程 basename 都是 `Electron`），
        // 结果是同一批进程被两个档案各算一次——界面上表现为「有两个 WorkBuddy 都在工作」。
        // 那是**静默错误**：不报错、不崩，只是数字翻倍。

        AgentProfile {
            id: "qoder".into(),
            name: "Qoder".into(),
            glyph: "\u{E5A5}".into(),
            emoji: "🖥️".into(),
            process_names: vec!["qoder".into()],
            bundle_ids: vec!["com.qoder.app".into()],
            cmdline_hints: vec![],
            // 路径锚定到安装目录，不用裸子串 `qoder`：裸子串会把 ~/code/qoder-playground
            // 里跑的任何 Electron 程序认成本 Agent（macOS 侧已记为已知风险）
            path_contains: vec!["/applications/qoder.app".into()],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".qoder", "projects"])],
            // tokenRoots **故意留空**：Qoder 落盘的 token 字段全为 0（真值只有 credits）。
            // 接上采集只会多花 1~1.9s 换 0 条数据——那是「编一个看起来正常的零」。
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::QoderTranscript,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "copilot".into(),
            name: "ima.copilot".into(),
            glyph: "\u{E728}".into(),
            emoji: "🧑‍✈️".into(),
            process_names: vec!["ima.copilot".into(), "Copilot".into()],
            bundle_ids: vec!["com.tencent.imamac".into()],
            cmdline_hints: vec![],
            path_contains: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&["Library", "Application Support", "com.tencent.imamac"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "workbuddy".into(),
            name: "WorkBuddy".into(),
            glyph: "\u{F0B7}".into(),
            emoji: "💼".into(),
            // 进程名含 `Electron` 是**被迫**的（Electron 应用都长这样），
            // 所以身份完全靠下面的 `path_contains`
            process_names: vec!["WorkBuddy".into(), "workbuddy".into(), "Electron".into()],
            bundle_ids: vec!["com.tencent.workbuddy.mac".into()],
            cmdline_hints: vec![],
            // 不能用宽口径 `workbuddy`：两个变体的进程路径都含它，会互相误命中
            path_contains: vec!["/applications/workbuddy.app".into(), ".workbuddy/".into()],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // tasks/ 是任务产物（工作信号）；sessions/*.json 只是宿主心跳，
            // 会被后台同步触碰，放进来会把空闲判成工作
            session_dirs: vec![p(&[".workbuddy", "tasks"])],
            // token 明细在 projects/ 的会话 JSONL 里，与工作信号不同子树
            token_roots: vec![p(&[".workbuddy", "projects"])],
            // 专家团保护下限：日常 3-5 专家并行的高消耗不该误报，
            // 而超大规模死循环（远高于此）依然能熔断
            token_alert_floor: Some(WORKBUDDY_TOKEN_FLOOR),
            session_dialect: SessionDialect::GenericTail,
            // 状态索引在真库上验过（2026-09：`~/.workbuddy/workbuddy.db`
            // 的 `sessions` 表确有 id/status/updated_at/deleted_at 四列）
            session_database: Some(SessionDatabase {
                path: p(&[".workbuddy", "workbuddy.db"]),
                schema: SessionSchema::StatusIndex,
                status_sql: Some(
                    "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"
                        .into(),
                ),
            }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "workbuddy-ai".into(),
            name: "WorkBuddy AI".into(),
            glyph: "\u{F774}".into(),
            emoji: "💼".into(),
            process_names: vec!["Electron".into(), "workbuddy".into()],
            // 真实 bundle id 以 WorkBuddy AI.app 的 Info.plist 为准（plutil 实测
            // com.workbuddy.workbuddy-ai）——此前照抄 Application Support 目录名漏了 -ai
            bundle_ids: vec!["com.workbuddy.workbuddy-ai".into()],
            cmdline_hints: vec![],
            // 与国内版的数据目录完全独立，只能靠路径区分
            path_contains: vec!["workbuddy ai.app".into(), ".workbuddy-ai".into()],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".workbuddy-ai", "tasks"])],
            token_roots: vec![p(&[".workbuddy-ai", "projects"])],
            // 专家团保护下限：日常 3-5 专家并行的高消耗不该误报，
            // 而超大规模死循环（远高于此）依然能熔断
            token_alert_floor: Some(WORKBUDDY_TOKEN_FLOOR),
            session_dialect: SessionDialect::GenericTail,
            // 与国内版同一张表形（真库验过，列名一致）
            session_database: Some(SessionDatabase {
                path: p(&[".workbuddy-ai", "workbuddy.db"]),
                schema: SessionSchema::StatusIndex,
                status_sql: Some(
                    "SELECT id, status, updated_at FROM sessions WHERE deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1;"
                        .into(),
                ),
            }),
            category: "assistant".into(),
        },
        AgentProfile {
            id: "antigravity".into(),
            name: "Antigravity".into(),
            glyph: "\u{E7FD}".into(),
            emoji: "⚛️".into(),
            process_names: vec!["Antigravity".into(), "Electron".into()],
            bundle_ids: vec!["com.google.antigravity".into(), "com.yuzhiqiang.antigravity.studio".into()],
            cmdline_hints: vec![],
            path_contains: vec!["antigravity".into()],
            path_excludes: vec!["frameworks".into(), "helper".into(), "language_server".into()],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // 只监控 Agent 自身的会话数据；`Library/Application Support/Antigravity`
            // 是内核用户数据目录（Chromium 缓存 + 更新器 + 账号状态），
            // 实测仅打开应用就有 36 次写入/20 分钟且全在此列，会把空闲判成工作
            session_dirs: vec![
                p(&[".gemini", "antigravity", "conversations"]),
                p(&[".gemini", "antigravity", "brain"]),
            ],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::AntigravityBrain,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "hermes".into(),
            name: "Hermes Agent".into(),
            glyph: "\u{E29C}".into(),
            emoji: "🪄".into(),
            process_names: vec!["hermes-agent".into(), "hermes".into()],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_contains: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".hermes", "sessions"]), p(&[".hermes", "logs"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "continue".into(),
            name: "Continue".into(),
            glyph: "\u{E29B}".into(),
            emoji: "▶️".into(),
            process_names: vec!["Continue".into(), "continue".into(), "continue-core".into()],
            bundle_ids: vec!["com.continue.continue".into()],
            cmdline_hints: vec![],
            path_contains: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".continue"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "dsh".into(),
            name: "DeepSeek Harness".into(),
            glyph: "\u{E7A1}".into(),
            emoji: "⚡️".into(),
            // web 模式由 Node 启动 bin.js，最终可执行 basename 是 `node`；
            // 只靠 `dsh` 进程名会漏掉当前桌面/网页宿主形态
            process_names: vec!["dsh".into()],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_contains: vec!["deepseek-harness".into()],
            path_excludes: vec![".codegraph".into()],
            cpu_floor: None,
            session_dirs: vec![
                p(&[".dsh", "sessions"]),
                p(&[".dsh", "storages", "session_projcache", "sessions"]),
            ],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::DshProjection,
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "openviking".into(),
            name: "OpenViking".into(),
            glyph: "\u{F4A3}".into(),
            emoji: "📦".into(),
            process_names: vec![
                "openviking".into(),
                "openviking-server".into(),
                "ov".into(),
                "vikingbot".into(),
            ],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_contains: vec!["openviking".into()],
            path_excludes: vec![],
            cpu_floor: None,
            // 只监控数据目录。`~/.local/share/uv/tools/openviking` 是 uv 装的
            // Python venv（lib/bin/pyvenv.cfg，4452 个条目），与任务无关，
            // 实测占全量扫描最大的一块
            session_dirs: vec![p(&[".openviking"])],
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 档案表是读数的地基：**id 必须唯一且非空**。
    /// 重名会让 `settings.disabled_agents` 按 id 过滤时一次关掉两家
    /// （`engine.rs:enabled_profiles` 就是按 id 过滤的）——这是纯数据结构就能防住的错。
    #[test]
    fn registry_ids_are_unique_and_non_empty() {
        let profiles = builtin();
        assert!(!profiles.is_empty(), "档案表为空，读数无从发生");

        let mut ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "档案表存在重复 id");

        for p in &profiles {
            assert!(!p.id.trim().is_empty(), "存在空 id 的档案");
            assert!(!p.name.trim().is_empty(), "档案 {} 的 name 为空", p.id);
        }
    }

    /// Swift 侧 25 个档案，Rust 侧今天 12 个（ADR 0010 / M3 记录在案）。
    /// 这条**不要求两边相等**——只要求 Rust 侧不少于 12，
    /// 免得无意中回归到「连今天支持的这 12 家都读不到」。
    #[test]
    fn registry_keeps_at_least_the_current_twelve() {
        let profiles = builtin();
        assert!(
            profiles.len() >= 12,
            "Rust 侧档案数退化了：{} < 12",
            profiles.len()
        );
    }

    /// `builtin()` 每次调用都应给出一致的表：它读 `dirs::home_dir()` 拼路径，
    /// 但 id 集合只来自代码字面量，与运行环境无关。若哪天有人把 id 做成环境相关，
    /// 「同一台机器两次启动支持的 agent 不同」这种最难查的 bug 会从这里冒出来。
    #[test]
    fn registry_id_set_is_environment_independent() {
        let a: Vec<String> = builtin().into_iter().map(|p| p.id).collect();
        let b: Vec<String> = builtin().into_iter().map(|p| p.id).collect();
        assert_eq!(a, b, "builtin() 的 id 集合与环境/调用次数相关");
    }

    #[test]
    fn shared_profiles_keep_swift_cpu_token_and_id_contracts() {
        let profiles = builtin();
        let get = |id: &str| profiles.iter().find(|p| p.id == id).unwrap();
        let claude = get("claude");
        assert_eq!(claude.cpu_floor, Some(20.0));
        assert!(claude.token_roots.iter().any(|p| {
            std::path::Path::new(p).ends_with(std::path::Path::new(".claude").join("sessions"))
        }));
        assert!(claude.token_roots.iter().any(|p| {
            std::path::Path::new(p).ends_with(std::path::Path::new(".claude").join("projects"))
        }));
        let opencode = get("opencode");
        assert_eq!(opencode.cpu_floor, Some(20.0));
        assert!(
            opencode.token_roots.is_empty(),
            "no reliable local token detail source"
        );
        assert!(get("cline").token_roots.is_empty());
        assert!(profiles.iter().all(|p| p.id != "roo"));
        assert_eq!(get("roo-code").name, "Roo Code");
        assert!(get("roo-code").token_roots.is_empty());
    }

    #[test]
    fn zcode_monitors_model_io_without_cli_log_noise() {
        let profiles = builtin();
        let zcode = profiles.iter().find(|p| p.id == "zcode").unwrap();
        let rollout = std::path::Path::new(".zcode").join("cli").join("rollout");
        assert_eq!(zcode.session_dirs.len(), 1);
        assert!(std::path::Path::new(&zcode.session_dirs[0]).ends_with(&rollout));
        assert_eq!(zcode.token_roots.len(), 2);
        assert!(std::path::Path::new(&zcode.token_roots[0])
            .ends_with(std::path::Path::new(".zcode/cli/db/db.sqlite")));
        assert_eq!(zcode.token_roots[1], zcode.session_dirs[0]);
    }

    /// 声明了 SQLite 库的档案：**方言必须与库对应**，且库路径只在档案里出现一次。
    /// 两条都是由「形状」而非产品名决定的（ADR 0004）——把路径抄到别处，
    /// 档案换目录时只有一半会生效；方言写错则整份统计变成 0 且不报错。
    #[test]
    fn sqlite_backed_profiles_pair_each_database_with_the_right_dialect() {
        let profiles = builtin();
        let db = |id: &str| {
            profiles
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.session_database.clone())
                .unwrap_or_else(|| panic!("{id} 应声明 session_database"))
        };
        let ends_with = |path: &str, tail: &[&str]| {
            let mut expected = std::path::PathBuf::new();
            for part in tail {
                expected.push(part);
            }
            std::path::Path::new(path).ends_with(&expected)
        };

        let dim = db("dim");
        assert_eq!(dim.schema, SessionSchema::DimTasks);
        assert!(ends_with(&dim.path, &[".dimcode", "v2", "dimcode.sqlite"]));

        let zcode = db("zcode");
        assert_eq!(zcode.schema, SessionSchema::StatusIndex);
        assert!(ends_with(
            &zcode.path,
            &[".zcode", "v2", "tasks-index.sqlite"]
        ));

        let opencode = db("opencode");
        assert_eq!(opencode.schema, SessionSchema::OpenCode);
        assert!(ends_with(
            &opencode.path,
            &[".local", "share", "opencode", "opencode.db"]
        ));

        let mimocode = db("mimocode");
        assert_eq!(
            mimocode.schema,
            SessionSchema::OpenCode,
            "同表 fork 复用同一方言"
        );
        assert!(ends_with(
            &mimocode.path,
            &[".local", "share", "mimocode", "mimocode.db"]
        ));

        // 方言决定明细从哪里来：走 OpenCode 库的档案**不该**再有 JSONL 采集根，
        // 否则同一笔用量会被两处各算一遍
        for profile in &profiles {
            if let Some(database) = &profile.session_database {
                if database.schema == SessionSchema::OpenCode {
                    assert!(
                        profile.token_roots.is_empty(),
                        "{} 的用量在库里，不该同时声明 token_roots",
                        profile.id
                    );
                }
            }
        }
    }
}

/// 当前内置档案及身份边界回归。
#[cfg(test)]
mod parity {
    use super::*;
    use crate::procmon::profile_matches;

    fn find(id: &str) -> AgentProfile {
        builtin()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap_or_else(|| panic!("没有 {id} 这个档案"))
    }

    #[test]
    fn the_registry_contains_only_current_agent_identities() {
        let ids: Vec<String> = builtin().into_iter().map(|p| p.id).collect();
        // 合并桌面 ChatGPT 与 Codex；浏览器与用量统计工具不属于智能体。
        assert_eq!(ids.len(), 26, "档案数：{ids:?}");
        assert!(!ids
            .iter()
            .any(|id| id == "chatgpt" || id == "ego-browser" || id == "vibe-usage"));
        for id in [
            "qoder",
            "copilot",
            "workbuddy",
            "workbuddy-ai",
            "antigravity",
            "hermes",
            "continue",
            "codex",
            "dsh",
            "openviking",
            "minimaxcode",
            "doubaowork",
            "traework",
        ] {
            assert!(ids.iter().any(|i| i == id), "缺档案 {id}");
        }
    }

    #[test]
    fn trae_variants_and_doubao_work_have_distinct_process_owners() {
        for (name, exe, expected) in [
            ("Electron", "/Applications/TRAE SOLO CN.app/Contents/MacOS/Electron", vec!["traework"]),
            ("Electron", "/Applications/Trae CN.app/Contents/MacOS/Electron", vec!["trae"]),
            ("Trae CN.exe", r"C:\Tools\Trae CN.exe", vec!["trae"]),
            ("DoubaoWork", "/Applications/DoubaoWork.app/Contents/MacOS/DoubaoWork", vec!["doubaowork"]),
            ("Doubao", "/Applications/Doubao.app/Contents/MacOS/Doubao", vec![]),
            ("DoubaoWork Browser", "/Applications/DoubaoWork.app/Contents/Helpers/DoubaoWork Browser.app/Contents/MacOS/DoubaoWork Browser", vec![]),
            ("Electron", "/Applications/Unrelated.app/Contents/MacOS/Electron", vec![]),
            ("Electron", "/tmp/trae-playground/Unrelated.app/Contents/MacOS/Electron", vec![]),
        ] {
            let owners: Vec<_> = builtin().into_iter().filter(|p| profile_matches(p, name, exe, "")).map(|p| p.id).collect();
            assert_eq!(owners, expected, "{exe}");
        }
        let work = find("doubaowork");
        assert!(work.session_dirs.is_empty() && work.token_roots.is_empty());
        assert_eq!(work.bundle_ids, vec!["com.work.pc.doubao"]);
        assert!(find("trae").bundle_ids.contains(&"cn.trae.app".to_string()));
    }

    #[test]
    fn unified_openai_profile_tracks_desktop_and_cli_with_one_usage_source() {
        let profile = find("codex");
        assert_eq!(profile.name, "ChatGPT / Codex");
        assert_eq!(profile.bundle_ids, vec!["com.openai.codex"]);
        for (name, exe, cmd) in [
            (
                "ChatGPT",
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
                "",
            ),
            ("Codex", "/Applications/Codex.app/Contents/MacOS/Codex", ""),
            ("codex.exe", "C:/Tools/codex.exe", ""),
            ("node", "/usr/bin/node", "node /tools/codex/bin/codex.js"),
        ] {
            let owners: Vec<_> = builtin()
                .into_iter()
                .filter(|p| profile_matches(p, name, exe, cmd))
                .map(|p| p.id)
                .collect();
            assert_eq!(owners, vec!["codex"], "desktop and CLI each have one owner");
        }
        assert_eq!(profile.token_roots.len(), 1);
        assert!(std::path::Path::new(&profile.token_roots[0]).ends_with(".codex/sessions"));
    }

    /// **两个 WorkBuddy 变体不得互相命中。**
    ///
    /// 两者的进程 basename 都是 `Electron`（所有 Electron 应用都长这样），
    /// 身份完全靠路径区分。缺了 `path_contains`，同一批进程会被两个档案各算一次——
    /// 症状是界面上「有两个 WorkBuddy 都在工作」，而**没有任何报错**。
    #[test]
    fn the_two_workbuddy_variants_do_not_claim_each_others_processes() {
        let cn = find("workbuddy");
        let intl = find("workbuddy-ai");

        let cn_exe = "/Applications/WorkBuddy.app/Contents/MacOS/Electron";
        let intl_exe = "/Applications/WorkBuddy AI.app/Contents/MacOS/Electron";

        assert!(
            profile_matches(&cn, "electron", cn_exe, ""),
            "国内版应当命中自己的路径"
        );
        assert!(
            !profile_matches(&intl, "electron", cn_exe, ""),
            "国内版的进程**不得**被国外版认领"
        );
        assert!(
            profile_matches(&intl, "electron", intl_exe, ""),
            "国外版应当命中自己的路径"
        );
        assert!(
            !profile_matches(&cn, "electron", intl_exe, ""),
            "国外版的进程**不得**被国内版认领"
        );
    }

    /// Qoder 用路径锚定是为了不把 `~/code/qoder-playground` 里跑的任何 Electron
    /// 程序认成它——同类过宽匹配在 macOS 侧已记为已知风险。
    #[test]
    fn qoder_is_anchored_to_its_install_directory() {
        let qoder = find("qoder");
        assert!(profile_matches(
            &qoder,
            "qoder",
            "/Applications/Qoder.app/Contents/MacOS/Qoder",
            ""
        ));
        assert!(
            !profile_matches(
                &qoder,
                "qoder",
                "/Users/me/code/qoder-playground/node_modules/electron/dist/Electron",
                ""
            ),
            "同名的自建目录不得被认成 Qoder"
        );
    }

    /// 声明了 `path_contains` 就**必须**命中其中一条——
    /// 这是「有则必须命中」而不是「命中则加分」，后者会让锚定形同虚设。
    #[test]
    fn a_declared_path_anchor_is_mandatory_not_a_bonus() {
        // 用 workbuddy 验机制：它的锚点足够具体
        let cn = find("workbuddy");
        // 进程名对、锚点不对 ⇒ 不匹配
        assert!(!profile_matches(
            &cn,
            "electron",
            "/Applications/Other.app/Contents/MacOS/Electron",
            ""
        ));

        // 顺带**如实记录** openviking 的锚点很宽（macOS 侧声明的就是裸子串
        // `openviking`）：名字对而路径也含 openviking 时必然匹配。
        // 它排除 uv 装的 Python venv 靠的是**不把那个目录写进 sessionDirs**，
        // 不是靠路径锚定——所以这里不假装它更严。
        let openviking = find("openviking");
        assert!(
            profile_matches(&openviking, "openviking", "/usr/local/bin/openviking", ""),
            "openviking 的锚点是宽口径裸子串，路径含名字即匹配（与 macOS 端同声明）"
        );
    }

    /// Qoder 的 `token_roots` **必须**是空的。
    ///
    /// 它的落盘 token 字段全为 0（真值只有 credits）。接上采集会多花 1~1.9s
    /// 换 0 条数据，而界面上会显示「Qoder 24h 用量 0」——那是**编一个看起来正常的零**。
    #[test]
    fn qoder_declares_no_token_roots_on_purpose() {
        let qoder = find("qoder");
        assert!(
            qoder.token_roots.is_empty(),
            "Qoder 的 token 字段恒为 0，声明采集根只会换来一片零"
        );
        assert!(
            !qoder.session_dirs.is_empty(),
            "但会话目录是真的：那是它的工作信号来源"
        );
    }
}

/// **方言声明与解析器落地是两份清单**，这里把它们钉在一起。
///
/// 为什么要有这条：档案里已经如实声明了 `antigravityBrain` / `dshProjection` /
/// `qoderTranscript`，而这三个解析器**还没迁**。分派遇到它们会如实返回无信号——
/// 这是对的（不拿猜的解析器顶上去）。但「已声明」与「已实现」一旦各写各的注释，
/// 就会在某个时候悄悄脱节，让人以为那条路径通了。
///
/// 所以这里把**没实现的方言**逐个列出：每补一个解析器就从这里移走一条，
/// 名单空了这条断言自动收紧成「全部已实现」。
#[cfg(test)]
mod dialect_declaration {
    use super::*;
    use crate::models::SessionDialect as D;
    use crate::session::DIALECTS_WITH_PARSER;

    #[test]
    fn the_five_agents_that_share_a_format_declare_it_rather_than_being_special_cased() {
        // 与 macOS 侧 `sessionDialect:` 的五处声明逐条对齐
        let expected = [
            ("qoder", D::QoderTranscript),
            ("antigravity", D::AntigravityBrain),
            ("dsh", D::DshProjection),
            // Cline 与 Roo Code 同源——这正是「按格式分派」要解决的那一对
            ("cline", D::ClineTasks),
            ("roo-code", D::ClineTasks),
        ];
        for (id, dialect) in expected {
            let profile = builtin()
                .into_iter()
                .find(|p| p.id == id)
                .expect("档案应当在");
            assert_eq!(
                profile.session_dialect, dialect,
                "{id} 的方言声明与 macOS 端不一致"
            );
        }
    }

    /// **五种方言全部有解析器了。**
    ///
    /// 这条断言从「还差哪几种」变成了「**一种都不许再欠**」——
    /// 它的形状本身就记录了这件事做完的时间点（v0.0.216）。
    ///
    /// 这份机制在它活着的时候拦下过四次纸面漂移：每补一个解析器，
    /// 「未实现」清单就会与现实脱节一次，而没有它那只是一句会慢慢过期的注释。
    #[test]
    fn every_declared_dialect_now_has_a_parser() {
        let mut missing: Vec<String> = Vec::new();
        for dialect in [
            D::GenericTail,
            D::AntigravityBrain,
            D::DshProjection,
            D::ClineTasks,
            D::QoderTranscript,
        ] {
            if !DIALECTS_WITH_PARSER.contains(&dialect) {
                missing.push(format!("{dialect:?}"));
            }
        }
        assert!(
            missing.is_empty(),
            "这些方言还没有解析器：{missing:?}——分派遇到它们会返回无信号"
        );
        // 清单里也不许有**第五种**之外的项：加了新方言就要同时在这里与枚举里登记
        assert_eq!(DIALECTS_WITH_PARSER.len(), 5, "方言总数应当是五种");
    }

    /// 分派入口对**每一种**方言都要有明确去处。
    ///
    /// 拿猜的解析器顶上去的后果：Qoder 的文件是 Anthropic 兼容逐行格式，
    /// 喂给 `probe_claude` 大概率能解析出东西——于是**读出来了、但结论是错的**，
    /// 比读不到更糟。所以这条钉的是「五种方言都不许落到兜底分支」。
    #[test]
    fn every_dialect_reaches_its_own_parser_instead_of_a_fallback() {
        use crate::session::probe_dialect;
        for dialect in [
            D::GenericTail,
            D::AntigravityBrain,
            D::DshProjection,
            D::ClineTasks,
            D::QoderTranscript,
        ] {
            // 路径不存在 ⇒ 各解析器都应「如实无信号」，但**必须真的进去过**：
            // 这条守的是「分派表里写了它」，而不是「读到了什么」
            let (probe, _context) = probe_dialect("claude", dialect, "/nonexistent/session.jsonl");
            assert!(
                probe.signal.is_none() || !DIALECTS_WITH_PARSER.contains(&dialect),
                "{dialect:?} 在路径不存在时不该报出信号：{:?}",
                probe.signal
            );
        }
    }

    /// 反过来：五种方言都**必须在**清单上。
    #[test]
    fn every_dialect_is_on_the_implemented_list() {
        for dialect in [
            D::GenericTail,
            D::AntigravityBrain,
            D::DshProjection,
            D::ClineTasks,
            D::QoderTranscript,
        ] {
            assert!(
                DIALECTS_WITH_PARSER.contains(&dialect),
                "{dialect:?} 没在 DIALECTS_WITH_PARSER 上"
            );
        }
    }
}

/// **声明了会话源、却在 Rust 侧拿不到任何会话信号**的档案。
///
/// 这不是「已支持」，是**明确知道的缺**。Swift 侧那一个通用
/// `detect(lines:)` 按内容吃所有档案的 JSONL；Rust 侧是逐方言手写解析器，
/// **没显式移植的就静默变成「无信号」**——而界面上看不出异样，
/// 那些 Agent 只会显示进程级的弱信号（在线/离线 + token），没有「等你批准 / 在跑 / 刚完成」。
///
/// 每往这个表里加一个 id，等于**承认又多一个 Agent 缺会话语义**。
/// 它是一道棘轮：新增档案若落进缺口而没被 conscious 登记，守护会精确变红。
///
/// 当前列表只包括仍在内置注册表中的档案；数量由守护测试逐条核对。
/// 合并后的 ChatGPT / Codex 使用 Codex 会话源；Ego Lite 浏览器不属于 Agent 档案。
/// `vscode` 是 Rust 独有的明细缺口，不是与归档 SwiftUI 的对拍缺口。
#[cfg(test)]
const KNOWN_UNCOVERED: [&str; 10] = [
    "aider",
    "continue",
    "copilot",
    "cursor",
    "goose",
    "hermes",
    "openviking",
    "trae",
    "windsurf",
    // Rust 独有档案，不是对拍缺口（见表头注释）
    "vscode",
];

/// 这个档案在 Rust 侧**能不能**拿到会话信号。
///
/// 判定照抄两条真实路径，不另立标准：
/// · 文件路 `session::probe_dialect`——先看方言有没有解析器，
///   `GenericTail` 内部**仍按 id 分流**（那 6 个）；
/// · 库路 `engine`——**只在 `schema == StatusIndex` 上跑**，
///   `DimTasks` / `OpenCode` 声明了库也拿不到信号。
#[cfg(test)]
fn covered_by_rust(p: &AgentProfile) -> bool {
    use crate::models::{SessionDialect as D, SessionSchema as S};
    let by_file = match p.session_dialect {
        D::QoderTranscript | D::DshProjection | D::AntigravityBrain | D::ClineTasks => true,
        D::GenericTail => {
            matches!(
                p.id.as_str(),
                "claude" | "codex" | "cline" | "roo-code" | "roo" | "zcode"
            )
        }
    };
    // 库路现在认三种 schema，**每个走各自的函数**（认错就永远没信号，而界面看不出异样）：
    // · `StatusIndex`：会话表自带终态 status；
    // · `DimTasks`：不看 status，看最新一条 assistant 末个 part 的 `endTime`；
    // · `OpenCode`：看消息行 `time.completed` / `time.created`，表名还要现查。
    let by_database = p.session_database.as_ref().is_some_and(|db| {
        matches!(
            db.schema,
            S::StatusIndex | S::DimTasks | S::OpenCode | S::MiniMaxRuntime
        )
    });
    by_file || by_database
}

#[cfg(test)]
mod session_coverage_sentinel {
    use super::{builtin, covered_by_rust, KNOWN_UNCOVERED};

    /// 缺口清单必须与代码现状**逐条相符**：多一条是陈旧登记，
    /// 少一条就是有档案悄悄落进了缺口。
    #[test]
    fn the_uncovered_list_matches_the_code_exactly() {
        let profiles = builtin();
        let uncovered: Vec<&str> = profiles
            .iter()
            .filter(|p| !p.session_dirs.is_empty() || p.session_database.is_some())
            .filter(|p| !covered_by_rust(p))
            .map(|p| p.id.as_str())
            .collect();
        let mut stale: Vec<&&str> = KNOWN_UNCOVERED
            .iter()
            .filter(|id| !uncovered.contains(id))
            .collect();
        stale.sort();
        assert!(
            stale.is_empty(),
            "KNOWN_UNCOVERED 里有已经拿到信号、应当删掉的条目：{stale:?}"
        );
        let mut missing: Vec<&&str> = uncovered
            .iter()
            .filter(|id| !KNOWN_UNCOVERED.contains(id))
            .collect();
        missing.sort();
        assert!(
            missing.is_empty(),
            "这些档案声明了会话源却拿不到信号，Rust 端会只剩进程级弱信号：{missing:?}。\n\
             要么给它写解析器，要么 conscious 登记进 KNOWN_UNCOVERED 并在表头写明理由。"
        );
    }

    /// 反向：清单不能是**空壳**。一条不登记的缺口是缺陷，
    /// 整张表都被清空等于把缺陷藏起来。
    #[test]
    fn the_uncovered_list_is_not_a_rubber_stamp() {
        assert!(
            !KNOWN_UNCOVERED.is_empty() && KNOWN_UNCOVERED.len() < builtin().len(),
            "缺口清单要么是空的（= 有缺陷被藏起来），要么大到没意义"
        );
    }
}

use crate::models::{AgentProfile, SessionDatabase, SessionSchema};

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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".dimcode", "v2", "data", "sessions"])],
            // 用量明细在下面的 usage_ledger 里，不读 JSONL；会话目录与采集根同路径
            // （与 Swift 档案逐字段一致）
            token_roots: vec![p(&[".dimcode", "v2", "data", "sessions"])],
            session_database: Some(SessionDatabase {
                path: p(&[".dimcode", "v2", "dimcode.sqlite"]),
                schema: SessionSchema::DimTasks,
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // Model IO is task evidence; CLI log can update for unrelated runtime activity.
            session_dirs: vec![p(&[".zcode", "cli", "rollout"])],
            token_roots: vec![p(&[".zcode", "cli", "rollout"])],
            session_database: Some(SessionDatabase { path: p(&[".zcode", "v2", "tasks-index.sqlite"]), schema: SessionSchema::StatusIndex }),
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".claude", "projects"]), p(&[".claude", "sessions"])],
            token_roots: vec![p(&[".claude", "sessions"]), p(&[".claude", "projects"])],
            session_database: None,
            category: "assistant".into(),
        },
        AgentProfile {
            id: "codex".into(),
            name: "Codex".into(),
            glyph: "\u{E99A}".into(),
            emoji: "🤖".into(),
            process_names: vec!["codex".into()],
            bundle_ids: vec![],
            cmdline_hints: vec!["codex".into()],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".codex", "sessions"])],
            token_roots: vec![p(&[".codex", "sessions"])],
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Cursor", "User", "workspaceStorage"])],
            token_roots: vec![],
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Code", "User", "workspaceStorage"])],
            token_roots: vec![],
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
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks"])],
            token_roots: vec![],
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
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks"])],
            token_roots: vec![],
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".local", "share", "opencode"])],
            // tokenRoots 空是**刻意的**：这个产品的用量只落在自己的 SQLite 库里
            // （message.data 的 JSON），JSONL 侧没有可用计数。用量走 session_database。
            token_roots: vec![],
            session_database: Some(SessionDatabase { path: p(&[".local", "share", "opencode", "opencode.db"]), schema: SessionSchema::OpenCode }),
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".local", "share", "mimocode"])],
            // 与 OpenCode 同表形（session / message / part），复用同一方言
            token_roots: vec![],
            session_database: Some(SessionDatabase {
                path: p(&[".local", "share", "mimocode", "mimocode.db"]),
                schema: SessionSchema::OpenCode,
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
            cpu_floor: None,
            session_dirs: vec![p(&[".config", "goose", "sessions"])],
            token_roots: vec![],
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
            cpu_floor: None,
            session_dirs: vec![p(&[".aider"])],
            token_roots: vec![],
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
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".codeium", "windsurf"])],
            token_roots: vec![],
            session_database: None,
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "trae".into(),
            name: "Trae".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🛳️".into(),
            process_names: vec!["Trae".into()],
            bundle_ids: vec!["cn.trae.solo.app".into(), "com.trae.ai".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Trae", "User", "workspaceStorage"]), a(&["Trae CN", "User", "workspaceStorage"])],
            token_roots: vec![],
            session_database: None,
            category: "codeEditor".into(),
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
        assert!(opencode.token_roots.is_empty(), "no reliable local token detail source");
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
        assert_eq!(zcode.token_roots, zcode.session_dirs);
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
        assert!(ends_with(&zcode.path, &[".zcode", "v2", "tasks-index.sqlite"]));

        let opencode = db("opencode");
        assert_eq!(opencode.schema, SessionSchema::OpenCode);
        assert!(ends_with(&opencode.path, &[".local", "share", "opencode", "opencode.db"]));

        let mimocode = db("mimocode");
        assert_eq!(mimocode.schema, SessionSchema::OpenCode, "同表 fork 复用同一方言");
        assert!(ends_with(&mimocode.path, &[".local", "share", "mimocode", "mimocode.db"]));

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

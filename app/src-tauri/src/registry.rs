use crate::models::AgentProfile;

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
            id: "zcode".into(),
            name: "ZCode".into(),
            glyph: "\u{E945}".into(),
            emoji: "⚡".into(),
            process_names: vec!["ZCode".into(), "zcode".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            // rollout = 每会话的模型 IO 流水（活动/动作/Token 信号源）；log = CLI 运行日志
            session_dirs: vec![p(&[".zcode", "cli", "rollout"]), p(&[".zcode", "cli", "log"])],
            token_roots: vec![p(&[".zcode", "cli", "rollout"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "claude".into(),
            name: "Claude".into(),
            glyph: "\u{E8BD}".into(),
            emoji: "🧠".into(),
            process_names: vec!["claude".into()],
            cmdline_hints: vec!["claude".into()],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".claude", "projects"]), p(&[".claude", "sessions"])],
            token_roots: vec![p(&[".claude", "projects"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "codex".into(),
            name: "Codex".into(),
            glyph: "\u{E99A}".into(),
            emoji: "🤖".into(),
            process_names: vec!["codex".into()],
            cmdline_hints: vec!["codex".into()],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".codex", "sessions"])],
            token_roots: vec![p(&[".codex", "sessions"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "cursor".into(),
            name: "Cursor".into(),
            glyph: "\u{E7C2}".into(),
            emoji: "💻".into(),
            process_names: vec!["Cursor".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Cursor", "User", "workspaceStorage"])],
            token_roots: vec![],
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "vscode".into(),
            name: "VS Code".into(),
            glyph: "\u{E943}".into(),
            emoji: "⌨️".into(),
            process_names: vec!["Code".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Code", "User", "workspaceStorage"])],
            token_roots: vec![],
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "cline".into(),
            name: "Cline".into(),
            glyph: "\u{E756}".into(),
            emoji: "🔗".into(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks"])],
            token_roots: vec![a(&["Code", "User", "globalStorage", "saoudrizwan.claude-dev", "tasks"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "roo".into(),
            name: "Roo Code".into(),
            glyph: "\u{E8A5}".into(),
            emoji: "🦘".into(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![a(&["Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks"])],
            token_roots: vec![a(&["Code", "User", "globalStorage", "rooveterinaryinc.roo-cline", "tasks"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "opencode".into(),
            name: "OpenCode".into(),
            glyph: "\u{E8A7}".into(),
            emoji: "📂".into(),
            process_names: vec!["opencode".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".local", "share", "opencode"])],
            token_roots: vec![p(&[".local", "share", "opencode"])],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "goose".into(),
            name: "Goose".into(),
            glyph: "\u{E7BB}".into(),
            emoji: "🪿".into(),
            process_names: vec!["goose".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".config", "goose", "sessions"])],
            token_roots: vec![],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "aider".into(),
            name: "Aider".into(),
            glyph: "\u{E943}".into(),
            emoji: "🛠️".into(),
            process_names: vec!["aider".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![p(&[".aider"])],
            token_roots: vec![],
            category: "assistant".into(),
        },
        AgentProfile {
            id: "windsurf".into(),
            name: "Windsurf".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🏄".into(),
            process_names: vec!["Windsurf".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![p(&[".codeium", "windsurf"])],
            token_roots: vec![],
            category: "codeEditor".into(),
        },
        AgentProfile {
            id: "trae".into(),
            name: "Trae".into(),
            glyph: "\u{E7C3}".into(),
            emoji: "🛳️".into(),
            process_names: vec!["Trae".into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: Some(DESKTOP_CPU_FLOOR),
            session_dirs: vec![a(&["Trae", "User", "workspaceStorage"]), a(&["Trae CN", "User", "workspaceStorage"])],
            token_roots: vec![],
            category: "codeEditor".into(),
        },
    ]
}

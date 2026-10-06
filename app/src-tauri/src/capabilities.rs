//! Read-only local inventories. Never returns commands, URLs, environment values or file contents.
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Serialize)]
pub struct CapabilityItem {
    pub name: String,
    pub kind: String,
    pub target: String,
    pub source: String,
    pub status: String,
}
#[derive(Debug, Default, Serialize)]
pub struct CapabilityInventory {
    pub items: Vec<CapabilityItem>,
    pub notices: Vec<String>,
}
fn display_name(name: &str) -> String {
    if name.len() > 160
        || name.chars().any(char::is_control)
        || name.contains("sk-")
        || name.contains("AIza")
    {
        "名称不在安全展示范围".into()
    } else {
        name.into()
    }
}
#[cfg(test)]
pub fn inventory_at(home: &Path) -> CapabilityInventory {
    inventory_in(home, &home.join(".codex"), "~/.codex")
}
fn inventory_in(home: &Path, codex: &Path, source: &str) -> CapabilityInventory {
    let mut out = CapabilityInventory::default();
    let config = codex.join("config.toml");
    if config.exists() {
        let doc = fs::metadata(&config)
            .ok()
            .filter(|m| m.len() <= 2_000_000)
            .and_then(|_| fs::read_to_string(&config).ok())
            .and_then(|s| s.parse::<toml_edit::DocumentMut>().ok());
        match doc {
            None => out
                .notices
                .push("Codex MCP 配置不可读；不代表没有配置".into()),
            Some(doc) => {
                if doc
                    .get("mcp_servers")
                    .is_some_and(|s| s.as_table_like().is_none())
                {
                    out.notices.push("Codex MCP 配置结构不可读".into());
                }
                if let Some(servers) = doc.get("mcp_servers").and_then(|s| s.as_table_like()) {
                    if servers.len() > 200 {
                        out.notices.push("MCP 清单仅展示前 200 项".into());
                    }
                    for (name, item) in servers.iter().take(200) {
                        let valid = ["command", "url"].iter().any(|key| {
                            item.get(key)
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| !s.trim().is_empty())
                        });
                        let disabled = item.get("enabled").and_then(|v| v.as_bool()) == Some(false);
                        out.items.push(CapabilityItem {
                            name: display_name(name),
                            kind: "MCP".into(),
                            target: "Codex".into(),
                            source: format!("{source}/config.toml"),
                            status: if disabled {
                                "配置为禁用"
                            } else if valid {
                                "已配置 · 未验证连接"
                            } else {
                                "缺少 command / url"
                            }
                            .into(),
                        });
                    }
                }
            }
        }
    }
    for (relative, target, source) in [
        (".codex/skills", "Codex", source),
        (".claude/skills", "Claude Code", "~/.claude/skills"),
        (".agents/skills", "共享目录", "~/.agents/skills"),
    ] {
        let path = if target == "Codex" {
            codex.join("skills")
        } else {
            home.join(relative)
        };
        let source = if target == "Codex" {
            format!("{source}/skills")
        } else {
            source.into()
        };
        if !path.exists() {
            continue;
        }
        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => {
                out.notices.push(format!("{source} 不可读"));
                continue;
            }
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        if entries.len() > 200 {
            out.notices.push(format!("{source} 仅检查前 200 项"));
        }
        for entry in entries.into_iter().take(200) {
            if !entry.path().is_dir() {
                continue;
            }
            let linked =
                fs::symlink_metadata(entry.path()).is_ok_and(|m| m.file_type().is_symlink());
            let valid = entry.path().join("SKILL.md").is_file();
            out.items.push(CapabilityItem {
                name: display_name(&entry.file_name().to_string_lossy()),
                kind: "Skill".into(),
                target: target.into(),
                source: source.clone(),
                status: if !valid {
                    "缺少 SKILL.md"
                } else if linked {
                    "链接目录 · 未验证加载"
                } else {
                    "本地目录 · 未验证加载"
                }
                .into(),
            });
        }
    }
    out
}
pub fn inventory() -> CapabilityInventory {
    dirs::home_dir()
        .map(|home| {
            let custom =
                std::env::var_os("CODEX_HOME").filter(|s| !s.to_string_lossy().trim().is_empty());
            let codex = custom
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            inventory_in(
                &home,
                &codex,
                if custom.is_some() {
                    "CODEX_HOME"
                } else {
                    "~/.codex"
                },
            )
        })
        .unwrap_or_else(|| CapabilityInventory {
            items: vec![],
            notices: vec!["用户目录不可用".into()],
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_exports_metadata_not_payload_and_distinguishes_errors() {
        let home = crate::testutil::Sandbox::new("capabilities");
        fs::create_dir_all(home.path().join(".codex/skills/sample")).unwrap();
        fs::write(
            home.path().join(".codex/skills/sample/SKILL.md"),
            "private skill content",
        )
        .unwrap();
        let config = home.path().join(".codex/config.toml");
        fs::write(&config, "[mcp_servers.fixture]\ncommand='private-command'\n[mcp_servers.fixture.env]\nTOKEN='private-value'\n").unwrap();
        let result = inventory_at(home.path());
        assert_eq!(result.items.len(), 2);
        let json = serde_json::to_string(&result).unwrap();
        for forbidden in [
            "private-command",
            "private-value",
            "private skill content",
            "TOKEN",
        ] {
            assert!(!json.contains(forbidden));
        }
        fs::write(config, "broken = [").unwrap();
        assert_eq!(inventory_at(home.path()).notices.len(), 1);
    }
}

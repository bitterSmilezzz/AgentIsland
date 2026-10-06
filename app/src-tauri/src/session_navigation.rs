//! Verified session routing. Unsupported tools open their registered application,
//! with an explicit capability flag; never guess an unverified conversation URI.
use crate::models::AgentProfile;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub agent_id: String,
    pub exact_session: bool,
    pub label: String,
    pub hint: String,
    pub bundle_id: Option<String>,
    pub url: Option<String>,
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub fn resolve(profile: &AgentProfile, source: Option<&str>) -> Option<Target> {
    let bundle = profile.bundle_ids.first()?.clone();
    let session = if profile.id == "codex" {
        source.and_then(|path| {
            let path = std::path::Path::new(path);
            if path.extension()?.to_str()? != "jsonl" {
                return None;
            }
            let name = path.file_stem()?.to_str()?;
            let id = name.get(name.len().checked_sub(36)?..)?;
            // Codex's selected rollout source, never the newest unrelated file.
            if name.starts_with("rollout-") && uuid(id) {
                Some(id.to_ascii_lowercase())
            } else {
                None
            }
        })
    } else {
        None
    };
    let exact = session.is_some();
    Some(Target {
        agent_id: profile.id.clone(),
        exact_session: exact,
        label: if exact {
            "打开会话"
        } else {
            "打开工具"
        }
        .into(),
        hint: if exact {
            "定位到这条状态对应的会话"
        } else {
            "此工具暂不支持定位会话，只打开应用"
        }
        .into(),
        bundle_id: Some(bundle),
        url: session.map(|id| format!("codex://threads/{id}")),
    })
}

pub fn launch(target: &Target) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = {
        let mut command = std::process::Command::new("/usr/bin/open");
        let bundle = target
            .bundle_id
            .as_deref()
            .ok_or("此智能体没有可打开的桌面应用")?;
        // The server-owned registry selects the app; no arbitrary URL/path input.
        command.args(["-b", bundle]);
        if let Some(url) = &target.url {
            command.arg(url);
        }
        command.status().map_err(|_| "无法打开智能体应用")?
    };
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        return Err("当前平台尚未接入工具会话跳转".into());
    }
    #[cfg(target_os = "macos")]
    if status.success() {
        Ok(())
    } else {
        Err("应用打开失败，请确认工具已安装".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(id: &str) -> AgentProfile {
        crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }
    #[test]
    fn codex_routes_the_selected_rollout_and_encodes_no_arbitrary_url() {
        let target = resolve(
            &profile("codex"),
            Some("/fixture/rollout-2026-10-03-019c6e27-e55b-73d1-87d8-4e01f1f75043.jsonl"),
        )
        .unwrap();
        assert!(target.exact_session);
        assert_eq!(
            target.url.as_deref(),
            Some("codex://threads/019c6e27-e55b-73d1-87d8-4e01f1f75043")
        );
        for source in [
            "/fixture/latest.jsonl",
            "javascript:evil",
            "/fixture/rollout-evil?x.jsonl",
        ] {
            assert!(
                !resolve(&profile("codex"), Some(source))
                    .unwrap()
                    .exact_session
            );
        }
    }
    #[test]
    fn unsupported_session_routes_are_honest_and_cli_only_agents_do_not_launch_an_arbitrary_app() {
        let target = resolve(&profile("traework"), Some("/fixture/session.jsonl")).unwrap();
        assert_eq!(target.label, "打开工具");
        assert!(!target.exact_session);
        assert!(target.url.is_none());
        assert!(resolve(&profile("aider"), None).is_none());
    }
}

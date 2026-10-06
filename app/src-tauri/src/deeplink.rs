//! `agentisland://` 深链的**解析**（对齐 Swift `URLSchemeParser` + `URLSchemeRouter`）。
//!
//! **这里只解析，不执行。** 执行要动窗口（展开 / 收起 / 切页），那是界面层的事。
//! 分开的理由与本仓其它地方一样：解析是纯函数、能表驱动测，而执行碰窗口测不了。
//!
//! 三条与 Swift 侧一致的硬约定：
//! · **投递目标必须解析到已知档案**，否则拒绝——`agentisland://notify?agent=..%2Fwhatever`
//!   这种任意字符串会直接成为事件身份，岛内出现一条不存在的 Agent 的告警。
//! · **深链不写剪贴板、不杀进程**：`export` 只跳到工作台；
//!   一个 npm postinstall 或 `.command` 脚本不该能静默销毁用户正准备粘贴的内容。
//! · **认不出的指令返回 `None`**，调用方如实说「不认识」，不猜一个最接近的动作。

use std::collections::HashMap;

/// 深链协议名。与 Swift `Info.plist` 的 `CFBundleURLSchemes` 同值。
pub const SCHEME: &str = "agentisland";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Expand,
    Collapse,
    /// 直达某智能体详情。`id` 已在解析期做过白名单校验。
    Agent(String),
    Analytics,
    Toolbox,
    /// 工作台（第三个独立窗口）。与 `Toolbox` 的区别：
    /// `Toolbox` 是「跳到工具箱那一块」，而它是**把整块面板叫到前面**。
    Workbench,
    /// 设置页（独立窗口，不经岛内路由）
    Settings(String),
    /// 跳到工作台的清理区——**不直接杀进程**
    Clean,
    /// 跳到工作台（深链不代用户写剪贴板）
    Export,
    /// 主动投递事件。`agent` 已在解析期校验过「能对上某个已知档案」。
    Notify {
        agent: String,
        kind: String,
        message: Option<String>,
        detail: Option<String>,
    },
}

impl Action {
    /// 这个动作**要不要把窗口叫出来**。
    ///
    /// `settings` 与 `notify` 除外：前者是独立窗口（岛保持当前形态），
    /// 后者只在岛里加一条事件、不该把用户正在做的事打断。
    pub fn reveals_window(&self) -> bool {
        !matches!(self, Action::Settings(_) | Action::Notify { .. })
    }
}

/// 解析 `agentisland://…`。`None` = 协议不对或指令认不出。
///
/// **形状**：`agentisland://<指令>[?k=v&…]`。主机段就是指令，
/// 与 Swift 侧 `URL.host` 读法一致——这也是为什么 `agentisland://agent?id=claude`
/// 里的 `id` 是在**查询串**而不是主机段。
pub fn parse(url: &str) -> Option<Action> {
    let rest = strip_scheme(url)?;
    let (head, query) = match rest.split_once('?') {
        Some((head, query)) => (head, query),
        None => (rest, ""),
    };
    // `agentisland://toggle` 的 host 是 `toggle`；但 `agentisland:///toggle`（无 host）
    // 也会落到路径上——两种都收，省得用户为多一个斜杠白忙一场
    //
    // 主机段**小写化**：按 URL 规范 host 是大小写不敏感的（Swift 的 `URL.host` 同样返回
    // 小写），系统派发时也常把它变形。而查询串里的值**不能**小写——
    // 那个位置放的是 agent id 与正文，改大小写会投递到不存在的档案。
    let head = head.trim_start_matches('/').to_lowercase();
    if head.is_empty() {
        return None;
    }
    let params = parse_query(query);

    Some(match head.as_str() {
        "toggle" => Action::Toggle,
        "expand" => Action::Expand,
        "collapse" => Action::Collapse,
        "analytics" => Action::Analytics,
        "toolbox" => Action::Toolbox,
        "workbench" => Action::Workbench,
        "clean" => Action::Clean,
        "export" => Action::Export,
        "agent" => {
            // 直达必须带 id；`agentisland://agent`（不带 id）没有可去的详情页
            let id = params.get("id")?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            Action::Agent(id)
        }
        "settings" => {
            let tab = params
                .get("tab")
                .cloned()
                .unwrap_or_else(|| "general".into());
            Action::Settings(tab)
        }
        "notify" => Action::Notify {
            // **这里不校验档案是否存在**：解析层拿不到注册表。
            // 校验在 `resolve_notify`（见下），它有档案表——把「拒绝」放在能拒绝的地方。
            agent: params.get("agent").cloned().unwrap_or_default(),
            kind: params
                .get("type")
                .cloned()
                .unwrap_or_else(|| "attention".into()),
            message: params.get("message").cloned(),
            detail: params.get("detail").cloned(),
        },
        // **不猜**：认不出的指令宁可返回 None，让调用方说「不认识」，
        // 也不要挑一个最接近的动作——猜错的话用户点了 A 结果发生了 B
        _ => return None,
    })
}

fn strip_scheme(url: &str) -> Option<&str> {
    let prefix = format!("{SCHEME}://");
    let lower = url.to_lowercase();
    if lower.starts_with(&prefix) {
        // 长度按**原串**切：协议名大小写不敏感，但后面的路径要原样保留
        Some(&url[prefix.len()..])
    } else {
        None
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            // 深链的值要还原百分号编码：`agentisland://agent?id=a%26b` 的 id 是 `a&b`。
            // 不还原的话，`&` 会在解码后重新出现并被上层当成分隔符。
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect()
}

/// 百分号解码。**认不出的转义保留原样**（不静默丢弃）——
/// 丢掉会让一个本该报错的 id 变成另一个看起来合法的 id。
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                // `%` 后面不是两位十六进制 ⇒ 当普通字符
                None => {}
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `notify` 的投递目标**必须解析到已知档案**。
///
/// 返回 `None` 表示拒绝。**这一层是唯一能拒绝的地方**——解析层没有档案表，
/// 而放行任意字符串的后果是岛内出现一条不存在的 Agent 的告警，
/// 而那条告警会被 `doctor` 当成真实证据汇报出去。
pub fn resolve_notify(
    action: &Action,
    known_ids: &[String],
) -> Option<(String, String, Option<String>, Option<String>)> {
    let Action::Notify {
        agent,
        kind,
        message,
        detail,
    } = action
    else {
        return None;
    };
    let needle = agent.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let resolved = known_ids
        .iter()
        .find(|id| id.to_lowercase() == needle)
        .cloned()?;
    Some((resolved, kind.clone(), message.clone(), detail.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_six_simple_actions_parse_without_anything_else() {
        assert_eq!(parse("agentisland://toggle"), Some(Action::Toggle));
        assert_eq!(parse("agentisland://expand"), Some(Action::Expand));
        assert_eq!(parse("agentisland://collapse"), Some(Action::Collapse));
        assert_eq!(parse("agentisland://analytics"), Some(Action::Analytics));
        assert_eq!(parse("agentisland://toolbox"), Some(Action::Toolbox));
        // 工作台深链：三个窗口都在时，这条必须能把面板叫到前面
        assert_eq!(parse("agentisland://workbench"), Some(Action::Workbench));
        assert_eq!(parse("agentisland://clean"), Some(Action::Clean));
        assert_eq!(parse("agentisland://export"), Some(Action::Export));
    }

    #[test]
    fn the_scheme_is_case_insensitive_but_the_rest_is_not() {
        assert_eq!(parse("AGENTISLAND://toggle"), Some(Action::Toggle));
        // 主机段大小写不敏感（系统派发时可能变形），而 id 必须原样
        assert_eq!(
            parse("agentisland://Agent?id=claude"),
            Some(Action::Agent("claude".into()))
        );
    }

    #[test]
    fn a_deep_link_to_an_agent_needs_an_id() {
        assert_eq!(
            parse("agentisland://agent?id=claude"),
            Some(Action::Agent("claude".into()))
        );
        // 不带 id / 空 id：没有可去的详情页 ⇒ 认不出，而不是去一个空 id 的页面
        assert_eq!(parse("agentisland://agent"), None);
        assert_eq!(parse("agentisland://agent?id="), None);
    }

    /// **认不出的指令返回 None，不猜。**
    /// 猜错的后果是用户点了 A、发生了 B，而界面上没有任何提示。
    #[test]
    fn an_unknown_verb_is_refused_rather_than_guessed() {
        assert_eq!(parse("agentisland://toggles"), None);
        assert_eq!(parse("agentisland://"), None);
        assert_eq!(parse("agentisland://"), None);
        assert_eq!(parse(""), None);
        // 别的协议不归我们管
        assert_eq!(parse("https://example.com/toggle"), None);
    }

    /// 投递目标**必须解析到已知档案**。
    /// 这是本模块最重要的一条：`..%2Fwhatever` 这种任意字符串放行之后
    /// 会直接成为事件身份，岛内出现一条不存在的 Agent 的告警，
    /// 而那条告警会被 `doctor` 当成真实证据汇报出去。
    #[test]
    fn notify_to_an_unknown_agent_is_refused() {
        let known = vec!["codex".to_string(), "claude".to_string()];
        let good = parse("agentisland://notify?agent=codex&type=attention").unwrap();
        assert_eq!(
            resolve_notify(&good, &known),
            Some(("codex".into(), "attention".into(), None, None))
        );

        for bogus in [
            "agentisland://notify?agent=..%2Fwhatever&type=attention",
            "agentisland://notify?agent=&type=attention",
            "agentisland://notify?type=attention",
        ] {
            let action = parse(bogus).unwrap();
            assert_eq!(resolve_notify(&action, &known), None, "{bogus} 必须被拒绝");
        }
    }

    /// 百分号解码要在**解析层**做，不能留给调用方：
    /// `a%26b` 解码后是 `a&b`，而 `&` 是查询串分隔符——
    /// 不还原的话，上层拿到的 id 会在别处被重新拆开。
    #[test]
    fn query_values_are_percent_decoded() {
        assert_eq!(
            parse("agentisland://agent?id=a%26b"),
            Some(Action::Agent("a&b".into()))
        );
        assert_eq!(percent_decode("a%20b"), "a b");
        // 认不出的转义**保留原样**，不静默丢弃——
        // 丢掉会让一个本该报错的 id 变成另一个看起来合法的 id
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn which_actions_are_allowed_to_pull_the_window_up() {
        // 设置页是独立窗口（岛保持当前形态）；notify 只在岛里加一条事件
        assert!(!Action::Settings("general".into()).reveals_window());
        assert!(!Action::Notify {
            agent: "codex".into(),
            kind: "attention".into(),
            message: None,
            detail: None
        }
        .reveals_window());
        // 其余都要把用户正在看的东西展开
        for action in [
            Action::Toggle,
            Action::Expand,
            Action::Analytics,
            Action::Toolbox,
        ] {
            assert!(action.reveals_window(), "{action:?} 应当展开窗口");
        }
    }
}

/// **深链不做两件危险的事**——这两条各有具体后果，钉在这里免得日后被"顺手加上"：
///
/// 1. **不写剪贴板**：`open agentisland://export` 无需任何确认，
///    而 `clearContents` 会让一个 npm postinstall 或 `.command` 脚本
///    静默销毁用户正准备粘贴的密码。自动化请走显式的 `agentisland report -o`。
/// 2. **不杀进程**：`clean` 只把用户带到工作台的清理区，点不点由人决定。
///    一个 URL 就能杀进程的话，任何能读网页的脚本都拿到了那个权限。
#[cfg(test)]
mod execution_safety {
    use super::*;

    /// 深链解析层**不产出**任何「执行」动作。
    ///
    /// 这不是形式检查：解析结果是执行面的唯一输入，
    /// 而执行面要动窗口与事件队列。万一将来加进一个 `Kill` / `CopyToClipboard` 变体，
    /// 这条会红。
    #[test]
    fn the_parser_cannot_express_a_destructive_action() {
        for url in [
            "agentisland://clean",
            "agentisland://export",
            "agentisland://toolbox",
        ] {
            let action = parse(url).expect("这三个都应当解析成功");
            let name = format!("{action:?}");
            for forbidden in ["Kill", "Terminate", "Copy", "Clipboard", "Export_"] {
                assert!(
                    !name.contains(forbidden),
                    "{url} 解析出了带 `{forbidden}` 的动作：{name}"
                );
            }
        }
    }

    /// `clean` 与 `export` 都要展开窗口，但**只是把人带到那里**——
    /// 动作本身不含任何终止或写盘含义（上面那条已钉），这里钉住它们仍属于「导航」。
    #[test]
    fn clean_and_export_are_navigation_not_action() {
        assert_eq!(parse("agentisland://clean"), Some(Action::Clean));
        assert_eq!(parse("agentisland://export"), Some(Action::Export));
        assert!(Action::Clean.reveals_window());
        assert!(Action::Export.reveals_window());
    }
}

/// **bundle 必须声明 `agentisland://`**，否则深链处理在这台机器上是不可达代码。
///
/// 这条不是形式检查，而是补一个**真机上查出来、离线测查不出**的洞：
/// v0.0.207 把 `deeplink.rs` 与插件接线都做完了，单测全绿、编译干净，
/// 但真机一验发现出包后的 `Info.plist` **没有 `CFBundleURLTypes`**——
/// macOS 只把 URL 派发给声明了该 scheme 的应用，于是 `open agentisland://…`
/// 叫起的是**别的应用**，本应用什么都不会发生。
///
/// 症状安静到极致：没有编译错、没有单测红、没有日志、界面一切正常。
/// 写死字符串会漂（协议名改了这里不会），所以从 `deeplink::SCHEME` 取。
#[cfg(test)]
mod bundle_registration {
    #[test]
    fn the_src_plist_declares_the_deep_link_scheme() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Info.plist");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("读不到 {}", path.display()));
        assert!(
            text.contains("CFBundleURLTypes"),
            "Info.plist 里没有 CFBundleURLTypes —— 深链永远不会被派发到这里"
        );
        assert!(
            text.contains("<key>CFBundleURLSchemes</key>"),
            "Info.plist 里没有 CFBundleURLSchemes"
        );
        // 协议名从常量取，不写死：将来改 `SCHEME` 时这条会红，而不是悄悄派发不到
        assert!(
            text.contains(&format!("<string>{}</string>", super::SCHEME)),
            "Info.plist 声明的协议名与 `deeplink::SCHEME`（{}）不一致",
            super::SCHEME
        );
    }
}

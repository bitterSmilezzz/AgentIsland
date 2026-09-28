use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(test)]
mod tests;

/// 一次会话探测的完整产出：强语义信号 + 探测失败原因。
///
/// **「读不到」与「读到但没事」必须分开**（与 Swift `AgentSessionProbe` 同一条纪律）：
/// 此前 `read_tail_lines` 的每个失败都经 `.ok()?` 塌成 `None`，而 `probe` 把 `None`
/// 映射成「无信号、无理由」——于是「会话源读不到」在界面上与「这个 Agent 真没在忙」
/// 长得一模一样，`doctor` 也会照着说「结论可信」。
#[derive(Debug, Clone, Default)]
pub struct SessionProbe {
    /// attention: Some(message) | completed: Some(()) | active: Some(action)
    pub signal: Option<Signal>,
    pub subagent_count: usize,
    /// 本轮为什么没读成（`None` = 探测本身没问题）
    pub health: Option<SessionProbeHealth>,
}

/// 这一轮「读不到」是哪一拍观测到的。
///
/// 是**最近值而不是事件**，所以必须能过期：源恢复之后若长时间没有新写入
/// （探测被跳过），旧故障会一直挂着，于是「读不到」反过来伪装成「坏了」——
/// 同样是 CONTEXT.md 反对的谎报。保质期见 [`SessionProbeHealth::is_fresh`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProbeHealth {
    pub failure: SessionProbeFailure,
    pub path: String,
    /// 由**引擎按采样时钟盖章**（不是构造点取当前时间）：
    /// 合成时间的测试才能稳定判定保质期。
    pub observed_at: i64,
}

impl SessionProbeHealth {
    /// 详情与报告里显示的那一行。先说结论，再说这条结论的边界。
    pub fn diagnostic_text(&self) -> String {
        format!(
            "会话源不可读：{}（{}）——此后的「待机」只代表没有读到信号，不代表智能体真的空闲",
            self.failure.label(),
            self.path
        )
    }

    /// 这条故障还新鲜吗。超过 `window_ms` 就当它已经过去——
    /// 挂着一条几小时前的「读不到」，而源早已修好，那是另一种谎报。
    pub fn is_fresh(&self, now_ms: i64, window_ms: i64) -> bool {
        (now_ms - self.observed_at).max(0) <= window_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProbeFailure {
    /// 会话文件在，但读不出来（权限拒绝 / 是目录 / 读取抛错）
    UnreadableFile,
    /// 读出来了，但**没有一行**是解析器认识的形状（改版、顶层类型漂移）
    ///
    /// 与「文件里确实没有待确认事项」是**两件事**，不得混为一谈：
    /// 前者是「我们读不懂」，后者是「读懂了、确实没事」。
    UndecodableFile,
}

impl SessionProbeFailure {
    pub fn label(&self) -> &'static str {
        match self {
            Self::UnreadableFile => "会话文件无法读取",
            Self::UndecodableFile => "会话文件格式与解析器不匹配",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Signal {
    /// 等待用户确认（fingerprint, message）
    Attention(String, String),
    /// 本轮明确结束（fingerprint）
    Completed(String),
    /// 在途（fingerprint, action）
    Active(String, Option<String>),
}



/// 会话尾部强语义解析（genericTail 方言族）。
/// 只读尾部有界字节；解析失败返回无信号，绝不谎报待机。
/// **已经有解析器的方言**。
///
/// 摆成常量而不是写在注释里，是因为注释会与现实悄悄脱节：三种方言的解析器
/// 还没迁（`antigravityBrain` / `dshProjection` / `qoderTranscript`，
/// 各自都要带会话定位与缓存，见对照表 §3.2），而**档案里已经如实声明了它们**。
/// 声明与「有解析器」分开记，就是为了让「声明了但还没实现」有一处可查，
/// 而不是让人以为那条路径已经通了。
pub const DIALECTS_WITH_PARSER: [crate::models::SessionDialect; 2] = [
    crate::models::SessionDialect::GenericTail,
    crate::models::SessionDialect::ClineTasks,
];

/// 方言分派。
///
/// ⚠️ `GenericTail` 内部**仍按 id 分流**——这是尚未消掉的一处偏差：
/// Swift 侧那一个 `detect(lines:)` **按内容**同时吃 claude 与 codex 两种形状，
/// 而 Rust 侧是三个独立解析器；合成一个内容驱动的检测器是独立一块。
/// 这一版先让**档案里的声明**成为分派入口，那才是 ADR 0010 要的形状。
pub fn probe_dialect(
    profile_id: &str,
    dialect: crate::models::SessionDialect,
    path: &str,
) -> SessionProbe {
    if !DIALECTS_WITH_PARSER.contains(&dialect) {
        // 已声明、尚无解析器：如实无信号，**不拿猜的解析器顶上去**
        return SessionProbe::default();
    }
    probe_by_id(profile_id, path)
}

fn probe_by_id(profile_id: &str, path: &str) -> SessionProbe {
    // 「读不到」**必须**留下理由：此前这里把每种失败都塌成「无信号」，
    // 于是界面上「会话源读不到」与「这个 Agent 真没在忙」完全一样
    let lines = match read_tail_lines(path) {
        Ok(l) => l,
        Err(failure) => {
            return SessionProbe {
                signal: None,
                subagent_count: 0,
                health: Some(SessionProbeHealth {
                    failure,
                    path: path.to_string(),
                    observed_at: 0, // 由引擎按采样时钟盖章
                }),
            }
        }
    };
    if lines.is_empty() {
        return SessionProbe::default();
    }
    match profile_id {
        "claude" => probe_claude(&lines, path),
        "codex" => probe_codex(&lines, path),
        // Cline / Roo Code 的 `ui_messages.json` 是**一个跨行 JSON 数组**，
        // 逐行解析必然失败——所以「整窗无一行解析得出来」这条只对
        // **逐行方言**成立，不能在这里统一判
        "cline" | "roo-code" | "roo" => probe_cline(&lines, path),
        "zcode" => probe_zcode(&lines, path),
        _ => SessionProbe { signal: None, subagent_count: 0, health: None },
    }
}

fn read_tail_lines(path: &str) -> Result<Vec<String>, SessionProbeFailure> {
    // model-io 一类的会话文件单行可达数 MB（请求体全量内嵌），
    // 固定小窗口里可能没有完整行。改为：读末尾大缓冲 → 以最后一个 \n 为界，
    // 只保留缓冲内的完整行（首段残行丢弃）。
    const MAX_TAIL: u64 = 8 * 1024 * 1024;
    // **每一类失败给出不同的理由**（与 Swift `SessionProbeFailure` 同形）：
    // 打不开、读不了、太大，是三件事——「都是读不到」会把修法也一起丢掉
    let mut f = File::open(path).map_err(|_| SessionProbeFailure::UnreadableFile)?;
    let len = f
        .metadata()
        .map_err(|_| SessionProbeFailure::UnreadableFile)?
        .len();
    // 超出上限是**有意的降级**而非故障：单行可达数 MB，固定小窗口里可能没有完整行
    let start = len.saturating_sub(MAX_TAIL);
    f.seek(SeekFrom::Start(start))
        .map_err(|_| SessionProbeFailure::UnreadableFile)?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)
        .map_err(|_| SessionProbeFailure::UnreadableFile)?;

    let complete: &[&str] = if start == 0 {
        &buf.lines().collect::<Vec<_>>()
    } else {
        // 丢弃首个残行
        match buf.find('\n') {
            Some(i) => &buf[i + 1..].lines().collect::<Vec<_>>(),
            None => &[], // 整个缓冲都在一行内（行比缓冲还大）：放弃
        }
    };
    let mut lines: Vec<String> = complete
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|s| s.to_string())
        .collect();
    while lines.len() > 600 {
        lines.remove(0);
    }
    Ok(lines)
}

fn fingerprint(path: &str, key: &str) -> String {
    format!("{:x}:{:x}", md5_lite(path), md5_lite(key))
}

/// 简易 FNV-1a 哈希（避免引全量 md5 依赖）
fn md5_lite(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// MARK: Claude Code JSONL

fn probe_claude(lines: &[String], path: &str) -> SessionProbe {
    let mut sidechains = 0usize;

    // 用户中断（Ctrl-C / 点停止）会留下一条永不交付 tool_result 的执行类调用。
    // 不撤销的话：本行之后的每一拍都拿到 `.active`，滞回与完成分支永远走不到，
    // 该 Agent 被钉死在 working——2s 快采样与高频全树扫描一并被锁住（耗电与 CPU 双输）。
    // Swift 侧同名机制 `isInterruptionNotice`，短语表逐条照搬。
    //
    // 语义与 Swift 一致：**只撤销中断之前**的调用。中断之后重新发起的命令仍然是在途。
    let last_interruption = lines.iter().rposition(|l| is_interruption_notice(l));

    // 读到了一堆行、却**没有一行**解析得出 JSON ⇒ 「我们读不懂这份文件」，
    // 而不是「读懂了、确实没事」。这两件事在 `doctor` 里必须分开说。
    if !lines.iter().any(|l| serde_json::from_str::<Value>(l).is_ok()) {
        return undecodable(path);
    }

    // 自尾向前找最新一条 assistant 条目
    for (index, line) in lines.iter().enumerate().rev() {
        // 走到中断点即止：它与更早的一切都已被用户撤销
        if last_interruption.is_some_and(|cut| index <= cut) {
            break;
        }
        let Ok(doc) = serde_json::from_str::<Value>(line) else { continue };
        let obj = match doc.as_object() { Some(o) => o, None => continue };
        let type_ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if obj.get("isSidechain") == Some(&Value::Bool(true)) {
            sidechains += 1;
        }
        if type_ != "assistant" {
            continue;
        }
        let Some(content) = obj
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        else {
            continue;
        };

        let mut tool_use: Option<&Value> = None;
        let mut has_text = false;
        let mut text = String::new();
        for item in content {
            let it = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if it == "tool_use" {
                tool_use = Some(item);
            } else if it == "text" {
                has_text = true;
                if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                    text.push_str(t);
                }
            }
        }

        if let Some(tu) = tool_use {
            let tool_id = tu.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let tool_name = tu.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_input = tu.get("input").cloned().unwrap_or(Value::Null);
            let closed = tail_has_tool_result(lines, tool_id);

            if !closed {
                if tool_name == "AskUserQuestion" || tool_name == "ExitPlanMode" {
                    let question = claude_question_text(&tool_input)
                        .unwrap_or_else(|| "等待你确认".into());
                    return SessionProbe {
                        signal: Some(Signal::Attention(fingerprint(path, tool_id), question)),
                        subagent_count: sidechains,
                        health: None,
                    };
                }
                let action = describe_claude_tool(tool_name, &tool_input);
                return SessionProbe {
                    signal: Some(Signal::Active(fingerprint(path, tool_id), action)),
                    subagent_count: sidechains,
                    health: None,
                };
            }
            // 工具已收口：继续向前找更早的未收口调用
            continue;
        }

        if has_text {
            let fp = fingerprint(path, &text.chars().take(64).collect::<String>());
            return SessionProbe {
                signal: Some(Signal::Completed(fp)),
                subagent_count: sidechains,
                health: None,
            };
        }
    }
    SessionProbe { signal: None, subagent_count: sidechains, health: None }
}

/// 中断短语表。与 Swift `AgentSessionInspector.interruptionPhrases` 逐条同值——
/// 这里少一条，就有一种打断方式会让 Agent 继续被钉在工作态。
const INTERRUPTION_PHRASES: [&str; 10] = [
    "interrupted by user",
    "request interrupted",
    "user interrupted",
    "turn_aborted",
    "turn aborted",
    "aborted by user",
    "cancelled by user",
    "canceled by user",
    "user cancelled",
    "user canceled",
];

/// 这一行是不是「用户按了停止」。
///
/// 三道与 Swift 相同的闸：① 短行（`line.count <= 400`）——长行里出现
/// "aborted by user" 多半是在转述别人的话，不是本轮被打断；
/// ② 必须是 user 角色/类型那一行；③ 命中短语表。
fn is_interruption_notice(line: &str) -> bool {
    if line.chars().count() > 400 {
        return false;
    }
    let lowered = line.to_lowercase();
    if !INTERRUPTION_PHRASES.iter().any(|p| lowered.contains(p)) {
        return false;
    }
    let Ok(doc) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let type_ = doc.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if type_ == "user" {
        return true;
    }
    doc.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|v| v.as_str())
        == Some("user")
}

fn tail_has_tool_result(lines: &[String], tool_use_id: &str) -> bool {
    // 尾部出现过该 tool_use 的 tool_result 即视为已收口
    // （tool_use 的存在已由上游确认；这里只回答「结果回来没有」）
    for line in lines.iter().rev() {
        if !line.contains(tool_use_id) {
            continue;
        }
        let Ok(doc) = serde_json::from_str::<Value>(line) else { continue };
        let Some(content) = doc
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        else {
            continue;
        };
        for item in content {
            if item.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                && item.get("tool_use_id").and_then(|v| v.as_str()) == Some(tool_use_id)
            {
                return true;
            }
        }
    }
    false
}

fn one_line(s: &str, max: usize) -> String {
    let folded: String = s
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let t = folded.trim();
    if t.chars().count() <= max {
        t.to_string()
    } else {
        let cut: String = t.chars().take(max).collect();
        format!("{}…", cut)
    }
}

fn describe_claude_tool(name: &str, input: &Value) -> Option<String> {
    let get = |key: &str| input.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
    Some(match name {
        "Bash" | "BashOutput" | "KillShell" => match get("command") {
            Some(c) => format!("运行: {}", one_line(&c, 100)),
            None => format!("运行: {name}"),
        },
        "Edit" | "Write" | "NotebookEdit" | "MultiEdit" => match get("file_path") {
            Some(f) => format!("正在修改: {f}"),
            None => format!("正在修改: {name}"),
        },
        "Read" => match get("file_path") {
            Some(f) => format!("正在读取: {f}"),
            None => "正在读取".into(),
        },
        "Grep" | "Glob" => match get("pattern").or_else(|| get("query")) {
            Some(p) => format!("正在搜索: {p}"),
            None => "正在搜索".into(),
        },
        "Task" | "Agent" => match get("description") {
            Some(d) => format!("子任务: {d}"),
            None => "运行子任务".into(),
        },
        "WebFetch" | "WebSearch" => match get("query").or_else(|| get("url")) {
            Some(q) => format!("正在检索: {}", one_line(&q, 100)),
            None => "正在检索".into(),
        },
        "TodoWrite" => "正在更新任务清单".into(),
        _ => format!("正在执行: {name}"),
    })
}

fn claude_question_text(input: &Value) -> Option<String> {
    if let Some(qs) = input.get("questions").and_then(|v| v.as_array()) {
        for q in qs {
            if let Some(t) = q.get("question").and_then(|v| v.as_str()) {
                return Some(t.to_string());
            }
        }
    }
    input.get("message").and_then(|v| v.as_str()).map(|s| s.to_string())
}

// MARK: Codex rollout JSONL

fn probe_codex(lines: &[String], path: &str) -> SessionProbe {
    // Read the bounded tail in order: tool outputs are only meaningful when they
    // resolve a call with the same ID. Accounting and ordinary messages are neutral.
    if !lines.iter().any(|l| serde_json::from_str::<Value>(l).is_ok()) {
        return undecodable(path);
    }
    let mut open: HashMap<String, (usize, String, bool, String)> = HashMap::new();
    let mut completion: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        let Ok(doc) = serde_json::from_str::<Value>(line) else { continue };
        let type_ = doc.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if type_ != "response_item" && type_ != "event_msg" {
            continue;
        }
        let Some(payload) = doc.get("payload") else { continue };
        let pt = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match pt {
            "function_call" | "custom_tool_call" => {
                let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("command");
                let args = payload.get("arguments").or_else(|| payload.get("input"))
                    .and_then(|v| v.as_str()).unwrap_or("");
                let id = payload.get("call_id").and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty()).map(str::to_owned)
                    .unwrap_or_else(|| format!("record-{index}"));
                let question = name == "request_user_input";
                let description = if question {
                    serde_json::from_str::<Value>(args).ok()
                        .and_then(|input| claude_question_text(&input))
                        .unwrap_or_else(|| "等待你确认".into())
                } else {
                    format!("运行: {}", one_line(&format!("{name} {args}"), 100))
                };
                open.insert(id, (index, name.to_string(), question, description));
                completion = None;
            }
            "function_call_output" | "custom_tool_call_output" => {
                if let Some(id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    open.remove(id);
                }
            }
            "task_complete" => {
                let turn = payload.get("turn_id").and_then(|v| v.as_str())
                    .unwrap_or(line);
                completion = Some(fingerprint(path, turn));
            }
            "reasoning" => completion = None,
            "message" if payload.get("role").and_then(|v| v.as_str()) == Some("user") => {
                completion = None;
                open.clear();
            }
            _ => {}
        }
    }
    if let Some((id, (_, _, question, description))) = open.iter()
        .max_by_key(|(_, (index, _, _, _))| index) {
        let id = fingerprint(path, id);
        let signal = if *question {
            Signal::Attention(id, description.clone())
        } else {
            Signal::Active(id, Some(description.clone()))
        };
        return SessionProbe { signal: Some(signal), subagent_count: 0, health: None };
    }
    SessionProbe { signal: completion.map(Signal::Completed), subagent_count: 0, health: None }
}

// MARK: Cline / Roo ui_messages.json（JSON 数组投影）

/// 「读到了一堆行，却一行都不是我们认识的形状」。抽出成一个函数是因为
/// 逐行方言有三条（claude / codex / zcode），而它们要报**同一种**理由。
fn undecodable(path: &str) -> SessionProbe {
    SessionProbe {
        signal: None,
        subagent_count: 0,
        health: Some(SessionProbeHealth {
            failure: SessionProbeFailure::UndecodableFile,
            path: path.to_string(),
            observed_at: 0,
        }),
    }
}

fn probe_cline(lines: &[String], path: &str) -> SessionProbe {
    let joined: String = lines.concat();
    let Ok(doc) = serde_json::from_str::<Value>(&joined) else {
        return SessionProbe { signal: None, subagent_count: 0, health: None };
    };
    let Some(arr) = doc.as_array() else {
        return SessionProbe { signal: None, subagent_count: 0, health: None };
    };
    let Some(el) = arr.last() else {
        return SessionProbe { signal: None, subagent_count: 0, health: None };
    };
    let ask = el.get("ask").and_then(|v| v.as_str()).unwrap_or("");
    let say = el.get("say").and_then(|v| v.as_str()).unwrap_or("");
    let text = el
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if matches!(ask, "command" | "tool" | "followup" | "plan_mode_respond") {
        let msg = if text.is_empty() { "等待你确认".to_string() } else { one_line(&text, 80) };
        return SessionProbe {
            signal: Some(Signal::Attention(
                fingerprint(path, &format!("{ask}{}", msg.len())),
                msg,
            )),
            subagent_count: 0,
            health: None,
        };
    }
    if matches!(say, "command" | "command_output" | "tool") {
        let action = if text.is_empty() {
            None
        } else {
            Some(format!("运行: {}", one_line(&text, 72)))
        };
        return SessionProbe {
            signal: Some(Signal::Active(
                fingerprint(path, &format!("{say}{}", text.len())),
                action,
            )),
            subagent_count: 0,
            health: None,
        };
    }
    if say == "completion_result" {
        return SessionProbe {
            signal: Some(Signal::Completed(fingerprint(path, &text.chars().take(64).collect::<String>()))),
            subagent_count: 0,
            health: None,
        };
    }
    SessionProbe { signal: None, subagent_count: 0, health: None }
}

// MARK: ZCode rollout JSONL（~/.zcode/cli/rollout/model-io-sess_*.jsonl）
// 每行 = 一次完成的模型请求：completedAt / durationMs / model.modelId / response.toolCalls。
// 强语义（待确认/完成）不在此文件里——安全降级为双信号近似（Mac 端同规则）；
// 但最近一次请求的工具调用可以提炼出实时动作（运行/正在修改…）。

fn probe_zcode(lines: &[String], path: &str) -> SessionProbe {
    for line in lines.iter().rev() {
        let Ok(doc) = serde_json::from_str::<Value>(line) else { continue };
        let Some(resp) = doc.get("response") else { continue };
        let Some(tool_calls) = resp.get("toolCalls").and_then(|v| v.as_array()) else { continue };

        // 取最近一次请求里最后一个有意义的工具调用作为实时动作
        for tc in tool_calls.iter().rev() {
            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let input = tc.get("input").cloned().unwrap_or(Value::Null);
            let action = describe_claude_tool(name, &input);
            let ts_ms = doc
                .get("completedAt")
                .and_then(|v| v.as_str())
                .and_then(super::tokens::parse_iso_ms_pub)
                .unwrap_or(0);
            let fresh = super::tokens::now_ms() - ts_ms < 180_000; // 3 分钟内的请求才算在活动
            if fresh {
                return SessionProbe {
                    signal: Some(Signal::Active(
                        fingerprint(path, &doc.get("requestId").and_then(|v| v.as_str()).unwrap_or("")),
                        action,
                    )),
                    subagent_count: 0,
                    health: None,
                };
            }
            // 最近请求已陈旧：回到双信号近似（进程 + 文件写入/CPU）
            return SessionProbe { signal: None, subagent_count: 0, health: None };
        }
        // 该行无工具调用（纯推理/回答）：看时间戳决定是否算活动
        let ts_ms = doc
            .get("completedAt")
            .and_then(|v| v.as_str())
            .and_then(super::tokens::parse_iso_ms_pub)
            .unwrap_or(0);
        if super::tokens::now_ms() - ts_ms < 180_000 {
            return SessionProbe {
                signal: Some(Signal::Active(fingerprint(path, "zcode-reasoning"), Some("正在推理".into()))),
                subagent_count: 0,
                health: None,
            };
        }
        return SessionProbe { signal: None, subagent_count: 0, health: None };
    }
    SessionProbe { signal: None, subagent_count: 0, health: None }
}

// MARK: 供 Token 监控复用的按行读取

pub fn for_each_line<F: FnMut(&str)>(path: &Path, start_offset: u64, mut f: F) -> Option<u64> {
    use std::io::BufRead;
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len < start_offset {
        file.seek(SeekFrom::Start(0)).ok()?;
    } else {
        file.seek(SeekFrom::Start(start_offset)).ok()?;
    }
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        match line {
            Ok(l) => f(&l),
            Err(_) => break,
        }
    }
    Some(len)
}

/// 按行读取，**只承认到最后一条完整行**，并回吐 `(真正承认到的字节数, 文件是否以换行结尾)`。
///
/// 与 `for_each_line` 的差别是刻意的，不是重复实现：
/// 会话尾读每轮重读尾部窗口，末尾半行解析失败也无所谓；而 token 明细是**续读游标**——
/// 一旦把半个 JSON 行承认下来并前进游标，那条记录下一次就再也读不回来了
/// （下一次从更后面开始）。所以这里宁可停在上一个换行处，等它写完再读。
pub fn for_each_complete_line<F: FnMut(&str)>(
    path: &Path,
    start_offset: u64,
    mut f: F,
) -> Option<(u64, bool)> {
    use std::io::BufRead;
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = if len < start_offset { 0 } else { start_offset };
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut buffer = Vec::new();
    let mut consumed = start;
    let mut ended_with_newline = false;
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        if buffer.last() != Some(&b'\n') {
            // 末尾半行：不承认（consumed 停在上一处换行之后），下次补全了再读
            ended_with_newline = false;
            break;
        }
        consumed += read as u64;
        ended_with_newline = true;
        let line = &buffer[..buffer.len() - 1];
        if !line.is_empty() {
            f(&String::from_utf8_lossy(line));
        }
    }
    Some((consumed, ended_with_newline))
}

/// 探测失败**必须留下理由**——这是本轮改动的全部要点。
#[cfg(test)]
mod health_chain {
    use super::*;

    fn write(lines: &[&str]) -> crate::testutil::Sandbox {
        let sandbox = crate::testutil::Sandbox::new("healthchain");
        std::fs::write(sandbox.path().join("s.jsonl"), lines.join("\n")).unwrap();
        sandbox
    }

    /// **读不到**与**读到但没事**是两件事，必须能分开。
    ///
    /// 此前 `read_tail_lines` 的每个失败都经 `.ok()?` 塌成 `None`，
    /// 而 `probe` 把 `None` 映射成「无信号、无理由」——于是界面上
    /// 「会话源读不到」与「这个 Agent 真没在忙」完全一样。
    #[test]
    fn an_unreadable_file_carries_a_reason_and_an_empty_one_does_not() {
        // ① 文件在，但内容解析不出任何东西 ⇒ 「读到了、但没信号」：**没有**理由
        let sandbox = write(&[r#"{"type":"user","content":"hi"}"#]);
        let ok = probe_dialect("claude", crate::models::SessionDialect::GenericTail, sandbox.path().join("s.jsonl").to_str().unwrap());
        assert!(ok.signal.is_none());
        assert!(
            ok.health.is_none(),
            "读到了只是没信号，不该报「读不到」"
        );

        // ② 文件读不出来（这里用「路径是目录」构造）⇒ **有**理由
        let dir_sandbox = crate::testutil::Sandbox::new("healthdir");
        std::fs::create_dir_all(dir_sandbox.path().join("s.jsonl")).unwrap();
        let broken = probe_dialect("claude", crate::models::SessionDialect::GenericTail, dir_sandbox.path().join("s.jsonl").to_str().unwrap());
        assert!(broken.signal.is_none());
        let health = broken.health.expect("读不到就必须留下理由");
        assert_eq!(health.failure, SessionProbeFailure::UnreadableFile);
        assert!(health.diagnostic_text().contains("会话文件无法读取"));
        assert!(health.diagnostic_text().contains("不代表智能体真的空闲"));
    }

    /// 故障是**最近值而不是事件**，所以必须会过期。
    ///
    /// 源恢复之后若长时间没有新写入（探测被跳过），旧故障会一直挂着，
    /// 于是「读不到」反过来伪装成「坏了」——同样是 CONTEXT.md 反对的谎报。
    /// 读到一堆行却**一行 JSON 都解析不出** ⇒ 「我们读不懂」，
    /// 不是「读懂了、确实没事」。`doctor` 里这两句必须分开。
    #[test]
    fn a_window_with_no_parseable_line_is_reported_as_undecodable() {
        let sandbox = write(&["这不是 JSON", "这也不是"]);
        let probe = probe_dialect("claude", crate::models::SessionDialect::GenericTail, sandbox.path().join("s.jsonl").to_str().unwrap());
        let health = probe.health.expect("读不懂就必须留下理由");
        assert_eq!(health.failure, SessionProbeFailure::UndecodableFile);
        assert!(health.diagnostic_text().contains("格式与解析器不匹配"));
    }

    /// 「整窗无一行解析得出来」**只对逐行方言成立**。
    ///
    /// Cline / Roo Code 的 `ui_messages.json` 是**一个跨行 JSON 数组**，
    /// 逐行解析必然失败——统一判的话会把这一族全打成「读不懂」，
    /// 而它们其实读得好好的。
    #[test]
    fn a_json_array_split_across_lines_is_not_mistaken_for_undecodable() {
        let sandbox = crate::testutil::Sandbox::new("cline-array");
        let path = sandbox.path().join("ui_messages.json");
        // 跨行的 JSON 数组
        std::fs::write(
            &path,
            "[\n  {\n    \"type\": \"ask\", \"ask\": \"command\", \"text\": \"ls\"\n  }\n]",
        )
        .unwrap();
        let probe = probe_dialect("cline", crate::models::SessionDialect::GenericTail, path.to_str().unwrap());
        assert!(
            probe.health.is_none(),
            "跨行数组不是「读不懂」：{:?}",
            probe.health
        );
    }

    #[test]
    fn a_stale_health_report_goes_stale() {
        let now = 1_000_000i64;
        let health = SessionProbeHealth {
            failure: SessionProbeFailure::UnreadableFile,
            path: "/tmp/x.jsonl".into(),
            observed_at: now,
        };
        assert!(health.is_fresh(now, 10 * 60 * 1000), "刚观测到的算新鲜");
        assert!(health.is_fresh(now + 10 * 60 * 1000, 10 * 60 * 1000), "边界取等号");
        assert!(
            !health.is_fresh(now + 10 * 60 * 1000 + 1, 10 * 60 * 1000),
            "超过保质期就不再算数"
        );
        // 时钟回拨不得让它变成「负龄」
        assert!(health.is_fresh(now - 5000, 10 * 60 * 1000));
    }

    /// 每种失败各有各的**修法**，「都是读不到」会把修法一起丢掉。
    ///
    /// 枚举里**只列真有的产生点**。Swift 侧还有三种（会话库打不开 / 结构变了 /
    /// 查询被中断），它们要等 SQLite 会话路径迁过来才用得上；文件超限那种
    /// 在 Rust 侧是**有意的降级而非故障**（单行可达数 MB，尾读本就按 8MB 窗口走），
    /// 不该报成「读不到」。
    /// 先摆在那儿只会让下一个人以为这些已经覆盖了。
    #[test]
    fn every_failure_carries_its_own_reason() {
        let cases = [
            (SessionProbeFailure::UnreadableFile, "会话文件无法读取"),
            (SessionProbeFailure::UndecodableFile, "格式与解析器不匹配"),
        ];
        for (failure, needle) in cases {
            let health = SessionProbeHealth {
                failure,
                path: "/tmp/x".into(),
                observed_at: 0,
            };
            let text = health.diagnostic_text();
            assert!(text.contains(needle), "{failure:?} 的文案缺「{needle}」：{text}");
        }
    }
}

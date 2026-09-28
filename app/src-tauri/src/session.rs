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
pub const DIALECTS_WITH_PARSER: [crate::models::SessionDialect; 5] = [
    crate::models::SessionDialect::GenericTail,
    crate::models::SessionDialect::ClineTasks,
    crate::models::SessionDialect::QoderTranscript,
    crate::models::SessionDialect::DshProjection,
    crate::models::SessionDialect::AntigravityBrain,
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
    match dialect {
        crate::models::SessionDialect::QoderTranscript => {
            return probe_qoder_dialect(profile_id, path);
        }
        crate::models::SessionDialect::DshProjection => {
            return probe_dsh_dialect(path);
        }
        crate::models::SessionDialect::AntigravityBrain => {
            return probe_antigravity_dialect(path);
        }
        _ => {}
    }
    if !DIALECTS_WITH_PARSER.contains(&dialect) {
        // 已声明、尚无解析器：如实无信号，**不拿猜的解析器顶上去**
        return SessionProbe::default();
    }
    probe_by_id(profile_id, path)
}

/// Qoder 的会话定位：取 `session_dirs` 下**最近修改**的那个 `.jsonl`。
///
/// 定位是这一族独有的——Qoder 的目录结构是
/// `projects/<项目 slug>/<会话 uuid>.jsonl`，两层，所以不能沿用别的方言的「根下最深一层」。
/// `session_key` 取**文件名**（去扩展名）：它是会话的稳定身份，
/// 而完成态指纹在认不出人类轮次时要退到它（见 [`probe_qoder`] 的注）。
fn probe_qoder_dialect(_profile_id: &str, path: &str) -> SessionProbe {
    // 传入的 path 就是本拍由 filemon 定位到的候选文件；文件名即会话身份
    let file = std::path::Path::new(path);
    let session_key = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file_age = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    match read_tail_lines(path) {
        Ok(lines) => probe_qoder(&lines, path, file_age, &session_key),
        Err(failure) => SessionProbe {
            signal: None,
            subagent_count: 0,
            health: Some(SessionProbeHealth {
                failure,
                path: path.to_string(),
                observed_at: 0,
            }),
        },
    }
}

/// DSH 那一族要**定位**投影文件，而不是尾读——投影目录实测有 500+ 会话文件，
/// 一趟 stat 约 15ms，每拍在主线程上重走不现实。
///
/// 这里走 `filemon` 已经定位好的候选（`session_dirs` 下按 mtime 排过序），
/// 取**最新的那个 `.json`**；定位不到就如实无信号。
fn probe_dsh_dialect(path: &str) -> SessionProbe {
    let file = std::path::Path::new(path);
    if file.extension().and_then(|e| e.to_str()) != Some("json") {
        return SessionProbe::default();
    }
    let age = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    probe_dsh(path, age)
}

fn probe_antigravity_dialect(path: &str) -> SessionProbe {
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if age > 24.0 * 3600.0 {
        return SessionProbe::default();
    }
    match read_tail_lines(path) {
        Ok(lines) => {
            if lines.is_empty() {
                return SessionProbe::default();
            }
            probe_antigravity(&lines, path, age)
        }
        Err(failure) => SessionProbe {
            signal: None,
            subagent_count: 0,
            health: Some(SessionProbeHealth {
                failure,
                path: path.to_string(),
                observed_at: 0,
            }),
        },
    }
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

// MARK: - Qoder 方言（`~/.qoder/projects/<slug>/<uuid>.jsonl`，Anthropic 兼容逐行）

/// 从 tool_use 的 `input` 里取一条**能给人看**的线索（命令 / 文件名 / 任务描述）。
///
/// 取的键**按顺序**而不是「随便哪个有值」：一份 input 里可能同时有
/// `path`（目录）与 `file_path`（文件），而用户想知道的是「在改哪个文件」。
/// 顺序与 Swift 侧逐条一致。
fn qoder_tool_hint(input: &Value) -> String {
    for key in [
        "command", "file_path", "path", "pattern", "prompt", "description", "toolName",
    ] {
        if let Some(text) = input.get(key).and_then(|v| v.as_str()) {
            if !text.is_empty() {
                return one_line(text, 40);
            }
        }
    }
    String::new()
}

fn qoder_action_text(name: &str, hint: &str) -> String {
    let name = name.to_lowercase();
    match (name.as_str(), hint.is_empty()) {
        ("bash", true) => "执行终端命令".into(),
        ("bash", false) => format!("运行: {hint}"),
        ("edit" | "write" | "multiedit", true) => "修改文件".into(),
        ("edit" | "write" | "multiedit", false) => format!("修改: {hint}"),
        ("read" | "grep" | "glob", true) => "读取代码".into(),
        ("read" | "grep" | "glob", false) => format!("读取: {hint}"),
        ("agent", true) => "派生子任务处理中".into(),
        ("agent", false) => format!("子任务: {hint}"),
        ("mcp_call", true) => "调用外部工具".into(),
        ("mcp_call", false) => format!("调用工具: {hint}"),
        (_, true) => format!("正在执行 {name}"),
        (_, false) => format!("{name}: {hint}"),
    }
}

/// Qoder 的「等确认」类工具（**球在用户这边**的那些）。
///
/// 与 Claude Code 侧那两个（`AskUserQuestion` / `ExitPlanMode`）同义，
/// 但 Qoder 用的是**自己的**一组名字，缺一条就漏报一种确认请求。
const QODER_REQUEST_TOOLS: [&str; 4] = [
    "askuserquestion",
    "ask_question",
    "exitplanmode",
    "exit_plan_mode",
];

/// Qoder 解析器。逐条对应 Swift `detectQoder`。
///
/// **完成态的指纹是「最近一次人类指令的身份」**，不是那条 assistant 消息的 id：
/// Qoder 每次模型调用都换 id，而一个长任务里模型会 `end_turn` 很多次
/// （等后台构建、等并发会话回话、被通知唤醒后续跑）——按消息 id 记的话，
/// 「同一件活」每续跑一轮就再弹一次「任务完成」。用户 2026-09-25 报的就是这个。
///
/// 指纹认不出来时退到**会话文件**的身份（`session_key`）而不是当前轮次 id：
/// 按轮次退会让每一次续跑都换个新指纹，等于把这条修复要治的病原地复发。
fn probe_qoder(lines: &[String], path: &str, file_age_secs: f64, session_key: &str) -> SessionProbe {
    struct Row {
        id: String,
        role: String,
        stop_reason: String,
        uses: Vec<(String, String, String)>, // (name, id, hint)
        results: Vec<String>,
        prompt_id: String,
        is_human_input: bool,
    }

    let mut rows: Vec<Row> = Vec::with_capacity(lines.len());
    for raw in lines {
        let Ok(doc) = serde_json::from_str::<Value>(raw) else { continue };
        let Some(message) = doc.get("message") else { continue };
        let Some(role) = message.get("role").and_then(|v| v.as_str()) else { continue };

        let mut uses = Vec::new();
        let mut results = Vec::new();
        if let Some(blocks) = message.get("content").and_then(|v| v.as_array()) {
            for block in blocks {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("tool_use") => {
                        let Some(name) = block.get("name").and_then(|v| v.as_str()) else { continue };
                        let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let hint = qoder_tool_hint(
                            block.get("input").unwrap_or(&Value::Null),
                        );
                        uses.push((name.to_string(), id, hint));
                    }
                    Some("tool_result") => {
                        if let Some(src) = block.get("tool_use_id").and_then(|v| v.as_str()) {
                            results.push(src.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        rows.push(Row {
            id: message.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            role: role.to_string(),
            stop_reason: message
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            uses,
            results,
            prompt_id: doc.get("promptId").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            is_human_input: doc.get("humanInput").is_some(),
        });
    }
    if rows.is_empty() {
        return SessionProbe::default();
    }

    // 哪些 tool_use 已经拿到结果
    let mut answered: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for row in &rows {
        for id in &row.results {
            if !id.is_empty() {
                answered.insert(id);
            }
        }
    }

    let Some(last_assistant) = rows.iter().rev().find(|r| r.role == "assistant") else {
        return SessionProbe::default();
    };

    let turn_id = rows
        .iter()
        .rev()
        .find(|r| !r.prompt_id.is_empty())
        .map(|r| r.prompt_id.clone())
        .unwrap_or_default();
    // 「谁起的头」：一轮的**首行**才带 humanInput（真人敲的）或 isMeta（系统注入）
    let human_turn = rows
        .iter()
        .rev()
        .find(|r| r.is_human_input && !r.prompt_id.is_empty())
        .map(|r| r.prompt_id.clone())
        .unwrap_or_default();
    let completion_identity = if !human_turn.is_empty() {
        human_turn
    } else if !session_key.is_empty() {
        session_key.to_string()
    } else if !turn_id.is_empty() {
        turn_id.clone()
    } else {
        last_assistant.id.clone()
    };
    let completion_fingerprint = format!("qoder-{completion_identity}");

    // 真人刚敲完、模型一个字都还没回：最后一条是带 humanInput 的 user 行。
    // 这时最后那条 assistant 还是**上一轮**的 end_turn——按它判就是
    // 「你一发出指令，岛就说上一件事完成了」，而它等的正是这条新指令。
    if let Some(last) = rows.last() {
        if last.role == "user" && last.is_human_input {
            let id = if turn_id.is_empty() { last.id.clone() } else { turn_id };
            return SessionProbe {
                signal: Some(Signal::Active(
                    fingerprint(path, &format!("qoder-turn-{id}")),
                    Some("正在处理你的新指令".into()),
                )),
                subagent_count: 0,
                health: None,
            };
        }
    }

    let pending: Vec<&(String, String, String)> = last_assistant
        .uses
        .iter()
        .filter(|(_, id, _)| !id.is_empty() && !answered.contains(id.as_str()))
        .collect();

    // ① 等确认（球在用户这边）优先于 ② 在途执行
    if let Some((name, id, _)) = pending
        .iter()
        .find(|(name, _, _)| QODER_REQUEST_TOOLS.contains(&name.to_lowercase().as_str()))
    {
        let message = if name.to_lowercase().contains("plan") {
            "等你确认下一步方案"
        } else {
            "等待你回答或选择"
        };
        return SessionProbe {
            signal: Some(Signal::Attention(
                fingerprint(path, &format!("qoder-{id}")),
                message.into(),
            )),
            subagent_count: 0,
            health: None,
        };
    }
    // ② 还在途
    if let Some((name, id, hint)) = pending.first() {
        return SessionProbe {
            signal: Some(Signal::Active(
                fingerprint(path, &format!("qoder-{id}")),
                Some(qoder_action_text(name, hint)),
            )),
            subagent_count: 0,
            health: None,
        };
    }
    // ③ 工具都收口了：看模型是怎么停的
    if last_assistant.stop_reason == "end_turn"
        || last_assistant.stop_reason == "stop_sequence"
    {
        // 完成态只保留一小段时间，之后自然回到「待机」——与其他方言同口径
        if file_age_secs > 15.0 * 60.0 {
            return SessionProbe::default();
        }
        return SessionProbe {
            signal: Some(Signal::Completed(fingerprint(path, &completion_fingerprint))),
            subagent_count: 0,
            health: None,
        };
    }
    // ④ 没停、也没在途 ⇒ 继续处理中
    SessionProbe {
        signal: Some(Signal::Active(
            fingerprint(path, &format!("qoder-cont-{}", last_assistant.id)),
            Some("继续处理中".into()),
        )),
        subagent_count: 0,
        health: None,
    }
}

/// Qoder 解析器的四条分支（对齐 Swift `detectQoder`）。
#[cfg(test)]
mod qoder_tests {
    use super::*;

    /// 造一份 Qoder 会话文件。`rows` 是 (role, stop_reason, prompt_id, human_input, blocks)
    fn session(rows: &[(&str, &str, &str, bool, &str)]) -> crate::testutil::Sandbox {
        let sandbox = crate::testutil::Sandbox::new("qoder");
        let body: String = rows
            .iter()
            .map(|(role, stop, pid, human, blocks)| {
                let human_field = if *human { r#","humanInput":true"# } else { "" };
                let pid_field = if pid.is_empty() {
                    String::new()
                } else {
                    format!(r#","promptId":"{pid}""#)
                };
                format!(
                    r#"{{"promptId":"{pid}"{human_field},"message":{{"id":"m-{role}-{stop}","role":"{role}","stop_reason":"{stop}","content":[{blocks}]}}}}"#
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(sandbox.path().join("sess-uuid.jsonl"), body).unwrap();
        sandbox
    }

    fn run(sandbox: &crate::testutil::Sandbox) -> SessionProbe {
        probe_dialect(
            "qoder",
            crate::models::SessionDialect::QoderTranscript,
            sandbox.path().join("sess-uuid.jsonl").to_str().unwrap(),
        )
    }

    /// ① 等确认优先于在途执行：球在用户这边的工具先判。
    #[test]
    fn an_unanswered_ask_question_is_attention_not_activity() {
        let s = session(&[(
            "assistant",
            "",
            "p1",
            false,
            r#"{"type":"tool_use","id":"ask1","name":"AskUserQuestion","input":{}}"#,
        )]);
        let probe = run(&s);
        match probe.signal {
            Some(Signal::Attention(_, message)) => assert_eq!(message, "等待你回答或选择"),
            other => panic!("应当是等确认：{other:?}"),
        }
    }

    /// plan 那一族给的是另一句话——**两个确认工具不能共用一句文案**，
    /// 否则用户分不清自己是在被问还是在被要求批准方案。
    #[test]
    fn the_plan_family_gets_its_own_wording() {
        let s = session(&[(
            "assistant",
            "",
            "p1",
            false,
            r#"{"type":"tool_use","id":"ask2","name":"exit_plan_mode","input":{}}"#,
        )]);
        match run(&s).signal {
            Some(Signal::Attention(_, message)) => assert_eq!(message, "等你确认下一步方案"),
            other => panic!("应当是等确认：{other:?}"),
        }
    }

    /// ② 工具已收口 + 模型 `end_turn` ⇒ 完成。
    #[test]
    fn a_finished_turn_with_all_results_answered_completes() {
        let s = session(&[
            ("user", "", "p1", true, ""),
            (
                "assistant",
                "end_turn",
                "p1",
                false,
                r#"{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}"#,
            ),
            (
                "assistant",
                "end_turn",
                "p1",
                false,
                r#"{"type":"tool_result","tool_use_id":"tu1"}"#,
            ),
        ]);
        assert!(matches!(run(&s).signal, Some(Signal::Completed(_))));
    }

    /// ③ 完成态**只保留一小段时间**（15 分钟），之后自然回到待机——
    /// 与其他方言同口径，否则岛会一直显示「已完成」。
    #[test]
    fn a_completion_goes_stale_after_fifteen_minutes() {
        let lines = vec![r#"{"promptId":"p1","message":{"id":"m1","role":"assistant","stop_reason":"end_turn","content":[]}}"#.to_string()];
        let fresh = probe_qoder(&lines, "/tmp/x.jsonl", 10.0 * 60.0, "sess");
        assert!(matches!(fresh.signal, Some(Signal::Completed(_))), "15 分钟内应当仍是完成态");
        let stale = probe_qoder(&lines, "/tmp/x.jsonl", 16.0 * 60.0, "sess");
        assert!(stale.signal.is_none(), "过了 15 分钟就该自然回到待机");
    }

    /// ④ 真人刚敲完、模型还没回：按**上一轮**的 end_turn 判就是
    /// 「你一发出指令，岛就说上一件事完成了」——而它等的正是这条新指令。
    #[test]
    fn a_fresh_human_turn_is_handled_before_the_previous_completion() {
        let s = session(&[
            (
                "assistant",
                "end_turn",
                "p1",
                false,
                r#"{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"ls"}}"#,
            ),
            (
                "assistant",
                "end_turn",
                "p1",
                false,
                r#"{"type":"tool_result","tool_use_id":"tu1"}"#,
            ),
            // 用户又敲了一条
            ("user", "", "p2", true, ""),
        ]);
        match run(&s).signal {
            Some(Signal::Active(_, action)) => assert_eq!(action.as_deref(), Some("正在处理你的新指令")),
            other => panic!("应当是「正在处理新指令」：{other:?}"),
        }
    }

    /// **完成指纹是「最近一次人类指令的身份」，不是 assistant 消息 id。**
    ///
    /// Qoder 每次模型调用都换 id，而长任务里模型会 `end_turn` 很多次
    /// （等后台构建、等并发会话回话）——按消息 id 记的话，
    /// 「同一件活」每续跑一轮就再弹一次「任务完成」。用户 2026-09-25 报的就是这个。
    #[test]
    fn the_completion_fingerprint_tracks_the_human_turn_not_the_assistant_message() {
        // ⚠️ 必须带一条**真人**（`humanInput`）的 user 行：
        // 「谁起的头」是靠它认的，没有它两次都会合法地回退到会话身份——
        // 那样测的就不是「换轮次会不会重新响」，而是「回退稳不稳定」了。
        let make = |pid: &str, msg: &str| {
            vec![
                format!(
                    r#"{{"promptId":"{pid}","humanInput":true,"message":{{"id":"u-{pid}","role":"user","content":[]}}}}"#
                ),
                format!(
                    r#"{{"promptId":"{pid}","message":{{"id":"{msg}","role":"assistant","stop_reason":"end_turn","content":[]}}}}"#
                ),
            ]
        };
        // 同一轮（同一个 promptId）、两次模型调用（两个消息 id）⇒ 指纹必须相同
        let first = probe_qoder(&make("p1", "m-A"), "/tmp/x.jsonl", 60.0, "sess");
        let second = probe_qoder(&make("p1", "m-B"), "/tmp/x.jsonl", 60.0, "sess");
        let second_fp = match &second.signal {
            Some(Signal::Completed(fp)) => fp.clone(),
            other => panic!("应是完成态：{other:?}"),
        };
        match (first.signal, Some(Signal::Completed(second_fp.clone()))) {
            (Some(Signal::Completed(a)), Some(Signal::Completed(b))) => {
                assert_eq!(a, b, "同一件活续跑一轮不该再响一次完成");
            }
            other => panic!("两条都应是完成态：{other:?}"),
        }
        // 换了人类轮次 ⇒ 指纹必须变
        let next = probe_qoder(&make("p2", "m-C"), "/tmp/x.jsonl", 60.0, "sess");
        match (Some(Signal::Completed(second_fp)), next.signal) {
            (Some(Signal::Completed(a)), Some(Signal::Completed(b))) => {
                assert_ne!(a, b, "用户真敲了新指令，就该重新响一次");
            }
            other => panic!("两条都应是完成态：{other:?}"),
        }
    }

    /// 认不出人类轮次时退到**会话文件**的身份，而不是当前轮次 id：
    /// 按轮次退会让每一次续跑都换个新指纹，等于把上面那条修复要治的病原地复发。
    #[test]
    fn an_unrecognised_turn_falls_back_to_the_session_identity() {
        let lines = vec![r#"{"message":{"id":"m1","role":"assistant","stop_reason":"end_turn","content":[]}}"#.to_string()];
        let a = probe_qoder(&lines, "/tmp/x.jsonl", 60.0, "sess-A");
        let b = probe_qoder(&lines, "/tmp/x.jsonl", 60.0, "sess-A");
        match (a.signal, b.signal) {
            (Some(Signal::Completed(x)), Some(Signal::Completed(y))) => assert_eq!(x, y),
            other => panic!("应退到会话身份：{other:?}"),
        }
    }

    /// 动作文案按工具名分类，缺一类就少一句可读的话。
    #[test]
    fn the_action_text_covers_the_main_tool_families() {
        assert_eq!(qoder_action_text("Bash", "ls"), "运行: ls");
        assert_eq!(qoder_action_text("Edit", "a.rs"), "修改: a.rs");
        assert_eq!(qoder_action_text("Read", "b.rs"), "读取: b.rs");
        assert_eq!(qoder_action_text("Agent", "查日志"), "子任务: 查日志");
        assert_eq!(qoder_action_text("mcp_call", "search"), "调用工具: search");
        // 没有线索时也要给一句话，不能是空的
        assert_eq!(qoder_action_text("Bash", ""), "执行终端命令");
        assert_eq!(qoder_action_text("Whatever", ""), "正在执行 whatever");
    }
}

// MARK: - DSH 方言（`session_projcache/sessions/<id>.json` 投影缓存）

/// 活跃保护期：文件 30 分钟内有更新就算「还在跑」。
///
/// 为什么是 30 分钟而完成态是 15 分钟：活跃期覆盖的是「投影缓存还在刷新」，
/// 完成后投影不再更新，所以活跃窗口必须**比完成窗口长**，否则一个跑了 20 分钟
/// 的任务会在第 15 分钟被判成「完成」，而它明明还在跑。
const DSH_ACTIVE_MAX_AGE_SECS: f64 = 30.0 * 60.0;
/// 完成态保留窗口（与其他方言同口径）
const DSH_COMPLETED_MAX_AGE_SECS: f64 = 15.0 * 60.0;

/// DSH 解析器。逐条对应 Swift `inspectDSHSession`。
///
/// 输入是**一个投影 JSON**（不是 JSONL）：`{"record":{"rows":{…}}}`，
/// 里面是投影缓存已经把这一轮的状态算好的结果。
/// 换句话说这一族**不做会话重放**——那正是它叫「投影」的原因。
fn probe_dsh(path: &str, file_age_secs: f64) -> SessionProbe {
    if file_age_secs > 24.0 * 3600.0 {
        return SessionProbe::default();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return SessionProbe {
            signal: None,
            subagent_count: 0,
            health: Some(SessionProbeHealth {
                failure: SessionProbeFailure::UnreadableFile,
                path: path.to_string(),
                observed_at: 0,
            }),
        };
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return SessionProbe {
            signal: None,
            subagent_count: 0,
            health: Some(SessionProbeHealth {
                failure: SessionProbeFailure::UndecodableFile,
                path: path.to_string(),
                observed_at: 0,
            }),
        };
    };
    // 投影文件名的 `<id>.json` 就是会话 id
    let session_id = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(rows) = root
        .get("record")
        .and_then(|r| r.get("rows"))
    else {
        return SessionProbe::default();
    };

    // 任务标题（`rows.title.val`）
    let raw_title = rows
        .get("title")
        .and_then(|t| t.get("val"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let clean_title = one_line(raw_title, 26);

    // ① 等确认：投影里挂着待批准的 id
    if let Some(pending) = rows
        .get("approval")
        .and_then(|a| a.get("val"))
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
    {
        if !pending.is_empty() {
            let tool = rows
                .get("approval")
                .and_then(|a| a.get("val"))
                .and_then(|v| v.get("toolName"))
                .and_then(|v| v.as_str())
                .unwrap_or("操作");
            return SessionProbe {
                signal: Some(Signal::Attention(
                    fingerprint(path, pending),
                    format!("等待你批准执行: {tool}"),
                )),
                subagent_count: 0,
                health: None,
            };
        }
    }

    // ② 轮次与步骤
    let open_turn_start_seq = rows
        .get("turnBoundary")
        .and_then(|t| t.get("val"))
        .and_then(|v| v.get("openTurnStartSeq"));
    let stats = rows.get("sessionStats").and_then(|s| s.get("val"));
    let open_step = stats.and_then(|v| v.get("openStep"));
    let as_int = |v: Option<&Value>| -> Option<i64> { v.and_then(|v| v.as_i64()) };
    let current_step = as_int(open_step.and_then(|o| o.get("step")))
        .or_else(|| as_int(stats.and_then(|v| v.get("steps"))));
    let current_turn = as_int(open_step.and_then(|o| o.get("turn")))
        .or_else(|| as_int(stats.and_then(|v| v.get("lastTurn"))))
        .unwrap_or(1);
    let is_open = open_turn_start_seq.is_some() || open_step.is_some();

    if is_open {
        // ③ 活跃：投影还在刷新。**过了保护期就退回无信号**——
        // 一个半小时没刷新的投影不代表任务还在跑，只代表它还躺在盘上。
        if file_age_secs > DSH_ACTIVE_MAX_AGE_SECS {
            return SessionProbe::default();
        }
        let action = match (clean_title.is_empty(), current_step.filter(|s| *s > 0)) {
            (false, Some(step)) => format!("执行中: {clean_title} (第 {step} 步)"),
            (false, None) => format!("执行中: {clean_title}"),
            (true, Some(step)) => format!("执行任务中 (第 {step} 步)"),
            (true, None) => "执行任务中".to_string(),
        };
        return SessionProbe {
            signal: Some(Signal::Active(
                fingerprint(path, &format!("dsh-{session_id}-turn{current_turn}")),
                Some(action),
            )),
            subagent_count: 0,
            health: None,
        };
    }

    // ④ 轮次已结束
    if file_age_secs > DSH_COMPLETED_MAX_AGE_SECS {
        return SessionProbe::default();
    }
    let total_steps = as_int(stats.and_then(|v| v.get("steps"))).unwrap_or(0);
    SessionProbe {
        signal: Some(Signal::Completed(fingerprint(
            path,
            &format!("dsh-{session_id}-t{current_turn}-s{total_steps}"),
        ))),
        subagent_count: 0,
        health: None,
    }
}

/// DSH 解析器的四个分支（对齐 Swift `inspectDSHSession`）。
#[cfg(test)]
mod dsh_tests {
    use super::*;

    /// 造一份投影文件。`rows` 直接就是 `record.rows` 那个对象。
    fn projection(rows: &str) -> crate::testutil::Sandbox {
        let sandbox = crate::testutil::Sandbox::new("dsh");
        std::fs::write(
            sandbox.path().join("sess-42.json"),
            format!(r#"{{"record":{{"rows":{rows}}}}}"#),
        )
        .unwrap();
        sandbox
    }

    fn run(sandbox: &crate::testutil::Sandbox) -> SessionProbe {
        probe_dsh(
            sandbox.path().join("sess-42.json").to_str().unwrap(),
            60.0,
        )
    }

    /// ① 等确认优先：投影里挂着待批准的 id 就先问用户，
    /// 哪怕同一次还记着 `openStep`（那只是它还没来得及清）。
    #[test]
    fn a_pending_approval_beats_everything_else() {
        let s = projection(
            r#"{"approval":{"val":{"id":"ap-1","toolName":"Bash"}},
                "sessionStats":{"val":{"openStep":{"step":2,"turn":3}}}}"#,
        );
        match run(&s).signal {
            Some(Signal::Attention(_, message)) => {
                assert_eq!(message, "等待你批准执行: Bash")
            }
            other => panic!("应当是等确认：{other:?}"),
        }
    }

    /// ② 活跃态：把标题与步骤都写进动作行。
    #[test]
    fn an_open_turn_reports_the_title_and_step() {
        let s = projection(
            r#"{"title":{"val":"修复登录崩溃"},
                "sessionStats":{"val":{"openStep":{"step":2,"turn":3}}}}"#,
        );
        match run(&s).signal {
            Some(Signal::Active(_, action)) => {
                assert_eq!(action.as_deref(), Some("执行中: 修复登录崩溃 (第 2 步)"))
            }
            other => panic!("应当是在途：{other:?}"),
        }
    }

    /// 标题里可能有换行——它会撑破单行动作行。
    #[test]
    fn a_multiline_title_is_flattened() {
        let s = projection(
            "{\"title\":{\"val\":\"第一行\\n第二行\"},\"sessionStats\":{\"val\":{\"openStep\":{\"step\":1}}}}",
        );
        match run(&s).signal {
            Some(Signal::Active(_, action)) => {
                let action = action.unwrap();
                assert!(!action.contains('\n'), "动作行里不该有换行：{action:?}");
            }
            other => panic!("应当是在途：{other:?}"),
        }
    }

    /// ③ 活跃保护期 30 分钟，比完成窗口长。
    ///
    /// 反过来（活跃窗口短于完成窗口）会让一个跑了 20 分钟的任务在第 15 分钟
    /// 被判成「完成」，而它明明还在跑——这是这类判定最典型的错法。
    #[test]
    fn the_active_window_is_longer_than_the_completed_one() {
        assert!(
            DSH_ACTIVE_MAX_AGE_SECS > DSH_COMPLETED_MAX_AGE_SECS,
            "活跃窗口必须比完成窗口长，否则长任务会被中途判成完成"
        );
        let s = projection(r#"{"sessionStats":{"val":{"openStep":{"step":1}}}}"#);
        let path = s.path().join("sess-42.json").to_string_lossy().into_owned();
        // 20 分钟：活跃窗口内、在完成窗口外 —— 必须是「在途」而不是「完成」
        let probe = probe_dsh(&path, 20.0 * 60.0);
        assert!(matches!(probe.signal, Some(Signal::Active(_, _))));
        // 35 分钟：两个窗口都过了 —— 必须无信号，而不是继续说它在跑
        let probe = probe_dsh(&path, 35.0 * 60.0);
        assert!(probe.signal.is_none(), "过了保护期就该退回无信号");
    }

    /// ④ 轮次已结束 ⇒ 完成，且指纹带会话 id / 轮次 / 总步数。
    #[test]
    fn a_finished_turn_completes_with_its_session_identity() {
        let s = projection(r#"{"sessionStats":{"val":{"steps":7,"lastTurn":3}}}"#);
        let a = run(&s).signal;
        let b = run(&s).signal;
        match (a, b) {
            (Some(Signal::Completed(x)), Some(Signal::Completed(y))) => assert_eq!(x, y),
            other => panic!("应两次都是完成态：{other:?}"),
        }
    }

    /// 投影文件读不出来 / 形状不对 ⇒ **都要带理由**，
    /// 不能与「读到、确实没事」混成同一个空信号。
    #[test]
    fn a_broken_projection_carries_a_reason() {
        let sandbox = crate::testutil::Sandbox::new("dsh-broken");
        let path = sandbox.path().join("sess-9.json");
        // 形状不对（`record.rows` 不在）⇒ 读到了但读不懂，**不得**报出信号
        std::fs::write(&path, r#"{"record":{"nope":1}}"#).unwrap();
        let probe = probe_dsh(path.to_str().unwrap(), 60.0);
        assert!(probe.signal.is_none(), "形状不对时不得报出信号");
        // 完全不是 JSON
        std::fs::write(&path, "这不是 JSON").unwrap();
        let probe = probe_dsh(path.to_str().unwrap(), 60.0);
        assert_eq!(
            probe.health.map(|h| h.failure),
            Some(SessionProbeFailure::UndecodableFile),
            "读不出来与读不懂要分开"
        );
    }
}

// MARK: - Antigravity 方言（`brain/<session>/.system_generated/logs/transcript.jsonl`）

/// Antigravity 的**在途保护期**：只有最近 5 分钟的记录才算「还在跑」。
const ANTIGRAVITY_ACTIVE_MAX_AGE_SECS: f64 = 300.0;
/// 完成态窗口（与其他方言同口径）
const ANTIGRAVITY_COMPLETED_MAX_AGE_SECS: f64 = 15.0 * 60.0;

/// 这些名字都在**等用户**：球在他们这边。
const ANTIGRAVITY_ASK_TOOLS: [&str; 3] = ["ask_question", "askquestion", "ask_user"];

/// Antigravity 解析器。逐条对应 Swift `detectAntigravitySession` 的**状态判定**部分。
///
/// ⚠️ **本版只做状态信号，没做上下文**。Swift 侧还会解析后台任务生命周期、
/// 子智能体角色/模型、Token 细分，产出 `SessionActiveContext`（后台任务胶囊、
/// 子任务数、Token 细分那一块）。Rust 侧连那个字段都还没有，
/// 所以这里先不做——**宁可少显示，不要显示错的**。
/// 剩余工作量记在对照表 §6.3。
fn probe_antigravity(lines: &[String], path: &str, file_age_secs: f64) -> SessionProbe {
    // 预扫：每行的 (step_index, type, 有无 tool_calls)
    // 用途是回答「这个 ask_question 是不是**已经有人答过**」——尾窗里可能同时
    // 留着请求与回答，只看当前行会把它当成仍在等。
    let meta: Vec<(i64, String, bool)> = lines
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .map(|obj| {
            (
                obj.get("step_index").and_then(|v| v.as_i64()).unwrap_or(0),
                obj.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                obj.get("tool_calls")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty()),
            )
        })
        .collect();

    // 从尾部逆序推导当前状态
    for obj in lines.iter().rev().filter_map(|raw| serde_json::from_str::<Value>(raw).ok()) {
        let step_index = obj.get("step_index").and_then(|v| v.as_i64()).unwrap_or(0);
        let step_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let fp = fingerprint(path, &format!("antigravity-step-{step_index}"));
        let tool_calls = obj.get("tool_calls").and_then(|v| v.as_array());

        // ① 等待用户选择或确认
        if let Some(calls) = tool_calls.filter(|c| !c.is_empty()) {
            if let Some(ask) = calls.iter().find(|c| {
                let name = c
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                ANTIGRAVITY_ASK_TOOLS.contains(&name.as_str())
            }) {
                // 尾窗里若已出现同名的后续调用 ⇒ 那是回答，不是仍在等
                let answered = meta.iter().any(|(_, ty, has_tc)| {
                    *has_tc && (ty == "TOOL_OUTPUT" || ty == "SYSTEM_MESSAGE")
                });
                if !answered {
                    let question = ask
                        .get("args")
                        .and_then(|a| a.get("questions"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .and_then(|v| {
                            v.as_array()
                                .and_then(|a| a.first())
                                .and_then(|q| q.get("question"))
                                .and_then(|q| q.as_str())
                                .map(str::to_string)
                        })
                        .filter(|q| !q.is_empty())
                        .unwrap_or_else(|| "等待你的确认".into());
                    return attention_like(path, &format!("ask-{step_index}"), &question);
                }
            }

            // ② 正在执行工具调用。**保护期只有 5 分钟**——
            // 之后那行仍留在文件里，但不代表它还在跑。
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return SessionProbe::default();
            }
            let action = tool_calls
                .and_then(|c| c.first())
                .and_then(|c| c.get("args"))
                .and_then(|a| {
                    a.get("toolAction")
                        .or_else(|| a.get("toolSummary"))
                })
                .and_then(|v| v.as_str())
                .map(|s| one_line(s, 100))
                .or_else(|| {
                    tool_calls
                        .and_then(|c| c.first())
                        .and_then(|c| c.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "执行中".into());
            return active_like(path, &fp, &action);
        }

        // ③ 规划响应 / 最终回答：有内容给用户，或明确 DONE 且没有活跃思考
        let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let thinking = obj.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
        if (!content.is_empty()) || (status == "DONE" && thinking.is_empty()) {
            if file_age_secs <= ANTIGRAVITY_COMPLETED_MAX_AGE_SECS {
                return SessionProbe {
                    signal: Some(Signal::Completed(fp)),
                    subagent_count: 0,
                    health: None,
                };
            }
            return SessionProbe::default(); // 15 分钟后自然转入待机
        }

        // ④ 只有思考、无工具也无最终内容 ⇒ 正在思考规划
        if !thinking.is_empty() {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return SessionProbe::default();
            }
            return active_like(path, &fp, "思考规划中");
        }

        // ⑤ 用户刚发完输入，模型正在启动准备
        if step_type == "USER_INPUT" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return SessionProbe::default();
            }
            return active_like(path, &fp, "思考规划中");
        }

        // ⑥ 工具输出返回，等待下一拍调度
        if step_type == "TOOL_OUTPUT" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return SessionProbe::default();
            }
            return active_like(path, &fp, "处理中");
        }

        // ⑦ 系统通知 / 任务完成结果
        if step_type == "SYSTEM_MESSAGE" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return SessionProbe::default();
            }
            return active_like(path, &fp, "处理任务结果中");
        }
    }
    SessionProbe::default()
}

fn active_like(path: &str, fingerprint_key: &str, action: &str) -> SessionProbe {
    SessionProbe {
        signal: Some(Signal::Active(
            fingerprint(path, fingerprint_key),
            Some(action.to_string()),
        )),
        subagent_count: 0,
        health: None,
    }
}

fn attention_like(path: &str, fingerprint_key: &str, message: &str) -> SessionProbe {
    SessionProbe {
        signal: Some(Signal::Attention(
            fingerprint(path, fingerprint_key),
            message.to_string(),
        )),
        subagent_count: 0,
        health: None,
    }
}

/// Antigravity 解析器的分支（对齐 Swift `detectAntigravitySession` 的状态判定部分）。
#[cfg(test)]
mod antigravity_tests {
    use super::*;

    fn lines_of(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|r| (*r).to_string()).collect()
    }

    /// ① `ask_question` 没被回答 ⇒ 等确认，且把问题原文带出来。
    #[test]
    fn an_unanswered_question_is_attention_with_its_text() {
        let lines = lines_of(&[r#"{"step_index":7,"type":"ASSISTANT","tool_calls":[{"name":"ask_question","args":{"questions":"[{\"question\":\"要不要继续？\"}]"}}]}"#]);
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal {
            Some(Signal::Attention(_, message)) => assert_eq!(message, "要不要继续？"),
            other => panic!("应当是等确认：{other:?}"),
        }
    }

    /// **在途保护期只有 5 分钟**——比完成窗口还短。
    ///
    /// 这一族与 DSH 相反：Antigravity 的 transcript 是**持续追加**的，
    /// 旧行不会消失，所以「文件里有这行」完全不代表「它还在跑」。
    /// 用 6 分钟的旧行去判在途，就会得到一个永远亮着的指示灯。
    #[test]
    fn an_old_tool_call_line_is_not_still_running() {
        let lines = lines_of(&[r#"{"step_index":7,"type":"ASSISTANT","tool_calls":[{"name":"run_terminal","args":{"toolAction":"npm test"}}]}"#]);
        assert!(matches!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal,
            Some(Signal::Active(_, _))
        ));
        assert!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 6.0 * 60.0).signal.is_none(),
            "超过 5 分钟保护期就不该再说它在跑"
        );
    }

    /// ② 有内容给用户 ⇒ 完成，但只保留 15 分钟。
    #[test]
    fn a_final_answer_completes_and_then_goes_stale() {
        let lines = lines_of(&[r#"{"step_index":9,"type":"ASSISTANT","content":"改好了"}"#]);
        assert!(matches!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 10.0 * 60.0).signal,
            Some(Signal::Completed(_))
        ));
        assert!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 16.0 * 60.0).signal.is_none(),
            "完成态过了 15 分钟就该自然回到待机"
        );
    }

    /// **`DONE` 但仍在思考**不算完成——那一行说明模型还没收尾。
    /// 只看 `status == "DONE"` 会把「正在想」判成「干完了」。
    #[test]
    fn done_with_active_thinking_is_not_yet_complete() {
        let lines = lines_of(&[r#"{"step_index":9,"type":"ASSISTANT","status":"DONE","thinking":"再想想"}"#]);
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal {
            Some(Signal::Active(_, action)) => {
                assert_eq!(action.as_deref(), Some("思考规划中"))
            }
            other => panic!("应当是「思考规划中」：{other:?}"),
        }
    }

    /// ③ `USER_INPUT` 分支 ⇒ 「思考规划中」——
    /// 但**只在 `content` 为空时**才轮得到它。
    #[test]
    fn a_user_input_without_content_counts_as_getting_ready() {
        let lines = lines_of(&[r#"{"step_index":1,"type":"USER_INPUT"}"#]);
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal {
            Some(Signal::Active(_, action)) => {
                assert_eq!(action.as_deref(), Some("思考规划中"))
            }
            other => panic!("应当是「思考规划中」：{other:?}"),
        }
    }

    /// ④ `TOOL_OUTPUT` 分支 ⇒ 「处理中」——同样只在 `content` 为空时。
    #[test]
    fn a_tool_output_without_content_means_waiting_for_the_next_turn() {
        let lines = lines_of(&[r#"{"step_index":8,"type":"TOOL_OUTPUT"}"#]);
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal {
            Some(Signal::Active(_, action)) => assert_eq!(action.as_deref(), Some("处理中")),
            other => panic!("应当是「处理中」：{other:?}"),
        }
    }

    /// **如实记录一处继承自 macOS 侧的行为**：带 `content` 的 `USER_INPUT` /
    /// `TOOL_OUTPUT` 会被判成「完成」，而**不是**走到它们各自的分支。
    ///
    /// 原因是 Swift 侧把「内容非空 ⇒ 轮次结束」那条放在**所有类型分支之前**，
    /// 于是任何带正文的行都先命中它。Rust 照搬了同一顺序——**这一版要的是一致，
    /// 不是「顺手修好」**；本仓的自由侧与跨平台侧不一样就是分叉的温床。
    ///
    /// 语义上它可疑（用户刚发完输入就说「完成」），但要改就得**两端一起改**，
    /// 那是独立一块，列在对照表 §3.2。这里用用例把它钉住，免得它悄悄漂走。
    #[test]
    fn a_typed_line_with_content_is_completed_even_though_its_type_says_otherwise() {
        for (row, why) in [
            (
                r#"{"step_index":1,"type":"USER_INPUT","content":"帮我改一下"}"#,
                "USER_INPUT",
            ),
            (
                r#"{"step_index":8,"type":"TOOL_OUTPUT","content":"exit 0"}"#,
                "TOOL_OUTPUT",
            ),
        ] {
            let lines = lines_of(&[row]);
            assert!(
                matches!(
                    probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).signal,
                    Some(Signal::Completed(_))
                ),
                "{why} 带 content 时被判成完成——这是从 macOS 侧照搬的行为，见本用例注释"
            );
        }
    }

    /// 尾窗里**没有任何能判定的行** ⇒ 无信号，且**不带理由**：
    /// 读到了、只是没有可判定的东西，与「读不到」是两件事。
    #[test]
    fn an_empty_window_is_not_reported_as_unreadable() {
        let probe = probe_antigravity(&[], "/tmp/x.jsonl", 60.0);
        assert!(probe.signal.is_none());
        assert!(probe.health.is_none(), "没有行不等于读不到");
    }
}

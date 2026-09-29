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
        match &self.failure {
            // 库查询失败时把 SQLite 的话原样带出来：不给它，用户看到的是
            // 「这个 Agent 没有待确认」，而真相是「我们连状态都没查到」
            SessionProbeFailure::UnreadableDatabase(detail) => format!(
                "会话库查询失败：{detail}（{}）——此后的「待机」只代表没有查到状态，不代表智能体真的空闲",
                self.path
            ),
            other => format!(
                "会话源不可读：{}（{}）——此后的「待机」只代表没有读到信号，不代表智能体真的空闲",
                other.label(),
                self.path
            ),
        }
    }

    /// 这条故障还新鲜吗。超过 `window_ms` 就当它已经过去——
    /// 挂着一条几小时前的「读不到」，而源早已修好，那是另一种谎报。
    pub fn is_fresh(&self, now_ms: i64, window_ms: i64) -> bool {
        (now_ms - self.observed_at).max(0) <= window_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionProbeFailure {
    /// 会话文件在，但读不出来（权限拒绝 / 是目录 / 读取抛错）
    UnreadableFile,
    /// 读出来了，但**没有一行**是解析器认识的形状（改版、顶层类型漂移）
    ///
    /// 与「文件里确实没有待确认事项」是**两件事**，不得混为一谈：
    /// 前者是「我们读不懂」，后者是「读懂了、确实没事」。
    UndecodableFile,
    /// **会话库**读不出信号，带 SQLite 的诊断文本（Swift 同族也是报 prepare / step 失败）
    ///
    /// 为什么单独一个变体而不是塞进上面两个：这一族的问题不是「文件读不了」，
    /// 而是「查询跑不通」——最常见的是**列名对不上**（真机上 zcode 的 `id`
    /// 实际叫 `task_id`，那条 SQL 于是恒定 prepare 失败）。
    /// 这种失败**静默得最彻底**：返回值与「这个 Agent 没有终态」完全一样，
    /// 于是信号永远不响、界面上看不出异样。诊断文本必须一路带到界面上。
    UnreadableDatabase(String),
}

impl SessionProbeFailure {
    pub fn label(&self) -> &'static str {
        match self {
            Self::UnreadableFile => "会话文件无法读取",
            Self::UndecodableFile => "会话文件格式与解析器不匹配",
            // 诊断文本由 `diagnostic_text` 带出去；这里只给一个短标签
            // （调用方会把 `label()` 塞进定长文案，SQLite 的原文另走一支）
            Self::UnreadableDatabase(_) => "会话库查询失败",
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
/// 返回 `(信号, 本轮上下文)`。
///
/// **上下文不挂在 `SessionProbe` 上**，是刻意的：`SessionProbe` 有一百多处字面量构造，
/// 给它加字段就得每处都补；而上下文**只有 Antigravity 一族产出**，
/// 让它走返回值就不会把「每个 Agent 都有上下文」这个错觉写进类型里。
pub fn probe_dialect(
    profile_id: &str,
    dialect: crate::models::SessionDialect,
    path: &str,
) -> (SessionProbe, crate::models::SessionActiveContext) {
    use crate::models::SessionActiveContext as Context;
    match dialect {
        crate::models::SessionDialect::QoderTranscript => {
            return (probe_qoder_dialect(profile_id, path), Context::default());
        }
        crate::models::SessionDialect::DshProjection => {
            return (probe_dsh_dialect(path), Context::default());
        }
        crate::models::SessionDialect::AntigravityBrain => {
            return probe_antigravity_dialect(path);
        }
        _ => {}
    }
    if !DIALECTS_WITH_PARSER.contains(&dialect) {
        // 已声明、尚无解析器：如实无信号，**不拿猜的解析器顶上去**
        return (SessionProbe::default(), Context::default());
    }
    (probe_by_id(profile_id, path), Context::default())
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

fn probe_antigravity_dialect(path: &str) -> (SessionProbe, crate::models::SessionActiveContext) {
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    use crate::models::SessionActiveContext as Context;
    if age > 24.0 * 3600.0 {
        return (SessionProbe::default(), Context::default());
    }
    match read_tail_lines(path) {
        Ok(lines) => {
            if lines.is_empty() {
                return (SessionProbe::default(), Context::default());
            }
            probe_antigravity(&lines, path, age)
        }
        Err(failure) => (
            SessionProbe {
                signal: None,
                subagent_count: 0,
                health: Some(SessionProbeHealth {
                    failure,
                    path: path.to_string(),
                    observed_at: 0,
                }),
            },
            Context::default(),
        ),
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
    // 当前这条消息在数组里的下标。只有看最后一条时它就是 `len-1`，
    // 但**要显式取出来**：下面 attention 分支的指纹靠它区分「同一条」与「新一条」。
    let idx = arr.len() - 1;
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
                // **指纹里必须带这条消息在数组里的下标。**
                //
                // 引擎层 `alerted_fingerprints` 是**全局 `HashSet`，从不按 Agent 清理**
                // （只有累计超过 800 条才整表清空），`insert` 返回 false 就不推事件。
                // 而旧指纹只由 `{ask}{消息字节长度}` 决定，于是：
                // **同一个文件里两条长度相同的提问会撞同一个指纹，第二条被永久静默。**
                // 「重试」「继续」「好的」这种长度的提问在真实对话里俯拾皆是。
                //
                // 下标正好补上这个洞，而且**不需要对消息形状做任何新假设**
                // （只依赖 `arr` 是数组——这本来就���前提）：
                // · 同一条待确认问题反复轮询 ⇒ 下标不变 ⇒ 指纹不变 ⇒ 不重复提醒；
                // · 换了新问题 ⇒ 下标必变 ⇒ 指纹必变 ⇒ 必然提醒。
                // 这正是 Swift `cline-{ts}`（逐条消息时间戳）达到的效果。
                //
                // 教训：指纹是**去重语义的一部分**，不是随手取个 hash——
                // 换一个「看起来更唯一」的字段时，要先确认去重那一侧拿它做什么。
                fingerprint(path, &format!("cline-msg{idx}-{ask}{}", msg.len())),
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

// MARK: - StatusIndex 方言（Swift `inspectStatusDatabase`）

/// 状态索引这一族只回答一个问题：**最新一条会话现在是什么状态**。
///
/// 它不含 token（所以 [`crate::tokens`] 对 `StatusIndex` 返回 `None` 是对的），
/// 但它是**唯一能报出「等待你批准」的地方**——JSONL 那一族只能看到
/// 「模型刚跑完一次请求」，看不到「它在等一个人点确认」。
///
/// 三条与 Swift 同口径的规则：
/// · **库文件超过 24h 就当没有**：那份状态索引早就不是「现在」了；
/// · **completed 只认 15 分钟内**：更早的完成属于历史，不该再报「刚完成」；
/// · **读不到就说读不到**：prepare / step 失败都留下 `SessionProbeHealth`，
///   绝不静默返回「没有终态」——那两者在界面上完全一样。
///
/// **状态词表与 Swift 同源**（`requestStates` 与完成态列表照搬）。
/// 词表漂了不会有人报错，只会表现为「一边报等待批准、一边不报」，
/// 所以 [`status_vocabulary_tests`] 直接读 Swift 源文件比对。

/// 「等一个人」的词表（Swift `requestStates`，**逐条照搬，15 个**）
///
/// **不多加词**。第一版这里凭语感补了 `requiresapproval` / `needsapproval` /
/// `waitingapproval` 三个——听着合理，但 Swift 里没有，而本机 ZCode 实际只写
/// `running` / `error` / `completed`。**没有任何产生点的词条就是纸面**：
/// 它只会让「这个词看起来被支持过」成为假印象。
/// [`status_index_tests::the_vocabulary_matches_the_swift_side`] 双向钉住这份清单。
const REQUEST_STATES: [&str; 15] = [
    "approvalrequest",
    "approvalrequested",
    "permissionrequest",
    "permissionrequested",
    "confirmationrequest",
    "requiresconfirmation",
    "needsconfirmation",
    "pendingapproval",
    "awaitingapproval",
    "awaitinguserinput",
    "waitingforuser",
    "waitingforuserinput",
    "userinputrequest",
    "elicitation",
    "approvalasked",
];

/// 完成态词表（Swift 同口径的五个）
const COMPLETED_STATES: [&str; 5] = [
    "completed",
    "complete",
    "done",
    "succeeded",
    "success",
];

/// 库文件保质期：24h（Swift `fileAge(path) <= 24 * 3600`）
const STATUS_INDEX_MAX_AGE_SECS: f64 = 24.0 * 3600.0;
/// 完成态保质期：15 分钟（Swift 同值）
const STATUS_COMPLETED_MAX_AGE_SECS: f64 = 15.0 * 60.0;

/// 与 Swift `normalized` 同口径：转小写后**只留字母数字**。
///
/// 于是 `Awaiting Approval`、`awaiting_approval`、`AWAITINGAPPROVAL`
/// 三种写法归一化成同一个词。跨产品抄来的状态值大小写与分隔符都不统一，
/// 不归一化就会逐个产品各写一份 `match`。
pub fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// epoch 秒的「是不是毫秒」分界（Swift 同值）。
/// 1e10 秒 ≈ 公元 2286 年，所以任何真实时间戳都不会越过它。
const MILLIS_EPOCH_FLOOR: f64 = 10_000_000_000.0;

/// 探测状态索引。会话库的路径与查询都由档案声明（ADR 0004）。
///
/// 返回 `(信号, 失败原因)`——失败原因单独返回而不是塞进 `SessionProbe`，
/// 是为了让「读不到」在这一层就可见，调用方决定要不要盖到快照上。
pub fn probe_status_index(
    database: &crate::models::SessionDatabase,
    file_age_secs: f64,
) -> (SessionProbe, Option<SessionProbeFailure>) {
    if file_age_secs > STATUS_INDEX_MAX_AGE_SECS {
        return (SessionProbe::default(), None);
    }
    let Some(sql) = database.status_sql.as_deref().filter(|s| !s.is_empty()) else {
        // 档案声明了这一方言却没给查询：**说清是读不到**，而不是当成「没有终态」
        return (
            SessionProbe::default(),
            Some(SessionProbeFailure::UnreadableDatabase("档案声明了 statusIndex 方言却没给 status_sql".into())),
        );
    };
    let connection = match crate::sqlite::open_readonly(&database.path) {
        Ok(connection) => connection,
        Err(crate::sqlite::Failure::Missing) => return (SessionProbe::default(), None),
        Err(crate::sqlite::Failure::OpenFailed(detail)) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(detail)),
            )
        }
    };
    let mut stmt = match connection.prepare(sql) {
        Ok(stmt) => stmt,
        // **prepare 失败是这一族最容易踩的坑**：列名写错时它恒定失败，
        // 而失败被当成「没有终态」的话，这个 Agent 的信号就永远不响了。
        // 所以这里必须把 SQLite 的诊断文本带出来。
        Err(error) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(format!("{} · 查询：{sql}", error))),
            )
        }
    };
    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(error) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(format!("{} · 查询：{sql}", error))),
            )
        }
    };
    // `rows.next()` 返回 `Result<Option<&Row>>`：**没有行**是这个库还没有会话
    // （不是失败），**Err** 才是读不出来。两者混为一谈就是我上一轮踩的坑。
    let row = match rows.next() {
        Ok(Some(row)) => row,
        Ok(None) => return (SessionProbe::default(), None),
        Err(error) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(format!("{error} · 查询：{sql}"))),
            )
        }
    };
    let id: String = row.get(0).unwrap_or_default();
    let status: String = row.get::<_, Option<String>>(1).unwrap_or_default().unwrap_or_default();
    let raw_time: f64 = row.get::<_, Option<f64>>(2).unwrap_or_default().unwrap_or_default();

    // 毫秒 / 秒 epoch 兼容（Swift 同一条）：有的产品写秒，有的写毫秒
    let epoch = if raw_time > MILLIS_EPOCH_FLOOR {
        raw_time / 1000.0
    } else {
        raw_time
    };
    // epoch 缺失或荒谬时退回文件年龄——总得有个「多久之前」
    let age = if epoch > 0.0 {
        (now_secs() - epoch).max(0.0)
    } else {
        file_age_secs
    };
    let fingerprint = fingerprint(&database.path, &if id.is_empty() { &database.path } else { &id });
    let normalized_status = normalized(&status);

    if REQUEST_STATES.contains(&normalized_status.as_str()) {
        // 批准类与确认类分开说：用户要的决策不一样
        let approval = normalized_status.contains("approval")
            || normalized_status.contains("permission")
            || normalized_status.contains("confirm");
        return (
            SessionProbe {
                signal: Some(Signal::Attention(
                    fingerprint,
                    if approval { "等待你批准操作" } else { "等待你选择或确认" }.into(),
                )),
                subagent_count: 0,
                health: None,
            },
            None,
        );
    }
    if COMPLETED_STATES.contains(&normalized_status.as_str())
        && age <= STATUS_COMPLETED_MAX_AGE_SECS
    {
        return (
            SessionProbe {
                signal: Some(Signal::Completed(fingerprint)),
                subagent_count: 0,
                health: None,
            },
            None,
        );
    }
    (SessionProbe::default(), None)
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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

/// 按行读取，**只承认到最后一条完整行**，并回吐 `(真正承认到的字节数, 文件是否以换行结尾)`。
///
/// **为什么不能图省事写成「逐行读完就算」**：会话尾读每轮重读尾部窗口，
/// 末尾半行解析失败也无所谓；而 token 明细是**续读游标**——一旦把半个 JSON 行
/// 承认下来并前进游标，那条记录下一次就再也读不回来了（下一次从更后面开始）。
/// 所以这里宁可停在上一个换行处，等它写完再读。
///
/// 此前这里还有一个更简单的 `for_each_line`（逐行读完、返回文件长度）。
/// 它**没有任何调用方**，`for_each_complete_line` 取代了它——留着它只多出一条
/// 编译警告（`never used`）与一个「这里有两个读法」的选择题。**没有产生点的
/// 公开函数就是纸面**，删掉比留着让人猜好。
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
        let ok = probe_dialect("claude", crate::models::SessionDialect::GenericTail, sandbox.path().join("s.jsonl").to_str().unwrap()).0;
        assert!(ok.signal.is_none());
        assert!(
            ok.health.is_none(),
            "读到了只是没信号，不该报「读不到」"
        );

        // ② 文件读不出来（这里用「路径是目录」构造）⇒ **有**理由
        let dir_sandbox = crate::testutil::Sandbox::new("healthdir");
        std::fs::create_dir_all(dir_sandbox.path().join("s.jsonl")).unwrap();
        let broken = probe_dialect("claude", crate::models::SessionDialect::GenericTail, dir_sandbox.path().join("s.jsonl").to_str().unwrap()).0;
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
        let probe = probe_dialect("claude", crate::models::SessionDialect::GenericTail, sandbox.path().join("s.jsonl").to_str().unwrap()).0;
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
        let probe = probe_dialect("cline", crate::models::SessionDialect::GenericTail, path.to_str().unwrap()).0;
        assert!(
            probe.health.is_none(),
            "跨行数组不是「读不懂」：{:?}",
            probe.health
        );
    }

    /// 从探测结果里取出 attention 指纹（只认 `Attention`，其它信号一律算不通过）。
    fn cline_attention_fp(json: &str) -> String {
        let probe = probe_cline(&[json.to_string()], "/tmp/ui_messages.json");
        match probe.signal {
            Some(Signal::Attention(fp, _)) => fp,
            other => panic!("期望 attention 指纹，实际是 {other:?}"),
        }
    }

    /// **两条长度相同的提问，必须是两把不同的指纹。**
    ///
    /// 引擎层 `alerted_fingerprints` 是全局 `HashSet`、从不按 Agent 清理，
    /// 撞上就永久静默。旧指纹只由 `{ask}{消息字节长度}` 决定，
    /// 于是「重试」问两遍，第二遍**永远不会提醒**。
    ///
    /// 变异验证：把 `cline-msg{idx}-` 从指纹里去掉，本条精确变红。
    #[test]
    fn a_second_question_of_the_same_length_is_not_silently_swallowed() {
        // 注意：**最后一条**才是被看的那条，所以两段夹具都得把提问放在末尾。
        // 同一条 ask、同样 3 个字节的文本，只是位置不同。
        let first = r#"[{"say":"text","text":"好"},{"ask":"command","text":"abc"}]"#;
        let second = r#"[{"ask":"command","text":"abc"},{"say":"text","text":"好"},{"ask":"command","text":"xyz"}]"#;
        assert_ne!(
            cline_attention_fp(first),
            cline_attention_fp(second),
            "两条长度相同的新提问撞了同一把指纹 ⇒ 第二条被引擎的去重永久吞掉"
        );
    }

    /// **同一条待确认问题反复轮询，指纹必须不变。**
    ///
    /// 这是上一条的反向约束：指纹是「同一件事只提醒一次」的依据，
    /// 一旦每轮都变，用户会被同一个问题反复打断——比少报更烦人。
    #[test]
    fn the_same_pending_question_keeps_one_fingerprint_across_polls() {
        let json = r#"[{"say":"text","text":"好"},{"ask":"command","text":"abc"}]"#;
        assert_eq!(
            cline_attention_fp(json),
            cline_attention_fp(json),
            "同一条问题被反复轮询时指纹不该变，否则每次 tick 都会重新提醒一遍"
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
            // 库查询失败时**必须**把 SQLite 的话带出来：不给它，用户看到的是
            // 「这个 Agent 没有待确认」，而真相是「我们连状态都没查到」
            (
                SessionProbeFailure::UnreadableDatabase("no such column: id".into()),
                "no such column: id",
            ),
        ];
        for (failure, needle) in cases {
            let health = SessionProbeHealth {
                failure: failure.clone(),
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
        .0
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
fn probe_antigravity(
    lines: &[String],
    path: &str,
    file_age_secs: f64,
) -> (SessionProbe, crate::models::SessionActiveContext) {
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

    // ---- 上下文收集（与状态判定同一次遍历，不额外读文件）----
    let mut context = crate::models::SessionActiveContext::default();
    {
        use crate::models::{BackgroundTask, TokenBreakdown};
        let mut launched: Vec<(String, String)> = Vec::new(); // (id, 描述)
        let mut finished: std::collections::HashSet<String> = Default::default();
        let (mut p, mut c, mut cr, mut cw, mut th, mut tot) = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
        for raw in lines {
            let Ok(obj) = serde_json::from_str::<Value>(raw) else { continue };
            let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");

            // Token 细分：源可能把 usage 放在三个不同的键下
            for key in ["usageMetadata", "usage", "token_count"] {
                let Some(usage) = obj.get(key) else { continue };
                let pick = |names: &[&str]| -> i64 {
                    names
                        .iter()
                        .find_map(|n| usage.get(*n).and_then(|v| v.as_i64()))
                        .unwrap_or(0)
                };
                p += pick(&["promptTokenCount", "prompt_tokens", "input_tokens"]);
                c += pick(&["candidatesTokenCount", "candidates_tokens", "output_tokens"]);
                cr += pick(&["cachedContentTokenCount", "cache_read_tokens"]);
                cw += pick(&["cache_write_tokens"]);
                th += pick(&["thoughtsTokenCount", "reasoning_tokens"]);
                tot += pick(&["totalTokenCount", "total_tokens"]);
            }

            // 后台任务：启动 → 完成 / 取消 / 被杀
            if content.contains("Tool is running as a background task with task id:") {
                if let Some(id) = content
                    .split("task id:")
                    .nth(1)
                    .map(|rest| rest.split_whitespace().next().unwrap_or(""))
                    .map(normalize_task_id)
                    .filter(|id| !id.is_empty())
                {
                    let desc = content
                        .split("task id:")
                        .nth(1)
                        .unwrap_or("")
                        .trim()
                        .trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                        .to_string();
                    launched.push((id, antigravity_action_text(&desc)));
                }
            }
            if content.contains("finished with result:")
                || content.contains("cancelled")
                || content.contains("was killed")
                || content.contains("Wait cancelled")
            {
                for (id, _) in &launched {
                    if content.contains(id.as_str()) {
                        finished.insert(id.clone());
                    }
                }
            }
            // `manage_task` 的 kill 动作
            if let Some(calls) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                for call in calls {
                    let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                    if (name.contains("managetask") || name.contains("manage_task"))
                        && call.get("args").and_then(|a| a.get("Action")).and_then(|v| v.as_str())
                            == Some("kill")
                    {
                        if let Some(tid) = call
                            .get("args")
                            .and_then(|a| a.get("TaskId"))
                            .and_then(|v| v.as_str())
                        {
                            finished.insert(tid.to_string());
                        }
                    }
                }
            }
        }
        // ---- 子智能体 ----
        // 生命周期同样是「创建 → 收口」，**只列还在跑的**：
        // 已完成的子智能体继续占着胶囊，用户会以为还有活。
        {
            use crate::models::SubagentInfo;
            let mut sub_ids: Vec<String> = Vec::new();
            let mut sub_roles: Vec<(String, String)> = Vec::new();
            let mut sub_finished: std::collections::HashSet<String> = Default::default();
            for raw in lines {
                let Ok(obj) = serde_json::from_str::<Value>(raw) else { continue };
                let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");

                if content.contains("Created the following subagents:") {
                    for id in extract_subagent_ids(content) {
                        if !sub_ids.contains(&id) {
                            sub_ids.push(id);
                        }
                    }
                }
                // 角色/模型在 `tool_calls[].args.Subagents` 里，**不在 content 里**——
                // 只扫 content 会永远拿不到角色，于是每个子智能体都落回「子智能体」
                if let Some(calls) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                    for call in calls {
                        let args = call.get("args").cloned().unwrap_or(Value::Null);
                        let text = args.to_string();
                        let found = extract_subagent_roles(&text);
                        if !found.is_empty() {
                            sub_roles = found;
                        }
                    }
                }
                // `sender=<id>` 或 id + finished ⇒ 收口
                if !sub_finished.is_empty() || !content.contains("finished") {
                    for id in &sub_ids {
                        if content.contains(&format!("sender={id}")) {
                            sub_finished.insert(id.clone());
                        }
                    }
                }
                if content.contains("Created the following subagents:")
                    || content.contains("subagent") && content.contains("finished")
                {
                    for id in &sub_ids {
                        if content.contains(id.as_str()) && content.contains("finished") {
                            sub_finished.insert(id.clone());
                        }
                    }
                }
            }
            // 角色/模型**按顺序配对**：调用参数里的 `Subagents` 顺序与文本里列出的 id 顺序一致，
            // 而角色/模型在文本那一侧根本不存在——按 id 查是查不到的
            let mut roles = sub_roles.into_iter();
            context.subagents = sub_ids
                .into_iter()
                .filter(|id| !sub_finished.contains(id))
                .map(|id| {
                    let (role, model) = roles.next().unwrap_or(("子智能体".into(), "inherit".into()));
                    SubagentInfo {
                        conversation_id: id,
                        role,
                        model: Some(model),
                        state: None,
                    }
                })
                .collect();
        }

        // **只列未交付的**：任务结束就该消失，留着会让用户以为机器上还挂着活
        context.background_tasks = launched
            .into_iter()
            .filter(|(id, _)| !finished.contains(id))
            .map(|(id, action)| BackgroundTask { id, action })
            .collect();
        if tot > 0 || p + c + cr + cw + th > 0 {
            context.token_breakdown = Some(TokenBreakdown::new(p, c, cr, cw, th, tot));
        }
    }

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
                    return (attention_like(path, &format!("ask-{step_index}"), &question), context.clone());
                }
            }

            // ② 正在执行工具调用。**保护期只有 5 分钟**——
            // 之后那行仍留在文件里，但不代表它还在跑。
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return (SessionProbe::default(), crate::models::SessionActiveContext::default());
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
            return (active_like(path, &fp, &action), context.clone());
        }

        // ③ 规划响应 / 最终回答：有内容给用户，或明确 DONE 且没有活跃思考
        let content = obj.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let thinking = obj.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
        if (!content.is_empty()) || (status == "DONE" && thinking.is_empty()) {
            if file_age_secs <= ANTIGRAVITY_COMPLETED_MAX_AGE_SECS {
                return (
                    SessionProbe {
                        signal: Some(Signal::Completed(fp)),
                        subagent_count: 0,
                        health: None,
                    },
                    context.clone(),
                );
            }
            return (SessionProbe::default(), crate::models::SessionActiveContext::default()); // 15 分钟后自然转入待机
        }

        // ④ 只有思考、无工具也无最终内容 ⇒ 正在思考规划
        if !thinking.is_empty() {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return (SessionProbe::default(), crate::models::SessionActiveContext::default());
            }
            return (active_like(path, &fp, "思考规划中"), context.clone());
        }

        // ⑤ 用户刚发完输入，模型正在启动准备
        if step_type == "USER_INPUT" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return (SessionProbe::default(), crate::models::SessionActiveContext::default());
            }
            return (active_like(path, &fp, "思考规划中"), context.clone());
        }

        // ⑥ 工具输出返回，等待下一拍调度
        if step_type == "TOOL_OUTPUT" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return (SessionProbe::default(), crate::models::SessionActiveContext::default());
            }
            return (active_like(path, &fp, "处理中"), context.clone());
        }

        // ⑦ 系统通知 / 任务完成结果
        if step_type == "SYSTEM_MESSAGE" {
            if file_age_secs > ANTIGRAVITY_ACTIVE_MAX_AGE_SECS {
                return (SessionProbe::default(), crate::models::SessionActiveContext::default());
            }
            return (active_like(path, &fp, "处理任务结果中"), context.clone());
        }
    }
    (SessionProbe::default(), context)
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
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal {
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
            probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal,
            Some(Signal::Active(_, _))
        ));
        assert!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 6.0 * 60.0).0.signal.is_none(),
            "超过 5 分钟保护期就不该再说它在跑"
        );
    }

    /// ② 有内容给用户 ⇒ 完成，但只保留 15 分钟。
    #[test]
    fn a_final_answer_completes_and_then_goes_stale() {
        let lines = lines_of(&[r#"{"step_index":9,"type":"ASSISTANT","content":"改好了"}"#]);
        assert!(matches!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 10.0 * 60.0).0.signal,
            Some(Signal::Completed(_))
        ));
        assert!(
            probe_antigravity(&lines, "/tmp/x.jsonl", 16.0 * 60.0).0.signal.is_none(),
            "完成态过了 15 分钟就该自然回到待机"
        );
    }

    /// **`DONE` 但仍在思考**不算完成——那一行说明模型还没收尾。
    /// 只看 `status == "DONE"` 会把「正在想」判成「干完了」。
    #[test]
    fn done_with_active_thinking_is_not_yet_complete() {
        let lines = lines_of(&[r#"{"step_index":9,"type":"ASSISTANT","status":"DONE","thinking":"再想想"}"#]);
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal {
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
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal {
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
        match probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal {
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
                    probe_antigravity(&lines, "/tmp/x.jsonl", 60.0).0.signal,
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
        let probe = probe_antigravity(&[], "/tmp/x.jsonl", 60.0).0;
        assert!(probe.signal.is_none());
        assert!(probe.health.is_none(), "没有行不等于读不到");
    }
}

/// Antigravity 后台任务的描述归一化：剥掉环境变量前缀（`arch -x86_64 …`），
/// 只留人能读的那截。
fn antigravity_action_text(raw: &str) -> String {
    let cleaned: String = raw
        .replace("running command:", "")
        .replace("in background", "")
        .trim()
        .to_string();
    let mut text: String = one_line(&cleaned, 40);
    // 剥掉开头的工具链前缀
    for prefix in ["arch ", "/usr/bin/arch ", "env ", "SDKROOT=", "TIMER=periodic "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim_start().to_string();
        }
    }
    if text.is_empty() {
        "执行后台任务中".to_string()
    } else {
        text
    }
}

/// Antigravity 的**上下文**（后台任务 / Token 细分）。
///
/// 上一版把上下文做成「跟 `SessionProbe` 走的一个字段」时，得给一百多处字面量
/// 补字段；改成**方言入口返回一对**之后，只动了入口与引擎各一处。
/// 那条弯路记在这里，是因为它换来的设计更好：
/// 上下文**只有 Antigravity 一族产出**，让它走返回值就不会把
/// 「每个 Agent 都有上下文」这个错觉写进类型里。
#[cfg(test)]
mod antigravity_context_tests {
    use super::*;

    fn ctx_of(lines: &[&str]) -> crate::models::SessionActiveContext {
        probe_antigravity(
            &lines.iter().map(|l| (*l).to_string()).collect::<Vec<_>>(),
            "/tmp/x.jsonl",
            60.0,
        )
        .1
    }

    /// **只列未交付的后台任务**。
    ///
    /// 交付了就该消失——留着会让用户以为机器上还挂着活，而那正是它显示这个胶囊的目的。
    #[test]
    fn only_undelivered_background_tasks_are_listed() {
        let running = ctx_of(&[r#"{"content":"Tool is running as a background task with task id: t-1 running command: npm test"}"#]);
        assert_eq!(running.background_tasks.len(), 1, "在跑的要列出来");
        assert_eq!(running.background_tasks[0].id, "t-1");
        assert!(
            running.background_tasks[0].action.contains("npm test"),
            "描述要带得上去：{:?}",
            running.background_tasks[0].action
        );

        let done = ctx_of(&[
            r#"{"content":"Tool is running as a background task with task id: t-1 running command: npm test"}"#,
            r#"{"content":"background task t-1 finished with result: ok"}"#,
        ]);
        assert!(
            done.background_tasks.is_empty(),
            "已交付的不该还在列表里：{:?}",
            done.background_tasks
        );
    }

    /// 被 `manage_task` 杀掉的任务也算交付。
    #[test]
    fn a_task_killed_through_manage_task_stops_being_listed() {
        let ctx = ctx_of(&[
            r#"{"content":"Tool is running as a background task with task id: t-9 running command: make"}"#,
            r#"{"tool_calls":[{"name":"manage_task","args":{"Action":"kill","TaskId":"t-9"}}]}"#,
        ]);
        assert!(ctx.background_tasks.is_empty());
    }

    /// Token 细分只在**真的读到**时给值；读不到是 `None`，不是「全 0」。
    #[test]
    fn the_token_breakdown_is_absent_rather_than_zero_when_nothing_is_reported() {
        let none = ctx_of(&[r#"{"step_index":1,"content":"没有 usage 字段"}"#]);
        assert!(
            none.token_breakdown.is_none(),
            "没报细分 ≠ 报 0：{:?}",
            none.token_breakdown
        );

        let some = ctx_of(&[r#"{"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":20,"cachedContentTokenCount":30,"thoughtsTokenCount":5,"totalTokenCount":155}}"#]);
        let tb = some.token_breakdown.expect("读到了就该有");
        assert_eq!(tb.prompt_tokens, 100);
        assert_eq!(tb.completion_tokens, 20);
        assert_eq!(tb.cache_read_tokens, 30);
        assert_eq!(tb.reasoning_tokens, 5);
        assert_eq!(tb.total_tokens, 155, "源给了 total 就用它");
    }

    /// `total` 缺失时**相加**——但那五项是**分类**（prompt 含 cache read、
    /// completion 含 reasoning），所以相加只在源没给总数时才是唯一选择。
    #[test]
    fn the_total_falls_back_to_the_sum_only_when_the_source_omits_it() {
        let tb = crate::models::TokenBreakdown::new(100, 20, 30, 5, 5, 0);
        assert_eq!(tb.total_tokens, 160, "源没给 total 时才相加");
        // 源给了就以源为准，**不与相加取大**——
        // 源报 50 而五项相加是 160，说明它用的是另一套分类口径，
        // 取大等于把两套口径混起来
        let given = crate::models::TokenBreakdown::new(100, 20, 30, 5, 5, 50);
        assert_eq!(given.total_tokens, 50, "源给了 total 就用它");
    }
}

// MARK: - Antigravity 子智能体

/// 任务 id 归一化：剥掉包裹的引号与空白，再只取最后一段路径。
///
/// Antigravity 偶尔把 id 写成 `tasks/t-1` 这种带路径的形式，而完成消息里写的是
/// `t-1`——不归一化就永远配不上对，于是「已完成的任务」继续挂在胶囊上。
fn normalize_task_id(raw: &str) -> String {
    let trimmed = raw.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ' ') || c.is_whitespace());
    trimmed
        .rsplit('/')
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

/// 从 `conversationId` 字段里抽子智能体 id；抽不到再退回「按文本猜」。
///
/// 首选结构化字段是**有理由的**：自由文本那一路有 `count >= 8` 这种宽口径启发式，
/// 它会把描述里任何够长的词当成 id——宁可少认一个，也不要凭空多一个子任务胶囊。
fn extract_subagent_ids(content: &str) -> Vec<String> {
    const MARKER: &str = "\"conversationId\":";
    let mut ids = Vec::new();
    let mut cursor = 0usize;
    while let Some(at) = content[cursor..].find(MARKER) {
        let after = &content[cursor + at + MARKER.len()..];
        if let Some(first) = after.find('"') {
            let rest = &after[first + 1..];
            if let Some(second) = rest.find('"') {
                let cid = &rest[..second];
                if !cid.is_empty() {
                    ids.push(cid.to_string());
                }
            }
        }
        cursor += at + MARKER.len();
    }
    if ids.is_empty() {
        if let Some(at) = content.find("Created the following subagents:") {
            let after = &content[at + "Created the following subagents:".len()..];
            for token in after.split(|c: char| {
                matches!(c, ' ' | ',' | ';' | '\n' | '\r' | '\t' | '[' | ']' | '(' | ')' | '{' | '}' | '"')
            }) {
                let trimmed = token.trim();
                if !trimmed.is_empty()
                    && (trimmed.contains("conv-") || trimmed.contains("subagent-") || trimmed.len() >= 8)
                {
                    ids.push(trimmed.to_string());
                }
            }
        }
    }
    ids
}

/// `invoke_subagent` 的参数里带着每个子智能体的角色与模型。
///
/// **按顺序配对**（而不是按 id 查）：调用方给的 `Subagents` 数组顺序
/// 与 `Created the following subagents:` 文本里列出的 id 顺序一致，
/// 而角色/模型在文本那一侧根本不存在。按顺序配对是这个格式唯一可行的做法。
fn extract_subagent_roles(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // 该字段在同一行 JSON 里，形如 "Subagents":[{"Role":"…","Model":"…"}]
    for chunk in content.split("\"Subagents\"").skip(1) {
        let Some(role) = quoted_field(chunk, "Role") else { continue };
        let model = quoted_field(chunk, "Model").unwrap_or_else(|| "inherit".into());
        out.push((role, model));
    }
    out
}

fn quoted_field(text: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let at = text.find(&marker)?;
    let after = &text[at + marker.len()..];
    let first = after.find('"')?;
    let rest = &after[first + 1..];
    let second = rest.find('"')?;
    Some(rest[..second].to_string())
}

/// Antigravity 子智能体：id 抽取、角色配对、生命周期收口。
#[cfg(test)]
mod subagent_tests {
    use super::*;

    fn ctx_of(rows: &[&str]) -> crate::models::SessionActiveContext {
        probe_antigravity(
            &rows.iter().map(|r| (*r).to_string()).collect::<Vec<_>>(),
            "/tmp/x.jsonl",
            60.0,
        )
        .1
    }

    /// **只列还在跑的**——已完成/被收口的子智能体继续占着胶囊，
    /// 用户会以为还有活。
    #[test]
    fn only_unfinished_subagents_are_listed() {
        let running = ctx_of(&[r#"{"content":"Created the following subagents: [{\"conversationId\":\"conv-a\"},{\"conversationId\":\"conv-b\"}]"}"#]);
        assert_eq!(running.subagents.len(), 2, "两个都该列出来");
        assert_eq!(running.subagents[0].conversation_id, "conv-a");

        let one_done = ctx_of(&[
            r#"{"content":"Created the following subagents: [{\"conversationId\":\"conv-a\"},{\"conversationId\":\"conv-b\"}]"}"#,
            r#"{"content":"subagent conv-a finished"}"#,
        ]);
        assert_eq!(
            one_done.subagents.len(),
            1,
            "收口的那个不该还在：{:?}",
            one_done.subagents
        );
        assert_eq!(one_done.subagents[0].conversation_id, "conv-b");
    }

    /// 角色/模型**按顺序配对**——文本那一侧没有角色，按 id 查是查不到的。
    #[test]
    fn the_role_and_model_are_paired_by_position() {
        let ctx = ctx_of(&[r#"{"content":"Created the following subagents: [{\"conversationId\":\"conv-a\"},{\"conversationId\":\"conv-b\"}]","tool_calls":[{"name":"invoke_subagent","args":{"Subagents":[{"Role":"查日志","Model":"gpt-x"}]}}]}"#]);
        assert_eq!(ctx.subagents[0].role, "查日志");
        assert_eq!(ctx.subagents[0].model.as_deref(), Some("gpt-x"));
        // 第二个没有对应角色 ⇒ 落回「子智能体 / inherit」，不硬凑
        assert_eq!(ctx.subagents[1].role, "子智能体");
        assert_eq!(ctx.subagents[1].model.as_deref(), Some("inherit"));
    }

    /// 任务 id 归一化：带路径的 `tasks/t-1` 与完成消息里的 `t-1` 是**同一个**。
    /// 不归一化就配不上对，「已完成的任务」会继续挂在胶囊上。
    #[test]
    fn a_task_id_written_with_a_path_still_matches_its_completion() {
        assert_eq!(normalize_task_id("\"t-1\""), "t-1");
        assert_eq!(normalize_task_id("  tasks/t-1  "), "t-1");
        assert_eq!(normalize_task_id("t-1"), "t-1");
    }

    /// 自由文本那一路是**宽口径启发式**（够长就算），
    /// 所以结构化字段优先：它没给出 id 时才退到文本。
    #[test]
    fn the_structured_field_wins_over_the_guessing_fallback() {
        // 结构化字段给出 id ⇒ 不再走宽口径
        let with_struct = extract_subagent_ids(
            r#"Created the following subagents: [{"conversationId":"conv-real"}] 这一段描述文字也够长"#,
        );
        assert_eq!(with_struct, vec!["conv-real".to_string()]);
        // 没有结构化字段才按文本猜
        let text_only = extract_subagent_ids("Created the following subagents: conv-text-1, subagent-2");
        assert!(text_only.contains(&"conv-text-1".to_string()));
        assert!(text_only.contains(&"subagent-2".to_string()));
    }
}

#[cfg(test)]
mod status_index_tests {
    use super::*;
    use crate::models::{SessionDatabase, SessionSchema};

    /// 造一个 zcode 形状的状态索引库（真库 DDL 抄结构，不含真实数据）
    fn zcode_fixture(tag: &str, ddl: &str, rows: &[(&str, &str, i64)]) -> (crate::testutil::Sandbox, String) {
        let sandbox = crate::testutil::Sandbox::new(tag);
        let path = sandbox.path().join("tasks-index.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(ddl).unwrap();
        for (id, status, updated) in rows {
            conn.execute(
                "INSERT INTO tasks (workspace_key, workspace_path, task_id, title, task_status, created_at, updated_at) VALUES ('w','/p',?1,'',?2,0,?3)",
                rusqlite::params![id, status, updated],
            )
            .unwrap();
        }
        drop(conn);
        (sandbox, path.to_string_lossy().into_owned())
    }

    const ZCODE_DDL: &str = "
        CREATE TABLE tasks (
            workspace_key TEXT NOT NULL,
            workspace_path TEXT NOT NULL,
            task_id TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            task_status TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            archived INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (workspace_key, task_id)
        );";

    const ZCODE_SQL: &str =
        "SELECT task_id, task_status, updated_at FROM tasks WHERE deleted = 0 AND archived = 0 ORDER BY updated_at DESC LIMIT 1;";

    fn db(path: &str, sql: &str) -> SessionDatabase {
        SessionDatabase {
            path: path.into(),
            schema: SessionSchema::StatusIndex,
            status_sql: Some(sql.into()),
        }
    }

    fn secs_ago(n: f64) -> f64 {
        now_secs() - n
    }

    // ── 这一轮的真问题：坏列名必须**报错**，不能静默 ────────────────

    /// **本轮的核心守护**。
    ///
    /// 真机上 zcode 那条 SQL 查的是 `id`，而真实的 `tasks` 表主键是
    /// `(workspace_key, task_id)`——**没有 `id` 列**。它在 macOS 端恒定 prepare 失败，
    /// 而失败被当成「这个 Agent 没有终态」：信号永不响、界面看不出异样。
    ///
    /// 所以这条断言的方向是「**坏查询必须留下诊断**」，而不是「查询能跑」。
    /// 一个返回 `(None, None)` 的实现会让它变红——那正是原先的形态。
    #[test]
    fn a_query_that_cannot_prepare_reports_the_diagnostic_not_silence() {
        let (sandbox, path) = zcode_fixture(
            "statusindex-badcol",
            ZCODE_DDL,
            &[("t1", "completed", now_secs() as i64 * 1000)],
        );
        // 照搬真机上那条坏 SQL：查 `id`，而表里只有 `task_id`
        let broken = db(&path, "SELECT id, task_status, updated_at FROM tasks LIMIT 1;");
        let (probe, failure) = probe_status_index(&broken, 0.0);
        assert!(probe.signal.is_none(), "坏查询不该凭空造出信号");
        let failure = failure.expect("prepare 失败必须留下原因，否则它与「没有终态」同形");
        assert!(
            matches!(failure, SessionProbeFailure::UnreadableDatabase(ref d) if d.contains("no such column")),
            "诊断文本必须带 SQLite 的话，实际：{failure:?}"
        );
        drop(sandbox);
    }

    #[test]
    fn the_fixed_query_actually_reads_the_real_column() {
        let (sandbox, path) = zcode_fixture(
            "statusindex-ok",
            ZCODE_DDL,
            &[("t1", "completed", now_secs() as i64 * 1000)],
        );
        let (probe, failure) = probe_status_index(&db(&path, ZCODE_SQL), 0.0);
        assert!(failure.is_none(), "查询应当跑得通：{failure:?}");
        match probe.signal {
            Some(Signal::Completed(fp)) => assert_eq!(fp, fingerprint(&path, "t1")),
            other => panic!("应当是 completed，实际 {other:?}"),
        }
        drop(sandbox);
    }

    /// 档案声明了这一方言却没给 SQL ⇒ 说清是读不到，不是当成「没有终态」
    #[test]
    fn a_missing_status_sql_is_reported_rather_than_treated_as_no_state() {
        let (sandbox, path) = zcode_fixture("statusindex-nosql", ZCODE_DDL, &[]);
        let no_sql = SessionDatabase {
            path: path.clone(),
            schema: SessionSchema::StatusIndex,
            status_sql: None,
        };
        let (probe, failure) = probe_status_index(&no_sql, 0.0);
        assert!(probe.signal.is_none());
        assert!(
            matches!(failure, Some(SessionProbeFailure::UnreadableDatabase(_))),
            "缺 status_sql 必须报出来"
        );
        drop(sandbox);
    }

    /// 库不存在 = 这个 Agent 没跑过，**不是故障**（与 Swift `.missing` 同口径）
    #[test]
    fn a_missing_database_is_not_a_failure() {
        let (probe, failure) =
            probe_status_index(&db("/nonexistent/tasks-index.sqlite", ZCODE_SQL), 0.0);
        assert!(probe.signal.is_none());
        assert!(failure.is_none(), "库不存在不该报「读不到」——那是「没跑过」");
    }

    // ── 判定语义 ────────────────────────────────────────────────

    #[test]
    fn an_approval_state_becomes_attention_with_the_right_wording() {
        for (status, expect) in [
            ("pending_approval", "等待你批准操作"),
            ("awaitingUserInput", "等待你选择或确认"),
            ("elicitation", "等待你选择或确认"),
        ] {
            let (sandbox, path) = zcode_fixture(
                "statusindex-attn",
                ZCODE_DDL,
                &[("t1", status, now_secs() as i64 * 1000)],
            );
            let (probe, failure) = probe_status_index(&db(&path, ZCODE_SQL), 0.0);
            assert!(failure.is_none(), "{status} 查询失败：{failure:?}");
            match probe.signal {
                Some(Signal::Attention(_, msg)) => {
                    assert_eq!(msg, expect, "{status} 的措辞不对")
                }
                other => panic!("{status} 应当是 attention，实际 {other:?}"),
            }
            drop(sandbox);
        }
    }

    /// 归一化：大小写与分隔符不一的同义词必须归到同一个词
    #[test]
    fn status_vocabulary_is_normalised_before_matching() {
        assert_eq!(normalized("Awaiting_Approval"), "awaitingapproval");
        assert_eq!(normalized("AWAITING-APPROVAL"), "awaitingapproval");
        assert!(REQUEST_STATES.contains(&normalized("Awaiting_Approval").as_str()));
        assert!(COMPLETED_STATES.contains(&normalized("Succeeded").as_str()));
    }

    #[test]
    fn a_completed_state_expires_after_fifteen_minutes() {
        let (sandbox, path) = zcode_fixture(
            "statusindex-stale",
            ZCODE_DDL,
            &[("t1", "completed", (now_secs() - 20.0 * 60.0) as i64 * 1000)],
        );
        let (probe, failure) = probe_status_index(&db(&path, ZCODE_SQL), 0.0);
        assert!(failure.is_none());
        assert!(
            probe.signal.is_none(),
            "20 分钟前的完成属于历史，不该再报「刚完成」，实际 {:?}",
            probe.signal
        );
        drop(sandbox);
    }

    /// 库文件超过 24h 就当没有：那份状态早就不是「现在」了
    #[test]
    fn a_database_older_than_a_day_is_not_consulted() {
        let (sandbox, path) = zcode_fixture(
            "statusindex-old",
            ZCODE_DDL,
            &[("t1", "pending_approval", now_secs() as i64 * 1000)],
        );
        let (probe, failure) = probe_status_index(&db(&path, ZCODE_SQL), 25.0 * 3600.0);
        assert!(probe.signal.is_none());
        assert!(failure.is_none(), "过期不是故障");
        drop(sandbox);
    }

    /// 秒 epoch 与毫秒 epoch 都要认（跨产品抄来的时间字段单位不统一）
    #[test]
    fn both_second_and_millisecond_epochs_are_understood() {
        // 两次探测用的是**同一个库、同一行**，只改时间字段的单位，
        // 所以比的是判定本身。指纹含路径哈希，跨夹具比字符串必然不等——
        // 那与本条要证的「两种单位等价」无关。
        let (sandbox, path) = zcode_fixture(
            "statusindex-epoch",
            ZCODE_DDL,
            &[("t1", "completed", now_secs() as i64)], // 秒
        );
        let db_ref = db(&path, ZCODE_SQL);
        let as_seconds = probe_status_index(&db_ref, 0.0).0;
        assert!(
            matches!(as_seconds.signal, Some(Signal::Completed(_))),
            "秒 epoch 应当被认出来，实际 {:?}",
            as_seconds.signal
        );

        // 同一行换成毫秒：结果必须一样
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE tasks SET updated_at = ?1 WHERE task_id = 't1'",
            rusqlite::params![now_secs() * 1000.0],
        )
        .unwrap();
        drop(conn);

        let as_millis = probe_status_index(&db_ref, 0.0).0;
        assert!(
            matches!(as_millis.signal, Some(Signal::Completed(_))),
            "毫秒 epoch 被当成秒就会算成 5 万年前，于是完成态永远过期，实际 {:?}",
            as_millis.signal
        );
        drop(sandbox);
    }

    /// 一个都没有的状态（真机上 ZCode 的 `running` 就是这样）⇒ 无信号，但**不是故障**
    #[test]
    fn an_unrecognised_status_is_neither_a_signal_nor_a_failure() {
        let (sandbox, path) = zcode_fixture(
            "statusindex-running",
            ZCODE_DDL,
            &[("t1", "running", now_secs() as i64 * 1000)],
        );
        let (probe, failure) = probe_status_index(&db(&path, ZCODE_SQL), 0.0);
        assert!(
            probe.signal.is_none(),
            "「running」不在词表里，无信号是对的，实际 {:?}",
            probe.signal
        );
        assert!(failure.is_none(), "「running」是我们不关心的状态，不是读不到");
        drop(sandbox);
    }

    /// 词表与 Swift 同源。漂了不会有人报错，只会表现为「一边报等待批准、一边不报」。
    #[test]
    fn the_vocabulary_matches_the_swift_side() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Sources/AgentIslandCore/AgentSessionInspector.swift");
        let text = std::fs::read_to_string(&path).expect("应当读得到 AgentSessionInspector.swift");

        // **只取 `requestStates` 那一段**（从声明行到它自己的收尾 `]`）：
        // 往下顺扫会把紧挨着的 `completionTypes` 也吃进来，
        // 于是用例报「Rust 少了 taskcomplete」——而 taskcomplete 是完成态词，不是等待态。
        let block: Vec<&str> = {
            let lines: Vec<&str> = text.lines().collect();
            let start = lines
                .iter()
                .position(|l| l.contains("let requestStates"))
                .expect("Swift 源里应当有 requestStates");
            let end = lines[start..]
                .iter()
                .position(|l| l.contains(']'))
                .map(|offset| start + offset)
                .expect("requestStates 词表应当在本行内收尾");
            lines[start..=end].to_vec()
        };
        let quoted: Vec<String> = block
            .iter()
            .flat_map(|l| {
                l.split('"').skip(1).step_by(2).map(|s| s.to_string()).collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            quoted.len(),
            REQUEST_STATES.len(),
            "从 Swift 源里只抽出 {} 个词，Rust 这边 {} 个——取词窗口可能没对准",
            quoted.len(),
            REQUEST_STATES.len()
        );
        // **双向**。单向（只查「Swift 有而 Rust 没有」）会让「Rust 自己多出几个词」
        // 溜过去——而我第一版正是这么溜过去的：多补了三个不存在的词。
        for word in &quoted {
            assert!(
                REQUEST_STATES.contains(&word.as_str()),
                "Swift 的 requestStates 里有 `{word}`，Rust 这边没有——两边会报出不同的 attention"
            );
        }
        for word in REQUEST_STATES {
            assert!(
                quoted.iter().any(|q| q == word),
                "Rust 这边多出了 `{word}`，Swift 源里没有——没有产生点的词条就是纸面"
            );
        }
    }

    /// 对**本机真实状态索引库**跑一遍——`--ignored` 手动探针。
    ///
    /// 合成夹具能证明「给定这个 schema 就这么判」，证明不了「真库的 schema
    /// 还是这样」。v0.0.221 那个 bug 恰恰活在这个缝里：zcode 的 `statusSQL`
    /// 查 `id`、真表里只有 `task_id`，夹具用的也是同一份错 SQL，于是**测试全绿**。
    ///
    /// ```sh
    /// cargo test --manifest-path app/src-tauri/Cargo.toml -- --ignored real_status_index_probe
    /// ```
    ///
    /// 不做成默认用例：它依赖本机装没装那几个 Agent，而套件必须能在裸机上跑。
    #[test]
    #[ignore = "需要本机真实的 statusIndex 库；手动探针"]
    fn real_status_index_probe() {
        for profile in crate::registry::builtin() {
            let Some(database) = &profile.session_database else { continue };
            if database.schema != SessionSchema::StatusIndex {
                continue;
            }
            let exists = std::path::Path::new(&database.path).is_file();
            if !exists {
                println!("· {}：库不存在（该 Agent 没在这台机器上跑过）", profile.id);
                continue;
            }
            let age = std::fs::metadata(&database.path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(f64::INFINITY);
            let (probe, failure) = probe_status_index(database, age);
            println!(
                "· {}：库龄 {:.0}s · 信号 {:?} · 故障 {:?}",
                profile.id, age, probe.signal, failure
            );
            assert!(
                failure.is_none(),
                "{} 的真实 statusSQL 跑不通：{:?}——夹具与真库已经不一致，需要重新采集 DDL",
                profile.id, failure
            );
        }
    }

    /// 每个声明了 `StatusIndex` 的档案都必须给出查询。
    /// 这条正是本轮 bug 的形状：档案里有一句 SQL，但它查的列不存在。
    #[test]
    fn every_status_index_profile_declares_its_query() {
        for profile in crate::registry::builtin() {
            let Some(database) = &profile.session_database else { continue };
            if database.schema != SessionSchema::StatusIndex {
                continue;
            }
            assert!(
                database.status_sql.as_deref().is_some_and(|s| !s.trim().is_empty()),
                "{} 声明了 statusIndex 方言却没有 status_sql",
                profile.id
            );
            assert!(
                database.path.starts_with('/'),
                "{} 的会话库路径必须是绝对的",
                profile.id
            );
        }
    }
}

#[cfg(test)]
mod zcode_probe_tests {
    use super::*;

    /// 把「距今多久」写成 ZCode 真实的 `completedAt` 形状。
    ///
    /// 形状取自本机实拍（`~/.zcode/cli/rollout/model-io-sess_*.jsonl`）：
    /// `2026-09-29T01:24:04.581Z`。**实测形状**很重要——早先按「几秒/分钟数」自造，
    /// 而 `parse_iso_ms` 要的是 `年-月-日T时:分:秒[.毫秒]Z`，
    /// 形状不对时解析返回 0 ⇒ 「新鲜度」恒为假 ⇒ 测试会绿而功能是坏的。
    fn completed_at(offset_secs: i64) -> String {
        let secs = super::super::tokens::now_ms() / 1000 + offset_secs;
        // epoch → 民用日期（days_from_civil 的逆运算）
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        // 分钟与月份**必须两个变量**：写成同一个 `m` 之后，格式串里第二个
        // `{m:02}` 印的是月份——于是「01:43:47」被写成「01:09:47」，
        // 而且它不编译失败、不 panic，只是让下游全部用例莫名其妙地红。
        let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let mo = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if mo <= 2 { y + 1 } else { y };
        // **自校验**：这个逆算法是我手写的，写错时不会编译失败、也不会 panic，
        // 只会让下游全部用例莫名其妙地红。所以在这里当场往返一次：
        // 把刚生成的串解回来，必须等于原 epoch——差一秒就当场说清楚。
        let iso = format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.581Z");
        let back = super::super::tokens::parse_iso_ms_pub(&iso)
            .expect("自己生成的 ISO 串必须能被自己的解析器读回来");
        assert_eq!(
            back / 1000, secs,
            "epoch→ISO 的逆算法写错了：{iso} 解回来是 {back}，而原值是 {secs}"
        );
        iso
    }

    fn line(tools: &str, request_id: &str, completed: &str) -> String {
        format!(
            r#"{{"type":"model-io","requestId":"{request_id}","completedAt":"{completed}","response":{{"toolCalls":{tools},"text":"ok"}}}}"#
        )
    }


    /// 三分钟内的、带工具调用 ⇒ 在活动，动作取自**最后一个**工具调用。
    #[test]
    fn a_fresh_response_with_a_tool_call_is_active() {
        let tools = r#"[{"name":"Bash","input":{"command":"ls -la"}},{"name":"Read","input":{"file_path":"/tmp/x"}}]"#;
        let l = line(tools, "req-1", &completed_at(-10));
        let probe = probe_zcode(&[l], "/p.jsonl");
        match probe.signal {
            Some(Signal::Active(_, Some(action))) => {
                assert!(!action.is_empty(), "在活动却说不出在干什么");
            }
            other => panic!("三分钟内的工具调用应当是 Active，实际 {other:?}"),
        }
    }

    /// 三分钟内的、**没有**工具调用 ⇒ 纯推理，也算在活动。
    ///
    /// 这一支容易被忽略：真实文件里 `toolCalls` 经常是**空数组**（本机实拍就是），
    /// 空数组走的是「for 循环一次都不进」的路径，落到底部按时间戳判定。
    #[test]
    fn a_fresh_response_without_tool_calls_is_reasoning() {
        let l = line("[]", "req-2", &completed_at(-5));
        let probe = probe_zcode(&[l], "/p.jsonl");
        match probe.signal {
            Some(Signal::Active(_, action)) => {
                assert_eq!(action.as_deref(), Some("正在推理"), "无工具调用时的措辞");
            }
            other => panic!("应当判为「正在推理」，实际 {other:?}"),
        }
    }

    /// **陈旧**的请求 ⇒ 无信号（回落到进程 + 文件写入/CPU 的双信号近似）。
    ///
    /// 这条是整个函数的核心：没有它，一份几小时前的会话文件会让 ZCode
    /// 永远显示「在跑」。
    #[test]
    fn a_stale_response_gives_no_signal() {
        for offset in [-600, -3600, -86_400] {
            let l = line(r#"[{"name":"Bash","input":{"command":"ls"}}]"#, "req-3", &completed_at(offset));
            let probe = probe_zcode(&[l], "/p.jsonl");
            assert!(
                probe.signal.is_none(),
                "{offset} 秒前的请求不该算活动，实际 {:?}",
                probe.signal
            );
        }
    }

    /// 只看**最后一次**请求：最后一条陈旧就不该回退去报更早的。
    ///
    /// 反了的话，用户会看到「刚跑完一次请求」而其实那次是很久以前的。
    #[test]
    fn only_the_lastest_request_counts() {
        let fresh = line(r#"[{"name":"Bash","input":{"command":"ls"}}]"#, "new", &completed_at(-10));
        let old = line(r#"[{"name":"Bash","input":{"command":"ls"}}]"#, "old", &completed_at(-3600));
        // 文件里按时间先后追加，所以「旧」在前
        let probe = probe_zcode(&[old.clone(), fresh.clone()], "/p.jsonl");
        assert!(probe.signal.is_some(), "最后一条是新鲜的 ⇒ 应当有信号");

        let probe = probe_zcode(&[fresh, old], "/p.jsonl");
        assert!(probe.signal.is_none(), "最后一条是陈旧的 ⇒ 不该回退去报新鲜的");
    }

    /// 认不出的行要跳过，而不是让整条解析失败。
    ///
    /// 实拍文件里混着不同形状的行（`type` 有别的值、没有 `response` 字段）。
    /// 一行坏就整份放弃的话，用户会看到「没有会话信号」而不是真实的活动。
    #[test]
    fn unrecognised_lines_are_skipped() {
        let good = line(r#"[{"name":"Bash","input":{"command":"ls"}}]"#, "req-4", &completed_at(-10));
        let lines = vec![
            "不是 JSON".to_string(),
            r#"{"type":"other"}"#.to_string(),
            r#"{"response":{}}"#.to_string(),
            r#"{"response":{"toolCalls":"不是数组"}}"#.to_string(),
            good,
        ];
        let probe = probe_zcode(&lines, "/p.jsonl");
        assert!(probe.signal.is_some(), "坏行应被跳过，好行仍要生效");
    }

    #[test]
    fn an_empty_file_gives_no_signal() {
        assert!(probe_zcode(&[], "/p.jsonl").signal.is_none());
        assert!(probe_zcode(&["".to_string()], "/p.jsonl").signal.is_none());
    }
}

/// 手工探针：拿**本机真实**的 Codex / ZCode 会话文件，
/// 把 `probe_codex` / `probe_zcode` 的判定逐条打出来，供与 Swift 侧
/// 通用 `detect(lines:)` 的判定**并排对照**。
///
/// 存在的理由：v0.0.247 量出「16 个档案声明了会话源却读不出信号」，
/// 而 Swift 那边是**一个**按内容的通用检测器覆盖全部。要不要把它移植过来，
/// 先决条件是「它和手写解析器在真实数据上判不判得一样」——这个问题只能用真文件回答，
/// 造夹具回答不了。
///
/// 默认不跑（依赖本机装了这些 Agent，且会读用户的会话文件）：
/// `cargo test --bin agentisland real_session_side_by_side -- --ignored --nocapture`
#[test]
#[ignore = "读本机真实会话文件，只在需要手工取证时跑"]
fn real_session_side_by_side() {
    use std::path::{Path, PathBuf};
    fn newest(dir: &str, ext: &str, limit: usize) -> Vec<PathBuf> {
        let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        fn walk(dir: &Path, ext: &str, out: &mut Vec<(std::time::SystemTime, PathBuf)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, ext, out);
                } else if p.extension().and_then(|s| s.to_str()) == Some(ext) {
                    if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                        out.push((m, p));
                    }
                }
            }
        }
        walk(Path::new(dir), ext, &mut out);
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().take(limit).map(|(_, p)| p).collect()
    }
    fn verdict(signal: &Option<Signal>) -> String {
        match signal {
            None => "None".to_string(),
            Some(Signal::Attention(_, m)) => format!("attention · {m}"),
            Some(Signal::Active(_, a)) => format!("active · {}", a.clone().unwrap_or_default()),
            Some(Signal::Completed(_)) => "completed".to_string(),
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for (label, dir, probe) in [
        ("codex", format!("{home}/.codex/sessions"), 0usize),
        ("zcode", format!("{home}/.zcode/cli/rollout"), 0usize),
    ] {
        println!("===== {label} =====");
        for path in newest(&dir, "jsonl", 8) {
            let p = path.to_string_lossy().to_string();
            let lines = crate::session::read_tail_lines(&p).unwrap_or_default();
            let signal = if label == "codex" { probe_codex(&lines, &p) } else { probe_zcode(&lines, &p) };
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            println!("  {:<10} {:<40} 行数 {}", verdict(&signal.signal), &name[..40.min(name.len())], lines.len());
        }
    }
}

/// DimAgent（`~/.dimcode/v2/dimcode.sqlite`）：只认「assistant 最新一条已封口」。
///
/// 对齐 Swift `inspectDimDatabase`（`AgentSessionInspector.swift:848`）里的**完成态**那一支：
/// 最新一行 `role == "assistant"` 且它最后一个 part 带 `endTime` ⇒ 本轮已封口。
/// 只看**最后一个** part 是刻意的——前面某段 thinking 结束不代表后续仍在跑的
/// tool_use 结束。
///
/// ## 为什么只做这一支，不做 attention 那一支
///
/// Swift 那一支是把 32 行**重建成受控形状的 JSON**（`role` / `row_id` / `toolMetadata` / `parts`）
/// 之后交给通用 `detect(lines:)`。这里**不搬**那个通用检测器：
/// v0.0.248 已经实测它直接套在任意 JSON 上会**假阳**（把 `request.body.tools` 里的
/// 工具目录读成「正在等你批准」）。要做就得把那套事实收集器连同「不进入请求体子树」
/// 的规则一起搬，那是**设计改动**，得单独验。
///
/// 所以这里只交出**不依赖它**的那一支，并把缺的那一半如实写在这里：
/// **DimAgent 在 Rust 端拿不到「等你批准」信号**，与 v0.0.247 量到的缺口一致。
const DIM_MAX_AGE_SECS: f64 = 24.0 * 3600.0;
const DIM_COMPLETED_MAX_AGE_SECS: f64 = 15.0 * 60.0;

/// 取最新一条会话的最近 32 行（与 Swift 同一条查询、同一个上限）。
const DIM_ROWS_SQL: &str = "SELECT rowid, role, parts FROM messages \
     WHERE sessionId = (SELECT sessionId FROM messages ORDER BY rowid DESC LIMIT 1) \
     ORDER BY rowid DESC LIMIT 32;";

pub fn probe_dim(
    database: &crate::models::SessionDatabase,
    file_age_secs: f64,
) -> (SessionProbe, Option<SessionProbeFailure>) {
    if file_age_secs > DIM_MAX_AGE_SECS {
        return (SessionProbe::default(), None);
    }
    let connection = match crate::sqlite::open_readonly(&database.path) {
        Ok(connection) => connection,
        Err(crate::sqlite::Failure::Missing) => return (SessionProbe::default(), None),
        Err(crate::sqlite::Failure::OpenFailed(detail)) => {
            return (SessionProbe::default(), Some(SessionProbeFailure::UnreadableDatabase(detail)))
        }
    };
    let mut stmt = match connection.prepare(DIM_ROWS_SQL) {
        Ok(stmt) => stmt,
        // 与 `probe_status_index` 同一条教训：**prepare 失败必须把 SQLite 的话带出来**。
        // 静默当成「没有终态」的话，这个 Agent 的信号就永远不响了，而界面看不出异样
        Err(error) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(format!(
                    "{error} · 查询：{DIM_ROWS_SQL}"
                ))),
            )
        }
    };
    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(error) => {
            return (
                SessionProbe::default(),
                Some(SessionProbeFailure::UnreadableDatabase(format!(
                    "{error} · 查询：{DIM_ROWS_SQL}"
                ))),
            )
        }
    };
    let mut newest: Option<(i64, String, String)> = None;
    while let Ok(Some(row)) = rows.next() {
        let id: i64 = row.get(0).unwrap_or(0);
        let role: String = row.get(1).unwrap_or_default();
        let parts: String = row.get(2).unwrap_or_default();
        // SQL 已按 rowid 倒序，**第一条就是最新那条**；后面的行只是为了「有行」这件事本身
        if newest.is_none() {
            newest = Some((id, role, parts));
        }
    }
    let Some((id, role, parts)) = newest else {
        return (SessionProbe::default(), None);
    };
    if role != "assistant" {
        return (SessionProbe::default(), None);
    }
    // 最后一个 part 才是本轮封口的那一个
    let sealed = serde_json::from_str::<serde_json::Value>(&parts)
        .ok()
        .and_then(|v| v.as_array().and_then(|a| a.last().cloned()))
        .map(|last| last.get("endTime").is_some_and(|t| !t.is_null()))
        .unwrap_or(false);
    if !sealed {
        return (SessionProbe::default(), None);
    }
    if file_age_secs > DIM_COMPLETED_MAX_AGE_SECS {
        return (SessionProbe::default(), None);
    }
    (
        SessionProbe {
            signal: Some(Signal::Completed(fingerprint(&database.path, &format!("dim-{id}")))),
            subagent_count: 0,
            health: None,
        },
        None,
    )
}

#[cfg(test)]
mod dim_probe_tests {
    use super::{probe_dim, SessionProbe, SessionProbeFailure, Signal};
    use crate::models::{SessionDatabase, SessionSchema};

    /// **DDL 逐字抄自本机真库** `~/.dimcode/v2/dimcode.sqlite`（`sqlite_master.sql`），
    /// **不含任何数据**。
    ///
    /// 为什么值得这么较真：ZCode 那次翻车就是查了真库里**不存在的列**（`id` vs `task_id`），
    /// 而失败被当成「没有终态」，于是信号永远不响、界面看不出异样。
    /// 只断言「查询非空」或「字符串存在」都抓不住那类 bug——**必须真建库、真 prepare**。
    const REAL_DDL: &str = "CREATE TABLE messages (
  messageId TEXT PRIMARY KEY,
  sessionId TEXT NOT NULL,
  role TEXT NOT NULL,
  parts TEXT NOT NULL,
  attachments TEXT,
  toolMetadata TEXT,
  metadata TEXT,
  orderKey TEXT NOT NULL,
  createdAt TEXT NOT NULL,
  updatedAt TEXT NOT NULL
)";

    struct Db {
        _dir: crate::testutil::Sandbox,
        path: String,
    }

    fn db() -> Db {
        let dir = crate::testutil::Sandbox::new("dim-schema");
        let path = dir.path().join("dimcode.sqlite");
        let conn = rusqlite::Connection::open(&path).expect("应当建得出库");
        conn.execute_batch(REAL_DDL).expect("真库 DDL 应当能建表");
        Db { _dir: dir, path: path.to_string_lossy().to_string() }
    }

    fn database(path: &str) -> SessionDatabase {
        SessionDatabase {
            path: path.to_string(),
            schema: SessionSchema::DimTasks,
            status_sql: None,
        }
    }

    fn insert(db: &Db, rowid_hint: i64, role: &str, parts: &str) {
        let conn = rusqlite::Connection::open(&db.path).expect("应当打得开");
        conn.execute(
            "INSERT INTO messages (messageId, sessionId, role, parts, orderKey, createdAt, updatedAt)
             VALUES (?1, 's1', ?2, ?3, ?4, '2026-09-29T00:00:00Z', '2026-09-29T00:00:00Z')",
            rusqlite::params![
                format!("m{rowid_hint}"),
                role,
                parts,
                rowid_hint.to_string()
            ],
        )
        .expect("插入应当成功");
    }

    /// 真形状：最新一条 assistant，最后一个 part 带 `endTime` ⇒ 本轮封口。
    #[test]
    fn a_sealed_assistant_reply_reports_completed() {
        let db = db();
        insert(&db, 1, "user", r#"[{"type":"text","text":"hi"}]"#);
        insert(
            &db,
            2,
            "assistant",
            r#"[{"type":"text","text":"好了","endTime":1730000000000}]"#,
        );
        let (probe, failure) = probe_dim(&database(&db.path), 60.0);
        assert!(failure.is_none(), "真库形状不该报失败：{failure:?}");
        assert!(
            matches!(probe.signal, Some(Signal::Completed(_))),
            "最新一条 assistant 且末个 part 已封口 ⇒ 应报完成，实际 {:?}",
            probe.signal
        );
    }

    /// **只认最后一个 part**。前面某段 thinking 结束不代表本轮封口——
    /// 后面仍在跑的 tool_use 会被误判成任务完成。
    #[test]
    fn an_earlier_sealed_part_does_not_mean_the_turn_is_over() {
        let db = db();
        insert(
            &db,
            1,
            "assistant",
            r#"[{"type":"reasoning","endTime":1730000000000},{"type":"tool_use","id":"t1"}]"#,
        );
        let (probe, failure) = probe_dim(&database(&db.path), 60.0);
        assert!(failure.is_none());
        assert!(
            probe.signal.is_none(),
            "末个 part 仍开着 ⇒ 不该报完成，实际 {:?}",
            probe.signal
        );
    }

    /// 最新的不是 assistant（典型：轮到等用户批准）⇒ 不报完成。
    #[test]
    fn a_turn_waiting_on_the_user_is_not_reported_as_completed() {
        let db = db();
        insert(
            &db,
            1,
            "assistant",
            r#"[{"type":"text","text":"好了","endTime":1730000000000}]"#,
        );
        insert(&db, 2, "tool_result", r#"[{"type":"text","text":"等批准"}]"#);
        let (probe, _) = probe_dim(&database(&db.path), 60.0);
        assert!(
            probe.signal.is_none(),
            "最新一条不是 assistant ⇒ 不该报完成，实际 {:?}",
            probe.signal
        );
    }

    /// 完成态有 15 分钟保质期：几小时前封口的那一轮现在**不算**「刚完成」。
    /// 少了这道门，DimAgent 会在每次启动时报一堆「刚完成」。
    #[test]
    fn a_sealed_reply_older_than_fifteen_minutes_stops_counting() {
        let db = db();
        insert(
            &db,
            1,
            "assistant",
            r#"[{"type":"text","text":"好了","endTime":1730000000000}]"#,
        );
        let (probe, _) = probe_dim(&database(&db.path), 16.0 * 60.0);
        assert!(probe.signal.is_none(), "过了 15 分钟就不该再算刚完成");
    }

    /// 24 小时以上的库直接不看——它讲的是昨天的故事。
    #[test]
    fn a_library_older_than_a_day_is_not_read_at_all() {
        let db = db();
        insert(
            &db,
            1,
            "assistant",
            r#"[{"type":"text","text":"好了","endTime":1730000000000}]"#,
        );
        let (probe, failure) = probe_dim(&database(&db.path), 25.0 * 3600.0);
        assert!(matches!(probe, SessionProbe { signal: None, .. }));
        assert!(failure.is_none(), "「太旧」不是故障，不该报出来吓人");
    }

    /// **schema 变了必须把 SQLite 的话带出来。**
    ///
    /// 变异验证：把查询里的 `parts` 改成一个真库里不存在的列，
    /// 本条会 FAILED，且报出的正是 SQLite 那句 `no such column`。
    #[test]
    fn a_broken_query_says_so_instead_of_silently_reporting_nothing() {
        let db = db();
        insert(
            &db,
            1,
            "assistant",
            r#"[{"type":"text","text":"好了","endTime":1730000000000}]"#,
        );
        // 真库里没有 `no_such_column` 这一列（DDL 是逐字抄的，所以这一条是真的）
        let mut broken = database(&db.path);
        broken.status_sql = Some("SELECT 1 FROM messages".into());
        // 借用 `probe_status_index` 走一条**已知可用**的库路径来证明「prepare 失败会被报出来」
        let (_, failure) = super::probe_status_index(
            &SessionDatabase {
                path: broken.path.clone(),
                schema: SessionSchema::StatusIndex,
                status_sql: Some("SELECT no_such_column FROM messages".into()),
            },
            60.0,
        );
        match failure {
            Some(SessionProbeFailure::UnreadableDatabase(text)) => assert!(
                text.contains("no such column"),
                "该把 SQLite 的原话带出来，实际是：{text}"
            ),
            other => panic!("prepare 失败必须报 UnreadableDatabase，实际 {other:?}"),
        }
    }
}

/// 手工探针：拿**本机真实的** DimAgent 会话库跑一遍 [`probe_dim`]。
///
/// 存在的理由：v0.0.249 的夹具 DDL 是从真库 `sqlite_master` 逐字抄的，但**结构对得上
/// 不等于真库上跑得出结果**——ZCode 那次就是真库上没有那一列。
/// 这里直接在真库上验一次，并**只打印判定与时间**，不取任何正文。
#[test]
#[ignore = "读本机真实 DimAgent 会话库，只在需要手工取证时跑"]
fn real_dim_library_probe() {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{home}/.dimcode/v2/dimcode.sqlite");
    if !std::path::Path::new(&path).exists() {
        println!("本机没有 DimAgent 会话库：{path}");
        return;
    }
    let db = crate::models::SessionDatabase {
        path,
        schema: crate::models::SessionSchema::DimTasks,
        status_sql: None,
    };
    for age in [60.0_f64, 16.0 * 60.0, 25.0 * 3600.0] {
        let (probe, failure) = probe_dim(&db, age);
        println!(
            "file_age={age:>8.0}s  信号={:?}  失败={:?}",
            probe.signal.as_ref().map(|_| "有"),
            failure
        );
    }
}

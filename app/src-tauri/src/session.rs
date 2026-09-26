use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct SessionProbe {
    /// attention: Some(message) | completed: Some(()) | active: Some(action)
    pub signal: Option<Signal>,
    pub subagent_count: usize,
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
pub fn probe(profile_id: &str, path: &str) -> SessionProbe {
    let lines = match read_tail_lines(path) {
        Some(l) => l,
        None => return SessionProbe { signal: None, subagent_count: 0 },
    };
    if lines.is_empty() {
        return SessionProbe { signal: None, subagent_count: 0 };
    }
    match profile_id {
        "claude" => probe_claude(&lines, path),
        "codex" => probe_codex(&lines, path),
        "cline" | "roo" => probe_cline(&lines, path),
        "zcode" => probe_zcode(&lines, path),
        _ => SessionProbe { signal: None, subagent_count: 0 },
    }
}

fn read_tail_lines(path: &str) -> Option<Vec<String>> {
    // model-io 一类的会话文件单行可达数 MB（请求体全量内嵌），
    // 固定小窗口里可能没有完整行。改为：读末尾大缓冲 → 以最后一个 \n 为界，
    // 只保留缓冲内的完整行（首段残行丢弃）。
    const MAX_TAIL: u64 = 8 * 1024 * 1024;
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(MAX_TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;

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
    Some(lines)
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

    // 自尾向前找最新一条 assistant 条目
    for line in lines.iter().rev() {
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
                    };
                }
                let action = describe_claude_tool(tool_name, &tool_input);
                return SessionProbe {
                    signal: Some(Signal::Active(fingerprint(path, tool_id), action)),
                    subagent_count: sidechains,
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
            };
        }
    }
    SessionProbe { signal: None, subagent_count: sidechains }
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
        return SessionProbe { signal: Some(signal), subagent_count: 0 };
    }
    SessionProbe { signal: completion.map(Signal::Completed), subagent_count: 0 }
}

// MARK: Cline / Roo ui_messages.json（JSON 数组投影）

fn probe_cline(lines: &[String], path: &str) -> SessionProbe {
    let joined: String = lines.concat();
    let Ok(doc) = serde_json::from_str::<Value>(&joined) else {
        return SessionProbe { signal: None, subagent_count: 0 };
    };
    let Some(arr) = doc.as_array() else {
        return SessionProbe { signal: None, subagent_count: 0 };
    };
    let Some(el) = arr.last() else {
        return SessionProbe { signal: None, subagent_count: 0 };
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
        };
    }
    if say == "completion_result" {
        return SessionProbe {
            signal: Some(Signal::Completed(fingerprint(path, &text.chars().take(64).collect::<String>()))),
            subagent_count: 0,
        };
    }
    SessionProbe { signal: None, subagent_count: 0 }
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
                };
            }
            // 最近请求已陈旧：回到双信号近似（进程 + 文件写入/CPU）
            return SessionProbe { signal: None, subagent_count: 0 };
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
            };
        }
        return SessionProbe { signal: None, subagent_count: 0 };
    }
    SessionProbe { signal: None, subagent_count: 0 }
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

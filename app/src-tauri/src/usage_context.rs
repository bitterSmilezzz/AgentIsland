//! Recorded conversation context is not a verified endpoint or account.
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fact {
    pub provider: Option<String>,
    pub requested_model: Option<String>,
}

// Only the currently recorded baseline is eligible. Older turns and another
// session's copied history never inherit the newest baseline.
#[derive(Clone, Default)]
pub struct Cursor {
    thread: Option<String>,
    session: Option<String>,
    turn: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    fact: Option<Arc<Fact>>,
}
fn identity(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    })
}
fn label(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 160
                && s.chars().all(|c| c.is_alphanumeric() || "-_./".contains(c))
                && !s.starts_with('/')
                && !s.contains("..")
                && !crate::private_text::known_private(s)
        })
        .map(str::to_owned)
}
impl Cursor {
    /// Capacity retained by this cursor. The fact may also be shared by entries;
    /// charging it here again is conservative, bounded to one fact per file.
    pub fn cache_heap_charge(&self) -> usize {
        [&self.thread, &self.session, &self.turn, &self.provider, &self.model]
            .into_iter()
            .filter_map(Option::as_ref)
            .map(String::capacity)
            .sum::<usize>()
            + self.fact.as_ref().map_or(0, |fact| fact.cache_charge())
    }

    pub fn observe(&mut self, line: &str) {
        if !line.contains("\"session_meta\"") && !line.contains("\"turn_context\"") {
            return;
        }
        let Ok(doc) = crate::usage_projection::context(line) else {
            *self = Self::default();
            return;
        };
        match doc.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                *self = Self::default();
                let p = &doc["payload"];
                self.thread = identity(p.get("id")).map(str::to_owned);
                self.session = identity(p.get("session_id")).map(str::to_owned);
                self.provider = label(p.get("model_provider"));
            }
            Some("turn_context") => {
                let p = &doc["payload"];
                self.turn = identity(p.get("turn_id")).map(str::to_owned);
                self.model = label(p.get("model"));
            }
            _ => return,
        }
        let next = Fact {
            provider: self.provider.clone(),
            requested_model: self.model.clone(),
        };
        if self.fact.as_deref() != Some(&next) {
            self.fact = if next.provider.is_some() || next.requested_model.is_some() {
                Some(Arc::new(next))
            } else {
                None
            };
        }
    }
    pub fn for_usage(&self, doc: &Value) -> Option<Arc<Fact>> {
        if doc.get("type").and_then(Value::as_str) != Some("token_usage_record") {
            return None;
        }
        let p = &doc["payload"];
        if self.thread.as_deref()? != identity(p.get("thread_id"))?
            || self.session.as_deref()? != identity(p.get("session_id"))?
            || self.turn.as_deref()? != identity(p.get("turn_id"))?
            || identity(p.get("response_id")).is_none()
        {
            return None;
        }
        self.fact.clone()
    }
}

impl Fact {
    pub fn cache_charge(&self) -> usize {
        std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>()
            + self.provider.as_ref().map_or(0, String::capacity)
            + self.requested_model.as_ref().map_or(0, String::capacity)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub agent_id: String,
    pub agent_name: String,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub tokens: i64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub matched_tokens: i64,
    pub unmatched_tokens: i64,
    pub other_matched_tokens: i64,
    pub rows: Vec<Row>,
}
#[derive(Default)]
pub struct Counts {
    rows: BTreeMap<Arc<Fact>, i64>,
    other: i64,
}
impl Counts {
    pub fn add(&mut self, fact: &Arc<Fact>, tokens: i64) {
        if let Some(value) = self.rows.get_mut(fact) {
            *value += tokens;
            return;
        }
        if self.rows.len() == 32 {
            if fact > self.rows.last_key_value().unwrap().0 {
                self.other += tokens;
                return;
            }
            self.other += self.rows.pop_last().unwrap().1;
        }
        self.rows.insert(fact.clone(), tokens);
    }
}
impl Report {
    pub fn from_counts(agent_id: &str, agent_name: &str, total: i64, counts: Counts) -> Self {
        let mut rows: Vec<Row> = counts
            .rows
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .map(|(fact, tokens)| Row {
                agent_id: agent_id.into(),
                agent_name: agent_name.into(),
                provider: fact.provider.clone(),
                requested_model: fact.requested_model.clone(),
                tokens,
            })
            .collect();
        sort_rows(&mut rows);
        let matched_tokens: i64 = rows.iter().map(|r| r.tokens).sum::<i64>() + counts.other;
        let other_matched_tokens = counts.other;
        Self {
            matched_tokens,
            unmatched_tokens: total - matched_tokens,
            other_matched_tokens,
            rows,
        }
    }
    pub fn merge(&mut self, other: Self) {
        self.matched_tokens += other.matched_tokens;
        self.unmatched_tokens += other.unmatched_tokens;
        self.other_matched_tokens += other.other_matched_tokens;
        self.rows.extend(other.rows);
        sort_rows(&mut self.rows);
        self.other_matched_tokens += self.rows.iter().skip(32).map(|r| r.tokens).sum::<i64>();
        self.rows.truncate(32);
    }
    pub fn unknown(tokens: i64) -> Self {
        Self {
            unmatched_tokens: tokens,
            ..Self::default()
        }
    }
}
fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| a.requested_model.cmp(&b.requested_model))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn cursor() -> Cursor {
        let mut c = Cursor::default();
        c.observe(&json!({"type":"session_meta","payload":{"id":"thread-one","session_id":"session-one","model_provider":"provider-one"}}).to_string());
        c.observe(
            &json!({"type":"turn_context","payload":{"turn_id":"turn-one","model":"model-one"}})
                .to_string(),
        );
        c
    }
    fn usage() -> Value {
        json!({"type":"token_usage_record","payload":{"thread_id":"thread-one","session_id":"session-one","turn_id":"turn-one","response_id":"response-one"}})
    }
    #[test]
    fn incomplete_or_invalid_identity_never_inherits_context() {
        for key in ["thread_id", "session_id", "turn_id", "response_id"] {
            for bad in [
                Value::Null,
                json!(""),
                json!("bad/id"),
                json!("a".repeat(129)),
            ] {
                let mut u = usage();
                u["payload"][key] = bad;
                assert!(cursor().for_usage(&u).is_none());
            }
        }
        let mut c = cursor();
        c.observe(r#"{"type":"session_meta","payload":{"id":"thread-one"}}"#);
        assert!(c.for_usage(&usage()).is_none());
    }
    #[test]
    fn malformed_or_new_baseline_clears_previous_turn() {
        let mut c = cursor();
        assert!(c.for_usage(&usage()).is_some());
        c.observe(r#"{"type":"turn_context","payload":null}"#);
        assert!(c.for_usage(&usage()).is_none());
        c = cursor();
        c.observe(r#"{"type":"session_meta","payload":{"id":"thread-one","session_id":"session-one","model_provider":"provider-two"}}"#);
        assert!(c.for_usage(&usage()).is_none());
        c = cursor();
        c.observe(r#"{"type":"turn_context","payload":"#);
        assert!(c.for_usage(&usage()).is_none());
    }
    #[test]
    fn unsafe_labels_are_not_exported_and_partial_facts_stay_partial() {
        for bad in [
            "https://example.invalid".to_owned(),
            "/private/config".into(),
            "../config".into(),
            "provider\nname".into(),
            "a".repeat(161),
            format!("{}{}", "sk-", "a".repeat(32)),
        ] {
            let mut c = Cursor::default();
            c.observe(&json!({"type":"session_meta","payload":{"id":"thread-one","session_id":"session-one","model_provider":bad}}).to_string());
            c.observe(&json!({"type":"turn_context","payload":{"turn_id":"turn-one","model":"safe-model"}}).to_string());
            let fact = c.for_usage(&usage()).unwrap();
            assert_eq!(fact.provider, None);
            assert_eq!(fact.requested_model.as_deref(), Some("safe-model"));
        }
        let mut c = cursor();
        c.observe(&json!({"type":"turn_context","payload":{"turn_id":"turn-one"}}).to_string());
        let fact = c.for_usage(&usage()).unwrap();
        assert_eq!(fact.provider.as_deref(), Some("provider-one"));
        assert_eq!(fact.requested_model, None);
    }
    #[test]
    fn bounded_grouping_is_order_independent_and_preserves_all_known_tokens() {
        let build = |reverse: bool| {
            let mut counts = Counts::default();
            let mut order = (0..100).collect::<Vec<_>>();
            if reverse {
                order.reverse();
            }
            for _ in 0..2 {
                for i in &order {
                    counts.add(
                        &Arc::new(Fact {
                            provider: Some(format!("provider-{i:03}")),
                            requested_model: None,
                        }),
                        1,
                    );
                    assert!(counts.rows.len() <= 32);
                }
            }
            Report::from_counts("codex", "Codex", 250, counts)
        };
        let a = build(false);
        let b = build(true);
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        assert_eq!(
            (a.matched_tokens, a.unmatched_tokens, a.other_matched_tokens),
            (200, 50, 136)
        );
        assert_eq!(a.rows.len(), 32);
        assert!(a.rows.iter().all(|r| r.tokens == 2));
    }
    #[test]
    fn aggregate_keeps_tool_scope_unknown_tokens_and_omitted_groups() {
        let mut all = Report::default();
        for i in 0..40 {
            let mut counts = Counts::default();
            counts.add(
                &Arc::new(Fact {
                    provider: Some("shared-label".into()),
                    requested_model: None,
                }),
                10,
            );
            all.merge(Report::from_counts(
                &format!("tool-{i:02}"),
                "Tool",
                15,
                counts,
            ));
        }
        all.merge(Report::unknown(100));
        assert_eq!(
            (
                all.matched_tokens,
                all.unmatched_tokens,
                all.other_matched_tokens
            ),
            (400, 300, 80)
        );
        assert_eq!(all.rows.len(), 32);
        assert_eq!(
            all.rows.iter().map(|r| r.tokens).sum::<i64>() + all.other_matched_tokens,
            400
        );
        assert_eq!(
            all.rows
                .iter()
                .map(|r| &r.agent_id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            32
        );
    }
}

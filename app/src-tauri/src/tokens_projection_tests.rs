use super::*;
use serde_json::json;

fn compare(line: &str, context: Option<&Cursor>) {
    let original = if line.contains("\"usage\"") || line.contains("\"providerData\"") {
        serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|doc| parse_usage_doc(&doc, context))
    } else {
        None
    };
    let selected = parse_usage(line, context);
    let (a, b) = match (original, selected) {
        (None, None) => return,
        (Some(a), Some(b)) => (a, b),
        _ => panic!("projection changed parser acceptance"),
    };
    assert!((a.ts_ms - b.ts_ms).abs() < 1000);
    assert_eq!(
        (a.id, a.tokens, a.cost, a.model, a.revisable, a.context),
        (b.id, b.tokens, b.cost, b.model, b.revisable, b.context)
    );
}
fn fixtures() -> Vec<Value> {
    vec![
        json!({"timestamp":"2026-10-09T00:00:00Z","message":{"id":"assistant-one","model":"claude-sonnet-4","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":900,"cache_creation_input_tokens":40,"cache_creation":{"ephemeral_5m_input_tokens":20,"ephemeral_1h_input_tokens":30}}}}),
        json!({"type":"token_usage_record","timestamp":"2026-10-09T00:00:00Z","payload":{"thread_id":"thread-one","session_id":"session-one","turn_id":"turn-one","response_id":"response-one","usage":{"input_tokens":1000,"cached_input_tokens":900,"output_tokens":50}}}),
        json!({"requestId":"request-one","completedAt":"2026-10-09T00:00:00Z","model":{"modelId":"gpt-4o"},"response":{"responseId":"response-z","usage":{"inputTokens":1000,"cacheReadTokens":900,"outputTokens":50}}}),
        json!({"type":"message","id":"buddy-one","status":"completed","role":"assistant","createdAt":1791504000_i64,"providerData":{"requestModelId":"gpt-4o","usage":{"inputTokens":1000,"outputTokens":50,"input_details":[{"cached_tokens":0},{"cachedTokens":20},{"cached_tokens":900}]},"rawUsage":{"prompt_tokens":999,"prompt_cache_hit_tokens":899,"prompt_cache_miss_tokens":100}}}),
    ]
}
#[test]
fn projection_preserves_all_supported_families_and_field_precedence() {
    let mut cursor = Cursor::default();
    cursor.observe(r#"{"type":"session_meta","payload":{"id":"thread-one","session_id":"session-one","model_provider":"provider-one"}}"#);
    cursor.observe(
        r#"{"type":"turn_context","payload":{"turn_id":"turn-one","model":"request-one"}}"#,
    );
    for doc in fixtures() {
        compare(&doc.to_string(), Some(&cursor));
    }
    let providers = [
        json!({"usage":{"input_tokens":1000,"output_tokens":50,"cachedInputTokens":900}}),
        json!({"usage":{"inputTokens":1000,"outputTokens":50,"inputDetails":{"cachedTokens":900}}}),
        json!({"usage":{"inputTokens":1000,"outputTokens":50,"inputTokensDetails":{"cached_tokens":900}}}),
        json!({"rawUsage":{"prompt_tokens":1000,"completion_tokens":50,"prompt_tokens_details":[{"cached_tokens":0},{"cached_tokens":900}]}}),
        json!({"rawUsage":{"prompt_tokens":1000,"completion_tokens":50,"prompt_cache_hit_tokens":900}}),
    ];
    for provider in providers {
        let mut doc = fixtures().pop().unwrap();
        doc["providerData"] = provider;
        compare(&doc.to_string(), None);
    }
    let mut fallback = fixtures().pop().unwrap();
    fallback["providerData"] = json!({"rawUsage":{"prompt_cache_miss_tokens":10}});
    fallback["message"] = json!({"role":"assistant","status":"success","usage":{"input_tokens":1000,"output_tokens":50}});
    fallback.as_object_mut().unwrap().remove("role");
    fallback.as_object_mut().unwrap().remove("status");
    compare(&fallback.to_string(), None);
}
#[test]
fn malformed_optional_values_never_expose_a_different_usage_family() {
    let values = [
        Value::Null,
        json!(false),
        json!(7),
        json!("wrong"),
        json!([]),
        json!({"wrong":["ignored"]}),
    ];
    for base in fixtures() {
        for key in [
            "message",
            "providerData",
            "model",
            "timestamp",
            "id",
            "uuid",
            "type",
        ] {
            for value in &values {
                let mut doc = base.clone();
                doc[key] = value.clone();
                compare(&doc.to_string(), None);
            }
        }
    }
    for value in values {
        let mut doc = fixtures().remove(0);
        doc["message"]["usage"]["input_tokens"] = value;
        compare(&doc.to_string(), None);
    }
    for bad in [
        r#"{"type":"token_usage_record","payload":{"response_id":"r","usage":{"input_tokens":10}},"message":null}"#,
        r#"{"message":{"usage":{"input_tokens":10}},"providerData":null}"#,
        r#"{"message":{"usage":{"input_tokens":10}},"extra":1e400}"#,
        r#"{"message":{"usage":{"input_tokens":10}},"extra":"\uD800"}"#,
    ] {
        compare(bad, None);
    }
}
#[test]
fn large_irrelevant_transcripts_do_not_change_ledger_context_or_totals() {
    let sandbox = crate::testutil::Sandbox::new("usage-projection");
    let text = "synthetic body\n".repeat(80_000);
    let now = iso_utc_from_ms(now_ms());
    let mut docs = fixtures();
    for doc in &mut docs {
        doc["timestamp"] = json!(now);
        doc["body"] = json!({"transcript":[{"text":text}],"tool_calls":[{"arguments":text}]});
        if doc.get("message").is_some() {
            doc["message"]["content"] = json!([{ "text":text }]);
        }
        compare(&doc.to_string(), None);
        let projected = crate::usage_projection::usage(&doc.to_string()).unwrap();
        assert!(projected.to_string().len() < 2048);
    }
    // Keep the same net amount for all four sources; no model-price inference.
    docs[0]["message"]["usage"] = json!({"input_tokens":100,"output_tokens":50});
    docs[3]["providerData"] =
        json!({"usage":{"inputTokens":1000,"outputTokens":50,"cachedInputTokens":900}});
    let meta=json!({"type":"session_meta","payload":{"id":"thread-one","session_id":"session-one","model_provider":"provider-one","instructions":text}}).to_string();
    let turn=json!({"type":"turn_context","payload":{"turn_id":"turn-one","model":"request-one","developer_instructions":text}}).to_string();
    assert!(
        crate::usage_projection::context(&meta)
            .unwrap()
            .to_string()
            .len()
            < 512
    );
    assert!(
        crate::usage_projection::context(&turn)
            .unwrap()
            .to_string()
            .len()
            < 512
    );
    fs::write(
        sandbox.path().join("ledger.jsonl"),
        format!(
            "{meta}\n{turn}\n{}\n",
            docs.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let mut profile = crate::registry::builtin()
        .into_iter()
        .find(|p| p.id == "codex")
        .unwrap();
    profile.token_roots = vec![sandbox.path().to_string_lossy().into_owned()];
    profile.session_database = None;
    let mut monitor = TokenUsageMonitor::new();
    let report = monitor.monitor(&profile);
    assert_eq!(report.usage.tokens_total, 600);
    assert_eq!(report.usage.tokens24h, 600);
    assert_eq!(report.context24h.matched_tokens, 150);
    assert_eq!(report.context24h.unmatched_tokens, 450);
    assert_eq!(report.hourly30d.iter().map(|r| r.1).sum::<i64>(), 600);
    assert_eq!(monitor.monitor(&profile).usage.tokens_total, 600);
}

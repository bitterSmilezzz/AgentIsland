//! Manual model discovery for a saved target, without generation or configuration writes.
use crate::{
    connection_http::{AuthHeader, Failure, Http},
    provider::CodexProfile,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct ReadError {
    pub code: &'static str,
    pub message: String,
}

impl ReadError {
    pub fn target_invalidated(_: String) -> Self {
        Self {
            code: "target_invalidated",
            message: "档位已变化或不可核对，请刷新目录后重新读取模型".into(),
        }
    }
}

impl From<String> for ReadError {
    fn from(message: String) -> Self {
        Self {
            code: "read_failed",
            message,
        }
    }
}

impl From<&str> for ReadError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

#[derive(Debug, Serialize)]
pub struct ModelList {
    pub checked_at_ms: u64,
    pub models: Vec<String>,
    pub total: usize,
    pub truncated: bool,
}

fn target(profile: &CodexProfile) -> Result<url::Url, String> {
    profile.clone().normalized()?.ensure_writable_protocol()?;
    let base = crate::connections::normalize_url(&profile.base_url)?;
    let base = url::Url::parse(&base).map_err(|_| "接口地址无效")?;
    let loopback = match base.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if base.scheme() != "https" && !loopback {
        return Err("读取模型需要 HTTPS，或本机回环 HTTP 地址".into());
    }
    base.join("models").map_err(|_| "接口地址无效".into())
}

pub fn read(profile: &CodexProfile, now: u64) -> Result<ModelList, String> {
    discover(
        profile,
        now,
        |name| std::env::var(name).map_err(|_| Failure::CredentialUnavailable),
        |url, secret| Http::shared()?.get_json(url, &AuthHeader::Bearer(secret.to_owned())),
    )
}

fn discover(
    profile: &CodexProfile,
    now: u64,
    credential: impl FnOnce(&str) -> Result<String, Failure>,
    fetch: impl FnOnce(url::Url, &str) -> Result<Value, Failure>,
) -> Result<ModelList, String> {
    let url = target(profile)?;
    let secret = credential(&profile.env_key).map_err(|e| e.reason())?;
    let value = fetch(url, &secret).map_err(|e| e.reason())?;
    parse(value, now, &secret)
}

fn parse(value: Value, now: u64, secret: &str) -> Result<ModelList, String> {
    let invalid = || "服务模型格式不受支持，未取得有效清单".to_string();
    if value.get("object").and_then(Value::as_str) != Some("list") || value.get("error").is_some() {
        return Err(invalid());
    }
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    if data.len() > 4096 {
        return Err("服务模型数量超过读取上限".into());
    }
    let mut ids = BTreeSet::new();
    for item in data {
        let id = item.get("id").and_then(Value::as_str).ok_or_else(invalid)?;
        if item.get("object").and_then(Value::as_str) != Some("model")
            || id.is_empty()
            || id.len() > 160
            || !id
                .chars()
                .all(|c| c.is_alphanumeric() || "-_./:[]@".contains(c))
            || id.contains("sk-")
            || id.contains("AIza")
            || crate::private_text::known_private(id)
            || !secret.is_empty() && id.contains(secret)
        {
            return Err(invalid());
        }
        ids.insert(id.to_string());
    }
    let total = ids.len();
    Ok(ModelList {
        checked_at_ms: now,
        models: ids.into_iter().take(200).collect(),
        total,
        truncated: total > 200,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn profile() -> CodexProfile {
        CodexProfile {
            reasoning_effort: None,
            id: "fixture".into(),
            name: "Fixture".into(),
            model: "before".into(),
            provider_id: "fixture".into(),
            provider_name: "Fixture".into(),
            base_url: "https://example.invalid/proxy/v1".into(),
            env_key: "FIXTURE_AUTH".into(),
            wire_api: "responses".into(),
        }
    }
    fn payload(ids: &[&str]) -> Value {
        json!({"object":"list","data":ids.iter().map(|id|json!({"object":"model","id":id,"owned_by":"ignored"})).collect::<Vec<_>>()})
    }
    #[test]
    fn prefix_and_auth_reference_are_bound_without_writes() {
        let p = profile();
        let result = discover(
            &p,
            123,
            |name| {
                assert_eq!(name, "FIXTURE_AUTH");
                Ok("fixture-value".into())
            },
            |url, secret| {
                assert_eq!(url.as_str(), "https://example.invalid/proxy/v1/models");
                assert_eq!(secret, "fixture-value");
                Ok(payload(&["z", "a", "a"]))
            },
        )
        .unwrap();
        assert_eq!(result.models, vec!["a", "z"]);
        assert_eq!(result.total, 2);
        assert_eq!(result.checked_at_ms, 123);
        assert!(!result.truncated);
        assert_eq!(p.model, "before");
    }
    #[test]
    fn invalid_targets_fail_before_credential_or_network() {
        for address in [
            "http://example.invalid/v1",
            "http://localhost/v1",
            "https://example.invalid/v1?token=x",
            "https://user@example.invalid/v1",
        ] {
            let mut p = profile();
            p.base_url = address.into();
            assert!(discover(&p, 0, |_| panic!("credential"), |_, _| panic!("network")).is_err());
        }
        for address in ["http://127.0.0.1:8888/v1", "http://[::1]:8888/v1/"] {
            let mut p = profile();
            p.base_url = address.into();
            assert!(target(&p).unwrap().as_str().ends_with("/v1/models"));
        }
        let mut p = profile();
        p.wire_api = "chat".into();
        assert!(target(&p).is_err());
    }
    #[test]
    fn errors_and_secret_echo_never_become_models_or_zero() {
        for value in [
            json!({"data":[]}),
            json!({"object":"list","data":[],"error":"private"}),
            json!({"object":"list","data":[{"id":"a"}]}),
            payload(&["<script>"]),
            payload(&["sk-fixture"]),
            payload(&["prefix-fixture-value-suffix"]),
        ] {
            let e = parse(value, 0, "fixture-value").err().unwrap();
            assert!(!e.contains("private"));
            assert!(!e.contains("fixture-value"));
        }
        assert!(parse(payload(&[]), 0, "").unwrap().models.is_empty());
        for prefix in ["sk_", "ghp_", "github_pat_", "AKIA", "xoxb-"] {
            let id = format!("{prefix}{}", "a".repeat(20));
            assert!(parse(payload(&[&id]), 0, "").is_err());
        }
        assert!(discover(
            &profile(),
            0,
            |_| Err(Failure::CredentialUnavailable),
            |_, _| panic!("network")
        )
        .unwrap_err()
        .contains("环境变量"));
        for e in [
            Failure::AuthFailed,
            Failure::Forbidden,
            Failure::Timeout,
            Failure::Redirect,
            Failure::TooLarge,
            Failure::InvalidJson,
        ] {
            assert_eq!(
                discover(
                    &profile(),
                    0,
                    |_| Ok("fixture".into()),
                    |_, _| Err(e.clone())
                )
                .err()
                .unwrap(),
                e.reason()
            );
        }
    }
    #[test]
    fn bounded_projection_retains_total_and_rejects_unbounded_rows() {
        let value = json!({"object":"list","data":(0..250).map(|i|json!({"object":"model","id":format!("model-{i:03}")})).collect::<Vec<_>>()});
        let list = parse(value, 0, "").unwrap();
        assert_eq!(list.models.len(), 200);
        assert_eq!(list.total, 250);
        assert!(list.truncated);
        assert!(parse(
            json!({"object":"list","data":vec![json!({"object":"model","id":"same"});4097]}),
            0,
            ""
        )
        .is_err());
    }

    #[test]
    fn actual_http_sends_one_scoped_get_and_returns_only_ids() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut p = profile();
        p.base_url = format!("http://{}/prefix/v1", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                assert!(!line.is_empty());
                request.push_str(&line);
            }
            assert!(request.starts_with("GET /prefix/v1/models HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-value\r\n"));
            let body = payload(&["discovered-model"]).to_string();
            write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let result = discover(
            &p,
            123,
            |_| Ok("fixture-value".into()),
            |url, secret| Http::shared()?.get_json(url, &AuthHeader::Bearer(secret.to_owned())),
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(result.models, vec!["discovered-model"]);
        let dto = serde_json::to_string(&result).unwrap();
        for private in ["fixture-value", "owned_by", "/prefix", "FIXTURE_AUTH"] {
            assert!(!dto.contains(private));
        }
    }
}

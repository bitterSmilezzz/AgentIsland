//! Preset IM requests; credentials never enter settings or diagnostic output.
use crate::notifier::Outcome;
use crate::remote::{Channel, ChannelConfig};
use crate::render::{HttpField, Message, Request};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

pub fn feishu_credentials(secret: &str) -> Option<(String, String)> {
    let (webhook, signing) = if let Ok(value) = serde_json::from_str::<Value>(secret) {
        (
            value.get("webhook")?.as_str()?.to_string(),
            value
                .get("signingSecret")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        )
    } else {
        (secret.trim().to_string(), String::new())
    };
    let url = url::Url::parse(&webhook).ok()?;
    let hook = url.path().strip_prefix("/open-apis/bot/v2/hook/")?;
    (url.scheme() == "https"
        && url.host_str() == Some("open.feishu.cn")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && !hook.is_empty()
        && hook.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'))
    .then_some((webhook, signing))
}

pub fn onebot_url(address: &str) -> Option<String> {
    let url = url::Url::parse(address.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && !address.chars().any(char::is_control))
    .then(|| url.as_str().trim_end_matches('/').to_string())
}

fn signature(timestamp: i64, secret: &str) -> String {
    let key = format!("{timestamp}\n{secret}");
    let mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

pub fn request(
    message: &Message,
    channel: Channel,
    config: &ChannelConfig,
    secret: Option<&str>,
    preview: bool,
) -> Request {
    let mut request = Request {
        method: "POST".into(),
        headers: vec![HttpField::new("Content-Type", "application/json")],
        response_check: Some(channel),
        ..Request::default()
    };
    let text = format!("AgentIsland\n{}\n{}", message.title, message.body);
    let key = secret.unwrap_or("").trim();
    let body = match channel {
        Channel::FeishuBot => {
            let (url, signing) = match feishu_credentials(key) {
                Some(credentials) => credentials,
                None if preview => (
                    "https://open.feishu.cn/open-apis/bot/v2/hook/••••••".into(),
                    String::new(),
                ),
                None => return Request::default(),
            };
            request.url = if preview {
                "https://open.feishu.cn/open-apis/bot/v2/hook/••••••".into()
            } else {
                url
            };
            let mut body = json!({"msg_type":"text", "content":{"text":text}});
            if !signing.is_empty() {
                let timestamp = crate::tokens::now_ms() / 1000;
                body["timestamp"] = json!(timestamp.to_string());
                body["sign"] = json!(if preview {
                    "••••••".to_string()
                } else {
                    signature(timestamp, &signing)
                });
            }
            body
        }
        Channel::WechatPushPlus | Channel::QqPushPlus => {
            request.url = "https://www.pushplus.plus/send".into();
            json!({"token":if preview { "••••••" } else { key }, "title":message.title, "content":text, "template":"txt", "channel":if channel == Channel::QqPushPlus { "qq" } else { "wechat" }, "option":config.topic_or_url})
        }
        Channel::QqOneBot => {
            let Some(base) = onebot_url(&config.url_template) else {
                return Request::default();
            };
            let Ok(group) = config.topic_or_url.parse::<u64>() else {
                return Request::default();
            };
            request.url = format!("{base}/send_group_msg");
            if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Request::default();
            }
            if !key.is_empty() {
                request.headers.push(HttpField::new(
                    "Authorization",
                    if preview {
                        "Bearer ••••••".into()
                    } else {
                        format!("Bearer {key}")
                    },
                ));
            }
            json!({"group_id":group,"message":[{"type":"text","data":{"text":text}}]})
        }
        _ => return Request::default(),
    };
    request.body = body.to_string();
    request
}

pub fn response(channel: Channel, bytes: &[u8]) -> Outcome {
    let value = serde_json::from_slice::<Value>(bytes).ok();
    let code = value
        .as_ref()
        .and_then(|v| match channel {
            Channel::FeishuBot => v.get("code").or_else(|| v.get("StatusCode")),
            Channel::WechatPushPlus | Channel::QqPushPlus => v.get("code"),
            Channel::QqOneBot => v.get("retcode"),
            _ => None,
        })
        .and_then(Value::as_i64);
    let success = match channel {
        Channel::FeishuBot => code == Some(0),
        Channel::WechatPushPlus | Channel::QqPushPlus => code == Some(200),
        Channel::QqOneBot => {
            code == Some(0)
                && value
                    .as_ref()
                    .and_then(|v| v.get("status"))
                    .and_then(Value::as_str)
                    == Some("ok")
        }
        _ => false,
    };
    if success {
        Outcome::Delivered
    } else {
        Outcome::Failed {
            reason: format!(
                "{}未确认受理（业务码 {}），请检查机器人配置与平台限制",
                channel.label(),
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "无有效回执".into())
            ),
            permanent: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_rejections_are_not_http_success() {
        assert!(!response(
            Channel::FeishuBot,
            br#"{"code":19024,"msg":"private text"}"#
        )
        .is_delivered());
        assert!(!response(Channel::WechatPushPlus, br#"{"code":401}"#).is_delivered());
        assert!(!response(Channel::QqOneBot, br#"{"retcode":0,"status":"failed"}"#).is_delivered());
        assert!(response(Channel::QqOneBot, br#"{"retcode":0,"status":"ok"}"#).is_delivered());
        assert!(!response(Channel::FeishuBot, b"not JSON").is_delivered());
    }
    #[test]
    fn credentials_and_preview_are_private() {
        let key = json!({"webhook":"https://open.feishu.cn/open-apis/bot/v2/hook/fixture-hook", "signingSecret":"fixture-sign"}).to_string();
        assert!(feishu_credentials(&key).is_some());
        assert!(feishu_credentials("https://example.com/open-apis/bot/v2/hook/fixture").is_none());
        assert!(onebot_url("http://localhost:3000?token=fixture").is_none());
        let message = Message {
            title: "标题\"".into(),
            body: "正文\n[CQ:at,qq=all]".into(),
            urgent: false,
        };
        let actual = request(
            &message,
            Channel::FeishuBot,
            &ChannelConfig::default(),
            Some(&key),
            false,
        );
        let body: Value = serde_json::from_str(&actual.body).unwrap();
        assert_eq!(body["msg_type"], "text");
        assert!(body["sign"].as_str().unwrap().len() > 20);
        let preview = actual.masked_preview();
        assert!(!preview.contains("fixture-hook"));
        assert!(!preview.contains(body["sign"].as_str().unwrap()));
        let push = request(
            &message,
            Channel::WechatPushPlus,
            &ChannelConfig::default(),
            Some("fixture-token"),
            false,
        );
        assert!(!push.masked_preview().contains("fixture-token"));
        let cfg = ChannelConfig {
            url_template: "http://localhost:3000".into(),
            topic_or_url: "123".into(),
            ..ChannelConfig::default()
        };
        let qq = request(&message, Channel::QqOneBot, &cfg, None, false);
        let body: Value = serde_json::from_str(&qq.body).unwrap();
        assert_eq!(body["group_id"], 123);
        assert_eq!(body["message"][0]["type"], "text");
    }
}

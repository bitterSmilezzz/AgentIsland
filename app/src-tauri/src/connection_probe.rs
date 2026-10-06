//! Service-specific public status checks. Reachability never grants model/usage permissions.
use crate::{
    connection_http::{Failure, Http},
    connections::{Connection, CredentialRef, Kind},
};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);
pub struct Lease;
impl Lease {
    pub fn acquire() -> Result<Self, String> {
        ACTIVE
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self)
            .map_err(|_| "已有服务读取正在进行，请稍后重试".into())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Readable,
    Disabled,
    Unsupported,
    Offline,
    AuthFailed,
    PermissionDenied,
    Error,
}
#[derive(Serialize)]
pub struct Probe {
    pub connection_id: String,
    pub checked_at_ms: u64,
    pub status: Status,
    pub service_version: Option<String>,
    pub reason: String,
    pub models_verified: bool,
    pub usage_verified: bool,
}
pub fn inspect(connection: &Connection, now: u64) -> Probe {
    probe(connection, now, |url, reference| {
        let credential = match reference {
            Some(CredentialRef::Environment { name }) => {
                Some(std::env::var(name).map_err(|_| Failure::CredentialUnavailable)?)
            }
            None => None,
        };
        Http::shared()?.get_json(url, credential.as_deref())
    })
}
fn probe(
    connection: &Connection,
    now: u64,
    fetch: impl FnOnce(url::Url, Option<&CredentialRef>) -> Result<Value, Failure>,
) -> Probe {
    let mut result = Probe {
        connection_id: connection.id.clone(),
        checked_at_ms: now,
        status: Status::Unsupported,
        service_version: None,
        reason: "此服务的只读接口尚未核实".into(),
        models_verified: false,
        usage_verified: false,
    };
    if !connection.enabled {
        result.status = Status::Disabled;
        result.reason = "连接已停用，未访问服务".into();
        return result;
    }
    if connection.kind == Kind::CcSwitch {
        return result;
    }
    let url = crate::connections::normalize_url(&connection.base_url)
        .ok()
        .and_then(|base| url::Url::parse(&base).ok())
        .and_then(|base| {
            if connection.kind == Kind::NewApi {
                base.join("api/status").ok()
            } else {
                Some(base)
            }
        });
    let Some(url) = url else {
        result.status = Status::Error;
        result.reason = Failure::InvalidAddress.reason().into();
        return result;
    };
    // New API /api/status is anonymous; never read a configured account credential for it.
    let reference = if connection.kind == Kind::Magpie {
        connection.credential_ref.as_ref()
    } else {
        None
    };
    match fetch(url, reference) {
        Err(failure) => {
            result.status = match failure {
                Failure::AuthFailed => Status::AuthFailed,
                Failure::Forbidden => Status::PermissionDenied,
                Failure::Unavailable | Failure::Timeout => Status::Offline,
                _ => Status::Error,
            };
            result.reason = failure.reason().into();
        }
        Ok(value) => {
            let version = match connection.kind {
                Kind::NewApi if value.get("success").and_then(Value::as_bool) == Some(true) => {
                    value.get("data").and_then(|data| data.get("version"))
                }
                Kind::Magpie if value.get("name").and_then(Value::as_str) == Some("magpie") => {
                    value.get("version")
                }
                _ => None,
            }
            .and_then(Value::as_str)
            .filter(|version| valid_version(version));
            if let Some(version) = version {
                result.status = Status::Readable;
                result.service_version = Some(version.to_owned());
                result.reason = "服务状态可读；模型和用量权限尚未检测".into();
            } else {
                result.reason = "服务状态格式或版本标识不受支持".into();
            }
        }
    }
    result
}
fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 80
        && !version.contains("sk-")
        && !version.contains("AIza")
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        && (version.as_bytes()[0].is_ascii_digit()
            || version.starts_with('v')
            || matches!(version, "dev" | "unknown"))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_rejects_overlap_and_releases_on_drop() {
        let first = Lease::acquire().unwrap();
        assert!(Lease::acquire().is_err());
        drop(first);
        let next = Lease::acquire().unwrap();
        drop(next);
    }

    fn connection(kind: Kind) -> Connection {
        Connection {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            name: "Fixture".into(),
            base_url: "http://127.0.0.1:8000/proxy/".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            enabled: true,
            created_ms: 1,
            updated_ms: 1,
        }
    }
    #[test]
    fn disabled_and_unverified_services_never_read_credentials_or_make_requests() {
        let mut c = connection(Kind::Magpie);
        c.enabled = false;
        assert!(matches!(
            probe(&c, 100, |_, _| panic!("disabled request")).status,
            Status::Disabled
        ));
        assert!(matches!(
            probe(&connection(Kind::CcSwitch), 100, |_, _| panic!(
                "unverified request"
            ))
            .status,
            Status::Unsupported
        ));
    }
    #[test]
    fn new_api_only_reads_public_status_and_preserves_proxy_prefix() {
        let c = connection(Kind::NewApi);
        let result = probe(&c, 100, |url, reference| {
            assert_eq!(url.path(), "/proxy/api/status");
            assert!(reference.is_none());
            Ok(
                serde_json::json!({"success":true,"data":{"version":"v1.2.3","ignored":"private-fixture-marker"}}),
            )
        });
        assert!(matches!(result.status, Status::Readable));
        assert!(!result.models_verified && !result.usage_verified);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("private-fixture-marker"));
    }
    #[test]
    fn magpie_requires_service_identity_and_safe_version() {
        let c = connection(Kind::Magpie);
        let good = probe(&c, 100, |url, reference| {
            assert_eq!(url.path(), "/proxy/");
            assert_eq!(reference, c.credential_ref.as_ref());
            Ok(serde_json::json!({"name":"magpie","version":"0.1.0"}))
        });
        assert!(matches!(good.status, Status::Readable));
        for body in [
            serde_json::json!({"name":"other","version":"0.1.0"}),
            serde_json::json!({"name":"magpie","version":"private-fixture-marker"}),
            serde_json::json!({"name":"magpie","version":null}),
        ] {
            assert!(matches!(
                probe(&c, 100, |_, _| Ok(body)).status,
                Status::Unsupported
            ));
        }
    }
    #[test]
    fn business_failure_and_access_errors_never_count_as_connected() {
        let c = connection(Kind::NewApi);
        assert!(matches!(probe(&c,100,|_,_|Ok(serde_json::json!({"success":false,"message":"private-fixture-marker","data":{"version":"1.2.3"}}))).status,Status::Unsupported));
        assert!(matches!(
            probe(&c, 100, |_, _| Err(Failure::AuthFailed)).status,
            Status::AuthFailed
        ));
        assert!(matches!(
            probe(&c, 100, |_, _| Err(Failure::Forbidden)).status,
            Status::PermissionDenied
        ));
        assert!(matches!(
            probe(&c, 100, |_, _| Err(Failure::Timeout)).status,
            Status::Offline
        ));
    }
}

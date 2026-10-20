//! Service-specific public status checks. Reachability never grants model/usage permissions.
use crate::{
    connection_http::{AuthHeader, Failure, Http},
    connections::{AuthScheme, Connection, CredentialRef, Kind},
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
    /// 失败时的稳定分类码（`Failure::code`）。`Status::Error` 会把限流、凭据来源
    /// 不可用、明文拒绝等不同原因压成同一个状态；没有这个字段，界面与门禁都
    /// 只能看到「服务读取失败」，分不出到底是哪一种。
    pub failure_code: Option<String>,
    pub service_version: Option<String>,
    pub reason: String,
    pub models_verified: bool,
    pub usage_verified: bool,
}
/// 有效认证方案：显式声明的优先，未声明时按服务类型沿用既有语义。
///
/// 为什么不直接照 `connection.auth` 执行：旧配置文件没有这个字段，缺省值是 `None`——
/// 直接照它执行会让既有 Magpie 连接**静默地不再发送凭据**，把一次可用的读取
/// 变成一次必然的 401。新增字段的缺省语义必须与它替换掉的行为逐条对齐。
fn effective_auth(connection: &Connection) -> AuthScheme {
    match connection.kind {
        Kind::CustomEndpoint => connection.auth.clone(),
        _ if connection.credential_ref.is_none() => AuthScheme::None,
        Kind::Magpie => AuthScheme::Bearer,
        _ => AuthScheme::None,
    }
}
pub fn inspect(connection: &Connection, now: u64) -> Probe {
    probe(connection, now, |url, reference, scheme| {
        // 每次读取都从来源重新取值：轮换环境变量后无需重启或清缓存即生效，
        // 也不存在「上一次的凭据被缓存下来继续用」的窗口。
        let resolve = || match reference {
            Some(CredentialRef::Environment { name }) => std::env::var(name)
                .map(Some)
                .map_err(|_| Failure::CredentialUnavailable),
            None => Ok(None),
        };
        let header = |credential: Option<String>| -> Result<AuthHeader, Failure> {
            Ok(match scheme {
                AuthScheme::None => AuthHeader::None,
                AuthScheme::Bearer => {
                    AuthHeader::Bearer(credential.ok_or(Failure::CredentialUnavailable)?)
                }
                AuthScheme::Header { name } => AuthHeader::Custom {
                    name: name.clone(),
                    value: credential.ok_or(Failure::CredentialUnavailable)?,
                },
            })
        };
        let auth = header(resolve()?)?;
        match Http::shared()?.get_json(url.clone(), &auth) {
            // 只有 401 才谈刷新：403/429/网络失败刷新也换不来权限，重试只是多花一次请求。
            Err(Failure::AuthFailed) => {
                let Some(refresh) = connection.refresh.as_ref() else {
                    return Err(Failure::AuthFailed);
                };
                if scheme.is_none() {
                    return Err(Failure::AuthFailed);
                }
                // 只换一次、只重试一次：刷新本身再失败就如实报 refresh_failed，
                // 不在「换令牌失败」上面再叠一层重试，否则失败会被无限放大。
                let token = crate::connection_http::refresh_access_token(refresh)?;
                let retried = header(Some(token))?;
                Http::shared()?.get_json(url, &retried)
            }
            other => other,
        }
    })
}

fn probe(
    connection: &Connection,
    now: u64,
    fetch: impl FnOnce(url::Url, Option<&CredentialRef>, &AuthScheme) -> Result<Value, Failure>,
) -> Probe {
    let mut result = Probe {
        connection_id: connection.id.clone(),
        checked_at_ms: now,
        status: Status::Unsupported,
        failure_code: None,
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
    let scheme = effective_auth(connection);
    let reference = if scheme.is_none() {
        None
    } else {
        connection.credential_ref.as_ref()
    };
    match fetch(url, reference, &scheme) {
        Err(failure) => {
            result.failure_code = Some(failure.code().to_owned());
            result.status = match failure {
                Failure::AuthFailed => Status::AuthFailed,
                Failure::Forbidden => Status::PermissionDenied,
                Failure::Unavailable | Failure::Timeout => Status::Offline,
                _ => Status::Error,
            };
            result.reason = failure.reason().into();
        }
        Ok(value) => {
            if connection.kind == Kind::CustomEndpoint {
                // 自定义端点没有可核对的版本契约：2xx 且 JSON 对象即视为可读，
                // 并在文案里说清「可读 ≠ 有模型/用量权限」，不冒充内置服务的身份校验。
                result.status = Status::Readable;
                result.reason = "自定义端点可读；模型和用量权限尚未检测".into();
                return result;
            }
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
            auth: AuthScheme::None,
            refresh: None,
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
            probe(&c, 100, |_, _, _| panic!("disabled request")).status,
            Status::Disabled
        ));
        assert!(matches!(
            probe(&connection(Kind::CcSwitch), 100, |_, _, _| panic!(
                "unverified request"
            ))
            .status,
            Status::Unsupported
        ));
    }
    #[test]
    fn new_api_only_reads_public_status_and_preserves_proxy_prefix() {
        let c = connection(Kind::NewApi);
        let result = probe(&c, 100, |url, reference, _scheme| {
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
        let good = probe(&c, 100, |url, reference, _scheme| {
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
                probe(&c, 100, |_, _, _| Ok(body)).status,
                Status::Unsupported
            ));
        }
    }
    #[test]
    fn business_failure_and_access_errors_never_count_as_connected() {
        let c = connection(Kind::NewApi);
        assert!(matches!(probe(&c,100,|_,_,_|Ok(serde_json::json!({"success":false,"message":"private-fixture-marker","data":{"version":"1.2.3"}}))).status,Status::Unsupported));
        assert!(matches!(
            probe(&c, 100, |_, _, _| Err(Failure::AuthFailed)).status,
            Status::AuthFailed
        ));
        assert!(matches!(
            probe(&c, 100, |_, _, _| Err(Failure::Forbidden)).status,
            Status::PermissionDenied
        ));
        assert!(matches!(
            probe(&c, 100, |_, _, _| Err(Failure::Timeout)).status,
            Status::Offline
        ));
    }

    #[test]
    fn legacy_kinds_keep_their_credential_semantics_without_the_new_field() {
        // 旧配置文件没有 auth 字段，缺省是 None：不能让既有 Magpie 连接静默地
        // 不再发送凭据（那会把一次可用的读取变成必然的 401）。
        let magpie = connection(Kind::Magpie);
        assert_eq!(magpie.auth, AuthScheme::None);
        assert_eq!(effective_auth(&magpie), AuthScheme::Bearer);

        let mut anonymous = connection(Kind::Magpie);
        anonymous.credential_ref = None;
        assert_eq!(effective_auth(&anonymous), AuthScheme::None);

        let mut public = connection(Kind::NewApi);
        public.auth = AuthScheme::Bearer;
        // 内置公开状态接口始终匿名：显式声明也不改变这条既有语义。
        assert_eq!(effective_auth(&public), AuthScheme::None);

        let mut custom = connection(Kind::CustomEndpoint);
        custom.auth = AuthScheme::Header {
            name: "x-api-key".into(),
        };
        assert_eq!(
            effective_auth(&custom),
            AuthScheme::Header {
                name: "x-api-key".into()
            }
        );
    }

    #[test]
    fn custom_endpoint_reads_its_base_address_and_claims_no_version_identity() {
        let mut c = connection(Kind::CustomEndpoint);
        c.auth = AuthScheme::Bearer;
        let reference = c.credential_ref.clone();
        let result = probe(&c, 100, move |url, seen, scheme| {
            assert_eq!(url.path(), "/proxy/");
            assert_eq!(seen, reference.as_ref());
            assert_eq!(scheme, &AuthScheme::Bearer);
            Ok(serde_json::json!({"object":"list","data":[{"id":"local-model"}]}))
        });
        assert!(matches!(result.status, Status::Readable));
        assert!(
            result.service_version.is_none(),
            "自定义端点不冒充版本身份：{:?}",
            result.service_version
        );
        assert!(!result.models_verified && !result.usage_verified);
    }

    #[test]
    fn transport_and_auth_failures_stay_distinguishable_at_the_probe() {
        let c = connection(Kind::CustomEndpoint);
        let insecure = probe(&c, 100, |_, _, _| Err(Failure::InsecureTransport));
        assert!(matches!(insecure.status, Status::Error));
        assert!(insecure.reason.contains("HTTPS"), "{}", insecure.reason);
        let missing = probe(&c, 100, |_, _, _| Err(Failure::CredentialUnavailable));
        assert!(matches!(missing.status, Status::Error));
        assert_ne!(insecure.reason, missing.reason);
        let refused = probe(&c, 100, |_, _, _| Err(Failure::AuthFailed));
        assert!(matches!(refused.status, Status::AuthFailed));
        let limited = probe(&c, 100, |_, _, _| Err(Failure::RateLimited));
        assert!(matches!(limited.status, Status::Error));
    }
}

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
    /// 提取出的余额/配额信息（GW-01 订阅与余额查询协议）
    pub balance: Option<BalanceInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BalanceInfo {
    pub amount: f64,
    pub unit: String,
    pub raw_text: String,
    /// 订阅套餐（plan/tier）**原样**：网关自己怎么说就怎么显示。
    /// `None` = 这次没取到（网关没给，或没声明路径）——**不显示占位**。
    pub plan: Option<String>,
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
    mut fetch: impl FnMut(url::Url, Option<&CredentialRef>, &AuthScheme) -> Result<Value, Failure>,
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
        balance: None,
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
            let is_custom = connection.kind == Kind::CustomEndpoint;
            let mut readable = false;
            if is_custom {
                readable = true;
            } else {
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
                if let Some(v) = version {
                    result.service_version = Some(v.to_owned());
                    readable = true;
                } else {
                    result.reason = "服务状态格式或版本标识不受支持".into();
                }
            }

            if readable {
                result.status = Status::Readable;
                // 基础端点可读后，按需查询订阅与余额（GW-01 协议）
                if let Some(quota) = &connection.quota {
                    let q_url = quota_url(&connection.base_url, &quota.endpoint);
                    if let Some(q_url) = q_url {
                        match fetch(q_url, reference, &scheme) {
                            Ok(q_val) => {
                                if let Some(amount) = extract_balance(&q_val, &quota.balance_path) {
                                    result.usage_verified = true;
                                    let unit = quota.unit.clone().unwrap_or_default();
                                    let raw_text = format!("{amount} {unit}").trim().to_owned();
                                    // 套餐：声明了路径才取，取到才带上；**原样**显示。
                                    let plan = quota
                                        .plan_path
                                        .as_deref()
                                        .and_then(|path| extract_plan(&q_val, path));
                                    result.balance = Some(BalanceInfo {
                                        amount,
                                        unit,
                                        raw_text: raw_text.clone(),
                                        plan,
                                    });
                                    let plan_note = match &result.balance.as_ref().and_then(|b| b.plan.clone()) {
                                        Some(plan) => format!("，套餐 {plan}"),
                                        None => String::new(),
                                    };
                                    result.reason =
                                        format!("服务状态可读；余额/用量已核实（{raw_text}{plan_note}）");
                                } else {
                                    result.usage_verified = false;
                                    result.reason = format!(
                                        "服务状态可读；但在余额响应路径「{}」中未解析到数值",
                                        quota.balance_path
                                    );
                                }
                            }
                            Err(f) => {
                                result.usage_verified = false;
                                result.reason = format!("服务状态可读；但余额查询失败（{}）", f.reason());
                            }
                        }
                    } else {
                        result.usage_verified = false;
                        result.reason = "服务状态可读；但余额查询端点地址格式无效".into();
                    }
                } else {
                    result.reason = if is_custom {
                        "自定义端点可读；模型和用量权限尚未检测".into()
                    } else {
                        "服务状态可读；模型和用量权限尚未检测".into()
                    };
                }
            }
        }
    }
    result
}
/// 解析余额查询端点 URL：绝对 URL 直接解析，相对路径挂在 base_url 之后。
fn quota_url(base_url: &str, endpoint: &str) -> Option<url::Url> {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        url::Url::parse(endpoint).ok()
    } else {
        let base = crate::connections::normalize_url(base_url).ok()?;
        let mut base = url::Url::parse(&base).ok()?;
        if !base.path().ends_with('/') {
            let new_path = format!("{}/", base.path());
            base.set_path(&new_path);
        }
        base.join(endpoint).ok()
    }
}

/// 按点分隔路径从 JSON 对象中提取**字符串**（套餐名用）。
///
/// 只认字符串与数字：对象/数组一律不认（那不是套餐名，是结构）。
/// 返回**原样**（仅去首尾空白）——套餐名是网关的说法，我们不翻译、不归一。
fn extract_plan(root: &Value, path: &str) -> Option<String> {
    let mut current = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = current.get(segment)?;
    }
    let text = match current {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if text.is_empty() || text.chars().count() > 64 || text.chars().any(char::is_control) {
        return None;
    }
    Some(text)
}

/// 按点分隔路径从 JSON 对象中提取浮点数值（支持数值类型与数字字符串）。
fn extract_balance(root: &Value, path: &str) -> Option<f64> {
    let mut current = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = current.get(segment)?;
    }
    match current {
        Value::Number(num) => num.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
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

    /// 套餐（plan/tier）语义：**按网关原话说**，取不到就没有。
    ///
    /// 三条：①声明了路径且网关给了值 → 原样取回（含大小写与内部连字符，不归一）；
    /// ②没声明路径 → 一律没有（不许「猜」一个套餐出来）；
    /// ③声明了但网关没给 / 给的不是字符串 → 也没有，**不显示占位**。
    #[test]
    fn the_plan_is_taken_verbatim_and_never_invented() {
        let payload: Value = serde_json::from_str(
            r#"{"data":{"plan":"Team-Enterprise","tier":"pro"},"quota":42}"#,
        )
        .unwrap();
        assert_eq!(extract_plan(&payload, "data.plan").as_deref(), Some("Team-Enterprise"));
        assert_eq!(extract_plan(&payload, "data.tier").as_deref(), Some("pro"));
        // 数字也能当套餐名（有些网关给序号），但仍按原样
        assert_eq!(extract_plan(&payload, "quota").as_deref(), Some("42"));
        // 没有的、非字符串的、空的 → 没有
        assert_eq!(extract_plan(&payload, "data.missing"), None);
        assert_eq!(extract_plan(&payload, "data"), None);
        let empty: Value = serde_json::from_str(r#"{"plan":"  "}"#).unwrap();
        assert_eq!(extract_plan(&empty, "plan"), None, "空字符串不算套餐");
        // 路径不许穿越
        assert_eq!(extract_plan(&payload, "..plan"), None);
    }

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
            quota: None,
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
                probe(&c, 100, |_, _, _| Ok(body.clone())).status,
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

    /// 套餐（plan/tier）走完一整条路：声明 → 提取 → 上屏。
    ///
    /// 三种情形都要分得清：**有就是有**（原样）、**没有就是没有**（不显示占位）、
    /// **没声明就当没这回事**（不许猜一个套餐出来）。
    #[test]
    fn the_plan_reaches_the_reason_only_when_the_gateway_actually_declares_it() {
        use crate::connections::QuotaConfig;

        let mut c = connection(Kind::CustomEndpoint);
        c.auth = AuthScheme::Bearer;
        c.quota = Some(QuotaConfig {
            plan_path: Some("data.plan".into()),
            endpoint: "api/user/quota".into(),
            balance_path: "data.remain_quota".into(),
            unit: Some("Tokens".into()),
        });
        // ① 网关给了 → 原样带上
        let result = probe(&c, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Ok(serde_json::json!({"data": {"remain_quota": 500000.0, "plan": "Team-Enterprise"}}))
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        let balance = result.balance.expect("balance info must exist");
        assert_eq!(balance.plan.as_deref(), Some("Team-Enterprise"), "套餐名必须原样");
        assert!(
            result.reason.contains("Team-Enterprise"),
            "状态文案里要能看到套餐：{}",
            result.reason
        );

        // ② 声明了但网关没给 → 没有套餐，也不显示占位
        let result = probe(&c, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Ok(serde_json::json!({"data": {"remain_quota": 500000.0}}))
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        let balance = result.balance.expect("balance info must exist");
        assert_eq!(balance.plan, None, "网关没给就不该有套餐");
        assert!(
            !result.reason.contains("套餐"),
            "没有套餐就不该出现套餐字样：{}",
            result.reason
        );

        // ③ 没声明路径 → 一律没有（不许猜）
        let mut without = c.clone();
        without.quota = Some(QuotaConfig {
            plan_path: None,
            endpoint: "api/user/quota".into(),
            balance_path: "data.remain_quota".into(),
            unit: Some("Tokens".into()),
        });
        let result = probe(&without, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Ok(serde_json::json!({"data": {"remain_quota": 500000.0, "plan": "Team-Enterprise"}}))
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        let balance = result.balance.expect("balance info must exist");
        assert_eq!(balance.plan, None, "没声明路径就当没有这回事");
    }

    #[test]
    fn quota_protocol_verifies_usage_and_extracts_balance() {
        use crate::connections::QuotaConfig;

        let mut c = connection(Kind::CustomEndpoint);
        c.auth = AuthScheme::Bearer;
        c.quota = Some(QuotaConfig {
            plan_path: None,
            endpoint: "api/user/quota".into(),
            balance_path: "data.remain_quota".into(),
            unit: Some("Tokens".into()),
        });

        // 成功提取数值
        let result = probe(&c, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Ok(serde_json::json!({
                    "data": { "remain_quota": 500000.0 }
                }))
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        assert!(matches!(result.status, Status::Readable));
        assert!(result.usage_verified);
        let b = result.balance.expect("balance info must exist");
        assert_eq!(b.amount, 500000.0);
        assert_eq!(b.unit, "Tokens");
        assert!(result.reason.contains("500000 Tokens"));

        // 路径不存在或非数字
        let mut c_bad = c.clone();
        c_bad.quota = Some(QuotaConfig {
            plan_path: None,
            endpoint: "api/user/quota".into(),
            balance_path: "data.missing_field".into(),
            unit: Some("USD".into()),
        });
        let result_bad = probe(&c_bad, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Ok(serde_json::json!({"data": {"other": "value"}}))
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        assert!(matches!(result_bad.status, Status::Readable));
        assert!(!result_bad.usage_verified);
        assert!(result_bad.balance.is_none());
        assert!(result_bad.reason.contains("未解析到数值"));

        // 余额查询遭遇 401 鉴权失败：基础服务依然保留可读，但 usage 未核实并标明原因
        let result_auth_fail = probe(&c, 100, |url, _, _| {
            if url.path() == "/proxy/api/user/quota" {
                Err(Failure::AuthFailed)
            } else {
                Ok(serde_json::json!({"status": "ok"}))
            }
        });
        assert!(matches!(result_auth_fail.status, Status::Readable));
        assert!(!result_auth_fail.usage_verified);
        assert!(result_auth_fail.reason.contains("服务拒绝凭据"));
    }
}

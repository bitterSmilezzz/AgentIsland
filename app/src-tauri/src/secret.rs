//! 密钥存取：**只进 macOS 钥匙串**。
//!
//! 本仓红线：源码与任何落盘配置里零硬编码 secret；密钥由用户自己录入，
//! 界面上永远只显示掩码，日志里绝不写值。这里落地的是这条红线的「存」与「取」。
//!
//! 对齐 Swift `RemoteSecret`，**service 与条目名逐字一致**——不一致的后果是
//! 「用户在 Swift 版存过、Rust 版读不到」：界面全绿、每次发送都失败，
//! 而这条失效在日志里没有任何线索（那正是它最难查的地方）。

use crate::remote::Channel;

/// 钥匙串 service（与 Swift `RemoteSecret.service` 逐字一致）
pub const SERVICE: &str = "com.agentisland.remote";

/// 该通道的密钥条目名。刻意**不做成可配字段**：「密钥存了但条目名对不上」
/// 是查不出来的失效——界面全绿、每次发送都失败。条目名由通道种类唯一决定。
pub fn default_secret_name(channel: Channel) -> String {
    format!("remote.{}", channel.as_str())
}

/// 写入结果。`Refused` 必须原样显示给用户：本 App 走 ad-hoc 签名，每次重新出包
/// 代码标识都变，钥匙串可能弹「允许访问」甚至直接拒绝——静默吞掉就等于
/// 用户以为存上了，之后每次外发都失败且没有任何线索。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WriteResult {
    Ok,
    Refused { reason: String },
}

/// 发送路径只从密钥源取一个值。
///
/// 设置页要的是「存过没有」，那是 [`exists`]（自由函数，走不带 `kSecReturnData` 的查询，
/// 值不进界面内存）——两件事分开，所以这里只有 `read`。
/// 引擎拿的是这个 trait，于是「总开关关着时不该碰钥匙串」这条可以被用例数出来。
pub trait SecretStore: Send + Sync {
    fn read(&self, name: &str) -> Option<String>;
}

/// 真的钥匙串
pub struct Keychain;

impl SecretStore for Keychain {
    fn read(&self, name: &str) -> Option<String> {
        read(name)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{WriteResult, SERVICE};
    use security_framework::base::Error;
    use security_framework::item::{ItemClass, ItemSearchOptions};
    use security_framework::passwords::{
        delete_generic_password, generic_password, set_generic_password, PasswordOptions,
    };
    use security_framework_sys::base::errSecAuthFailed;

    /// `security-framework-sys` 没有导出这一条（base.rs 里只列了常见的一部分）。
    /// 值与名字都取自 Apple `SecBase.h`：`errSecInteractionNotAllowed = -25308`。
    /// 具名常量而不是魔数，理由和别处一样：看代码的人得知道这个码代表什么。
    const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

    /// 把 OSStatus 翻成人话；拿不到系统描述时退回状态码，绝不显示成「未知错误」。
    /// 与 Swift `RemoteSecret.describe` 同一条口径：前两条是 ad-hoc 签名下最常撞的，
    /// 给的必须是「怎么办」而不是一个码。
    fn describe(error: Error) -> String {
        if error.code() == ERR_SEC_INTERACTION_NOT_ALLOWED {
            return "钥匙串已锁定或要求交互但被拒绝".to_string();
        }
        if error.code() == errSecAuthFailed {
            return "钥匙串访问被拒（代码标识变更后需在弹窗点「始终允许」）".to_string();
        }
        match error.message() {
            Some(message) if !message.is_empty() => format!("{message}（{}）", error.code()),
            _ => format!("OSStatus {}", error.code()),
        }
    }

    /// 读取；没有条目或读不到都返回 `None`（读失败不该让调用方以为「密钥是空串」）
    pub fn read(name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        let options = PasswordOptions::new_generic_password(SERVICE, name);
        let bytes = generic_password(options).ok()?;
        String::from_utf8(bytes).ok()
    }

    /// 只看存在性：**刻意不取数据**，值不进本进程内存
    pub fn exists(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(name)
            .load_data(false)
            .limit(1)
            .search()
            .map(|items| !items.is_empty())
            .unwrap_or(false)
    }

    /// 写入即覆盖同名条目。
    ///
    /// 与 Swift 的差别（有意）：Swift 用裸 `SecItemAdd`，撞到重复条目时得
    /// 先删再加，而它在注释里写明那一步在 ad-hoc 签名下很危险（旧条目能删、新条目被拒，
    /// 结果旧值也没了）。`security-framework` 的实现撞到重复走的是 **SecItemUpdate**，
    /// 于是那个窗口在这里不存在——同时也就少了一处要防的失效。
    pub fn write(name: &str, value: &str) -> WriteResult {
        if name.is_empty() {
            return WriteResult::Refused {
                reason: "条目名为空".to_string(),
            };
        }
        match set_generic_password(SERVICE, name, value.as_bytes()) {
            Ok(()) => WriteResult::Ok,
            Err(error) => WriteResult::Refused {
                reason: describe(error),
            },
        }
    }

    pub fn delete(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        delete_generic_password(SERVICE, name).is_ok()
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::WriteResult;

    /// 非 macOS 平台如实说「未接」：这个 App 是 macOS 的，静默返回 None 会变成
    /// 「配好了却永远不发」那类查不出来的失效
    pub fn read(_name: &str) -> Option<String> {
        None
    }

    pub fn exists(_name: &str) -> bool {
        false
    }

    pub fn write(_name: &str, _value: &str) -> WriteResult {
        WriteResult::Refused {
            reason: "本平台未接钥匙串（只支持 macOS）".to_string(),
        }
    }

    pub fn delete(_name: &str) -> bool {
        false
    }
}

pub use platform::{delete, exists, read, write};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_and_entry_names_are_pinned_to_the_shipped_swift_versions() {
        // 这两个字符串是**跨版本契约**：Swift 版已经把它们写进了用户的钥匙串。
        // 改任何一个都会让老用户的密钥「消失」（读不到 ⇒ 闸门报未配置），
        // 而现象只是「配好了却发不出去」。所以钉死。
        assert_eq!(SERVICE, "com.agentisland.remote");
        assert_eq!(default_secret_name(Channel::Ntfy), "remote.ntfy");
        assert_eq!(
            default_secret_name(Channel::CustomHttp),
            "remote.customHTTP"
        );
        assert_eq!(default_secret_name(Channel::SmtpEmail), "remote.smtpEmail");
    }

    #[test]
    fn every_channel_has_its_own_entry_and_no_two_share_one() {
        // 共用条目名会让「换了通道却还在用旧密钥」变成静默错误
        let names: Vec<String> = [Channel::Ntfy, Channel::CustomHttp, Channel::SmtpEmail]
            .into_iter()
            .map(default_secret_name)
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "条目名必须两两不同");
    }

    #[test]
    fn an_empty_entry_name_is_never_touched() {
        // 空名会命中「所有空名条目」，是数据串味的入口
        assert_eq!(read(""), None);
        assert!(!exists(""));
        assert!(!delete(""));
        assert_eq!(
            write("", "x"),
            WriteResult::Refused {
                reason: "条目名为空".to_string()
            }
        );
    }

    #[test]
    fn a_name_that_was_never_stored_reads_as_absent() {
        // 真钥匙串的**只读**路径：这个条目名不可能存在（带进程号），用例没有任何副作用。
        // 它验的是「读不到时要如实说没有」，而不是把失败当成空串。
        let name = format!("remote.selftest-nonexistent-{}", std::process::id());
        assert_eq!(read(&name), None);
        assert!(!exists(&name), "从未写过的条目必须报不存在");
    }

    /// **手动探针**（默认不跑）：真的往钥匙串写、读、删一次。
    ///
    /// 为什么不做成默认用例：它会在用户的钥匙串里留下条目（虽然是本 App 自己的 service
    /// 与一个测试名）。Swift 侧的既有做法是只测纯函数，这里保持同一条线；
    /// 但「这条路径到底通不通」需要一次实物取证，所以留成 `--ignored` 的手动探针：
    ///
    /// ```sh
    /// cargo test --manifest-path app/src-tauri/Cargo.toml -- --ignored manual_keychain_round_trip
    /// ```
    #[test]
    #[ignore]
    fn manual_keychain_round_trip() {
        let name = format!("remote.selftest-{}", std::process::id());
        let value = "selftest-not-a-real-secret";
        assert_eq!(write(&name, value), WriteResult::Ok, "写入应成功");
        assert!(exists(&name), "写过之后存在性应为真");
        assert_eq!(read(&name).as_deref(), Some(value), "读回的值应与写入一致");
        // 覆盖写：不该变成两条
        assert_eq!(write(&name, "second-value"), WriteResult::Ok);
        assert_eq!(read(&name).as_deref(), Some("second-value"));
        assert!(delete(&name), "删除应成功");
        assert!(!exists(&name), "删除后存在性应为假");
    }
}

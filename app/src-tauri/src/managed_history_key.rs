//! AgentIsland 自有执行历史项目密钥。
//!
//! 独立于 `secret.rs` 的远程通知 service。当前仅 macOS 钥匙串实现；其他平台明确拒绝。
//! 此接口暂不连接 Tauri 命令或任务写路径，等待 TASK-03 授权和 TASK-05 生命周期接线。

#![allow(dead_code)] // 分片交付：凭据接口先完成，授权/存储入口仍关闭。

use crate::managed_history_crypto::{CryptoError, DataKey};
use uuid::Uuid;

pub(crate) const SERVICE: &str = "com.agentisland.managed-history.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyStoreError {
    InvalidProjectId,
    AlreadyExists,
    Missing,
    InvalidKey,
    KeychainUnavailable,
    UnsupportedPlatform,
    Internal,
}

impl KeyStoreError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidProjectId => "invalid_project_id",
            Self::AlreadyExists => "project_key_already_exists",
            Self::Missing => "project_key_missing",
            Self::InvalidKey => "project_key_invalid",
            Self::KeychainUnavailable => "project_keychain_unavailable",
            Self::UnsupportedPlatform => "project_key_store_unsupported",
            Self::Internal => "project_key_store_failed",
        }
    }
}

impl std::fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidProjectId => "项目身份无效，无法访问自有执行历史密钥",
            Self::AlreadyExists => "该项目已有自有执行历史密钥，未覆盖原密钥",
            Self::Missing => "项目自有执行历史密钥不存在；密文仍保留，未回退明文",
            Self::InvalidKey => "项目自有执行历史密钥格式无效；密文仍保留",
            Self::KeychainUnavailable => {
                "macOS 钥匙串不可用或拒绝访问；请解锁钥匙串并允许 AgentIsland 访问后重试，历史密文仍保留"
            }
            Self::UnsupportedPlatform => "本平台尚未接入自有执行历史安全凭据库，写入已关闭",
            Self::Internal => "项目自有执行历史密钥操作失败，密文仍保留",
        })
    }
}

pub(crate) trait ProjectKeyStore: Send + Sync {
    /// 仅在显式首次启用且确认项目没有既有历史时调用；密钥缺失时不得用它恢复。
    /// 生成独立随机密钥并以 create-only 语义发布；重复项目身份不得覆盖旧条目。
    fn create(&self, project_id: &str) -> Result<(), KeyStoreError>;
    fn load(&self, project_id: &str) -> Result<DataKey, KeyStoreError>;
    fn delete(&self, project_id: &str) -> Result<(), KeyStoreError>;
}

pub(crate) struct PlatformProjectKeyStore;

impl ProjectKeyStore for PlatformProjectKeyStore {
    fn create(&self, project_id: &str) -> Result<(), KeyStoreError> {
        let account = account_for_project(project_id)?;
        #[cfg(target_os = "macos")]
        {
            let key = DataKey::generate().map_err(map_crypto_error)?;
            platform::create(&account, &key)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            Err(KeyStoreError::UnsupportedPlatform)
        }
    }

    fn load(&self, project_id: &str) -> Result<DataKey, KeyStoreError> {
        let account = account_for_project(project_id)?;
        #[cfg(target_os = "macos")]
        {
            platform::load(&account)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            Err(KeyStoreError::UnsupportedPlatform)
        }
    }

    fn delete(&self, project_id: &str) -> Result<(), KeyStoreError> {
        let account = account_for_project(project_id)?;
        #[cfg(target_os = "macos")]
        {
            platform::delete(&account)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            Err(KeyStoreError::UnsupportedPlatform)
        }
    }
}

fn map_crypto_error(error: CryptoError) -> KeyStoreError {
    match error {
        CryptoError::InvalidKey => KeyStoreError::InvalidKey,
        _ => KeyStoreError::Internal,
    }
}

fn account_for_project(project_id: &str) -> Result<String, KeyStoreError> {
    let parsed = Uuid::parse_str(project_id).map_err(|_| KeyStoreError::InvalidProjectId)?;
    if parsed.to_string() != project_id {
        return Err(KeyStoreError::InvalidProjectId);
    }
    Ok(format!("project-key.v1:{project_id}"))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{DataKey, KeyStoreError, SERVICE};
    use core_foundation::data::CFData;
    use security_framework::base::Error;
    use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass};
    use security_framework::passwords::{
        delete_generic_password, generic_password, PasswordOptions,
    };
    use security_framework_sys::base::{errSecAuthFailed, errSecDuplicateItem, errSecItemNotFound};
    use zeroize::Zeroizing;

    /// security-framework-sys 未导出 `errSecInteractionNotAllowed`；值来自 Apple SecBase.h.
    const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

    fn map_error(error: Error) -> KeyStoreError {
        if error.code() == errSecDuplicateItem {
            return KeyStoreError::AlreadyExists;
        }
        if error.code() == errSecItemNotFound {
            return KeyStoreError::Missing;
        }
        if error.code() == errSecAuthFailed || error.code() == ERR_SEC_INTERACTION_NOT_ALLOWED {
            return KeyStoreError::KeychainUnavailable;
        }
        KeyStoreError::KeychainUnavailable
    }

    /// `ItemAddOptions::add` 调用 `SecItemAdd`；它不会查询后 upsert，也不会覆盖同名 key。
    pub(super) fn create(account: &str, key: &DataKey) -> Result<(), KeyStoreError> {
        let mut options = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(key.as_bytes()),
        });
        options
            .set_service(SERVICE)
            .set_account_name(account)
            .set_label("AgentIsland managed execution history project key");
        options.add().map_err(map_error)
    }

    pub(super) fn load(account: &str) -> Result<DataKey, KeyStoreError> {
        let options = PasswordOptions::new_generic_password(SERVICE, account);
        let bytes = Zeroizing::new(generic_password(options).map_err(map_error)?);
        DataKey::from_slice(bytes.as_slice()).map_err(|_| KeyStoreError::InvalidKey)
    }

    pub(super) fn delete(account: &str) -> Result<(), KeyStoreError> {
        delete_generic_password(SERVICE, account).map_err(map_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_key_service_is_separate_from_remote_notifications() {
        assert_eq!(SERVICE, "com.agentisland.managed-history.v1");
        assert_ne!(SERVICE, crate::secret::SERVICE);
    }

    #[test]
    fn canonical_project_ids_map_to_independent_keychain_accounts() {
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        assert_ne!(
            account_for_project(first).unwrap(),
            account_for_project(second).unwrap()
        );
        assert_eq!(
            account_for_project("../remote.ntfy"),
            Err(KeyStoreError::InvalidProjectId)
        );
        assert_eq!(
            account_for_project("00000000-0000-4000-8000-000000000001 "),
            Err(KeyStoreError::InvalidProjectId)
        );
    }

    #[test]
    fn key_store_errors_explain_recovery_without_including_project_ids_or_key_values() {
        let message = KeyStoreError::KeychainUnavailable.to_string();
        assert!(message.contains("允许 AgentIsland 访问后重试"));
        assert!(!message.contains("project-key.v1"));
        assert!(!message.contains(SERVICE));
        assert_eq!(
            KeyStoreError::AlreadyExists.code(),
            "project_key_already_exists"
        );
        assert_eq!(
            KeyStoreError::UnsupportedPlatform.code(),
            "project_key_store_unsupported"
        );
    }
}

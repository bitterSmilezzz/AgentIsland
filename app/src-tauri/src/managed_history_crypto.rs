//! AgentIsland 自有执行历史的加密原语。
//!
//! 该模块不连接任务 JSON、文件存储或 Tauri 命令。只有真实受管理运行授权与策略
//! 接线完成后，调用方才能把它用于持久化正文。

#![allow(dead_code)] // 分片交付：在 TASK-03 授权及 TASK-05 存储完成前不接可写产品入口。

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"AIHM";
const FORMAT_VERSION: u8 = 1;
const KEY_LEN: usize = 32;
const HEADER_LEN: usize = MAGIC.len() + 1 + NONCE_LEN;
const TAG_LEN: usize = 16;
const MAX_ID_BYTES: usize = 1024;

/// Data key 用零化容器承载，避免普通 Vec/数组在释放时留下长期副本。
pub(crate) struct DataKey(Zeroizing<[u8; KEY_LEN]>);

impl DataKey {
    pub(crate) fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        SystemRandom::new()
            .fill(&mut *bytes)
            .map_err(|_| CryptoError::Internal)?;
        Ok(Self(bytes))
    }

    pub(crate) fn from_slice(bytes: &[u8]) -> Result<Self, CryptoError> {
        let source: &[u8; KEY_LEN] = bytes.try_into().map_err(|_| CryptoError::InvalidKey)?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(source);
        Ok(Self(key))
    }

    pub(crate) fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

/// 内容密文所绑定的完整逻辑身份。正文版本从 1 开始。
#[derive(Clone, Copy)]
pub(crate) struct ContentIdentity<'a> {
    pub(crate) project_id: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) artifact_id: &'a str,
    pub(crate) content_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CryptoError {
    NotEnvelope,
    UnsupportedFormat,
    Truncated,
    InvalidIdentity,
    InvalidKey,
    AuthenticationFailed,
    Internal,
}

impl CryptoError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NotEnvelope => "not_managed_history_envelope",
            Self::UnsupportedFormat => "unsupported_managed_history_format",
            Self::Truncated => "truncated_managed_history_envelope",
            Self::InvalidIdentity => "invalid_managed_history_identity",
            Self::InvalidKey => "invalid_managed_history_key",
            Self::AuthenticationFailed => "managed_history_authentication_failed",
            Self::Internal => "managed_history_crypto_failed",
        }
    }
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotEnvelope => "内容不是 AgentIsland 自有执行历史密文",
            Self::UnsupportedFormat => "自有执行历史密文格式暂不支持",
            Self::Truncated => "自有执行历史密文不完整",
            Self::InvalidIdentity => "自有执行历史身份无效",
            Self::InvalidKey => "自有执行历史密钥格式无效",
            Self::AuthenticationFailed => "自有执行历史认证失败，内容或身份可能已变化",
            Self::Internal => "自有执行历史加密操作失败",
        })
    }
}

/// 加密正文。身份以长度前缀编码进 AAD，防止不同字段拼接产生歧义。
pub(crate) fn seal(
    key: &DataKey,
    identity: &ContentIdentity<'_>,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let aad = aad_for(identity)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| CryptoError::Internal)?;

    let unbound =
        UnboundKey::new(&AES_256_GCM, key.as_bytes()).map_err(|_| CryptoError::Internal)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut ciphertext = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut ciphertext)
        .map_err(|_| CryptoError::Internal)?;

    let capacity = HEADER_LEN
        .checked_add(ciphertext.len())
        .ok_or(CryptoError::Internal)?;
    let mut envelope = Vec::with_capacity(capacity);
    envelope.extend_from_slice(MAGIC);
    envelope.push(FORMAT_VERSION);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

/// 只有完整 AEAD 认证成功后才返回正文；拒绝时调用方仍持有原始密文。
pub(crate) fn open(
    key: &DataKey,
    identity: &ContentIdentity<'_>,
    envelope: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if envelope.len() < MAGIC.len() {
        return Err(CryptoError::Truncated);
    }
    if &envelope[..MAGIC.len()] != MAGIC {
        return Err(CryptoError::NotEnvelope);
    }
    if envelope.len() < HEADER_LEN + TAG_LEN {
        return Err(CryptoError::Truncated);
    }
    if envelope[MAGIC.len()] != FORMAT_VERSION {
        return Err(CryptoError::UnsupportedFormat);
    }

    let aad = aad_for(identity)?;
    let nonce_start = MAGIC.len() + 1;
    let nonce_bytes: [u8; NONCE_LEN] = envelope[nonce_start..HEADER_LEN]
        .try_into()
        .map_err(|_| CryptoError::Truncated)?;
    let unbound =
        UnboundKey::new(&AES_256_GCM, key.as_bytes()).map_err(|_| CryptoError::Internal)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut ciphertext = envelope[HEADER_LEN..].to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::from(aad.as_slice()), &mut ciphertext)
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    Ok(plaintext.to_vec())
}

fn aad_for(identity: &ContentIdentity<'_>) -> Result<Vec<u8>, CryptoError> {
    if identity.content_version == 0 {
        return Err(CryptoError::InvalidIdentity);
    }
    let fields = [
        identity.project_id.as_bytes(),
        identity.run_id.as_bytes(),
        identity.artifact_id.as_bytes(),
    ];
    if fields
        .iter()
        .any(|field| field.is_empty() || field.len() > MAX_ID_BYTES)
    {
        return Err(CryptoError::InvalidIdentity);
    }

    let mut aad =
        Vec::with_capacity(4 + 1 + 3 * 4 + fields.iter().map(|f| f.len()).sum::<usize>() + 8);
    aad.extend_from_slice(MAGIC);
    aad.push(FORMAT_VERSION);
    for field in fields {
        let length = u32::try_from(field.len()).map_err(|_| CryptoError::InvalidIdentity)?;
        aad.extend_from_slice(&length.to_be_bytes());
        aad.extend_from_slice(field);
    }
    aad.extend_from_slice(&identity.content_version.to_be_bytes());
    Ok(aad)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"plan: never persist this as plaintext";

    fn key() -> DataKey {
        DataKey::from_slice(&[0x5a; KEY_LEN]).unwrap()
    }

    fn identity() -> ContentIdentity<'static> {
        ContentIdentity {
            project_id: "project-1",
            run_id: "run-2",
            artifact_id: "artifact-3",
            content_version: 4,
        }
    }

    #[test]
    fn envelope_round_trips_without_exposing_plaintext() {
        let identity = identity();
        let envelope = seal(&key(), &identity, SECRET).unwrap();
        assert_eq!(&envelope[..MAGIC.len()], MAGIC);
        assert!(!envelope
            .windows(SECRET.len())
            .any(|window| window == SECRET));
        assert_eq!(open(&key(), &identity, &envelope).unwrap(), SECRET);
    }

    #[test]
    fn each_seal_uses_a_fresh_random_nonce() {
        let identity = identity();
        let first = seal(&key(), &identity, SECRET).unwrap();
        let second = seal(&key(), &identity, SECRET).unwrap();
        assert_ne!(first, second);
        assert_eq!(open(&key(), &identity, &first).unwrap(), SECRET);
        assert_eq!(open(&key(), &identity, &second).unwrap(), SECRET);
    }

    #[test]
    fn ciphertext_tampering_wrong_key_and_wrong_identity_fail_closed() {
        let identity = identity();
        let envelope = seal(&key(), &identity, SECRET).unwrap();

        let mut changed = envelope.clone();
        *changed.last_mut().unwrap() ^= 0x80;
        assert_eq!(
            open(&key(), &identity, &changed),
            Err(CryptoError::AuthenticationFailed)
        );
        assert_eq!(
            open(
                &DataKey::from_slice(&[0xa5; KEY_LEN]).unwrap(),
                &identity,
                &envelope
            ),
            Err(CryptoError::AuthenticationFailed)
        );

        let wrong_project = ContentIdentity {
            project_id: "project-2",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_project, &envelope),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_run = ContentIdentity {
            run_id: "run-9",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_run, &envelope),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_artifact = ContentIdentity {
            artifact_id: "artifact-8",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_artifact, &envelope),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_version = ContentIdentity {
            content_version: 5,
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_version, &envelope),
            Err(CryptoError::AuthenticationFailed)
        );
    }

    #[test]
    fn unknown_format_foreign_data_and_truncation_are_distinct() {
        let identity = identity();
        assert_eq!(
            open(&key(), &identity, b"foreign"),
            Err(CryptoError::NotEnvelope)
        );
        assert_eq!(
            open(&key(), &identity, b"AIHM"),
            Err(CryptoError::Truncated)
        );
        let mut unknown = seal(&key(), &identity, SECRET).unwrap();
        unknown[MAGIC.len()] = FORMAT_VERSION + 1;
        assert_eq!(
            open(&key(), &identity, &unknown),
            Err(CryptoError::UnsupportedFormat)
        );
        let truncated = seal(&key(), &identity, SECRET).unwrap();
        assert_eq!(
            open(&key(), &identity, &truncated[..HEADER_LEN + TAG_LEN - 1]),
            Err(CryptoError::Truncated)
        );
    }

    #[test]
    fn empty_identity_fields_and_zero_versions_are_rejected() {
        let empty_project = ContentIdentity {
            project_id: "",
            ..identity()
        };
        assert_eq!(
            seal(&key(), &empty_project, SECRET),
            Err(CryptoError::InvalidIdentity)
        );
        let zero_version = ContentIdentity {
            content_version: 0,
            ..identity()
        };
        assert_eq!(
            seal(&key(), &zero_version, SECRET),
            Err(CryptoError::InvalidIdentity)
        );
    }

    #[test]
    fn key_material_must_have_the_exact_aes_256_length() {
        assert!(matches!(
            DataKey::from_slice(&[0; KEY_LEN - 1]),
            Err(CryptoError::InvalidKey)
        ));
        assert!(matches!(
            DataKey::from_slice(&[0; KEY_LEN + 1]),
            Err(CryptoError::InvalidKey)
        ));
    }
}

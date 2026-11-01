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
    let capacity = HEADER_LEN
        .checked_add(plaintext.len())
        .and_then(|length| length.checked_add(TAG_LEN))
        .ok_or(CryptoError::Internal)?;
    let mut envelope = Vec::with_capacity(capacity);
    envelope.extend_from_slice(MAGIC);
    envelope.push(FORMAT_VERSION);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(plaintext);
    let tag = key
        .seal_in_place_separate_tag(
            nonce,
            Aad::from(aad.as_slice()),
            &mut envelope[HEADER_LEN..],
        )
        .map_err(|_| CryptoError::Internal)?;
    envelope.extend_from_slice(tag.as_ref());
    Ok(envelope)
}

/// 只有完整 AEAD 认证成功后才返回正文；拒绝时调用方仍持有原始密文。
pub(crate) fn open(
    key: &DataKey,
    identity: &ContentIdentity<'_>,
    envelope: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
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
    // The returned buffer is already decrypted in place; keep it in a zeroizing
    // owner and avoid a second full-size plaintext copy.
    let mut plaintext = Zeroizing::new(envelope[HEADER_LEN..].to_vec());
    let authenticated = key
        .open_in_place(nonce, Aad::from(aad.as_slice()), &mut plaintext)
        .map_err(|_| CryptoError::AuthenticationFailed)?;
    let plaintext_len = authenticated.len();
    plaintext.truncate(plaintext_len);
    Ok(plaintext)
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
        assert_eq!(
            open(&key(), &identity, &envelope).unwrap().as_slice(),
            SECRET
        );
    }

    #[test]
    fn each_seal_uses_a_fresh_random_nonce() {
        let identity = identity();
        let first = seal(&key(), &identity, SECRET).unwrap();
        let second = seal(&key(), &identity, SECRET).unwrap();
        assert_ne!(first, second);
        assert_eq!(open(&key(), &identity, &first).unwrap().as_slice(), SECRET);
        assert_eq!(open(&key(), &identity, &second).unwrap().as_slice(), SECRET);
    }

    #[test]
    fn ciphertext_tampering_wrong_key_and_wrong_identity_fail_closed() {
        let identity = identity();
        let envelope = seal(&key(), &identity, SECRET).unwrap();

        let mut changed = envelope.clone();
        *changed.last_mut().unwrap() ^= 0x80;
        assert_eq!(
            open(&key(), &identity, &changed).map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );
        assert_eq!(
            open(
                &DataKey::from_slice(&[0xa5; KEY_LEN]).unwrap(),
                &identity,
                &envelope
            )
            .map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );

        let wrong_project = ContentIdentity {
            project_id: "project-2",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_project, &envelope).map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_run = ContentIdentity {
            run_id: "run-9",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_run, &envelope).map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_artifact = ContentIdentity {
            artifact_id: "artifact-8",
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_artifact, &envelope).map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );
        let wrong_version = ContentIdentity {
            content_version: 5,
            ..identity
        };
        assert_eq!(
            open(&key(), &wrong_version, &envelope).map(|plain| plain.to_vec()),
            Err(CryptoError::AuthenticationFailed)
        );
    }

    #[test]
    fn unknown_format_foreign_data_and_truncation_are_distinct() {
        let identity = identity();
        assert_eq!(
            open(&key(), &identity, b"foreign").map(|plain| plain.to_vec()),
            Err(CryptoError::NotEnvelope)
        );
        assert_eq!(
            open(&key(), &identity, b"AIHM").map(|plain| plain.to_vec()),
            Err(CryptoError::Truncated)
        );
        let mut unknown = seal(&key(), &identity, SECRET).unwrap();
        unknown[MAGIC.len()] = FORMAT_VERSION + 1;
        assert_eq!(
            open(&key(), &identity, &unknown).map(|plain| plain.to_vec()),
            Err(CryptoError::UnsupportedFormat)
        );
        let truncated = seal(&key(), &identity, SECRET).unwrap();
        assert_eq!(
            open(&key(), &identity, &truncated[..HEADER_LEN + TAG_LEN - 1])
                .map(|plain| plain.to_vec()),
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

/// Opt-in, synthetic-only storage/crypto measurement. It writes only under a
/// fresh temporary directory, uses generated test keys, and emits aggregate
/// byte/timing metrics without payloads, IDs, or paths.
#[cfg(all(test, target_os = "macos"))]
mod measurement {
    use super::*;
    use std::fs;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is after Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "agentisland-managed-history-measure-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create isolated measurement directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn identity_parts(index: usize) -> (String, String) {
        (
            format!("synthetic-run-{}", index / 32),
            format!("synthetic-artifact-{index}"),
        )
    }

    fn median_ms(samples: &mut [f64]) -> f64 {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    }

    #[test]
    #[ignore = "Opt-in synthetic storage measurement; reads and writes no user history"]
    fn measure_synthetic_managed_history_storage_and_crypto() {
        let key = DataKey::generate().expect("synthetic key");
        let sizes = [1_024usize, 16_384, 65_536, 262_144, 1_048_576, 4_194_304];
        let crypto_samples = 15usize;
        let crypto = sizes
            .iter()
            .map(|size| {
                let plaintext = vec![0x53; *size];
                let mut seal_ms = Vec::with_capacity(crypto_samples);
                let mut open_ms = Vec::with_capacity(crypto_samples);
                let (run_id, artifact_id) = identity_parts(*size);
                let id = ContentIdentity {
                    project_id: "synthetic-project",
                    run_id: &run_id,
                    artifact_id: &artifact_id,
                    content_version: 1,
                };
                for _ in 0..crypto_samples {
                    let started = Instant::now();
                    let envelope = seal(&key, &id, &plaintext).expect("seal synthetic payload");
                    seal_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
                    assert_eq!(envelope.len(), plaintext.len() + HEADER_LEN + TAG_LEN);

                    let started = Instant::now();
                    let recovered = open(&key, &id, &envelope).expect("open synthetic payload");
                    open_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
                    assert_eq!(recovered.as_slice(), plaintext.as_slice());
                }
                serde_json::json!({
                    "plaintext_bytes": size,
                    "envelope_bytes": size + HEADER_LEN + TAG_LEN,
                    "envelope_overhead_bytes": HEADER_LEN + TAG_LEN,
                    "seal_median_ms": median_ms(&mut seal_ms),
                    "open_median_ms": median_ms(&mut open_ms),
                    "samples": crypto_samples,
                })
            })
            .collect::<Vec<_>>();

        // Every profile stores exactly 1 MiB of synthetic plaintext. Different
        // object counts expose filesystem allocation cost for tiny content.
        let profiles = [
            (1_024usize, 1_024usize),
            (16_384, 64),
            (65_536, 16),
            (262_144, 4),
            (1_048_576, 1),
        ];
        let scratch = Scratch::new();
        let mut storage = Vec::with_capacity(profiles.len());
        for (profile_index, (object_bytes, object_count)) in profiles.into_iter().enumerate() {
            let root = scratch.0.join(format!("profile-{profile_index}"));
            fs::create_dir(&root).expect("create profile directory");
            let plaintext = vec![0x6b; object_bytes];
            let started = Instant::now();
            for index in 0..object_count {
                let (run_id, artifact_id) = identity_parts(profile_index * 2_000 + index);
                let id = ContentIdentity {
                    project_id: "synthetic-project",
                    run_id: &run_id,
                    artifact_id: &artifact_id,
                    content_version: 1,
                };
                let envelope = seal(&key, &id, &plaintext).expect("seal stored payload");
                let target = root.join(format!("{}.blob", uuid::Uuid::new_v4().simple()));
                crate::atomicfile::atomic_create_validated(&target, &envelope, |staged| {
                    let bytes = fs::read(staged)?;
                    let recovered = open(&key, &id, &bytes).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "synthetic envelope invalid")
                    })?;
                    if recovered.as_slice() != plaintext.as_slice() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "synthetic payload mismatch",
                        ));
                    }
                    Ok(())
                })
                .expect("atomically publish validated synthetic envelope");
            }
            let publish_ms = started.elapsed().as_secs_f64() * 1_000.0;
            let mut logical_file_bytes = 0u64;
            let mut allocated_file_bytes = 0u64;
            let mut count = 0usize;
            for entry in fs::read_dir(&root).expect("enumerate synthetic files") {
                let metadata = entry.expect("read synthetic entry").metadata().unwrap();
                logical_file_bytes += metadata.len();
                allocated_file_bytes += metadata.blocks() * 512;
                count += 1;
            }
            let directory_allocated_bytes = fs::metadata(&root).unwrap().blocks() * 512;
            storage.push(serde_json::json!({
                "object_bytes": object_bytes,
                "object_count": object_count,
                "logical_plaintext_bytes": object_bytes * object_count,
                "logical_file_bytes": logical_file_bytes,
                "allocated_file_bytes": allocated_file_bytes,
                "directory_allocated_bytes": directory_allocated_bytes,
                "files_observed": count,
                "file_sync_publish_and_authenticate_ms": publish_ms,
            }));
        }

        println!(
            "{}",
            serde_json::json!({
                "schema_version": 1,
                "measurement": "managed-history-synthetic-v1",
                "crate_version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "crypto": crypto,
                "storage_profiles": storage,
                "notes": [
                    "synthetic payloads only; no user data read",
                    "storage uses production AEAD and atomic create helper",
                    "allocated_file_bytes uses st_blocks multiplied by 512",
                    "parent directory sync and crash durability are not measured",
                    "index, run frequency, backup copies, and app-wide RSS are not measured",
                ],
            })
        );
    }
}

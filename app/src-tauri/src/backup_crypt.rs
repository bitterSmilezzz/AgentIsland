//! 加密备份容器（GW-09「加密备份」）。
//!
//! 口径：
//! * 备份文件里**只有密文**——连文件名与内容都不泄露里面有什么。
//! * **口令不落盘**：密钥由口令经 PBKDF2-HMAC-SHA256 派生，盐随机且**只存文件头**；
//!   口令本身永远不写进任何文件、也不进日志。
//! * 口令错 = 解不开：AEAD 校验失败必须**明确报错**，绝不返回半截明文
//!   （半截明文比报错更危险——用户会以为备份是好的）。
//! * 容器自描述：magic + 版本 + 盐 + nonce，之后是密文与认证标签。
//!   版本号让将来换算法/参数时旧文件仍可识别，而不是被当成「损坏」。
//!
//! 为什么用 ring 而不是自己写：AEAD 与 KDF 是**不该自己实现**的原语。

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};

/// 容器魔数（"AIBK"）。认它是为了**拒绝不是本格式的文件**，而不是让解密失败报「口令错」。
const MAGIC: &[u8; 4] = b"AIBK";
/// 容器版本。1 = PBKDF2-HMAC-SHA256 + AES-256-GCM。
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// PBKDF2 迭代次数。取 210_000（OWASP 2023 对 PBKDF2-HMAC-SHA256 的建议值），
/// 代价是**每次加密/解密要花约 0.2~0.5 秒**——这是刻意的：备份不常做，
/// 而离线猜解口令的成本必须高。
const ITERATIONS: u32 = 210_000;

/// 头部长度：magic(4) + version(1) + salt(16) + nonce(12)
const HEADER_LEN: usize = 4 + 1 + SALT_LEN + NONCE_LEN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupError {
    /// 不是本格式的备份文件（魔数/版本不对）
    NotABackup,
    /// 口令不对，或文件被改动过（AEAD 两者都表现为校验失败）
    DecryptFailed,
    /// 容器被截断
    Truncated,
    /// 内部错误（随机数/派生失败）
    Internal,
}

impl BackupError {
    /// 稳定分类，供界面与门禁断言使用。
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotABackup => "not_a_backup",
            Self::DecryptFailed => "decrypt_failed",
            Self::Truncated => "truncated",
            Self::Internal => "internal",
        }
    }
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotABackup => "这不是 AgentIsland 的加密备份文件",
            Self::DecryptFailed => "口令不对，或文件已被改动",
            Self::Truncated => "备份文件不完整",
            Self::Internal => "加解密内部错误",
        })
    }
}

/// 派生密钥。盐来自文件头，**每次加密都重新随机**。
fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], BackupError> {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        std::num::NonZeroU32::new(ITERATIONS).expect("迭代次数为常量且非零"),
        salt,
        passphrase.as_bytes(),
        &mut key,
    );
    Ok(key)
}

fn seal(key_bytes: &[u8; KEY_LEN], nonce_bytes: &[u8; NONCE_LEN], plaintext: &[u8]) -> Result<Vec<u8>, BackupError> {
    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| BackupError::Internal)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(*nonce_bytes);
    let mut buffer = plaintext.to_vec();
    // AAD 把头部绑进认证范围：改盐/改版本会让解密失败，而不是被当成另一份文件。
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut buffer)
        .map_err(|_| BackupError::Internal)?;
    Ok(buffer)
}

fn open(key_bytes: &[u8; KEY_LEN], nonce_bytes: &[u8; NONCE_LEN], ciphertext: &[u8]) -> Result<Vec<u8>, BackupError> {
    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|_| BackupError::Internal)?;
    let key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(*nonce_bytes);
    let mut buffer = ciphertext.to_vec();
    // 解封失败 = 口令错**或**文件被改过。两者都返回同一个错误：
    // 分开报错会告诉攻击者「口令猜对了，只是文件坏了」。
    // 注意：`open_in_place` **返回明文切片**（认证标签已被摘掉），
    // 直接返回整个 buffer 会把标签一起交出去——那是 16 字节的伪明文。
    let plain = key
        .open_in_place(nonce, Aad::empty(), &mut buffer)
        .map_err(|_| BackupError::DecryptFailed)?;
    Ok(plain.to_vec())
}

/// 把明文封成加密备份容器。
///
/// `passphrase` 只在这次调用里用一次：派生完就不必留在内存里，也从不落盘。
pub fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, BackupError> {
    if passphrase.is_empty() {
        return Err(BackupError::Internal);
    }
    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut salt).map_err(|_| BackupError::Internal)?;
    rng.fill(&mut nonce_bytes).map_err(|_| BackupError::Internal)?;
    let key = derive_key(passphrase, &salt)?;
    let ciphertext = seal(&key, &nonce_bytes, plaintext)?;
    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 打开加密备份容器，还原明文。
pub fn decrypt(container: &[u8], passphrase: &str) -> Result<Vec<u8>, BackupError> {
    if container.len() < HEADER_LEN {
        return Err(BackupError::Truncated);
    }
    if &container[..4] != MAGIC {
        return Err(BackupError::NotABackup);
    }
    if container[4] != VERSION {
        return Err(BackupError::NotABackup);
    }
    let salt = &container[5..5 + SALT_LEN];
    let nonce_bytes: [u8; NONCE_LEN] = container[5 + SALT_LEN..HEADER_LEN]
        .try_into()
        .map_err(|_| BackupError::Truncated)?;
    let key = derive_key(passphrase, salt)?;
    open(&key, &nonce_bytes, &container[HEADER_LEN..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_round_trips_and_the_ciphertext_hides_the_plaintext() {
        let secret = "{\"profile\":\"acme\",\"env\":\"ACME_API_KEY\"}";
        let container = encrypt(secret.as_bytes(), "correct horse").unwrap();
        assert!(!String::from_utf8_lossy(&container).contains("ACME_API_KEY"), "密文里不该出现明文");
        assert_eq!(&container[..4], MAGIC);
        let plain = decrypt(&container, "correct horse").unwrap();
        assert_eq!(String::from_utf8_lossy(&plain), secret);
    }

    #[test]
    fn the_wrong_passphrase_fails_loudly_instead_of_returning_partial_plaintext() {
        let container = encrypt(b"top secret payload", "right").unwrap();
        let error = decrypt(&container, "wrong").unwrap_err();
        assert_eq!(error, BackupError::DecryptFailed);
        assert_eq!(error.code(), "decrypt_failed");
    }

    #[test]
    fn a_tampered_container_is_rejected() {
        let mut container = encrypt(b"top secret payload", "right").unwrap();
        let last = container.len() - 1;
        container[last] ^= 0x01;
        assert_eq!(decrypt(&container, "right").unwrap_err(), BackupError::DecryptFailed);
    }

    #[test]
    fn a_foreign_file_is_rejected_before_any_key_work() {
        // 够长但魔数不对 → 「不是本格式」；连头都不够 → 「不完整」。两者分开说。
        let foreign = b"this is definitely not an AgentIsland backup container";
        assert_eq!(decrypt(foreign, "x").unwrap_err(), BackupError::NotABackup);
        assert_eq!(decrypt(b"short", "x").unwrap_err(), BackupError::Truncated);
    }

    #[test]
    fn every_encryption_uses_a_fresh_salt_and_nonce() {
        let first = encrypt(b"same plaintext", "same passphrase").unwrap();
        let second = encrypt(b"same plaintext", "same passphrase").unwrap();
        assert_ne!(first, second, "同样的明文与口令不该产出同样的密文");
        // 但两者都能解开，且解出同样的明文
        assert_eq!(decrypt(&first, "same passphrase").unwrap(), b"same plaintext");
        assert_eq!(decrypt(&second, "same passphrase").unwrap(), b"same plaintext");
    }
}

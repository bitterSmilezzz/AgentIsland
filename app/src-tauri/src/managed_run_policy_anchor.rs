//! Native credential-store anchor for the managed-run policy document.
//!
//! Platform credential stores do not expose compare-and-swap writes through
//! the cross-platform API. Callers must serialize every read/compare/write
//! transaction with the policy store's OS file lock. Only macOS is enabled
//! until Windows and Linux receive their own native acceptance.

use serde::{Deserialize, Serialize};

const ANCHOR_FORMAT_VERSION: u32 = 1;
#[cfg(target_os = "macos")]
const KEYRING_SERVICE: &str = "dev.agentisland.managed-run-policy";
#[cfg(target_os = "macos")]
const KEYRING_ACCOUNT: &str = "authorization-anchor-v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub(crate) enum AnchorPhase {
    Pending,
    Committed,
}

/// A tiny value stored in the operating system credential store. The digest
/// binds the exact policy bytes; epoch prevents a fresh installation from
/// accepting an older policy document after an explicit re-confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorRecord {
    format_version: u32,
    pub(crate) epoch: String,
    pub(crate) revision: u64,
    pub(crate) policy_digest: [u8; 32],
    pub(crate) phase: AnchorPhase,
}

impl AnchorRecord {
    pub(crate) fn new(
        epoch: String,
        revision: u64,
        policy_digest: [u8; 32],
        phase: AnchorPhase,
    ) -> Self {
        Self {
            format_version: ANCHOR_FORMAT_VERSION,
            epoch,
            revision,
            policy_digest,
            phase,
        }
    }

    pub(crate) fn decode(bytes: &str) -> Result<Self, AnchorError> {
        let record: Self = serde_json::from_str(bytes).map_err(|_| AnchorError::Corrupt)?;
        if record.format_version != ANCHOR_FORMAT_VERSION
            || record.revision == 0
            || uuid::Uuid::parse_str(&record.epoch).is_err()
        {
            return Err(AnchorError::Corrupt);
        }
        Ok(record)
    }

    pub(crate) fn encode(&self) -> Result<String, AnchorError> {
        serde_json::to_string(self).map_err(|_| AnchorError::Corrupt)
    }

    pub(crate) fn committed(&self) -> Self {
        let mut committed = self.clone();
        committed.phase = AnchorPhase::Committed;
        committed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnchorError {
    Unavailable,
    Corrupt,
}

pub(crate) trait CredentialAnchor: Send + Sync {
    fn supported(&self) -> bool;
    fn read_raw(&self) -> Result<Option<String>, AnchorError>;
    fn write_raw(&self, value: &str) -> Result<(), AnchorError>;
}

#[derive(Debug, Default)]
pub(crate) struct NativeCredentialAnchor;

impl CredentialAnchor for NativeCredentialAnchor {
    fn supported(&self) -> bool {
        // Other platforms remain fail-closed until their native credential
        // store behavior has its own L5 acceptance.
        cfg!(target_os = "macos")
    }

    fn read_raw(&self) -> Result<Option<String>, AnchorError> {
        if !self.supported() {
            return Err(AnchorError::Unavailable);
        }
        #[cfg(target_os = "macos")]
        {
            let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
                .map_err(|_| AnchorError::Unavailable)?;
            match entry.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(AnchorError::Unavailable),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(AnchorError::Unavailable)
        }
    }

    fn write_raw(&self, value: &str) -> Result<(), AnchorError> {
        if !self.supported() {
            return Err(AnchorError::Unavailable);
        }
        #[cfg(target_os = "macos")]
        {
            let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
                .map_err(|_| AnchorError::Unavailable)?;
            entry
                .set_password(value)
                .map_err(|_| AnchorError::Unavailable)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = value;
            Err(AnchorError::Unavailable)
        }
    }
}

#[cfg(test)]
pub(crate) struct FileCredentialAnchor {
    path: std::path::PathBuf,
}

#[cfg(test)]
impl FileCredentialAnchor {
    pub(crate) fn for_policy_path(path: &std::path::Path) -> Self {
        Self {
            path: path.with_extension("test-credential-anchor"),
        }
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
impl CredentialAnchor for FileCredentialAnchor {
    fn supported(&self) -> bool {
        true
    }

    fn read_raw(&self) -> Result<Option<String>, AnchorError> {
        match std::fs::read_to_string(&self.path) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(AnchorError::Unavailable),
        }
    }

    fn write_raw(&self, value: &str) -> Result<(), AnchorError> {
        let parent = self.path.parent().ok_or(AnchorError::Unavailable)?;
        std::fs::create_dir_all(parent).map_err(|_| AnchorError::Unavailable)?;
        let staged = self.path.with_extension("test-credential-anchor.next");
        std::fs::write(&staged, value).map_err(|_| AnchorError::Unavailable)?;
        std::fs::rename(&staged, &self.path).map_err(|_| AnchorError::Unavailable)
    }
}

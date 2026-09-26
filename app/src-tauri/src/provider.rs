//! Codex profile groundwork. This module never reads or writes the user's
//! installed configuration on its own; Phase 2 supplies a chosen destination.

use serde::{Serialize, Serializer};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::{value, DocumentMut};

const REDACTED: &str = "••••";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// An outbound secret. There is deliberately no raw-string accessor or
/// `Deserialize`: only the provider implementation may hold plaintext input.
pub(crate) struct MaskedSecret(Box<str>);

impl MaskedSecret {
    pub(crate) fn from_plaintext(value: String) -> Self {
        Self(value.into_boxed_str())
    }

    fn redacted(&self) -> &'static str {
        if self.0.is_empty() {
            "—"
        } else {
            REDACTED
        }
    }
}

impl fmt::Debug for MaskedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.redacted())
    }
}

impl fmt::Display for MaskedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.redacted())
    }
}

impl Serialize for MaskedSecret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.redacted())
    }
}

/// The only shape intended for an outbound profile summary. In particular,
/// raw TOML and plaintext credentials are absent from this type.
#[derive(Debug, Serialize)]
pub(crate) struct ProviderProfilePreview {
    pub id: String,
    pub name: String,
    pub credential: MaskedSecret,
}

struct StagedFile(PathBuf);

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Replace a provider configuration file only after its staged bytes pass the
/// caller's parser. The target must already have a parent directory. A failed
/// write or validation leaves the old file byte-for-byte unchanged. The staged
/// file lives beside the target, so rename is on the same filesystem.
pub(crate) fn atomic_replace_validated<F>(
    target: &Path,
    bytes: &[u8],
    validate: F,
) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "provider target needs a parent",
            )
        })?;
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "provider target needs a filename",
        )
    })?;
    if fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "provider target must be a regular file",
        ));
    }

    let mut staged = None;
    for _ in 0..32 {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{}.agentisland-{}-{serial}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                staged = Some((StagedFile(path), file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (guard, mut file): (StagedFile, File) = staged.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "provider staging name exhausted",
        )
    })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    validate(&guard.0)?;
    fs::rename(&guard.0, target)?;
    Ok(())
}

fn invalid_toml() -> io::Error {
    // Parser diagnostics may quote the input line, which could contain a key.
    io::Error::new(io::ErrorKind::InvalidData, "invalid provider TOML")
}

fn replace_value_preserving_decor(doc: &mut DocumentMut, key: &str, new_value: &str) {
    let decor = doc
        .get(key)
        .and_then(|item| item.as_value())
        .map(|value| value.decor().clone());
    let mut item = value(new_value);
    if let Some(decor) = decor {
        *item
            .as_value_mut()
            .expect("toml_edit::value returns a value")
            .decor_mut() = decor;
    }
    doc[key] = item;
}

/// Update the native Codex profile layer while retaining unrelated TOML
/// comments, key order, plugin tables, and arrays. Provider definitions belong
/// in the base config; this layer only selects their name. A running Codex
/// process must be restarted separately before this change can take effect.
pub(crate) fn update_codex_profile(
    target: &Path,
    model: &str,
    model_provider: Option<&str>,
) -> io::Result<()> {
    if !target
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name.ends_with(".config.toml") && name != ".config.toml")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a Codex profile path",
        ));
    }
    if model.trim().is_empty() || model_provider.is_some_and(|p| p.trim().is_empty()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty Codex profile field",
        ));
    }

    let original = match fs::read_to_string(target) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut doc: DocumentMut = original.parse().map_err(|_| invalid_toml())?;
    replace_value_preserving_decor(&mut doc, "model", model);
    if let Some(provider) = model_provider {
        replace_value_preserving_decor(&mut doc, "model_provider", provider);
    } else {
        // No override means inherit from the base config, not keep the previous one.
        doc.as_table_mut().remove("model_provider");
    }
    let updated = doc.to_string();
    atomic_replace_validated(target, updated.as_bytes(), |staged| {
        let text = fs::read_to_string(staged)?;
        text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agentisland-provider-test-{}-{stamp}-{serial}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn only_target_remains(root: &Path, target_name: &str) {
        let entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![target_name],
            "staging file leaked: {entries:?}"
        );
    }

    #[test]
    fn preview_never_serializes_or_formats_plaintext() {
        let secret = "synthetic-provider-secret-value".to_string(); // nosec: synthetic masking fixture, not a credential
        let preview = ProviderProfilePreview {
            id: "synthetic".into(),
            name: "Fixture profile".into(),
            credential: MaskedSecret::from_plaintext(secret.clone()),
        };
        for output in [
            serde_json::to_string(&preview).unwrap(),
            format!("{preview:?}"),
            format!("{}", preview.credential),
        ] {
            assert!(
                !output.contains(&secret),
                "outbound profile exposed its credential"
            );
            assert!(output.contains(REDACTED));
        }
    }

    #[test]
    fn successful_replacement_validates_staged_bytes_then_replaces_atomically() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "model = 'before'\n").unwrap();
        atomic_replace_validated(&target, b"model = 'after'\n", |stage| {
            assert_eq!(fs::read_to_string(&target)?, "model = 'before'\n");
            assert_eq!(fs::read_to_string(stage)?, "model = 'after'\n");
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'after'\n");
        only_target_remains(&sandbox.0, "fixture.config.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_staged_config_preserves_original_and_cleans_up() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "model = 'before'\n").unwrap();
        let result = atomic_replace_validated(&target, b"not valid TOML = [", |stage| {
            assert_eq!(fs::read_to_string(stage)?, "not valid TOML = [");
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic parse rejection",
            ))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'before'\n");
        only_target_remains(&sandbox.0, "fixture.config.toml");
    }

    #[test]
    fn invalid_new_config_does_not_create_the_target() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        let result = atomic_replace_validated(&target, b"broken", |_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "synthetic parse rejection",
            ))
        });
        assert!(result.is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 0);
    }

    #[test]
    fn symlink_target_is_rejected_without_touching_its_destination() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let sandbox = Sandbox::new();
            let destination = sandbox.0.join("real.config.toml");
            fs::write(&destination, "keep").unwrap();
            let link = sandbox.0.join("fixture.config.toml");
            symlink(&destination, &link).unwrap();
            let result = atomic_replace_validated(&link, b"replace", |_| Ok(()));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
            assert_eq!(fs::read_to_string(destination).unwrap(), "keep");
        }
    }

    #[test]
    fn codex_profile_update_preserves_unrelated_formatting_and_comments() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        let before = "# Keep this comment\nmodel = 'old-model' # chosen by hand\nmodel_provider = 'old-provider' # account selection\n\n[plugins.\"fixture@marketplace\"]\nenabled = true\npaths = [\n  'alpha',\n  'beta',\n]\n";
        fs::write(&target, before).unwrap();
        update_codex_profile(&target, "new-model", Some("official-provider")).unwrap();
        let after = fs::read_to_string(&target).unwrap();
        assert!(after.contains("# Keep this comment"));
        assert!(
            after.contains("# chosen by hand"),
            "inline model comment was lost: {after}"
        );
        assert!(
            after.contains("# account selection"),
            "inline provider comment was lost: {after}"
        );
        assert!(after.contains("[plugins.\"fixture@marketplace\"]\nenabled = true\npaths = [\n  'alpha',\n  'beta',\n]"));
        let doc: DocumentMut = after.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new-model"));
        assert_eq!(doc["model_provider"].as_str(), Some("official-provider"));
        only_target_remains(&sandbox.0, "fixture.config.toml");
    }

    #[test]
    fn codex_profile_update_creates_a_missing_native_profile() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        update_codex_profile(&target, "new-model", Some("official-provider")).unwrap();
        let text = fs::read_to_string(&target).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new-model"));
        assert_eq!(doc["model_provider"].as_str(), Some("official-provider"));
        only_target_remains(&sandbox.0, "fixture.config.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_existing_profile_and_auth_path_cannot_be_overwritten() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "invalid = [").unwrap();
        assert_eq!(
            update_codex_profile(&target, "new-model", None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "invalid = [");
        let auth = sandbox.0.join("auth.json");
        fs::write(&auth, "leave this file alone").unwrap();
        assert_eq!(
            update_codex_profile(&auth, "new-model", None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read_to_string(auth).unwrap(), "leave this file alone");
    }

    #[test]
    fn clearing_provider_override_keeps_the_rest_of_the_profile() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(
            &target,
            "# keep\nmodel = 'old'\nmodel_provider = 'old-provider'\n",
        )
        .unwrap();
        update_codex_profile(&target, "new", None).unwrap();
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("# keep\n"));
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new"));
        assert!(doc.get("model_provider").is_none());
    }
}

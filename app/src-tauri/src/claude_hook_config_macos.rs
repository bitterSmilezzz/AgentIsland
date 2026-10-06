use super::*;
use crate::skill_files as files;
use std::{fs::File, io::Read, os::unix::fs::MetadataExt};
#[derive(Clone, PartialEq, Eq, Serialize)]
struct FileId {
    dev: u64,
    ino: u64,
    birth: std::time::SystemTime,
}
fn id(file: &File) -> Result<FileId, Error> {
    let meta = file.metadata().map_err(|_| fail("invalid"))?;
    Ok(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
        birth: meta.created().map_err(|_| fail("invalid"))?,
    })
}
fn record_identity(file: &File) -> Result<FileId, Error> {
    let meta = file.metadata().map_err(|_| fail("invalid"))?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
        return Err(fail("invalid"));
    }
    id(file)
}
struct Anchor {
    path: PathBuf,
    dir: File,
    chain: Vec<FileId>,
}
impl Anchor {
    fn open(path: &Path) -> Result<Self, Error> {
        use std::path::Component;
        if !path.is_absolute()
            || path.components().count() > 32
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(fail("invalid"));
        }
        let mut dir = files::open_dir(Path::new("/")).map_err(|_| fail("invalid"))?;
        let mut chain = vec![];
        for part in path.components() {
            if let Component::Normal(part) = part {
                dir = files::child(&dir, part.to_str().ok_or_else(|| fail("invalid"))?, true)
                    .map_err(|_| fail("missing"))?;
                chain.push(id(&dir)?);
            }
        }
        let meta = dir.metadata().map_err(|_| fail("invalid"))?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            return Err(fail("invalid"));
        }
        Ok(Self {
            path: path.into(),
            dir,
            chain,
        })
    }
    fn current(&self) -> Result<(), Error> {
        if Self::open(&self.path)?.chain != self.chain {
            return Err(fail("changed"));
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Image {
    bytes: Option<Vec<u8>>,
    file: Option<FileId>,
    stamp: Option<String>,
}
fn read(dir: &File, name: &str) -> Result<Image, Error> {
    if !files::exists(dir, name).map_err(|_| fail("invalid"))? {
        return Ok(Image {
            bytes: None,
            file: None,
            stamp: None,
        });
    }
    let mut file = files::child(dir, name, false).map_err(|_| fail("invalid"))?;
    let meta = file.metadata().map_err(|_| fail("invalid"))?;
    let cap = if name == "receipt.json" {
        CAP * 2 + 8192
    } else {
        CAP
    };
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
        || meta.len() > cap as u64
    {
        return Err(fail("invalid"));
    }
    let identity = id(&file)?;
    let stamp = files::file_stamp(&file).map_err(|_| fail("invalid"))?;
    let mut bytes = vec![];
    file.by_ref()
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("invalid"))?;
    if bytes.len() > cap
        || files::file_stamp(&file).map_err(|_| fail("changed"))? != stamp
        || id(&files::child(dir, name, false).map_err(|_| fail("changed"))?)? != identity
    {
        return Err(fail("changed"));
    }
    Ok(Image {
        bytes: Some(bytes),
        file: Some(identity),
        stamp: Some(stamp),
    })
}
fn bytes_hash(bytes: Option<&[u8]>) -> String {
    match bytes {
        None => hash(b"absent"),
        Some(bytes) => {
            let mut data = b"present:".to_vec();
            data.extend_from_slice(bytes);
            hash(&data)
        }
    }
}
fn owner(image: &Image) -> Result<Option<Owner>, Error> {
    match &image.bytes {
        None => Ok(None),
        Some(bytes) => {
            let value =
                crate::claude_plan_capture::strict_json(bytes).map_err(|_| fail("invalid"))?;
            let owner: Owner = serde_json::from_value(value).map_err(|_| fail("invalid"))?;
            validate_owner(&owner)?;
            Ok(Some(owner))
        }
    }
}
fn validate_owner(owner: &Owner) -> Result<(), Error> {
    let suffix = format!("' {}", crate::claude_plan_receiver::FLAG);
    let path = owner
        .command
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix(&suffix))
        .map(|s| s.replace("'\\''", "'"));
    let canonical = path
        .as_ref()
        .filter(|p| Path::new(p).is_absolute())
        .map(|p| {
            format!(
                "'{}' {}",
                p.replace('\'', "'\\''"),
                crate::claude_plan_receiver::FLAG
            )
        });
    if canonical.as_deref() != Some(owner.command.as_str())
        || owner.schema_version != 1
        || owner.command.len() > 4096
        || !owner.command.starts_with('\'')
        || owner.command.chars().any(char::is_control)
        || !owner.command.ends_with(crate::claude_plan_receiver::FLAG)
        || crate::private_text::known_private(&owner.command)
    {
        return Err(fail("invalid"));
    }
    Ok(())
}
fn backup(image: &Image) -> Result<Backup, Error> {
    let value = crate::claude_plan_capture::strict_json(
        image.bytes.as_deref().ok_or_else(|| fail("invalid"))?,
    )
    .map_err(|_| fail("invalid"))?;
    let backup: Backup = serde_json::from_value(value).map_err(|_| fail("invalid"))?;
    if backup.schema_version != 1 {
        return Err(fail("invalid"));
    }
    if let Some(owner) = &backup.owner_before {
        validate_owner(owner)?;
    }
    let value = decode(backup.before.as_deref().map(str::as_bytes))?;
    if !safe(&value, false) {
        return Err(fail("private"));
    }
    Ok(backup)
}
fn owner_bytes(owner: Option<&Owner>) -> Result<Option<Vec<u8>>, Error> {
    owner
        .map(|owner| serde_json::to_vec(owner).map_err(|_| fail("invalid")))
        .transpose()
}
struct Snapshot {
    config: Anchor,
    records: Option<Anchor>,
    parent: Anchor,
    config_image: Image,
    owner_image: Image,
    value: Value,
    owner: Option<Owner>,
    installed: bool,
    revision: String,
}
struct Cleanup {
    name: String,
    identity: FileId,
    receipt: Image,
    trash_parent: Anchor,
    trash: Option<Anchor>,
}
struct Pending {
    id: String,
    created: Instant,
    snapshot: Snapshot,
    after: Option<Vec<u8>>,
    after_owner: Option<Owner>,
    installed: bool,
    action: Action,
    cleanup: Option<Cleanup>,
}
pub(crate) struct Store {
    config: PathBuf,
    parent: PathBuf,
    binary: PathBuf,
    trash: PathBuf,
    pending: Option<Pending>,
}
impl Store {
    pub(crate) fn new(config: PathBuf, parent: PathBuf, binary: PathBuf, trash: PathBuf) -> Self {
        Self {
            config,
            parent,
            binary,
            trash,
            pending: None,
        }
    }
    pub(crate) fn at_default() -> Result<Self, Error> {
        let home = dirs::home_dir().ok_or_else(|| fail("missing"))?;
        Ok(Self::new(
            home.join(".claude"),
            crate::settings::config_dir(),
            std::env::current_exe().map_err(|_| fail("invalid"))?,
            home.join(".Trash"),
        ))
    }
    fn command(&self) -> Result<String, Error> {
        let meta = std::fs::symlink_metadata(&self.binary).map_err(|_| fail("invalid"))?;
        let path = self.binary.to_str().ok_or_else(|| fail("invalid"))?;
        if !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.mode() & 0o111 == 0
            || !self.binary.is_absolute()
            || path.len() > 2048
            || path.chars().any(char::is_control)
            || crate::private_text::known_private(path)
        {
            return Err(fail("invalid"));
        }
        Ok(format!(
            "'{}' {}",
            path.replace('\'', "'\\''"),
            crate::claude_plan_receiver::FLAG
        ))
    }
    fn binary_revision(&self) -> Result<String, Error> {
        self.command()?;
        let meta = std::fs::symlink_metadata(&self.binary).map_err(|_| fail("invalid"))?;
        Ok(hash(
            &serde_json::to_vec(&serde_json::json!([
                meta.dev(),
                meta.ino(),
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.mode(),
                meta.created().map_err(|_| fail("invalid"))?
            ]))
            .map_err(|_| fail("invalid"))?,
        ))
    }
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let config = Anchor::open(&self.config)?;
        let parent = Anchor::open(&self.parent)?;
        let records =
            if files::exists(&parent.dir, "claude-plan-hooks").map_err(|_| fail("invalid"))? {
                Some(Anchor::open(&self.parent.join("claude-plan-hooks"))?)
            } else {
                None
            };
        let config_image = read(&config.dir, "settings.json")?;
        let value = decode(config_image.bytes.as_deref())?;
        let owner_image = match &records {
            None => Image {
                bytes: None,
                file: None,
                stamp: None,
            },
            Some(records) => read(&records.dir, "owner.json")?,
        };
        let owner = owner(&owner_image)?;
        let installed = installed(&value, owner.as_ref(), &self.command()?)?;
        config.current()?;
        parent.current()?;
        if let Some(records) = &records {
            records.current()?;
        }
        let revision = hash(
            &serde_json::to_vec(&serde_json::json!([
                bytes_hash(config_image.bytes.as_deref()),
                bytes_hash(owner_image.bytes.as_deref()),
                self.command()?,
                self.binary_revision()?,
                config.chain,
                parent.chain,
                records.as_ref().map(|r| &r.chain),
                config_image.file,
                config_image.stamp,
                owner_image.file,
                owner_image.stamp
            ]))
            .map_err(|_| fail("invalid"))?,
        );
        Ok(Snapshot {
            config,
            records,
            parent,
            config_image,
            owner_image,
            value,
            owner,
            installed,
            revision,
        })
    }
    fn record_ids(snapshot: &Snapshot) -> Result<Vec<String>, Error> {
        let Some(records) = &snapshot.records else {
            return Ok(vec![]);
        };
        let mut ids = vec![];
        for name in files::names(&records.dir).map_err(|_| fail("invalid"))? {
            if name == "owner.json" {
                continue;
            }
            if !name.starts_with("backup-")
                || uuid::Uuid::parse_str(name.trim_start_matches("backup-")).is_err()
            {
                return Err(fail("invalid"));
            }
            let record = files::child(&records.dir, &name, true).map_err(|_| fail("invalid"))?;
            record_identity(&record)?;
            let image = read(&record, "receipt.json")?;
            backup(&image)?;
            ids.push(name);
        }
        if ids.len() > LIMIT {
            return Err(fail("capacity"));
        }
        Ok(ids)
    }
    pub(crate) fn lifecycle_check(
        &self,
        revision: String,
    ) -> impl Fn() -> bool + Send + Sync + 'static {
        // Independent, read-only probe: receiver checks never consume a pending preview.
        let probe = Self::new(
            self.config.clone(),
            self.parent.clone(),
            self.binary.clone(),
            self.trash.clone(),
        );
        move || {
            probe
                .runtime_proof()
                .is_ok_and(|p| p.eligible && p.revision == revision)
                && probe
                    .runtime_preference()
                    .is_ok_and(|p| p.is_some_and(|p| p.enabled && p.binding == revision))
        }
    }
    pub(crate) fn runtime_proof(&self) -> Result<RuntimeProof, Error> {
        let snapshot = self.snapshot()?;
        Ok(RuntimeProof {
            revision: snapshot.revision,
            eligible: snapshot.installed
                && snapshot
                    .owner
                    .as_ref()
                    .is_some_and(|o| self.command().is_ok_and(|command| o.command == command)),
        })
    }
    fn preference(image: &Image) -> Result<Option<RuntimePreference>, Error> {
        let Some(bytes) = image.bytes.as_deref() else {
            return Ok(None);
        };
        if bytes.len() > 1024 {
            return Err(fail("invalid"));
        }
        let value = crate::claude_plan_capture::strict_json(bytes).map_err(|_| fail("invalid"))?;
        let preference: RuntimePreference =
            serde_json::from_value(value).map_err(|_| fail("invalid"))?;
        if preference.schema_version != 1
            || preference.binding.len() != 64
            || !preference.binding.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(fail("invalid"));
        }
        Ok(Some(preference))
    }
    pub(crate) fn runtime_preference(&self) -> Result<Option<RuntimePreference>, Error> {
        let parent = Anchor::open(&self.parent)?;
        let preference = Self::preference(&read(&parent.dir, "claude-plan-runtime.json")?)?;
        parent.current()?;
        Ok(preference)
    }
    pub(crate) fn set_runtime_preference(
        &self,
        enabled: bool,
        revision: &str,
    ) -> Result<(), Error> {
        let snapshot = self.snapshot()?;
        if snapshot.revision != revision || (enabled && !self.runtime_proof()?.eligible) {
            return Err(fail("changed"));
        }
        let previous = read(&snapshot.parent.dir, "claude-plan-runtime.json")?;
        Self::preference(&previous)?;
        let bytes = serde_json::to_vec(&RuntimePreference {
            schema_version: 1,
            enabled,
            binding: revision.into(),
        })
        .map_err(|_| fail("invalid"))?;
        let stage = format!("plan-control-stage-{}", uuid::Uuid::new_v4());
        files::write_new(&snapshot.parent.dir, &stage, &bytes, false)
            .map_err(|_| fail("invalid"))?;
        let staged = read(&snapshot.parent.dir, &stage)?;
        if self.recheck(&snapshot).is_err()
            || read(&snapshot.parent.dir, "claude-plan-runtime.json")? != previous
        {
            remove_checked(&snapshot.parent.dir, &stage, &staged)?;
            return Err(fail("changed"));
        }
        files::publish(
            &snapshot.parent.dir,
            &stage,
            &snapshot.parent.dir,
            "claude-plan-runtime.json",
            previous.bytes.is_some(),
        )
        .map_err(|_| fail("changed"))?;
        if previous.bytes.is_some() && read(&snapshot.parent.dir, &stage)? != previous {
            if read(&snapshot.parent.dir, "claude-plan-runtime.json")
                .ok()
                .as_ref()
                == Some(&staged)
            {
                let _ = files::publish(
                    &snapshot.parent.dir,
                    &stage,
                    &snapshot.parent.dir,
                    "claude-plan-runtime.json",
                    true,
                );
            }
            return Err(uncertain());
        }
        self.recheck(&snapshot).map_err(|_| uncertain())?;
        if read(&snapshot.parent.dir, "claude-plan-runtime.json").map_err(|_| uncertain())?
            != staged
        {
            return Err(uncertain());
        }
        if previous.bytes.is_some() {
            remove_checked(&snapshot.parent.dir, &stage, &previous).map_err(|_| uncertain())?;
        }
        Ok(())
    }
    pub(crate) fn inspect(&mut self) -> Result<Inspection, Error> {
        self.pending = None;
        self.observe()
    }
    /// Background status must not consume a user's outstanding confirmation.
    pub(crate) fn observe(&self) -> Result<Inspection, Error> {
        let snapshot = self.snapshot()?;
        let writable = safe(&snapshot.value, false);
        let backups = Self::record_ids(&snapshot)?;
        let mut backup_records = vec![];
        if let Some(records) = &snapshot.records {
            for name in &backups {
                let record = files::child(&records.dir, name, true).map_err(|_| fail("invalid"))?;
                let identity = record_identity(&record)?;
                let b = backup(&read(&record, "receipt.json")?)?;
                let configured_before = installed(
                    &decode(b.before.as_deref().map(str::as_bytes))?,
                    b.owner_before.as_ref(),
                    &self.command()?,
                )?;
                let created_ms = identity
                    .birth
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .ok()
                    .and_then(|age| i64::try_from(age.as_millis()).ok())
                    .unwrap_or(0);
                backup_records.push(BackupRecord {
                    id: name.clone(),
                    created_ms,
                    configured_before,
                    restore_available: writable
                        && b.schema_version == 1
                        && b.after_hash == bytes_hash(snapshot.config_image.bytes.as_deref())
                        && b.owner_after_hash == bytes_hash(snapshot.owner_image.bytes.as_deref())
                        && safe(&decode(b.before.as_deref().map(str::as_bytes))?, false),
                });
            }
            records.current()?;
        }
        backup_records.sort_by(|a, b| {
            b.created_ms
                .cmp(&a.created_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(Inspection {
            revision: snapshot.revision,
            installed: snapshot.installed,
            writable,
            notice: if !writable {
                "配置含认证项，暂停修改；旧备份可单独清理。"
            } else if snapshot.installed {
                "只读钩子已配置，接收状态需另行核对。"
            } else {
                "方案查看尚未配置。"
            },
            backups,
            backup_records,
        })
    }
    pub(crate) fn preview(
        &mut self,
        action: Action,
        revision: &str,
        backup_id: Option<&str>,
    ) -> Result<Preview, Error> {
        self.pending = None;
        let snapshot = self.snapshot()?;
        if snapshot.revision != revision {
            return Err(fail("changed"));
        }
        if !safe(&snapshot.value, false) && action != Action::TrashBackup {
            return Err(fail("private"));
        }
        let command = self.command()?;
        let mut cleanup = None;
        let (after, after_owner, installed_after) = match action {
            Action::Enable => {
                let mut value = snapshot.value.clone();
                if snapshot.installed {
                    let previous = &snapshot
                        .owner
                        .as_ref()
                        .ok_or_else(|| fail("ownership"))?
                        .command;
                    if previous == &command {
                        return Err(fail("ownership"));
                    }
                    value = edit(value, false, previous)?;
                }
                let value = edit(value, true, &command)?;
                (
                    Some(encode(&value)?),
                    Some(Owner {
                        schema_version: 1,
                        command,
                    }),
                    true,
                )
            }
            Action::Disable => {
                if !snapshot.installed {
                    return Err(fail("ownership"));
                }
                let value = edit(
                    snapshot.value.clone(),
                    false,
                    snapshot
                        .owner
                        .as_ref()
                        .ok_or_else(|| fail("ownership"))?
                        .command
                        .as_str(),
                )?;
                (Some(encode(&value)?), None, false)
            }
            Action::TrashBackup => {
                let name = backup_id
                    .filter(|name| {
                        name.starts_with("backup-")
                            && uuid::Uuid::parse_str(name.trim_start_matches("backup-")).is_ok()
                    })
                    .ok_or_else(|| fail("invalid"))?;
                let records = snapshot.records.as_ref().ok_or_else(|| fail("invalid"))?;
                let record = files::child(&records.dir, name, true).map_err(|_| fail("invalid"))?;
                if files::names(&record).map_err(|_| fail("invalid"))?
                    != vec!["receipt.json".to_string()]
                {
                    return Err(fail("invalid"));
                }
                let receipt = read(&record, "receipt.json")?;
                backup(&receipt)?;
                let trash_parent =
                    Anchor::open(self.trash.parent().ok_or_else(|| fail("invalid"))?)?;
                let trash_name = self
                    .trash
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| fail("invalid"))?;
                let trash =
                    if files::exists(&trash_parent.dir, trash_name).map_err(|_| fail("invalid"))? {
                        Some(Anchor::open(&self.trash)?)
                    } else {
                        None
                    };
                cleanup = Some(Cleanup {
                    name: name.into(),
                    identity: record_identity(&record)?,
                    receipt,
                    trash_parent,
                    trash,
                });
                (
                    snapshot.config_image.bytes.clone(),
                    snapshot.owner.clone(),
                    snapshot.installed,
                )
            }
            Action::Restore => {
                let name = backup_id
                    .filter(|name| {
                        name.starts_with("backup-")
                            && uuid::Uuid::parse_str(name.trim_start_matches("backup-")).is_ok()
                    })
                    .ok_or_else(|| fail("invalid"))?;
                let records = snapshot.records.as_ref().ok_or_else(|| fail("invalid"))?;
                let record = files::child(&records.dir, name, true).map_err(|_| fail("invalid"))?;
                record_identity(&record)?;
                let image = read(&record, "receipt.json")?;
                let b = backup(&image)?;
                if b.schema_version != 1
                    || b.after_hash != bytes_hash(snapshot.config_image.bytes.as_deref())
                    || b.owner_after_hash != bytes_hash(snapshot.owner_image.bytes.as_deref())
                {
                    return Err(fail("changed"));
                }
                let value = decode(b.before.as_deref().map(str::as_bytes))?;
                if !safe(&value, false) {
                    return Err(fail("private"));
                }
                let state = installed(&value, b.owner_before.as_ref(), &command)?;
                (b.before.map(String::into_bytes), b.owner_before, state)
            }
        };
        if Self::record_ids(&snapshot)?.len() >= LIMIT && action != Action::TrashBackup {
            return Err(fail("capacity"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let plan = Preview {
            plan_id: id.clone(),
            action,
            installed_before: snapshot.installed,
            installed_after,
            scope: if action == Action::TrashBackup {
                "AgentIsland 本地恢复记录"
            } else {
                "Claude Code 默认用户设置 · 只读方案钩子"
            },
            notice: if action == Action::TrashBackup {
                "移入废纸篓后，此条恢复入口移除；Claude 配置保持原样。"
            } else {
                "核对后修改并保留旧版本；不会回答或批准方案。"
            },
        };
        self.pending = Some(Pending {
            id,
            created: Instant::now(),
            snapshot,
            after,
            after_owner,
            installed: installed_after,
            action,
            cleanup,
        });
        Ok(plan)
    }
    pub(crate) fn cancel(&mut self) {
        self.pending = None;
    }
    fn recheck(&self, old: &Snapshot) -> Result<(), Error> {
        old.config.current()?;
        old.parent.current()?;
        if let Some(records) = &old.records {
            records.current()?;
        }
        let now = self.snapshot()?;
        if now.revision != old.revision
            || now.config.chain != old.config.chain
            || now.parent.chain != old.parent.chain
            || now.records.as_ref().map(|r| &r.chain) != old.records.as_ref().map(|r| &r.chain)
            || now.config_image != old.config_image
            || now.owner_image != old.owner_image
        {
            return Err(fail("changed"));
        }
        Ok(())
    }
    fn trash_record(&self, p: &Pending) -> Result<Applied, Error> {
        let cleanup = p.cleanup.as_ref().ok_or_else(|| fail("invalid"))?;
        let records = p.snapshot.records.as_ref().ok_or_else(|| fail("invalid"))?;
        let verify = || -> Result<(), Error> {
            records.current()?;
            let dir =
                files::child(&records.dir, &cleanup.name, true).map_err(|_| fail("changed"))?;
            if record_identity(&dir)? != cleanup.identity
                || read(&dir, "receipt.json")? != cleanup.receipt
                || files::names(&dir).map_err(|_| fail("changed"))?
                    != vec!["receipt.json".to_string()]
            {
                return Err(fail("changed"));
            }
            Ok(())
        };
        verify()?;
        let trash_parent = &cleanup.trash_parent;
        trash_parent.current()?;
        let name = self
            .trash
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| fail("invalid"))?;
        if let Some(trash) = &cleanup.trash {
            trash.current()?;
        } else {
            if files::exists(&trash_parent.dir, name).map_err(|_| fail("changed"))? {
                return Err(fail("changed"));
            }
            files::create_dir(&trash_parent.dir, name).map_err(|_| fail("invalid"))?;
        }
        let trash = Anchor::open(&self.trash)?;
        self.recheck(&p.snapshot)?;
        verify()?;
        trash_parent.current()?;
        trash.current()?;
        let destination = format!("AgentIsland-plan-backup-{}", uuid::Uuid::new_v4());
        files::publish(&records.dir, &cleanup.name, &trash.dir, &destination, false)
            .map_err(|_| fail("changed"))?;
        trash_parent.current().map_err(|_| uncertain())?;
        trash.current().map_err(|_| uncertain())?;
        self.recheck(&p.snapshot).map_err(|_| uncertain())?;
        let moved = files::child(&trash.dir, &destination, true).map_err(|_| uncertain())?;
        if record_identity(&moved).map_err(|_| uncertain())? != cleanup.identity
            || read(&moved, "receipt.json").map_err(|_| uncertain())? != cleanup.receipt
            || files::names(&moved).map_err(|_| uncertain())? != vec!["receipt.json".to_string()]
            || files::exists(&records.dir, &cleanup.name).map_err(|_| uncertain())?
        {
            return Err(uncertain());
        }
        Ok(Applied {
            verified: true,
            installed: p.installed,
            backup_id: cleanup.name.clone(),
            notice: "恢复记录已移入废纸篓，Claude 配置保持原样。",
        })
    }
    pub(crate) fn preflight(&mut self, id: &str) -> Result<Action, Error> {
        let Some(p) = self
            .pending
            .as_ref()
            .filter(|p| p.id == id && p.created.elapsed() < TTL)
        else {
            self.cancel();
            return Err(fail("expired"));
        };
        let result = self.recheck(&p.snapshot).map(|_| p.action);
        if result.is_err() {
            self.cancel();
        }
        result
    }
    pub(crate) fn apply(&mut self, plan_id: &str) -> Result<Applied, Error> {
        let p = self
            .pending
            .take()
            .filter(|p| p.id == plan_id && p.created.elapsed() < TTL)
            .ok_or_else(|| fail("expired"))?;
        self.recheck(&p.snapshot)?;
        if p.action == Action::TrashBackup {
            return self.trash_record(&p);
        }
        if Self::record_ids(&p.snapshot)?.len() >= LIMIT {
            return Err(fail("capacity"));
        }
        let backup_id = format!("backup-{}", uuid::Uuid::new_v4());
        let records = match &p.snapshot.records {
            Some(records) => files::open_dir(&records.path).map_err(|_| fail("changed"))?,
            None => files::create_dir(&p.snapshot.parent.dir, "claude-plan-hooks")
                .map_err(|_| fail("changed"))?,
        };
        let record = files::create_dir(&records, &backup_id).map_err(|_| fail("invalid"))?;
        let after_owner = owner_bytes(p.after_owner.as_ref())?;
        let b = Backup {
            schema_version: 1,
            before: p
                .snapshot
                .config_image
                .bytes
                .as_ref()
                .map(|b| String::from_utf8(b.clone()))
                .transpose()
                .map_err(|_| fail("invalid"))?,
            owner_before: p.snapshot.owner.clone(),
            after_hash: bytes_hash(p.after.as_deref()),
            owner_after_hash: bytes_hash(after_owner.as_deref()),
        };
        let receipt = serde_json::to_vec(&b).map_err(|_| fail("invalid"))?;
        if receipt.len() > 512 * 1024 {
            return Err(fail("invalid"));
        }
        files::write_new(&record, "receipt.json", &receipt, false).map_err(|_| fail("invalid"))?;
        // Recheck source independently after preparing backup; the newly created owned record root
        // is expected, but its parent and original configuration/owner remain unchanged.
        p.snapshot.config.current()?;
        p.snapshot.parent.current()?;
        if read(&p.snapshot.config.dir, "settings.json")? != p.snapshot.config_image {
            return Err(fail("changed"));
        }
        if read(&records, "owner.json")? != p.snapshot.owner_image {
            return Err(fail("changed"));
        }
        let stage = format!("stage-{}", uuid::Uuid::new_v4());
        if let Some(after) = &p.after {
            files::write_new(&p.snapshot.config.dir, &stage, after, false)
                .map_err(|_| fail("invalid"))?;
            let staged = read(&p.snapshot.config.dir, &stage)?;
            if read(&p.snapshot.config.dir, "settings.json")? != p.snapshot.config_image
                || p.snapshot.config.current().is_err()
            {
                remove_checked(&p.snapshot.config.dir, &stage, &staged)?;
                return Err(fail("changed"));
            }
            files::publish(
                &p.snapshot.config.dir,
                &stage,
                &p.snapshot.config.dir,
                "settings.json",
                p.snapshot.config_image.bytes.is_some(),
            )
            .map_err(|_| fail("changed"))?;
            if p.snapshot.config_image.bytes.is_some()
                && read(&p.snapshot.config.dir, &stage).ok().as_ref()
                    != Some(&p.snapshot.config_image)
            {
                if read(&p.snapshot.config.dir, "settings.json").ok().as_ref() == Some(&staged) {
                    let _ = files::publish(
                        &p.snapshot.config.dir,
                        &stage,
                        &p.snapshot.config.dir,
                        "settings.json",
                        true,
                    );
                }
                return Err(uncertain());
            }
            if p.snapshot.config.current().is_err()
                || read(&p.snapshot.config.dir, "settings.json").ok().as_ref() != Some(&staged)
            {
                return Err(uncertain());
            }
        } else {
            remove_checked(
                &p.snapshot.config.dir,
                "settings.json",
                &p.snapshot.config_image,
            )?;
        }
        // The config changed already. Any later owner/readback problem is an uncertain applied result.
        let owner_stage = format!("stage-{}", uuid::Uuid::new_v4());
        let write_owner = || -> Result<(), Error> {
            p.snapshot.parent.current()?;
            if let Some(anchor) = &p.snapshot.records {
                anchor.current()?;
            }
            if read(&records, "owner.json")? != p.snapshot.owner_image {
                return Err(fail("changed"));
            }
            if let Some(bytes) = &after_owner {
                files::write_new(&records, &owner_stage, bytes, false)
                    .map_err(|_| fail("invalid"))?;
                let staged_owner = read(&records, &owner_stage)?;
                if read(&records, "owner.json")? != p.snapshot.owner_image {
                    return Err(fail("changed"));
                }
                files::publish(
                    &records,
                    &owner_stage,
                    &records,
                    "owner.json",
                    p.snapshot.owner_image.bytes.is_some(),
                )
                .map_err(|_| fail("changed"))?;
                if p.snapshot.owner_image.bytes.is_some()
                    && read(&records, &owner_stage)? != p.snapshot.owner_image
                {
                    if read(&records, "owner.json")? == staged_owner {
                        let _ =
                            files::publish(&records, &owner_stage, &records, "owner.json", true);
                    }
                    return Err(fail("changed"));
                }
            } else {
                remove_checked(&records, "owner.json", &p.snapshot.owner_image)?;
            }
            if read(&records, "owner.json")?.bytes != after_owner {
                return Err(fail("changed"));
            }
            Ok(())
        };
        if write_owner().is_err() {
            return Err(uncertain());
        }
        if p.after.is_some() && p.snapshot.config_image.bytes.is_some() {
            remove_checked(&p.snapshot.config.dir, &stage, &p.snapshot.config_image)
                .map_err(|_| uncertain())?;
        }
        if p.snapshot.owner_image.bytes.is_some() && after_owner.is_some() {
            remove_checked(&records, &owner_stage, &p.snapshot.owner_image)
                .map_err(|_| uncertain())?;
        }
        p.snapshot.config.current().map_err(|_| uncertain())?;
        p.snapshot.parent.current().map_err(|_| uncertain())?;
        let now = self.snapshot().map_err(|_| uncertain())?;
        if now.config_image.bytes != p.after
            || now.owner_image.bytes != after_owner
            || now.installed != p.installed
        {
            return Err(uncertain());
        }
        Ok(Applied {
            verified: true,
            installed: p.installed,
            backup_id,
            notice: match p.action {
                Action::Enable => "配置已核实；接收服务须另行启用。",
                Action::Disable => "自有钩子已移除，其他配置保留。",
                Action::Restore => "旧版本已还原并核对。",
                Action::TrashBackup => "恢复记录已移入废纸篓。",
            },
        })
    }
}
fn remove_checked(dir: &File, name: &str, expected: &Image) -> Result<(), Error> {
    use std::{ffi::CString, os::fd::AsRawFd};
    if expected.bytes.is_none() {
        if files::exists(dir, name).map_err(|_| fail("changed"))? {
            return Err(fail("changed"));
        }
        return Ok(());
    }
    if read(dir, name)? != *expected {
        return Err(fail("changed"));
    }
    let name = CString::new(name).map_err(|_| fail("invalid"))?;
    if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(fail("changed"));
    }
    Ok(())
}
#[cfg(test)]
#[path = "claude_hook_config_tests.rs"]
mod tests;

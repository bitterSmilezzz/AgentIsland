//! Versioned, local-only storage for TASK-03 project grants.
//!
//! This store is deliberately separate from `tasks.v1.json`. The production UI
//! and executor are not connected yet; `confirm_project` is the only grant
//! issuing API and is intended for a future user-confirmation command. Other
//! write APIs only adjust history capture or revoke an existing grant.
//! There is no import or restore API, and the root identity is rechecked before
//! a stored grant is returned for use.

#![allow(dead_code)] // Production command wiring follows the consent UI slice.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};

const FORMAT_VERSION: u32 = 1;
const MAX_POLICY_BYTES: usize = 1024 * 1024;
const MAX_PROJECTS: usize = 10_000;
static POLICY_WRITE_GATE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub(crate) enum AllowedRole {
    Planner,
    Executor,
    Reviewer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
enum RootPlatform {
    Macos,
    Linux,
    OtherUnix,
    Windows,
}

/// A project root's canonical local path plus a stable identity from an open
/// directory handle. The path is stored only in the local policy file; no API
/// in this module exports it to the remote ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectRootIdentity {
    canonical_path: String,
    platform: RootPlatform,
    volume_id: u64,
    file_id: [u8; 16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootIdentityError {
    InvalidPath,
    Missing,
    Symlink,
    NotDirectory,
    Changed,
    UnsupportedPlatform,
    Unavailable,
}

impl RootIdentityError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::InvalidPath => "project_root_invalid",
            Self::Missing => "project_root_missing",
            Self::Symlink => "project_root_symlink",
            Self::NotDirectory => "project_root_not_directory",
            Self::Changed => "project_root_changed",
            Self::UnsupportedPlatform => "project_root_identity_unsupported",
            Self::Unavailable => "project_root_unavailable",
        }
    }
}

impl std::fmt::Display for RootIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidPath => "项目目录路径无效；未授予权限",
            Self::Missing => "项目目录不存在；旧授权仍保留且不会复用",
            Self::Symlink => "项目目录不能是符号链接或重解析点；未授予权限",
            Self::NotDirectory => "所选项目根目录不是目录；未授予权限",
            Self::Changed => "项目目录身份已变化；请重新确认目录后再授权",
            Self::UnsupportedPlatform => "本平台无法核实项目目录身份；授权保持关闭",
            Self::Unavailable => "项目目录无法安全读取；未授予权限",
        })
    }
}

impl ProjectRootIdentity {
    pub(crate) fn capture(path: &Path) -> Result<Self, RootIdentityError> {
        if !path.is_absolute() {
            return Err(RootIdentityError::InvalidPath);
        }

        let input_metadata = fs::symlink_metadata(path).map_err(map_root_io)?;
        if input_metadata.file_type().is_symlink() {
            return Err(RootIdentityError::Symlink);
        }
        if !input_metadata.is_dir() {
            return Err(RootIdentityError::NotDirectory);
        }
        let input_directory = open_directory_nofollow(path).map_err(map_root_io)?;
        let input_identity = directory_identity(&input_directory)?;

        // Store the resolved path so an ancestor redirection cannot silently
        // bind a different directory under the original spelling.
        let canonical = path.canonicalize().map_err(map_root_io)?;
        let canonical_text = canonical
            .to_str()
            .filter(|text| !text.is_empty())
            .ok_or(RootIdentityError::InvalidPath)?
            .to_owned();
        let canonical_metadata = fs::symlink_metadata(&canonical).map_err(map_root_io)?;
        if canonical_metadata.file_type().is_symlink() {
            return Err(RootIdentityError::Symlink);
        }
        if !canonical_metadata.is_dir() {
            return Err(RootIdentityError::NotDirectory);
        }

        // Open the final directory without following a symlink/reparse point.
        // Its handle supplies the file identity, avoiding path metadata alone.
        let directory = open_directory_nofollow(&canonical).map_err(map_root_io)?;
        let (platform, volume_id, file_id) = directory_identity(&directory)?;
        if input_identity != (platform, volume_id, file_id) {
            return Err(RootIdentityError::Changed);
        }

        // Detect a directory swap or redirection that raced canonicalization.
        let canonical_after = path.canonicalize().map_err(map_root_io)?;
        let input_after = fs::symlink_metadata(path).map_err(map_root_io)?;
        if input_after.file_type().is_symlink() {
            return Err(RootIdentityError::Symlink);
        }
        if canonical_after != canonical {
            return Err(RootIdentityError::Changed);
        }
        let input_after_directory = open_directory_nofollow(path).map_err(map_root_io)?;
        if directory_identity(&input_after_directory)? != (platform, volume_id, file_id) {
            return Err(RootIdentityError::Changed);
        }

        Ok(Self {
            canonical_path: canonical_text,
            platform,
            volume_id,
            file_id,
        })
    }

    fn validate(&self) -> Result<(), PolicyError> {
        let path = Path::new(&self.canonical_path);
        if !path.is_absolute() || self.canonical_path.is_empty() {
            return Err(PolicyError::Corrupt);
        }
        let current_platform = current_platform().ok_or(PolicyError::UnsupportedPlatform)?;
        if self.platform != current_platform {
            return Err(PolicyError::RootIdentityChanged);
        }
        Ok(())
    }

    fn verify_current(&self) -> Result<(), PolicyError> {
        self.validate()?;
        let current =
            Self::capture(Path::new(&self.canonical_path)).map_err(|error| match error {
                RootIdentityError::Changed | RootIdentityError::Symlink => {
                    PolicyError::RootIdentityChanged
                }
                RootIdentityError::Missing => PolicyError::RootMissing,
                RootIdentityError::UnsupportedPlatform => PolicyError::UnsupportedPlatform,
                _ => PolicyError::RootUnavailable,
            })?;
        if &current != self {
            return Err(PolicyError::RootIdentityChanged);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectGrant {
    pub(crate) root: Option<ProjectRootIdentity>,
    pub(crate) enabled: bool,
    pub(crate) grant_version: u64,
    pub(crate) history_capture_enabled: bool,
    pub(crate) history_policy_version: u64,
    pub(crate) roles: BTreeSet<AllowedRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyDocument {
    format_version: u32,
    pub(crate) revision: u64,
    pub(crate) projects: BTreeMap<String, ProjectGrant>,
}

impl Default for PolicyDocument {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            revision: 0,
            projects: BTreeMap::new(),
        }
    }
}

impl PolicyDocument {
    fn validate(&self, allow_unpublished_revision: bool) -> Result<(), PolicyError> {
        if self.format_version != FORMAT_VERSION {
            return Err(PolicyError::UnsupportedFormat);
        }
        if (!allow_unpublished_revision && self.revision == 0) || self.projects.len() > MAX_PROJECTS
        {
            return Err(PolicyError::Corrupt);
        }
        for (project_id, grant) in &self.projects {
            if !valid_project_id(project_id)
                || grant.grant_version == 0
                || grant.history_policy_version == 0
            {
                return Err(PolicyError::Corrupt);
            }
            if grant.enabled {
                let root = grant.root.as_ref().ok_or(PolicyError::Corrupt)?;
                root.validate()?;
                if grant.roles.is_empty() {
                    return Err(PolicyError::Corrupt);
                }
            } else if grant.root.is_some()
                || !grant.roles.is_empty()
                || grant.history_capture_enabled
            {
                return Err(PolicyError::Corrupt);
            }
            if grant.history_capture_enabled && !grant.enabled {
                return Err(PolicyError::Corrupt);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyError {
    UnsupportedFormat,
    Corrupt,
    Symlink,
    InvalidFileType,
    InsecurePermissions,
    Io,
    Capacity,
    InvalidProjectId,
    InvalidRoles,
    UnknownProject,
    ProjectNotAuthorized,
    RootMissing,
    RootIdentityChanged,
    RootUnavailable,
    UnsupportedPlatform,
    RevisionConflict,
    VersionExhausted,
}

impl PolicyError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "project_policy_unsupported_format",
            Self::Corrupt => "project_policy_corrupt",
            Self::Symlink => "project_policy_symlink",
            Self::InvalidFileType => "project_policy_invalid_file_type",
            Self::InsecurePermissions => "project_policy_permissions",
            Self::Io => "project_policy_io_error",
            Self::Capacity => "project_policy_capacity",
            Self::InvalidProjectId => "invalid_project_id",
            Self::InvalidRoles => "invalid_project_roles",
            Self::UnknownProject => "unknown_project",
            Self::ProjectNotAuthorized => "project_not_authorized",
            Self::RootMissing => "project_root_missing",
            Self::RootIdentityChanged => "project_root_changed",
            Self::RootUnavailable => "project_root_unavailable",
            Self::UnsupportedPlatform => "project_root_identity_unsupported",
            Self::RevisionConflict => "project_policy_revision_conflict",
            Self::VersionExhausted => "project_policy_version_exhausted",
        }
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedFormat => "项目授权格式暂不支持；原文件已保留，授权已拒绝",
            Self::Corrupt => "项目授权文件损坏或无效；原文件已保留，授权已拒绝",
            Self::Symlink => "项目授权文件不能是符号链接；原文件已保留，授权已拒绝",
            Self::InvalidFileType => "项目授权路径不是受支持的普通文件；原件已保留",
            Self::InsecurePermissions => "项目授权文件权限过宽；原件已保留，授权已拒绝",
            Self::Io => "项目授权文件无法安全读取或保存；原件已保留",
            Self::Capacity => "项目授权文件达到容量上限；原记录未改变",
            Self::InvalidProjectId => "项目身份无效；未授予权限",
            Self::InvalidRoles => "至少要选择一种项目能力；未授予权限",
            Self::UnknownProject => "本机项目记录不存在；未授予权限",
            Self::ProjectNotAuthorized => "该项目没有可调整的运行授权",
            Self::RootMissing => "原项目目录已不存在；旧授权仍保留且不会复用",
            Self::RootIdentityChanged => "项目目录身份已变化；旧授权仍保留，请重新确认目录",
            Self::RootUnavailable => "项目目录无法安全读取；旧授权仍保留且不会复用",
            Self::UnsupportedPlatform => "本平台无法核实项目目录身份；授权保持关闭",
            Self::RevisionConflict => "项目授权已更新，请刷新后重试",
            Self::VersionExhausted => "项目授权版本已达上限；原记录未改变",
        })
    }
}

pub(crate) struct Store {
    path: PathBuf,
}

impl Store {
    pub(crate) fn at_default() -> Self {
        Self::at_path(crate::settings::config_dir().join("managed-run-authorization.v1.json"))
    }

    pub(crate) fn at_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self) -> Result<PolicyDocument, PolicyError> {
        let Some(bytes) = read_policy_bytes(&self.path)? else {
            return Ok(PolicyDocument::default());
        };
        let document = decode_document(&bytes)?;
        document.validate(false)?;
        Ok(document)
    }

    /// This is the only grant-issuing API. Its future Tauri caller must invoke it
    /// only after the person has reviewed the exact root and capability choices.
    /// No file import, runner identity, source association, or task creation path
    /// can call a deserialization-based grant constructor.
    pub(crate) fn confirm_project(
        &self,
        project_id: &str,
        root_path: &Path,
        roles: BTreeSet<AllowedRole>,
        history_capture_enabled: bool,
        expected_revision: u64,
    ) -> Result<PolicyDocument, PolicyError> {
        if !valid_project_id(project_id) {
            return Err(PolicyError::InvalidProjectId);
        }
        if roles.is_empty() {
            return Err(PolicyError::InvalidRoles);
        }
        let root = ProjectRootIdentity::capture(root_path).map_err(|error| match error {
            RootIdentityError::UnsupportedPlatform => PolicyError::UnsupportedPlatform,
            RootIdentityError::Missing => PolicyError::RootMissing,
            RootIdentityError::Changed | RootIdentityError::Symlink => {
                PolicyError::RootIdentityChanged
            }
            _ => PolicyError::RootUnavailable,
        })?;

        self.update(expected_revision, |document| {
            let previous = document.projects.get(project_id);
            let grant_changed = previous.is_none_or(|grant| {
                !grant.enabled || grant.root.as_ref() != Some(&root) || grant.roles != roles
            });
            let history_changed = previous
                .is_none_or(|grant| grant.history_capture_enabled != history_capture_enabled);
            let grant_version = match previous {
                Some(grant) if grant_changed => next_version(grant.grant_version)?,
                Some(grant) => grant.grant_version,
                None => 1,
            };
            let history_policy_version = match previous {
                Some(grant) if history_changed => next_version(grant.history_policy_version)?,
                Some(grant) => grant.history_policy_version,
                // Version 1 represents the default-off policy. Explicitly
                // opting in during the first confirmation is a real transition.
                None if history_capture_enabled => 2,
                None => 1,
            };
            document.projects.insert(
                project_id.to_owned(),
                ProjectGrant {
                    root: Some(root),
                    enabled: true,
                    grant_version,
                    history_capture_enabled,
                    history_policy_version,
                    roles,
                },
            );
            Ok(grant_changed || history_changed)
        })
    }

    pub(crate) fn set_history_capture(
        &self,
        project_id: &str,
        enabled: bool,
        expected_revision: u64,
    ) -> Result<PolicyDocument, PolicyError> {
        if !valid_project_id(project_id) {
            return Err(PolicyError::InvalidProjectId);
        }
        self.update(expected_revision, |document| {
            let grant = document
                .projects
                .get_mut(project_id)
                .ok_or(PolicyError::UnknownProject)?;
            if !grant.enabled {
                return Err(PolicyError::ProjectNotAuthorized);
            }
            if enabled {
                grant
                    .root
                    .as_ref()
                    .ok_or(PolicyError::Corrupt)?
                    .verify_current()?;
            }
            if grant.history_capture_enabled == enabled {
                return Ok(false);
            }
            grant.history_policy_version = next_version(grant.history_policy_version)?;
            grant.history_capture_enabled = enabled;
            Ok(true)
        })
    }

    pub(crate) fn revoke(
        &self,
        project_id: &str,
        expected_revision: u64,
    ) -> Result<PolicyDocument, PolicyError> {
        if !valid_project_id(project_id) {
            return Err(PolicyError::InvalidProjectId);
        }
        self.update(expected_revision, |document| {
            let grant = document
                .projects
                .get_mut(project_id)
                .ok_or(PolicyError::UnknownProject)?;
            if !grant.enabled {
                return Ok(false);
            }
            grant.grant_version = next_version(grant.grant_version)?;
            if grant.history_capture_enabled {
                grant.history_policy_version = next_version(grant.history_policy_version)?;
            }
            grant.root = None;
            grant.enabled = false;
            grant.history_capture_enabled = false;
            grant.roles.clear();
            Ok(true)
        })
    }

    pub(crate) fn verified_grant(
        &self,
        project_id: &str,
    ) -> Result<Option<ProjectGrant>, PolicyError> {
        if !valid_project_id(project_id) {
            return Err(PolicyError::InvalidProjectId);
        }
        let document = self.load()?;
        let Some(grant) = document.projects.get(project_id) else {
            return Ok(None);
        };
        if !grant.enabled {
            return Ok(None);
        }
        grant
            .root
            .as_ref()
            .ok_or(PolicyError::Corrupt)?
            .verify_current()?;
        Ok(Some(grant.clone()))
    }

    fn update<F>(&self, expected_revision: u64, update: F) -> Result<PolicyDocument, PolicyError>
    where
        F: FnOnce(&mut PolicyDocument) -> Result<bool, PolicyError>,
    {
        // A process-wide gate also serializes distinct Store handles that point
        // at the same policy file. The app's single-instance guard remains the
        // cross-process boundary when this store is wired into Tauri.
        let _guard = POLICY_WRITE_GATE.lock().map_err(|_| PolicyError::Io)?;
        let mut document = self.load()?;
        if document.revision != expected_revision {
            return Err(PolicyError::RevisionConflict);
        }
        let changed = update(&mut document)?;
        if !changed {
            return Ok(document);
        }
        document.revision = next_version(document.revision)?;
        document.validate(false)?;
        self.write(&document)?;
        Ok(document)
    }

    fn write(&self, document: &PolicyDocument) -> Result<(), PolicyError> {
        document.validate(false)?;
        let bytes = serde_json::to_vec_pretty(document).map_err(|_| PolicyError::Corrupt)?;
        if bytes.len() > MAX_POLICY_BYTES {
            return Err(PolicyError::Capacity);
        }
        let parent = self.path.parent().ok_or(PolicyError::Io)?;
        fs::create_dir_all(parent).map_err(|_| PolicyError::Io)?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |staged| {
            let file = open_policy_file(staged)?;
            ensure_private_regular_file(&file)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.code()))?;
            let mut staged_bytes = Vec::new();
            file.take((MAX_POLICY_BYTES + 1) as u64)
                .read_to_end(&mut staged_bytes)?;
            let decoded = decode_document(&staged_bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.code()))?;
            decoded
                .validate(false)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.code()))
        })
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                PolicyError::Corrupt
            } else if error.kind() == io::ErrorKind::InvalidInput {
                PolicyError::Symlink
            } else {
                PolicyError::Io
            }
        })
    }
}

fn decode_document(bytes: &[u8]) -> Result<PolicyDocument, PolicyError> {
    let header: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PolicyError::Corrupt)?;
    if header
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        != Some(FORMAT_VERSION as u64)
    {
        return Err(PolicyError::UnsupportedFormat);
    }
    serde_json::from_slice(bytes).map_err(|_| PolicyError::Corrupt)
}

fn read_policy_bytes(path: &Path) -> Result<Option<Vec<u8>>, PolicyError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(PolicyError::Io),
        Ok(metadata) if metadata.file_type().is_symlink() => return Err(PolicyError::Symlink),
        Ok(metadata) if !metadata.is_file() => return Err(PolicyError::InvalidFileType),
        Ok(_) => {}
    }

    let file = open_policy_file(path).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            PolicyError::Symlink
        } else {
            PolicyError::Io
        }
    })?;
    ensure_private_regular_file(&file)?;
    let metadata = file.metadata().map_err(|_| PolicyError::Io)?;
    if !metadata.is_file() {
        return Err(PolicyError::InvalidFileType);
    }
    if metadata.len() > MAX_POLICY_BYTES as u64 {
        return Err(PolicyError::Capacity);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_POLICY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PolicyError::Io)?;
    if bytes.len() > MAX_POLICY_BYTES {
        return Err(PolicyError::Capacity);
    }
    Ok(Some(bytes))
}

fn ensure_private_regular_file(file: &File) -> Result<(), PolicyError> {
    let metadata = file.metadata().map_err(|_| PolicyError::Io)?;
    if !metadata.is_file() || is_reparse_point(file)? {
        return Err(PolicyError::InvalidFileType);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(PolicyError::InsecurePermissions);
        }
    }
    Ok(())
}

fn open_policy_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn open_directory_nofollow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(
            windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS
                | windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
        );
    }
    options.open(path)
}

fn is_reparse_point(file: &File) -> Result<bool, PolicyError> {
    #[cfg(windows)]
    {
        let information = windows_file_information(file).map_err(|_| PolicyError::Io)?;
        return Ok(information.dwFileAttributes
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0);
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Ok(false)
    }
}

fn directory_identity(file: &File) -> Result<(RootPlatform, u64, [u8; 16]), RootIdentityError> {
    let metadata = file
        .metadata()
        .map_err(|_| RootIdentityError::Unavailable)?;
    if !metadata.is_dir() || is_reparse_point(file).map_err(|_| RootIdentityError::Unavailable)? {
        return Err(RootIdentityError::Symlink);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mut file_id = [0u8; 16];
        file_id[..8].copy_from_slice(&metadata.ino().to_be_bytes());
        return Ok((
            current_platform().ok_or(RootIdentityError::UnsupportedPlatform)?,
            metadata.dev(),
            file_id,
        ));
    }
    #[cfg(windows)]
    {
        let (volume_id, file_id) =
            windows_file_id(file).map_err(|_| RootIdentityError::Unavailable)?;
        return Ok((RootPlatform::Windows, volume_id, file_id));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = metadata;
        Err(RootIdentityError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
fn windows_file_information(
    file: &File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let ok = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as HANDLE, information.as_mut_ptr())
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { information.assume_init() })
    }
}

#[cfg(windows)]
fn windows_file_id(file: &File) -> io::Result<(u64, [u8; 16])> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO},
    };
    let mut information = MaybeUninit::<FILE_ID_INFO>::uninit();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileIdInfo,
            information.as_mut_ptr().cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        let information = unsafe { information.assume_init() };
        Ok((
            information.VolumeSerialNumber,
            information.FileId.Identifier,
        ))
    }
}

fn map_root_io(error: io::Error) -> RootIdentityError {
    match error.kind() {
        io::ErrorKind::NotFound => RootIdentityError::Missing,
        io::ErrorKind::InvalidInput => RootIdentityError::Symlink,
        _ => RootIdentityError::Unavailable,
    }
}

fn current_platform() -> Option<RootPlatform> {
    #[cfg(target_os = "macos")]
    {
        Some(RootPlatform::Macos)
    }
    #[cfg(target_os = "linux")]
    {
        Some(RootPlatform::Linux)
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    {
        Some(RootPlatform::OtherUnix)
    }
    #[cfg(windows)]
    {
        Some(RootPlatform::Windows)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn valid_project_id(project_id: &str) -> bool {
    uuid::Uuid::parse_str(project_id).is_ok_and(|uuid| uuid.to_string() == project_id)
}

fn next_version(current: u64) -> Result<u64, PolicyError> {
    current.checked_add(1).ok_or(PolicyError::VersionExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    const PROJECT: &str = "00000000-0000-4000-8000-000000000001";

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "agentisland-managed-run-policy-{}-{stamp}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self) -> Store {
            Store::at_path(self.0.join("authorization.json"))
        }

        fn root(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn roles(values: &[AllowedRole]) -> BTreeSet<AllowedRole> {
        values.iter().copied().collect()
    }

    fn write_private_fixture(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn confirm(store: &Store, root: &Path, revision: u64) -> PolicyDocument {
        store
            .confirm_project(
                PROJECT,
                root,
                roles(&[AllowedRole::Planner]),
                false,
                revision,
            )
            .unwrap()
    }

    #[test]
    fn missing_policy_starts_with_no_grants_and_no_history_capture() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let document = store.load().unwrap();
        assert_eq!(document.revision, 0);
        assert!(document.projects.is_empty());
        assert!(store.verified_grant(PROJECT).unwrap().is_none());
        assert!(!store.path.exists());
    }

    #[test]
    fn first_explicit_confirmation_publishes_a_private_versioned_project_grant() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let document = confirm(&store, &root, 0);
        let grant = document.projects.get(PROJECT).unwrap();
        assert_eq!(document.revision, 1);
        assert!(grant.enabled);
        assert_eq!(grant.grant_version, 1);
        assert_eq!(grant.history_policy_version, 1);
        assert!(!grant.history_capture_enabled);
        assert_eq!(grant.roles, roles(&[AllowedRole::Planner]));
        assert!(store.verified_grant(PROJECT).unwrap().is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn project_id_must_be_a_canonical_local_project_uuid() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        assert_eq!(
            store.confirm_project("project-1", &root, roles(&[AllowedRole::Planner]), false, 0),
            Err(PolicyError::InvalidProjectId)
        );
        assert!(!store.path.exists());
    }

    #[test]
    fn root_must_be_absolute_existing_directory_and_not_a_symlink() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        assert_eq!(
            ProjectRootIdentity::capture(Path::new("relative/project")),
            Err(RootIdentityError::InvalidPath)
        );
        assert_eq!(
            ProjectRootIdentity::capture(&sandbox.0.join("missing")),
            Err(RootIdentityError::Missing)
        );
        assert_eq!(
            ProjectRootIdentity::capture(&root.join("child.txt")),
            Err(RootIdentityError::Missing)
        );
        let not_directory = sandbox.0.join("not-a-directory");
        fs::write(&not_directory, b"file").unwrap();
        assert_eq!(
            ProjectRootIdentity::capture(&not_directory),
            Err(RootIdentityError::NotDirectory)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = sandbox.0.join("project-link");
            symlink(&root, &link).unwrap();
            assert_eq!(
                ProjectRootIdentity::capture(&link),
                Err(RootIdentityError::Symlink)
            );
        }
        assert!(!store.path.exists());
    }

    #[test]
    fn replacing_the_authorized_directory_rejects_the_old_grant_without_rewriting_policy() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        confirm(&store, &root, 0);
        let original = fs::read(&store.path).unwrap();
        let moved = sandbox.0.join("moved-project");
        fs::rename(&root, &moved).unwrap();
        fs::create_dir(&root).unwrap();
        assert_eq!(
            store.verified_grant(PROJECT),
            Err(PolicyError::RootIdentityChanged)
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn redirecting_an_authorized_ancestor_rejects_the_old_grant_without_rewriting_policy() {
        use std::os::unix::fs::symlink;
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let parent = sandbox.0.join("parent");
        fs::create_dir_all(parent.join("project")).unwrap();
        let root = parent.join("project");
        let confirmed = confirm(&store, &root, 0);
        let original = fs::read(&store.path).unwrap();

        let alternate = sandbox.0.join("alternate-parent");
        fs::create_dir_all(alternate.join("project")).unwrap();
        let moved = sandbox.0.join("original-parent");
        fs::rename(&parent, &moved).unwrap();
        symlink(&alternate, &parent).unwrap();

        assert_eq!(
            store.verified_grant(PROJECT),
            Err(PolicyError::RootIdentityChanged)
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        assert_eq!(confirmed.revision, 1);
    }

    #[test]
    fn moving_the_authorized_directory_requires_new_user_confirmation_at_its_new_path() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let first = confirm(&store, &root, 0);
        let moved = sandbox.0.join("moved-project");
        fs::rename(&root, &moved).unwrap();
        assert_eq!(store.verified_grant(PROJECT), Err(PolicyError::RootMissing));
        let confirmed = store
            .confirm_project(
                PROJECT,
                &moved,
                roles(&[AllowedRole::Planner]),
                false,
                first.revision,
            )
            .unwrap();
        assert_eq!(
            confirmed.projects[PROJECT].grant_version,
            first.projects[PROJECT].grant_version + 1
        );
        assert!(store.verified_grant(PROJECT).unwrap().is_some());
    }

    #[test]
    fn role_changes_and_history_opt_in_advance_independent_versions() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let first = confirm(&store, &root, 0);
        let history = store
            .set_history_capture(PROJECT, true, first.revision)
            .unwrap();
        assert_eq!(history.projects[PROJECT].grant_version, 1);
        assert_eq!(history.projects[PROJECT].history_policy_version, 2);
        assert!(history.projects[PROJECT].history_capture_enabled);
        let roles_changed = store
            .confirm_project(
                PROJECT,
                &root,
                roles(&[AllowedRole::Planner, AllowedRole::Reviewer]),
                true,
                history.revision,
            )
            .unwrap();
        assert_eq!(roles_changed.projects[PROJECT].grant_version, 2);
        assert_eq!(roles_changed.projects[PROJECT].history_policy_version, 2);
        let disabled = store
            .set_history_capture(PROJECT, false, roles_changed.revision)
            .unwrap();
        assert_eq!(disabled.projects[PROJECT].grant_version, 2);
        assert_eq!(disabled.projects[PROJECT].history_policy_version, 3);
    }

    #[test]
    fn revocation_advances_versions_clears_sensitive_root_and_is_idempotent() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let first = store
            .confirm_project(PROJECT, &root, roles(&[AllowedRole::Planner]), true, 0)
            .unwrap();
        assert_eq!(first.projects[PROJECT].history_policy_version, 2);
        let revoked = store.revoke(PROJECT, first.revision).unwrap();
        let grant = &revoked.projects[PROJECT];
        assert!(!grant.enabled);
        assert!(grant.root.is_none());
        assert!(grant.roles.is_empty());
        assert!(!grant.history_capture_enabled);
        assert_eq!(grant.grant_version, 2);
        assert_eq!(grant.history_policy_version, 3);
        let unchanged = store.revoke(PROJECT, revoked.revision).unwrap();
        assert_eq!(unchanged, revoked);
        assert!(store.verified_grant(PROJECT).unwrap().is_none());
    }

    #[test]
    fn stale_global_revision_is_refused_without_changing_policy_bytes() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let first = confirm(&store, &root, 0);
        let original = fs::read(&store.path).unwrap();
        assert_eq!(
            store.set_history_capture(PROJECT, true, 0),
            Err(PolicyError::RevisionConflict)
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        assert_eq!(store.load().unwrap(), first);
    }

    #[test]
    fn concurrent_handles_cannot_overwrite_each_other_at_the_same_revision() {
        use std::sync::{Arc, Barrier};

        let sandbox = Sandbox::new();
        let root = sandbox.root("project");
        let policy_path = sandbox.0.join("authorization.json");
        let barrier = Arc::new(Barrier::new(2));
        let spawn_confirmation = |role, barrier: Arc<Barrier>| {
            let path = policy_path.clone();
            let root = root.clone();
            std::thread::spawn(move || {
                let store = Store::at_path(path);
                barrier.wait();
                store.confirm_project(PROJECT, &root, roles(&[role]), false, 0)
            })
        };

        let first = spawn_confirmation(AllowedRole::Planner, Arc::clone(&barrier));
        let second = spawn_confirmation(AllowedRole::Executor, barrier);
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| {
                    matches!(result.as_ref(), Err(error) if *error == PolicyError::RevisionConflict)
                })
                .count(),
            1
        );
        assert_eq!(Store::at_path(policy_path).load().unwrap().revision, 1);
    }

    #[test]
    fn grant_and_history_version_overflow_rejects_without_mutating_file() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let identity = ProjectRootIdentity::capture(&root).unwrap();
        let mut document = PolicyDocument {
            format_version: FORMAT_VERSION,
            revision: 7,
            projects: BTreeMap::from([(
                PROJECT.to_owned(),
                ProjectGrant {
                    root: Some(identity),
                    enabled: true,
                    grant_version: u64::MAX,
                    history_capture_enabled: false,
                    history_policy_version: u64::MAX,
                    roles: roles(&[AllowedRole::Planner]),
                },
            )]),
        };
        store.write(&document).unwrap();
        let original = fs::read(&store.path).unwrap();
        assert_eq!(
            store.confirm_project(
                PROJECT,
                &root,
                roles(&[AllowedRole::Reviewer]),
                false,
                document.revision,
            ),
            Err(PolicyError::VersionExhausted)
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
        document.projects.get_mut(PROJECT).unwrap().grant_version = 2;
        store.write(&document).unwrap();
        let original = fs::read(&store.path).unwrap();
        assert_eq!(
            store.set_history_capture(PROJECT, true, document.revision),
            Err(PolicyError::VersionExhausted)
        );
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }

    #[test]
    fn corrupt_future_and_unknown_field_files_are_refused_and_preserved() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        for (bytes, expected) in [
            (b"{bad".as_slice(), PolicyError::Corrupt),
            (
                br#"{"format_version":2,"revision":0,"projects":{}}"#.as_slice(),
                PolicyError::UnsupportedFormat,
            ),
            (
                br#"{"format_version":1,"revision":0,"projects":{},"unknown":true}"#.as_slice(),
                PolicyError::Corrupt,
            ),
        ] {
            write_private_fixture(&store.path, bytes);
            let original = fs::read(&store.path).unwrap();
            assert_eq!(store.load(), Err(expected));
            assert_eq!(
                store.confirm_project(PROJECT, &root, roles(&[AllowedRole::Planner]), false, 0,),
                Err(expected)
            );
            assert_eq!(fs::read(&store.path).unwrap(), original);
            fs::remove_file(&store.path).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_policy_file_is_refused_without_touching_link_or_target() {
        use std::os::unix::fs::symlink;
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let target = sandbox.0.join("target.json");
        fs::write(&target, b"keep original").unwrap();
        symlink(&target, &store.path).unwrap();
        assert_eq!(store.load(), Err(PolicyError::Symlink));
        assert_eq!(fs::read(&target).unwrap(), b"keep original");
        assert!(fs::symlink_metadata(&store.path)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn policy_file_with_group_or_world_permissions_is_refused_and_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        confirm(&store, &root, 0);
        let original = fs::read(&store.path).unwrap();
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(store.load(), Err(PolicyError::InsecurePermissions));
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn failed_atomic_publication_keeps_the_prior_policy_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let root = sandbox.root("project");
        let first = confirm(&store, &root, 0);
        let original = fs::read(&store.path).unwrap();
        fs::set_permissions(&sandbox.0, fs::Permissions::from_mode(0o500)).unwrap();
        let result = store.set_history_capture(PROJECT, true, first.revision);
        fs::set_permissions(&sandbox.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(result, Err(PolicyError::Io));
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }

    #[test]
    fn persisted_grant_without_a_verified_root_is_rejected_and_preserved() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        let forged = serde_json::json!({
            "format_version": FORMAT_VERSION,
            "revision": 1,
            "projects": { PROJECT: {
                "root": null,
                "enabled": true,
                "grant_version": 1,
                "history_capture_enabled": false,
                "history_policy_version": 1,
                "roles": ["planner"]
            }}
        });
        let original = serde_json::to_vec(&forged).unwrap();
        write_private_fixture(&store.path, &original);
        assert_eq!(store.load(), Err(PolicyError::Corrupt));
        assert_eq!(store.verified_grant(PROJECT), Err(PolicyError::Corrupt));
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }
}

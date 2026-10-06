//! Bounded, descriptor-relative skill trees. Contents never leave this module's callers.
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
};
const MAX_NODES: usize = 512;
const MAX_FILES: usize = 256;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILE: usize = 4 * 1024 * 1024;

pub(crate) struct Node {
    pub bytes: Option<Vec<u8>>,
    pub executable: bool,
}
pub(crate) struct Tree {
    pub nodes: BTreeMap<String, Node>,
    pub bytes: usize,
    pub files: usize,
    pub revision: String,
}
fn rehash(tree: &mut Tree) {
    let mut hash = Sha256::new();
    for (name, node) in &tree.nodes {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update([u8::from(node.bytes.is_some()), u8::from(node.executable)]);
        if let Some(bytes) = &node.bytes {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
    tree.revision = format!("{:x}", hash.finalize());
}
pub(crate) fn replace_skill(tree: &mut Tree, content: Vec<u8>) -> Result<(), String> {
    if content.len() > MAX_FILE
        || crate::private_text::known_private(&String::from_utf8_lossy(&content))
    {
        return Err("技能正文超过4 MiB或包含已识别的凭据，未修改文件".into());
    }
    let node = tree.nodes.get_mut("SKILL.md").ok_or("技能正文缺失")?;
    let old = node.bytes.as_ref().ok_or("技能正文类型不可核实")?;
    let size = tree.bytes - old.len() + content.len();
    if size > MAX_BYTES {
        return Err("修改后技能包超过32 MiB，未修改文件".into());
    }
    node.bytes = Some(content);
    tree.bytes = size;
    rehash(tree);
    Ok(())
}
pub(crate) fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::{MetadataExt, PermissionsExt},
        },
    };

    fn component(name: &str) -> Result<CString, String> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.len() > 255
            || name.contains('/')
            || name.contains('\\')
            || name.chars().any(char::is_control)
            || crate::private_text::known_private(name)
        {
            return Err("技能包包含不受支持的文件名".into());
        }
        CString::new(name).map_err(|_| "技能包文件名不可读".into())
    }
    pub(crate) fn open_dir(path: &Path) -> Result<File, String> {
        #[cfg(unix)]
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| "目录路径不受支持")?;
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err("目录不可读或为链接，未修改文件".into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) fn child(parent: &File, name: &str, directory: bool) -> Result<File, String> {
        let name = component(name)?;
        let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        if directory {
            flags |= libc::O_DIRECTORY;
        }
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err("技能文件不可读、已变化或为链接".into());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    pub(crate) fn create_dir(parent: &File, name: &str) -> Result<File, String> {
        let c = component(name)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) } != 0 {
            return Err("目录无法独占创建，未覆盖现有文件".into());
        }
        child(parent, name, true)
    }
    pub(crate) fn ensure_dir(parent: &File, name: &str) -> Result<File, String> {
        let c = component(name)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
        {
            return Err("目录无法创建，未修改已有文件".into());
        }
        child(parent, name, true)
    }
    pub(crate) fn names(dir: &File) -> Result<Vec<String>, String> {
        let duplicate = unsafe { libc::dup(dir.as_raw_fd()) };
        if duplicate < 0 {
            return Err("目录扫描不可用".into());
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            unsafe {
                libc::close(duplicate);
            }
            return Err("目录扫描不可用".into());
        }
        struct Guard(*mut libc::DIR);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let stream = Guard(stream);
        // dup shares the directory offset with the original descriptor.
        unsafe {
            libc::rewinddir(stream.0);
        }
        let mut names = vec![];
        loop {
            #[cfg(target_os = "macos")]
            unsafe {
                *libc::__error() = 0;
            }
            #[cfg(target_os = "linux")]
            unsafe {
                *libc::__errno_location() = 0;
            }
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                    return Err("技能目录读取不完整".into());
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| "技能文件名不是 UTF-8")?;
            if name == "." || name == ".." || name == ".DS_Store" {
                continue;
            }
            component(name)?;
            names.push(name.to_owned());
            if names.len() > MAX_NODES {
                return Err("技能目录条目超过上限，未截断安装".into());
            }
        }
        names.sort();
        Ok(names)
    }
    fn walk(
        dir: &File,
        prefix: &str,
        depth: usize,
        tree: &mut Tree,
        record: bool,
    ) -> Result<(), String> {
        if depth > 12 + usize::from(record) {
            return Err("技能目录深度超过支持上限，未截断读取".into());
        }
        let mut folded = std::collections::HashSet::new();
        for name in names(dir)? {
            if !folded.insert(name.to_lowercase()) {
                return Err("技能文件名有大小写冲突".into());
            }
            if name.starts_with('.')
                || [
                    "auth.json",
                    "credentials.json",
                    "credentials",
                    "id_rsa",
                    "id_ed25519",
                ]
                .contains(&name.as_str())
            {
                return Err("技能包含隐藏或凭据文件，拒绝复制".into());
            }
            if tree.nodes.len() >= MAX_NODES + 2 * usize::from(record) {
                return Err("技能超过 512 项，未截断安装".into());
            }
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let mut file = child(dir, &name, false)?;
            let metadata = file.metadata().map_err(|_| "技能文件信息不可读")?;
            if metadata.is_dir() {
                tree.nodes.insert(
                    relative.clone(),
                    Node {
                        bytes: None,
                        executable: false,
                    },
                );
                walk(&file, &relative, depth + 1, tree, record)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE as u64 || tree.files >= MAX_FILES + usize::from(record)
                {
                    return Err("技能文件数量或大小超过上限".into());
                }
                let mut bytes = Vec::new();
                (&mut file)
                    .take((MAX_FILE + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "技能文件读取失败")?;
                let after = file.metadata().map_err(|_| "技能文件信息不可读")?;
                if bytes.len() > MAX_FILE
                    || metadata.len() != bytes.len() as u64
                    || metadata.dev() != after.dev()
                    || metadata.ino() != after.ino()
                    || metadata.mtime() != after.mtime()
                    || metadata.mtime_nsec() != after.mtime_nsec()
                    || metadata.len() != after.len()
                    || metadata.mode() != after.mode()
                {
                    return Err("技能文件在扫描中变化，请重新选择".into());
                }
                if crate::private_text::known_private(&String::from_utf8_lossy(&bytes)) {
                    return Err("技能文件含已识别的凭据形式，拒绝复制；未输出内容".into());
                }
                tree.bytes += bytes.len();
                tree.files += 1;
                if tree.bytes > MAX_BYTES + 4096 * usize::from(record) {
                    return Err("技能包超过 32 MiB，未截断安装".into());
                }
                tree.nodes.insert(
                    relative,
                    Node {
                        bytes: Some(bytes),
                        executable: metadata.mode() & 0o100 != 0,
                    },
                );
            } else {
                return Err("技能包含特殊文件，拒绝复制".into());
            }
        }
        Ok(())
    }
    pub(crate) fn scan(dir: &File) -> Result<Tree, String> {
        scan_bounded(dir, false)
    }
    pub(crate) fn scan_record(dir: &File) -> Result<Tree, String> {
        let entries = names(dir)?;
        if entries
            .iter()
            .any(|name| !matches!(name.as_str(), "package" | "receipt.json"))
        {
            return Err("安装记录含额外内容，保留原件；请先在本机核对".into());
        }
        if exists(dir, "receipt.json")? {
            let receipt = child(dir, "receipt.json", false)?;
            if !receipt
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() <= 4096)
            {
                return Err("安装收据类型或大小不可核实，未移出".into());
            }
        }
        if exists(dir, "package")? {
            child(dir, "package", true)?;
        }
        let tree = scan_bounded(dir, true)?;
        if tree.nodes.iter().any(|(name, node)| {
            !matches!(name.as_str(), "package" | "receipt.json") && !name.starts_with("package/")
                || name == "receipt.json"
                    && node.bytes.as_ref().is_none_or(|bytes| bytes.len() > 4096)
                || name == "package" && node.bytes.is_some()
        }) {
            return Err("安装记录结构在核对中变化，未移出".into());
        }
        Ok(tree)
    }
    pub(crate) fn file_stamp(file: &File) -> Result<String, String> {
        let m = file.metadata().map_err(|_| "技能文件元数据不可读")?;
        if !m.is_file() || m.len() > MAX_FILE as u64 {
            return Err("技能文件元数据类型或大小不支持".into());
        }
        Ok(format!(
            "{}:{}:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.mode()
        ))
    }
    pub(crate) fn identity(dir: &File) -> Result<(u64, u64), String> {
        let metadata = dir.metadata().map_err(|_| "目录身份不可核实")?;
        Ok((metadata.dev(), metadata.ino()))
    }
    fn scan_bounded(dir: &File, record: bool) -> Result<Tree, String> {
        let mut tree = Tree {
            nodes: BTreeMap::new(),
            files: 0,
            bytes: 0,
            revision: String::new(),
        };
        walk(dir, "", 0, &mut tree, record)?;
        rehash(&mut tree);
        Ok(tree)
    }
    pub(crate) fn write_new(
        dir: &File,
        name: &str,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), String> {
        let name = component(name)?;
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err("技能暂存文件创建失败，未覆盖原文件".into());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "技能暂存写入失败")?;
        if executable {
            file.set_permissions(std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "技能执行位无法保留")?;
        }
        Ok(())
    }
    pub(crate) fn stage(dir: &File, tree: &Tree) -> Result<(), String> {
        for (name, node) in &tree.nodes {
            let mut components = name.split('/').collect::<Vec<_>>();
            let last = components.pop().ok_or("技能路径为空")?;
            let mut parent = dir.try_clone().map_err(|_| "暂存目录不可用")?;
            for part in components {
                parent = child(&parent, part, true)?;
            }
            if let Some(bytes) = &node.bytes {
                write_new(&parent, last, bytes, node.executable)?;
            } else {
                create_dir(&parent, last)?;
            }
        }
        if scan(dir)?.revision != tree.revision {
            return Err("技能暂存校验失败，未发布".into());
        }
        Ok(())
    }
    pub(crate) fn exists(parent: &File, name: &str) -> Result<bool, String> {
        let name = component(name)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(true);
        }
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err("目录条目不可核实".into())
        }
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn publish(
        from: &File,
        source: &str,
        to: &File,
        target: &str,
        swap: bool,
    ) -> Result<(), String> {
        let source = component(source)?;
        let target = component(target)?;
        // Darwin: SWAP=2, EXCL=4. Both operate on directory entries, never copy or execute content.
        let flags = if swap { 2 } else { 4 };
        if unsafe {
            libc::renameatx_np(
                from.as_raw_fd(),
                source.as_ptr(),
                to.as_raw_fd(),
                target.as_ptr(),
                flags,
            )
        } != 0
        {
            return Err("技能目录发布失败或目标已变化；暂存与旧目录保留".into());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn publish(_: &File, _: &str, _: &File, _: &str, _: bool) -> Result<(), String> {
        Err("本平台的技能目录原子发布尚未验证".into())
    }
}
#[cfg(unix)]
pub(crate) use unix::*;

// An unsupported platform is a capability result; never emulate swap with destructive copies.
#[cfg(not(unix))]
pub(crate) fn open_dir(_: &Path) -> Result<File, String> {
    Err("本平台技能安装尚未接入".into())
}

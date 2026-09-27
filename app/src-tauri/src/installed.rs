//! 已安装 CLI / 应用包探测（对齐 Swift `InstalledAppsCache`）
//!
//! **「读不到」≠「没装」。** 这一条是整个可观测性结论的地基：没有它，进程不在时
//! 只能说「离线」——而离线是**关于进程**的结论，不是关于**装没装**的结论。
//! 用户问「我装了 Cursor 为什么不在列表里」时，那才是真正要回答的问题。
//!
//! 三条从 Swift 侧搬过来、每条都注明它防什么症状：
//!
//! 1. **扫描器注入**：生产走真实扫描，测试传闭包 ⇒ 用例永远不碰用户的文件系统
//!    （「这台机器刚好装了什么」不该让测试时绿时红）。
//! 2. **「这趟扫到空集」不等于「全卸载了」**：沿用上一份非空值并留证据，
//!    否则一次失败的扫描会把所有 CLI 档案判成未安装，而界面上不会有任何说明。
//! 3. **否定结论也要有证据**：[`InstalledApps::is_installed`] 返回 `Option<bool>`，
//!    `None` = 没核实。档案登记了 bundle id、而这份扫描根本没能力给出否定证据时
//!    （Windows 没有应用包概念、Info.plist 是 binary 形状读不出来），
//!    **给 `None` 而不是 `Some(false)`**——后者会凭空造出一批「未安装」。
//!
//! 缓存按 TTL 刷新；引擎每个采样拍问一次，命中 TTL 就不重扫。

use crate::models::AgentProfile;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// 默认缓存年龄。装机状态变化很慢（用户装完应用才多一个），
/// 而 `/Applications` 枚举 + 逐个读 `Info.plist` 在应用多时要几十毫秒，不该每拍都做。
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

/// 扫 CLI：给出要找的命令名，返回命中的那些（小写）。
pub type CliScan = Arc<dyn Fn(&[String]) -> HashSet<String> + Send + Sync>;

#[derive(Debug, Default, Clone)]
pub struct BundleScanResult {
    /// 命中的 bundle id（小写）
    pub ids: HashSet<String>,
    /// 这份扫描**能否支撑否定结论**。`false` ⇒ 「没扫到」只是没证据，
    /// 界面不得据此宣布未安装（Windows 无应用包概念、plist 读不出来都落在这里）。
    pub can_prove_absence: bool,
}

impl BundleScanResult {
    /// 真实扫描的起始值：**没有应用包这个概念的平台一律不可否定**。
    fn unsupported() -> Self {
        Self {
            ids: HashSet::new(),
            can_prove_absence: false,
        }
    }
}

pub type BundleScan = Arc<dyn Fn() -> BundleScanResult + Send + Sync>;

pub struct InstalledApps {
    /// 要找的命令名（小写），由档案 `process_names` 汇总而来。
    ///
    /// **不另维护一份「已知 CLI 名」清单**：那份清单漏一条，那个档案就永远显示未安装，
    /// 而漏的那条没有测试会红——它只在某台机器上悄悄错。
    cli_names: Vec<String>,
    clis: HashSet<String>,
    bundles: HashSet<String>,
    bundle_can_prove_absence: bool,
    last_refresh: Option<SystemTime>,
    scan_clis: CliScan,
    scan_bundles: BundleScan,
}

impl InstalledApps {
    /// 生产实例：要找的命令名由调用方从档案集合里汇总（见 [`Self::cli_names_for`]）。
    pub fn new(cli_names: Vec<String>) -> Self {
        Self::with_scanners(cli_names, Arc::new(scan_clis), Arc::new(scan_bundles))
    }

    /// 注入扫描器。**测试专用**——它让「扫到 / 没扫到 / 读坏了」这些形态可以离线构造。
    pub fn with_scanners(
        cli_names: Vec<String>,
        scan_clis: CliScan,
        scan_bundles: BundleScan,
    ) -> Self {
        let mut names = cli_names;
        names.sort();
        names.dedup();
        for name in &mut names {
            *name = name.to_lowercase();
        }
        Self {
            cli_names: names,
            clis: HashSet::new(),
            bundles: HashSet::new(),
            bundle_can_prove_absence: false,
            // 冷启动：未热 ⇒ `is_installed` 一律 `None`，不给假否定
            last_refresh: None,
            scan_clis,
            scan_bundles,
        }
    }

    /// 从档案集合汇总要探测的命令名（已小写、去重、排序）。GUI 应用名也在里面——
    /// 同名的 CLI 装了就算命中，找不着只是白找几个 `stat`，不会给出错误结论。
    pub fn cli_names_for(profiles: &[AgentProfile]) -> Vec<String> {
        let mut names: Vec<String> = profiles
            .iter()
            .flat_map(|p| p.process_names.iter())
            .map(|n| n.to_lowercase())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn is_warmed(&self) -> bool {
        self.last_refresh.is_some()
    }

    /// 缓存够新就跳过重扫。返回是否真的重扫过（便于用例观察命中）。
    pub fn refresh_if_needed(&mut self, max_age: Duration) -> bool {
        if let Some(last) = self.last_refresh {
            if let Ok(age) = SystemTime::now().duration_since(last) {
                if age < max_age {
                    return false;
                }
            }
        }
        self.refresh();
        true
    }

    pub fn refresh(&mut self) {
        let found_clis = (self.scan_clis)(&self.cli_names);
        let found_bundles = (self.scan_bundles)();

        // 「这趟没扫到」不能当成「全卸载了」：沿用上一份非空值，
        // 否则一次失败的扫描会把所有 CLI 档案判成未安装，界面上却没有任何说明。
        if found_clis.is_empty() && !self.clis.is_empty() {
            // 沿用旧值
        } else {
            self.clis = found_clis;
        }
        self.bundles = found_bundles.ids;
        self.bundle_can_prove_absence = found_bundles.can_prove_absence;
        self.last_refresh = Some(SystemTime::now());
    }

    /// 档案是否已安装。**`None` = 没核实，不是「没装」**（见模块头第 3 条）。
    pub fn is_installed(&self, profile: &AgentProfile) -> Option<bool> {
        if !self.is_warmed() {
            return None;
        }
        if profile
            .bundle_ids
            .iter()
            .any(|b| self.bundles.contains(&b.to_lowercase()))
        {
            return Some(true);
        }
        if profile
            .process_names
            .iter()
            .any(|n| self.clis.contains(&n.to_lowercase()))
        {
            return Some(true);
        }
        // 档案登记了应用包、而这趟扫描没有否定能力 ⇒ 缺的是证据，不是「没装」
        if !profile.bundle_ids.is_empty() && !self.bundle_can_prove_absence {
            return None;
        }
        Some(false)
    }
}

// MARK: - 真实扫描（生产默认）

#[cfg(windows)]
fn path_list() -> Vec<String> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[cfg(not(windows))]
fn path_list() -> Vec<String> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(str::to_string)
        .collect()
}

/// PATH 之外常见的安装位置：这些目录里的 CLI 实际可用，
/// 只是没被写进 PATH（homebrew、npm 全局目录、用户自建 bin）。
fn extra_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    #[cfg(windows)]
    {
        let mut dirs: Vec<PathBuf> = [home.join("bin"), home.join("AppData/Roaming/npm")]
            .into_iter()
            .collect();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs"));
        }
        dirs
    }
    #[cfg(not(windows))]
    {
        vec![
            home.join(".local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            home.join("bin"),
        ]
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        // Windows 上可执行位没有意义，扩展名才决定能不能跑
        path.is_file()
    }
}

/// 命令名 → 候选文件名。Windows 要补 `PATHEXT` 里那一串后缀。
fn candidate_names(cli: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        let mut names: Vec<String> = exts
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| format!("{cli}{}", e.to_lowercase()))
            .collect();
        names.push(cli.to_string());
        names
    }
    #[cfg(not(windows))]
    {
        vec![cli.to_string()]
    }
}

pub fn scan_clis(names: &[String]) -> HashSet<String> {
    let mut dirs = path_list().into_iter().map(PathBuf::from).collect::<Vec<_>>();
    dirs.extend(extra_dirs());
    let mut found = HashSet::new();
    for cli in names {
        for dir in &dirs {
            if candidate_names(cli)
                .iter()
                .any(|name| is_executable(&dir.join(name)))
            {
                found.insert(cli.clone());
                break;
            }
        }
    }
    found
}

/// 应用包目录。macOS 是 `/Applications` 与 `~/Applications`。
#[cfg(not(windows))]
fn app_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        dirs::home_dir().unwrap_or_default().join("Applications"),
    ]
}

pub fn scan_bundles() -> BundleScanResult {
    #[cfg(windows)]
    {
        // Windows 没有 bundle 这个概念，本仓也没有「已安装 GUI 应用」的证据源。
        // 明确返回「不可否定」：让 GUI 档案落到 `None`（未核实），
        // 而不是被 PATH 扫描判成「未安装」。
        return BundleScanResult::unsupported();
    }
    #[cfg(not(windows))]
    {
        let mut result = BundleScanResult::unsupported();
        let mut ids = HashSet::new();
        // 读到但形状不认识的 plist（多为 binary plist `bplist00`）——非零即说明这趟扫描有盲区，
        // 于是它不再有否定能力，相关档案落「未核实」而不是「未安装」
        let mut unreadable = 0usize;
        for dir in app_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("app") {
                    continue;
                }
                let plist = path.join("Contents/Info.plist");
                match std::fs::read(&plist) {
                    Ok(bytes) => match bundle_id_from_plist(&bytes) {
                        Some(id) => {
                            ids.insert(id.to_lowercase());
                        }
                        // 读到但形状不认识（多为 binary plist `bplist00`）：
                        // 计入盲区，于是这趟扫描不再有否定能力
                        None => unreadable += 1,
                    },
                    Err(_) => unreadable += 1,
                }
            }
        }
        result.ids = ids;
        result.can_prove_absence = unreadable == 0;
        result
    }
}

/// 从 `Info.plist` 里取 `CFBundleIdentifier`。
///
/// **只认 XML plist。** Swift 用 `PropertyListSerialization`，两种形状都吃；
/// Rust 侧不引新依赖，于是 binary plist（`bplist00` 开头）落到 `None`。
/// 这不是缺陷：`None` 会让这趟扫描**失去否定能力**，于是相关档案给「未核实」——
/// 而错报「未安装」是要用户去核对事实的，代价远大于暂时不肯定。
fn bundle_id_from_plist(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"bplist00") {
        return None;
    }
    let text = String::from_utf8_lossy(bytes);
    let key = text.find("<key>CFBundleIdentifier</key>")?;
    let rest = &text[key..];
    let open = rest.find("<string>")? + "<string>".len();
    let close = rest[open..].find("</string>")? + open;
    let id = rest[open..close].trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Sandbox;

    fn profile_with(process: &[&str], bundles: &[&str]) -> AgentProfile {
        AgentProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: process.iter().map(|s| (*s).into()).collect(),
            bundle_ids: bundles.iter().map(|s| (*s).into()).collect(),
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
            session_database: None,
            category: "assistant".into(),
        }
    }

    fn scanner(found: &[&str]) -> CliScan {
        let found: HashSet<String> = found.iter().map(|s| (*s).into()).collect();
        Arc::new(move |_names: &[String]| found.clone())
    }

    fn bundle_scanner(result: BundleScanResult) -> BundleScan {
        Arc::new(move || result.clone())
    }

    fn bundles(ids: &[&str], can_prove: bool) -> BundleScanResult {
        BundleScanResult {
            ids: ids.iter().map(|s| (*s).into()).collect(),
            can_prove_absence: can_prove,
        }
    }

    #[test]
    fn cold_cache_reports_unknown_never_absent() {
        let cache = InstalledApps::with_scanners(
            vec!["fixture".into()],
            scanner(&[]),
            bundle_scanner(bundles(&[], true)),
        );
        assert!(!cache.is_warmed());
        assert_eq!(cache.is_installed(&profile_with(&["fixture"], &[])), None);
    }

    #[test]
    fn cli_and_bundle_hits_both_count_as_installed() {
        let mut cache = InstalledApps::with_scanners(
            vec!["fixture".into()],
            scanner(&["fixture"]),
            bundle_scanner(bundles(&["com.example.app"], true)),
        );
        cache.refresh();
        assert_eq!(cache.is_installed(&profile_with(&["fixture"], &[])), Some(true));
        assert_eq!(
            cache.is_installed(&profile_with(&["other"], &["com.example.app"])),
            Some(true)
        );
    }

    #[test]
    fn a_gui_profile_without_bundle_evidence_is_unknown_not_absent() {
        // 这一条是本模块存在的理由：GUI 档案（Cursor 之类）没有 CLI 可探测。
        // 若这里返回 Some(false)，界面会给出一批凭空来的「未安装」。
        let mut cache = InstalledApps::with_scanners(
            vec!["code".into()],
            scanner(&[]),
            bundle_scanner(bundles(&[], false)),
        );
        cache.refresh();
        let gui = profile_with(&["Code"], &["com.microsoft.VSCode"]);
        assert_eq!(cache.is_installed(&gui), None);
        // 纯 CLI 档案没有 bundle id，不受这条影响——否定成立
        assert_eq!(cache.is_installed(&profile_with(&["codex"], &[])), Some(false));
    }

    #[test]
    fn an_empty_scan_does_not_erase_the_previous_cli_set() {
        // 防的症状：一次失败的扫描把所有 CLI 档案判成未安装，而界面上没有任何说明。
        let hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = hit.clone();
        let mut cache = InstalledApps::with_scanners(
            vec!["fixture".into()],
            Arc::new(move |_names: &[String]| {
                if flag.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    HashSet::from(["fixture".to_string()])
                } else {
                    HashSet::new()
                }
            }),
            bundle_scanner(bundles(&[], true)),
        );
        cache.refresh();
        assert_eq!(cache.is_installed(&profile_with(&["fixture"], &[])), Some(true));
        cache.refresh();
        assert_eq!(
            cache.is_installed(&profile_with(&["fixture"], &[])),
            Some(true),
            "第二趟扫到空集应当沿用上一份非空值"
        );
    }

    #[test]
    fn ttl_cache_skips_a_second_scan() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        let mut cache = InstalledApps::with_scanners(
            vec!["fixture".into()],
            Arc::new(move |_names: &[String]| {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                HashSet::new()
            }),
            bundle_scanner(bundles(&[], true)),
        );
        assert!(cache.refresh_if_needed(Duration::from_secs(300)));
        assert!(!cache.refresh_if_needed(Duration::from_secs(300)));
        assert!(cache.refresh_if_needed(Duration::ZERO));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    }


    #[test]
    fn plist_reader_takes_xml_and_refuses_binary() {
        let xml = br#"<?xml version="1.0"?>
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key>
  <string>com.example.app</string>
</dict></plist>"#;
        assert_eq!(bundle_id_from_plist(xml).as_deref(), Some("com.example.app"));
        assert_eq!(bundle_id_from_plist(b"bplist00\x00\x01binary"), None);
        assert_eq!(bundle_id_from_plist(b"not a plist"), None);
    }

    #[test]
    fn cli_names_come_from_profiles_and_are_normalized() {
        let mut a = profile_with(&["Code", "codex"], &[]);
        a.id = "a".into();
        let mut b = profile_with(&["codex"], &[]);
        b.id = "b".into();
        assert_eq!(InstalledApps::cli_names_for(&[a, b]), vec!["code", "codex"]);
    }

    #[test]
    fn production_scanners_run_without_panicking() {
        // 真机形状的冒烟：读的是这台机器自己的 PATH 与 /Applications，
        // 因此只断言「能跑完、形状自洽」，不断言扫到了什么。
        let names = vec!["sh".to_string()];
        let clis = scan_clis(&names);
        assert!(clis.is_subset(&names.into_iter().collect()));
        // 读不出形状的 plist 一旦非零，这趟扫描就不再有否定能力（`scan_bundles` 内已折进标志位）
        let result = scan_bundles();
        assert!(
            result.ids.iter().all(|id| id == &id.to_lowercase()),
            "bundle id 应当已小写化"
        );
    }

    #[test]
    fn a_sandbox_scan_does_not_touch_the_users_applications() {
        // 扫描器是注入的：用例里出现的目录只能是沙箱——
        // 真实扫描会去读这台机器的 PATH 与 /Applications，那不该由断言决定。
        let sandbox = Sandbox::new("installed");
        let dir = sandbox.path().join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        let probe = dir.clone();
        let found: CliScan = Arc::new(move |names: &[String]| {
            let mut set = HashSet::new();
            if probe.join("fixture").is_file() {
                set.insert("fixture".to_string());
            }
            assert!(
                names.iter().all(|n| n == &"fixture".to_string()),
                "档案汇总出来的命令名应当只有 fixture"
            );
            set
        });
        let mut cache = InstalledApps::with_scanners(
            vec!["fixture".into()],
            found,
            bundle_scanner(bundles(&[], true)),
        );
        // 没装：CLI 名字查得到，文件不存在
        cache.refresh();
        assert_eq!(cache.is_installed(&profile_with(&["fixture"], &[])), Some(false));
        // 装上：同一份缓存重扫后转为命中
        std::fs::write(dir.join("fixture"), b"#!/bin/sh\n").unwrap();
        cache.refresh();
        assert_eq!(cache.is_installed(&profile_with(&["fixture"], &[])), Some(true));
    }
}

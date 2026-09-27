//! 极简待办清单（Phase 3）。
//!
//! 三个刻意保持简单的决定：
//! ① **无分组、无日期、无优先级**——「极简列表，回车加条，勾选划掉」是这一期的全部范围；
//! ② **顺序就是插入顺序**，做完的不下沉（下沉会让「我刚才加的那条跑哪去了」变成问题）；
//! ③ **id 用数字**，取「现有最大 id + 1」。这样重启后不会重号，
//!    也不需要靠时钟或进程内计数器——那两样都会在重启/并发时撞号。
//!
//! 存储是单个 JSON 文件，原子替换（`atomicfile`）。**读坏了不 panic**：
//! 降级为空清单，并且**先把坏文件改名留档**再继续——直接覆盖等于把用户的东西删了还不说。

use crate::atomicfile::atomic_replace_validated;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

/// 单条待办
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    /// 纯数字字符串（见文件头第 ③ 条）
    pub id: String,
    pub text: String,
    pub done: bool,
    pub created_ms: i64,
}

/// 一次操作之后返回给界面的完整状态。
///
/// 一次返回全部（而不是只回被改动的那条）：界面拿到的永远是**同一个视图**，
/// 不会出现「列表是新鲜的、计数是旧的」这种对不上的状态。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TodoList {
    pub items: Vec<Todo>,
    /// **只数未完成**：侧栏角标用这个数。数「总条数」会让角标在勾掉之后还不消失
    pub pending: usize,
    /// 读坏过文件时，这里是留档文件名（界面应当说出来，而不是让用户以为清单被清空了）
    pub broken_backup: Option<String>,
}

/// 单条文本的上限。粘贴一屏东西进来会把文件与界面都撑坏，
/// 而且待办本来就不该是长文——超了**拒绝**，不静默截断（静默截断会让人以为存进去了）。
pub const MAX_TEXT_CHARS: usize = 500;

pub struct TodoStore {
    dir: PathBuf,
}

impl TodoStore {
    pub fn new(dir: PathBuf) -> Self {
        TodoStore { dir }
    }

    pub fn at_default() -> Self {
        TodoStore::new(crate::settings::config_dir())
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("todos.json")
    }

    /// 读清单。**任何读/解析失败都降级为空**：一份坏文件不该让应用起不来。
    /// 返回 `(清单, 是否读坏过)`。
    fn load(&self) -> (Vec<Todo>, bool) {
        let Ok(text) = fs::read_to_string(self.path()) else {
            return (Vec::new(), false); // 不存在也算「没坏」，只是还没有
        };
        match serde_json::from_str::<Vec<Todo>>(&text) {
            Ok(items) => (items, false),
            Err(_) => (Vec::new(), true),
        }
    }

    /// 写清单（原子）。返回是否成功。
    fn save(&self, items: &[Todo]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_string_pretty(items)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        atomic_replace_validated(&self.path(), json.as_bytes(), |staged| {
            let text = fs::read_to_string(staged)?;
            serde_json::from_str::<Vec<Todo>>(&text)
                .map(|_| ())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "todo JSON"))
        })
    }

    /// 读坏的文件留档（`todos.json.broken-<时间戳>`），返回留档文件名。
    /// 不留档就直接写，等于把用户的东西删掉还说都没说。
    fn stash_broken(&self, now_ms: i64) -> Option<String> {
        let name = format!("todos.json.broken-{now_ms}");
        fs::rename(self.path(), self.dir.join(&name)).ok()?;
        Some(name)
    }

    /// 读（给界面）。读坏过就顺手留档，并把留档名带回界面。
    pub fn snapshot(&self, now_ms: i64) -> TodoList {
        let (items, broken) = self.load();
        let broken_backup = if broken {
            self.stash_broken(now_ms)
        } else {
            None
        };
        TodoList {
            pending: items.iter().filter(|t| !t.done).count(),
            items,
            broken_backup,
        }
    }

    /// 加一条。空白文本拒绝；超长拒绝（不截断）。
    pub fn add(&self, text: &str, now_ms: i64) -> Result<TodoList, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("待办内容不能是空的".into());
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(format!("待办内容超过 {MAX_TEXT_CHARS} 字"));
        }
        let (mut items, broken) = self.load();
        let broken_backup = if broken { self.stash_broken(now_ms) } else { None };
        let next_id = items
            .iter()
            .filter_map(|t| t.id.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        items.push(Todo {
            id: next_id.to_string(),
            text: text.to_string(),
            done: false,
            created_ms: now_ms,
        });
        self.save(&items).map_err(|e| format!("保存失败：{e}"))?;
        Ok(TodoList {
            pending: items.iter().filter(|t| !t.done).count(),
            items,
            broken_backup,
        })
    }

    /// 勾选/取消勾选
    pub fn toggle(&self, id: &str) -> Result<TodoList, String> {
        let (mut items, broken) = self.load();
        let broken_backup = if broken { self.stash_broken(0) } else { None };
        let Some(item) = items.iter_mut().find(|t| t.id == id) else {
            return Err(format!("没有这条待办：{id}"));
        };
        item.done = !item.done;
        self.save(&items).map_err(|e| format!("保存失败：{e}"))?;
        Ok(TodoList {
            pending: items.iter().filter(|t| !t.done).count(),
            items,
            broken_backup,
        })
    }

    pub fn remove(&self, id: &str) -> Result<TodoList, String> {
        let (mut items, broken) = self.load();
        let broken_backup = if broken { self.stash_broken(0) } else { None };
        let before = items.len();
        items.retain(|t| t.id != id);
        if items.len() == before {
            return Err(format!("没有这条待办：{id}"));
        }
        self.save(&items).map_err(|e| format!("保存失败：{e}"))?;
        Ok(TodoList {
            pending: items.iter().filter(|t| !t.done).count(),
            items,
            broken_backup,
        })
    }

    /// 清掉已完成。**只删已完成的**，未完成的一条不碰。
    pub fn clear_done(&self) -> Result<TodoList, String> {
        let (mut items, broken) = self.load();
        let broken_backup = if broken { self.stash_broken(0) } else { None };
        items.retain(|t| !t.done);
        self.save(&items).map_err(|e| format!("保存失败：{e}"))?;
        Ok(TodoList {
            pending: items.iter().filter(|t| !t.done).count(),
            items,
            broken_backup,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "agentisland-todos-{}-{}-{serial}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn store(&self) -> TodoStore {
            TodoStore::new(self.0.clone())
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn add_toggle_remove_clear_round_trips() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        assert_eq!(store.snapshot(0).items, Vec::new());

        let after_add = store.add("写 Phase 3", 1_000).unwrap();
        assert_eq!(after_add.items.len(), 1);
        assert_eq!(after_add.items[0].id, "1");
        assert_eq!(after_add.pending, 1);
        assert!(!after_add.items[0].done);

        let after_second = store.add("再写一条", 2_000).unwrap();
        assert_eq!(after_second.items[1].id, "2", "id 递增而不是复用");

        // 勾掉第一条：清单顺序不变，未完成数减一
        let after_toggle = store.toggle("1").unwrap();
        assert!(after_toggle.items[0].done);
        assert_eq!(after_toggle.pending, 1);
        assert_eq!(after_toggle.items[1].text, "再写一条");

        // 取消勾选
        let back = store.toggle("1").unwrap();
        assert!(!back.items[0].done);
        assert_eq!(back.pending, 2);

        // 删一条
        let after_remove = store.remove("2").unwrap();
        assert_eq!(after_remove.items.len(), 1);
        assert_eq!(after_remove.pending, 1);
        assert_eq!(after_remove.items[0].id, "1");

        // 清已完成：先勾上再清
        store.toggle("1").unwrap();
        let cleared = store.clear_done().unwrap();
        assert!(cleared.items.is_empty());
        assert_eq!(cleared.pending, 0);
        // 真的落盘了（换一个 store 实例读）
        assert!(sandbox.store().snapshot(0).items.is_empty());
    }

    #[test]
    fn clear_done_only_touches_finished_items() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        store.add("未完成 A", 1).unwrap();
        store.add("已完成 B", 2).unwrap();
        store.add("未完成 C", 3).unwrap();
        store.toggle("2").unwrap();
        let after = store.clear_done().unwrap();
        assert_eq!(
            after.items.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            vec!["未完成 A", "未完成 C"],
            "清已完成不该动未完成的"
        );
        assert_eq!(after.pending, 2);
    }

    #[test]
    fn a_corrupted_file_degrades_to_empty_and_is_stashed_not_destroyed() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        fs::write(store.path(), "{ 这不是 JSON").unwrap();
        let snapshot = store.snapshot(7_777);
        assert!(snapshot.items.is_empty(), "坏文件降级为空清单");
        assert_eq!(snapshot.pending, 0);
        let backup = snapshot.broken_backup.expect("坏文件必须留档");
        assert_eq!(backup, "todos.json.broken-7777");
        assert_eq!(
            fs::read_to_string(sandbox.0.join(&backup)).unwrap(),
            "{ 这不是 JSON",
            "留档内容应与原文件一致"
        );
        assert!(!store.path().exists(), "留档之后原文件不应还在");
        // 留档之后还能正常用
        let after = store.add("恢复之后再加一条", 8_000).unwrap();
        assert_eq!(after.items.len(), 1);
        assert_eq!(after.broken_backup, None);
    }

    #[test]
    fn a_stale_broken_file_is_stashed_before_the_first_write() {
        // 先不读（跳过 snapshot），直接写：坏文件必须被留档，而不是被覆盖
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        fs::write(store.path(), "garbage").unwrap();
        let after = store.add("新的一条", 42).unwrap();
        assert_eq!(after.broken_backup.as_deref(), Some("todos.json.broken-42"));
        assert_eq!(
            fs::read_to_string(sandbox.0.join("todos.json.broken-42")).unwrap(),
            "garbage"
        );
        assert_eq!(after.items.len(), 1);
    }

    #[test]
    fn empty_and_overlong_text_are_rejected_instead_of_silently_stored() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        assert!(store.add("", 1).is_err());
        assert!(store.add("   \n  ", 1).is_err());
        let long = "字".repeat(MAX_TEXT_CHARS + 1);
        assert!(store.add(&long, 1).is_err());
        let ok = "字".repeat(MAX_TEXT_CHARS);
        assert!(store.add(&ok, 1).is_ok());
        assert_eq!(store.snapshot(0).items.len(), 1);
        // 前后空白被裁掉
        let trimmed = store.add("  有内容  ", 2).unwrap();
        assert_eq!(trimmed.items[1].text, "有内容");
    }

    #[test]
    fn unknown_ids_are_reported_not_silently_ignored() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        store.add("存在的一条", 1).unwrap();
        assert!(store.toggle("99").is_err());
        assert!(store.remove("99").is_err());
        assert_eq!(store.snapshot(0).items.len(), 1, "报错时不该改动清单");
    }

    #[test]
    fn writing_leaves_no_staging_file_behind() {
        let sandbox = Sandbox::new();
        let store = sandbox.store();
        store.add("一条", 1).unwrap();
        store.toggle("1").unwrap();
        store.remove("1").unwrap();
        let entries: Vec<String> = fs::read_dir(&sandbox.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["todos.json"], "暂存文件泄漏：{entries:?}");
    }

    /// id 的规则：**不与现存 id 撞号**，但**可以复用已被删掉的号**。
    ///
    /// 我第一版把期望写成「不重用已删的号」，结果红了——错的是期望不是实现：
    /// 「现有最大值 + 1」防的是**撞号**（`len + 1` 会撞：`[1,3]` 的下一个是 `3`），
    /// 而复用已删号在待办这种场景里无害——界面每次都按完整快照重画，
    /// 不存在「拿着旧 id 的陈旧引用」。要真的不复用，就得在文件里多存一个计数器，
    /// 那点复杂度换不来任何东西。
    #[test]
    fn ids_never_collide_with_existing_ones_across_a_restart() {
        let sandbox = Sandbox::new();
        {
            let store = sandbox.store();
            store.add("一", 1).unwrap();
            store.add("二", 2).unwrap();
            store.add("三", 3).unwrap();
            store.remove("2").unwrap();
        }
        // 「重启」：新实例从文件里算出下一个 id。剩的是 1 与 3，所以下一个必须是**不撞号**的
        let store = sandbox.store();
        let after = store.add("四", 4).unwrap();
        let ids: Vec<&str> = after.items.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "3", "4"], "取现有最大值 + 1");
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "id 不许重复：{ids:?}");
    }
}

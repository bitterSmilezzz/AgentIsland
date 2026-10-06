//! 第三方 Agent 库的**只读**访问层。
//!
//! 迁移自 Swift `ReadonlyDB`（ADR 0010 / M3 的 token 明细组）。与那边同口径：
//! 只读打开、**等锁**而不是当场判失败、永不写。
//!
//! 与 Swift 的两处**有意差异**（写在这里，免得日后被当成漏搬）：
//! · **不缓存连接**。Swift 缓存句柄是因为一次采样拍里 5 个探测器要反复查同一个库；
//!   Rust 这里每档案每拍只查一次，省下的 open（Swift 实测 50–150µs）不值一个
//!   「外部重建库后句柄指向旧 inode」的失效面。每次新建、用完即关。
//! · 失败原因给 SQLite 的诊断文本，而不是 Swift 那个数字返回码。

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// 只读连接的等锁上限（毫秒）。与 Swift `ReadonlyDB.busyTimeoutMs` **是同一个数**。
/// 不设它，一次瞬态争用会被下游记成「这个源读不到」——依据见 CONTEXT
/// 「只读库的锁争用」：官方 WAL 文档列出只读方仍会撞上 `SQLITE_BUSY` 的三种情形。
pub const BUSY_TIMEOUT_MS: u64 = 1000;

/// 打不开的原因。只回 `None` 的调用方分不出「库里没数据」与「这个源根本读不到」，
/// 而会话探测需要用后者（Swift 的 `ConnectionFailure` 同口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 文件不存在（Agent 没跑过 / 库换了位置）
    Missing,
    /// 文件在但打不开（权限、被独占、不是库文件…），带 SQLite 的诊断文本
    OpenFailed(String),
}

/// 只读打开。调用方负责用完即弃（`Connection` 离开作用域即关）。
///
/// 契约（与 Swift 逐条一致）：
/// · `SQLITE_OPEN_READ_ONLY`——绝不创建缺失的库，也绝不写；
/// · 不设 `journal_mode`、不做 checkpoint、**不用 `immutable=1`**
///   （后者等于声明文件不会变，对正在被 Agent 写的库会读到陈旧内容）；
/// · 等锁上限见 [`BUSY_TIMEOUT_MS`]。
pub fn open_readonly(path: &str) -> Result<Connection, Failure> {
    if !Path::new(path).is_file() {
        return Err(Failure::Missing);
    }
    // FULL_MUTEX：让 SQLite 自己串行化。目前没有跨线程共享连接，但这一层是公共面，
    // 宁可让 SQLite 兜住，也不留一个「谁先跨线程谁踩」的坑。
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| Failure::OpenFailed(error.to_string()))?;
    // 设不上只意味着「不再等锁」，不影响连接可用性，因此不为此拒绝这次打开
    let _ = connection.busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));
    Ok(connection)
}

// MARK: - OpenCode 方言的表名

/// OpenCode 一族（本体与同表结构的 fork，如小米 MiMo Code）的**真实表名**。
///
/// **表名在版本之间变过**：老库是 `message` / `session`，当前版本迁到了
/// `session_message` / `session_v2`（本机 `opencode.db` 实测，最新一条 migration
/// 为 `20260923013825_project_time_active`）。表名不能绑参，拼进 SQL 是唯一写法，
/// 所以这里一次查 `sqlite_master` 定下名字，之后所有 SQL 都用 [`Self`] 里的字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCodeTables {
    /// 消息表：`session_message` 或 `message`
    pub message: &'static str,
    /// 会话表：本方言所有查询都不强制要它，缺了就查不到会话钻取那一层
    pub session: Option<&'static str>,
}

/// 表名候选。**只认这些字面量**——不接受任何外部输入，因此拼进 SQL 没有注入面。
const MESSAGE_TABLES: [&str; 2] = ["session_message", "message"];
const SESSION_TABLES: [&str; 2] = ["session_v2", "session"];

impl OpenCodeTables {
    /// 读出这张库里**实际存在**的表名。两种 schema 都能认；
    /// 两种 message 表都没有 ⇒ 这不是这一族的库，返回 `None`。
    ///
    /// 顺序上优先新名：万一某个 fork 同时留着两张表，新的是当前在写的那张。
    pub fn resolve(connection: &Connection) -> Option<Self> {
        let message = MESSAGE_TABLES
            .iter()
            .find(|name| table_exists(connection, name))?;
        let session = SESSION_TABLES
            .iter()
            .find(|name| table_exists(connection, name))
            .copied();
        Some(Self { message, session })
    }
}

fn table_exists(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |_| Ok(()),
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 沙箱取名统一走 `testutil`：并行跑时 `as_nanos()` 会撞名，两个用例共用一个目录，
    /// 先结束的那个把另一个的文件删掉——症状是「偶发挂几条」。
    fn temp_path(name: &str) -> String {
        crate::testutil::temp_dir("sqlite")
            .join(name)
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn missing_file_is_reported_as_missing_not_as_an_open_failure() {
        let path = temp_path("nope.db");
        assert_eq!(open_readonly(&path).unwrap_err(), Failure::Missing);
    }

    #[test]
    fn read_only_connection_really_cannot_write() {
        // 「只读那侧永远只读」不是靠自觉：这里真的去写一次，必须被 SQLite 挡回来。
        let path = temp_path("writable.db");
        {
            let seed = Connection::open(&path).unwrap();
            seed.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('ok');")
                .unwrap();
        }
        let readonly = open_readonly(&path).expect("只读打开应成功");
        let write = readonly.execute("INSERT INTO t VALUES ('nope')", []);
        assert!(write.is_err(), "只读连接竟然写成功了：{write:?}");
        let ddl = readonly.execute("CREATE TABLE t2 (v TEXT)", []);
        assert!(ddl.is_err(), "只读连接竟然建表了：{ddl:?}");
        // 读当然要能读
        let value: String = readonly
            .query_row("SELECT v FROM t", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "ok");
    }

    #[test]
    fn both_open_code_schemas_resolve_to_the_table_that_actually_exists() {
        // 这一族换过表名。只认老名字的话，当前版本的库会**静默读到零**——
        // 而「读到零」在界面上和「真的没用过」长得一模一样。
        let legacy = temp_path("legacy.db");
        {
            let seed = Connection::open(&legacy).unwrap();
            seed.execute_batch("CREATE TABLE message (data TEXT); CREATE TABLE session (id TEXT);")
                .unwrap();
        }
        let resolved = OpenCodeTables::resolve(&open_readonly(&legacy).unwrap()).unwrap();
        assert_eq!(resolved.message, "message");
        assert_eq!(resolved.session, Some("session"));

        let current = temp_path("current.db");
        {
            let seed = Connection::open(&current).unwrap();
            seed.execute_batch(
                "CREATE TABLE session_message (data TEXT); CREATE TABLE session_v2 (id TEXT);",
            )
            .unwrap();
        }
        let resolved = OpenCodeTables::resolve(&open_readonly(&current).unwrap()).unwrap();
        assert_eq!(resolved.message, "session_message");
        assert_eq!(resolved.session, Some("session_v2"));

        // 别的 fork（同表结构但只有消息表）也要能用，只是拿不到会话钻取
        let message_only = temp_path("message-only.db");
        {
            let seed = Connection::open(&message_only).unwrap();
            seed.execute_batch("CREATE TABLE message (data TEXT);")
                .unwrap();
        }
        let resolved = OpenCodeTables::resolve(&open_readonly(&message_only).unwrap()).unwrap();
        assert_eq!(resolved.message, "message");
        assert_eq!(resolved.session, None);

        // 两张表都没有 ⇒ 不是这一族，返回 None 而不是随便挑一个名字去查
        let alien = temp_path("alien.db");
        {
            let seed = Connection::open(&alien).unwrap();
            seed.execute_batch("CREATE TABLE kv (k TEXT);").unwrap();
        }
        assert!(OpenCodeTables::resolve(&open_readonly(&alien).unwrap()).is_none());
    }

    #[test]
    fn a_file_that_is_not_a_database_fails_without_panicking() {
        // 第三方库坏了 / 根本不是库：必须走「读不到」这条路，不许 panic。
        // 失败可能发生在打开，也可能发生在第一次查询——两处都算读不到。
        let path = temp_path("garbage.db");
        fs::write(&path, b"this is definitely not sqlite").unwrap();
        let outcome = open_readonly(&path).and_then(|connection| {
            connection
                .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| Failure::OpenFailed(error.to_string()))
        });
        assert!(outcome.is_err(), "非库文件应当读不到，而不是当成空库");
        let _ = fs::remove_file(&path);
    }
}

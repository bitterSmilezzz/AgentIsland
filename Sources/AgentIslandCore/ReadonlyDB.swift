import Foundation
import SQLite3

// MARK: - SQLite 只读访问层（主线程探测走缓存连接，后台流水走专用连接）

/// SQLite 只读访问层：AgentActionInspector 的 5 个探测器与 AgentLogStreamer 的
/// 5 个 DB 流水源共用。此前各处手写 open/prepare/finalize/close 样板，且每拍对
/// 大库（数百 MB）现开现关——重复支付 open 成本（~50–150µs/次）并抖动文件缓存。
///
/// 连接缓存（按路径）：首次 open 后长驻复用；每次调用用 inode 校验外部替换
/// （库被删除/重建后旧连接对新文件无效，stat 一次即决定复用或重开）。
/// 连接集合有上界（每个出现过的库路径各一条，当前 ≤6 条）。
///
/// 线程契约：`withConnection` 的锁**跨整段 SQL 持有**（不可重入，见下），因此它只服务
/// 主线程的采样拍与探测。实时流水页在后台队列刷新，走 `withDedicatedConnection`——
/// 否则一次 500 行扫描就会把主线程那一拍堵住。
enum ReadonlyDB {

    /// 文本参数析构器 TRANSIENT：让 SQLite 自行复制一份。
    /// Swift 侧 `String` 桥接出的 C 缓冲区**只在 `sqlite3_bind_text` 那一行有效**，而绑定
    /// 之后才 `step`；传 `nil`（= SQLITE_STATIC）等于让 SQLite 在缓冲区可能已被回收之后才去读
    /// ——轻则把别的字符串当 session_id 查错会话，重则崩溃。
    /// C 的 `SQLITE_TRANSIENT` 是 `((sqlite3_destructor_type)-1)` 强转宏，不导入 Swift，故自建。
    static let transientDestructor = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    /// 只读连接的**等锁上限（毫秒）**，全仓唯一的这个数——`TokenUsageMonitor` 的只读层
    /// 也读它，不许各留一份。
    ///
    /// 为什么必须有：官方 WAL 文档列出只读方仍会吃到 `SQLITE_BUSY` 的三种情形——
    /// ① 对方以 exclusive locking mode 打开（文档点名 Chrome/Firefox 就是这样，
    /// Electron 系 Agent 完全可能同形）；② 最后一个连接关闭时清理 WAL/-shm 的那一瞬；
    /// ③ 上次崩溃后第一个连接做恢复期间。**不设 busy handler 时 `SQLITE_BUSY` 立刻返回**，
    /// 于是一次瞬态争用会被下游记成「这个源读不到」——把「没读到」的口径用在了瞬态上，
    /// 而这正是本仓最在意的那类误报。
    ///
    /// 取 1000ms 而不是更长：这条路径跑在采样拍的主线程上，等太久会把那一拍堵住。
    static let busyTimeoutMs: Int32 = 1000

    private static let lock = NSLock()
    private static var connections: [String: OpaquePointer] = [:]
    /// 连接打开时文件的 (设备号, inode)：外部替换/重挂载后据此失效缓存连接
    private static var identities: [String: (dev: UInt64, inode: UInt64)] = [:]

    /// 作废某路径的缓存连接（关旧句柄，下次调用重新 open）。
    /// 供调用方在「prepare 失败」时排除句柄陈旧这一成因——见 AgentSessionInspector.withDB。
    static func invalidate(_ path: String) {
        lock.lock()
        defer { lock.unlock() }
        if let db = connections.removeValue(forKey: path) {
            sqlite3_close(db)
        }
        identities.removeValue(forKey: path)
    }

    /// 打开（或复用缓存的）只读连接并执行 `body`，返回 `body` 的结果。
    /// 库缺失 / 不可读 / open 失败时返回 nil——与调用方既有「查不到数据」语义一致。
    /// - Warning: body 执行期间持有内部锁，body 内**不得**再进 withConnection（不可重入死锁）。
    ///   锁也保证 `invalidate` 不会关掉正在使用的句柄。后台队列请改用 `withDedicatedConnection`。
    /// 需要区分「查不到数据」与「库读不到」时改用下面的 onFailure 变体。
    static func withConnection<T>(_ path: String, _ body: (OpaquePointer) -> T) -> T? {
        withConnection(path, onFailure: { _ in }, body)
    }

    /// 后台调用方专用：开一条**不进缓存**的只读连接，用完即关，全程不碰共享锁。
    /// 缓存 open 的价值只在每拍都读同一库的热路径上成立；流水页 2s 刷新一次，
    /// 一条 open（实测 ~50–150µs）远比让主线程等锁便宜。
    static func withDedicatedConnection<T>(_ path: String, _ body: (OpaquePointer) -> T) -> T? {
        // READONLY 不会创建缺失的库文件；打不开（含缺失/权限）一律返回 nil
        guard let db = openReadonly(path).handle else { return nil }
        // close_v2：body 若漏 finalize 语句，close 会返回 SQLITE_BUSY 而不是真的关掉——
        // 缓存路径上有界无所谓，专用连接每次调用都新建一条，漏一次就漏一个 fd
        defer { sqlite3_close_v2(db) }
        return body(db)
    }

    /// **只读打开的唯一出口**：`open_v2` + `busy_timeout` 都在这里设。
    /// 存在的理由就是「同一口径只有一处」：此前两条打开路径各写一遍 open，
    /// `busy_timeout` 只在别处（`TokenUsageMonitor`）设过，于是同一台机器上
    /// 「会话探测」与「token 统计」对同一类锁争用给出两种结论。
    /// 失败时返回 nil 并带上 rc（调用方要用它区分 missing 与 openFailed）；
    /// open 失败仍可能已分配 handle（rc=14 实测约 1.5KB/次），必须关掉。
    private static func openReadonly(_ path: String) -> (handle: OpaquePointer?, rc: Int32) {
        var db: OpaquePointer?
        let rc = sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil)
        guard rc == SQLITE_OK, let db else {
            if let db { sqlite3_close(db) }
            return (nil, rc)
        }
        // 瞬态 BUSY 要等，不要当场判成「读不到」——理由见 busyTimeoutMs
        sqlite3_busy_timeout(db, busyTimeoutMs)
        return (db, rc)
    }

    /// 「打不开」的原因：只返回 nil 时调用方无法区分「库里确实没数据」与
    /// 「这个源根本读不到」——会话探测需要后者（见 SessionProbeHealth）。
    enum ConnectionFailure: Equatable {
        /// 文件不存在（App 没跑过 / 库换了位置）
        case missing
        /// 文件在但 open_v2 返回非 OK（权限拒绝、被独占等），带 SQLite 返回码
        case openFailed(code: Int32)
    }

    static func withConnection<T>(_ path: String, onFailure: (ConnectionFailure) -> Void,
                                  _ body: (OpaquePointer) -> T) -> T? {
        lock.lock()
        defer { lock.unlock() }
        guard let db = connection(for: path, onFailure: onFailure) else { return nil }
        return body(db)
    }

    /// 取可用连接：文件标识未变则复用；库被删除或替换则关旧开新；失败返回 nil（不留脏缓存）
    private static func connection(for path: String,
                                   onFailure: (ConnectionFailure) -> Void) -> OpaquePointer? {
        // stat(2) 一次拿到当前 (设备号, inode)（文件不存在时为 nil）。
        // 刻意不用 FileManager.attributesOfItem / URL.resourceValues：
        // · attributesOfItem 为一次比对要分配 NSDictionary + 若干 NSNumber，本机实测
        //   26.2µs/次，stat(2) 是 0.6µs/次（缺失文件 2.0µs vs 0.8µs）；命中缓存的整次
        //   withConnection 因此从 ~27µs 降到 0.8µs。本函数每个采样拍对每个出现过的库
        //   路径各调一次，且只落在主线程（5 个探测器；未安装的 Agent 走的
        //   正是「文件不存在」这一支，每次也要抛一个 NSError）。
        // · resourceValues 的结果有毫秒级陈旧缓存窗口，「库刚被外部重建就必须在这一拍
        //   发现」正是这里要的语义——同一问题曾咬到尾读备忘，见 LogTailReader 开头注释。
        let current = identity(ofPath: path)

        if let cached = connections[path] {
            if let current, let saved = identities[path], current.dev == saved.dev, current.inode == saved.inode {
                return cached
            }
            // 库被删除或已被替换：旧连接作废
            sqlite3_close(cached)
            connections[path] = nil
            identities[path] = nil
            guard current != nil else {
                onFailure(.missing)   // 已删除，不再重开
                return nil
            }
        } else if current == nil {
            onFailure(.missing)
            return nil   // 文件不存在，免一次注定失败的 open
        }

        let (opened, rc) = openReadonly(path)
        guard let db = opened else {
            onFailure(.openFailed(code: rc))
            return nil
        }
        connections[path] = db
        identities[path] = current
        return db
    }

    /// 单次 stat(2) 取 (设备号, inode)，不经任何 Foundation 层缓存。
    /// st_dev 在 Darwin 上是有符号 Int32，用 truncatingIfNeeded 换算，避免负值 trap。
    private static func identity(ofPath path: String) -> (dev: UInt64, inode: UInt64)? {
        var st = stat()
        guard stat(path, &st) == 0 else { return nil }
        return (UInt64(truncatingIfNeeded: st.st_dev), UInt64(truncatingIfNeeded: st.st_ino))
    }
}

// MARK: - OpenCode 方言的表名

/// OpenCode 一族（本体与同表结构的 fork，如小米 MiMo Code）的**真实表名**。
///
/// **表名在版本之间变过**：老库是 `message` / `session`，当前版本迁到了
/// `session_message` / `session_v2`（本机 `opencode.db` 实测，最新一条 migration
/// 为 `20260923013825_project_time_active`）。
///
/// 为什么必须现查而不写死：写死的后果**不是报错，是读到零**——而「读到零」在界面上
/// 与「这个 Agent 真的没用过」完全一样。本仓宁可说「读不到」，也不能编一个看起来
/// 正常的数字出来。
///
/// 表名不能绑参，拼进 SQL 是唯一写法，所以候选表**只认下面这些字面量**，
/// 不接受任何外部输入——没有注入面。
enum OpenCodeTables {

    struct Tables: Equatable {
        /// 消息表：`session_message` 或 `message`
        let message: String
        /// 会话表：本方言的查询不强制要它，缺了就查不到会话钻取那一层
        let session: String?
    }

    static let messageCandidates = ["session_message", "message"]
    static let sessionCandidates = ["session_v2", "session"]

    /// 读出这张库里**实际存在**的表名。两种 schema 都能认；
    /// 两种 message 表都没有 ⇒ 这不是这一族的库，返回 `nil`。
    ///
    /// 顺序上优先新名：万一某个 fork 同时留着两张表，新的是当前在写的那张。
    static func resolve(_ db: OpaquePointer) -> Tables? {
        guard let message = messageCandidates.first(where: { tableExists(db, $0) }) else {
            return nil
        }
        return Tables(
            message: message,
            session: sessionCandidates.first(where: { tableExists(db, $0) })
        )
    }

    private static func tableExists(_ db: OpaquePointer, _ name: String) -> Bool {
        let sql = "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt else {
            sqlite3_finalize(stmt)
            return false
        }
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_text(stmt, 1, name, -1, ReadonlyDB.transientDestructor)
        return sqlite3_step(stmt) == SQLITE_ROW
    }
}

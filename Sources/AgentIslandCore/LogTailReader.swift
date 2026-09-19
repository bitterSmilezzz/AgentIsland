import Foundation

/// 有界尾读：不追随持续增长的日志；截断首行在解码前丢弃。
enum LogTailReader {
    // MARK: - 同一拍内重复尾读的合并
    //
    // 「会话强语义」与「当前动作文案」两条链路每拍各尾读一次同一个 transcript，
    // 而尾读本身比解析更贵（262KB 实测 3.7ms，120 行 JSON 解析 2ms）。文件 mtime 与
    // 长度都没变 ⇒ 尾部字节没变，直接复用上一次的行。有效期短于采样周期（2s），
    // 即使出现「同一 mtime 刻度内原地改写且长度不变」这种极端写入，最多也只陈旧半秒。
    private static let lock = NSLock()
    private struct Entry {
        let file: URL
        let mtime: Date
        let size: UInt64
        let maxLines: Int
        let maxBytes: Int
        let lines: [String]
        let until: Date
    }
    // 四个 Agent 同时在跑时单槽会互相踢出（每拍换文件 ⇒ 命中率归零）；
    // 尾读的是各自会话树里最新的少数几个文件，四槽足以覆盖常见并发规模。
    private static var entries: [Entry] = []
    private static let slotCount = 4
    private static let reuseWindow: TimeInterval = 0.5

    static func read(from file: URL, maxLines: Int, maxBytes: Int) -> [String] {
        guard maxLines > 0, maxBytes > 0 else { return [] }
        // 只有「普通文件」才进缓存；特殊文件（FIFO 等）与 stat 失败的情形退回直读，
        // 与此前行为一致。新鲜度取 stat() 而非 URL.resourceValues：后者的结果有毫秒级
        // 缓存窗口，而这里恰好要比对「几十微秒内被改写过的同一个文件」，实测会误命中
        // 一次陈旧（2s 采样节律下其他环节的毫秒级陈旧无影响，故不一并改动）。
        guard let stamp = Self.statRegular(file.path) else {
            return readUncached(file: file, maxLines: maxLines, maxBytes: maxBytes)
        }
        let now = Date()
        lock.lock()
        if let slot = entries.firstIndex(where: {
            $0.file == file && $0.mtime == stamp.mtime && $0.size == stamp.size
                && $0.maxLines == maxLines && $0.maxBytes == maxBytes && now < $0.until
        }) {
            let lines = entries[slot].lines
            lock.unlock()
            return lines
        }
        let lines = readUncached(file: file, maxLines: maxLines, maxBytes: maxBytes)
        // 新读到的放到最前面；被不同文件顶掉的是最久没用的那一头（近似 LRU，够用）
        entries.insert(Entry(file: file, mtime: stamp.mtime, size: stamp.size, maxLines: maxLines,
                             maxBytes: maxBytes, lines: lines, until: now.addingTimeInterval(reuseWindow)), at: 0)
        if entries.count > slotCount { entries.removeLast(entries.count - slotCount) }
        lock.unlock()
        return lines
    }

    /// 单次 stat() 取 mtime（纳秒、不经任何缓存）。目录同样适用，
    /// 供「目录是否变过」这类失效判定使用；`URL.resourceValues` 有毫秒级缓存窗口，
    /// 不足以支撑「刚写入就下一秒读到」的场景。
    static func statModificationDate(_ path: String) -> Date? {
        var st = stat()
        guard stat(path, &st) == 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(st.st_mtimespec.tv_sec)
            + TimeInterval(st.st_mtimespec.tv_nsec) / 1_000_000_000)
    }

    /// 单次 stat() 取 mtime（纳秒）与长度；非普通文件或取不到时返回 nil。
    private static func statRegular(_ path: String) -> (mtime: Date, size: UInt64)? {
        guard let stamp = statRegularFile(path) else { return nil }
        return (stamp.mtime, UInt64(bitPattern: Int64(stamp.size)))
    }

    /// 单次 stat() 同时取普通文件的 (mtime, size, inode)。
    /// 三个事实一次 syscall 拿齐：`URL.resourceValues` 与 `attributesOfItem` 各有自己的
    /// 缓存/装箱代价（后者为一次比对要组装 NSDictionary + NSNumber，本机实测 26µs/次，
    /// 而 stat(2) 是 0.6µs/次；前者的毫秒级陈旧窗口就是上面说的那类误命中来源）。
    /// 增量索引需要「inode 防同名替换 + mtime/size 防原地改写」，三者缺一不可。
    static func statRegularFile(_ path: String) -> (mtime: Date, size: Int, inode: UInt64)? {
        var st = stat()
        guard stat(path, &st) == 0, (st.st_mode & S_IFMT) == S_IFREG else { return nil }
        let seconds = TimeInterval(st.st_mtimespec.tv_sec) + TimeInterval(st.st_mtimespec.tv_nsec) / 1_000_000_000
        // st_size 是 off_t(Int64)：本机 64 位下与 Int 等宽，truncatingIfNeeded 无损
        return (Date(timeIntervalSince1970: seconds),
                Int(truncatingIfNeeded: st.st_size),
                UInt64(truncatingIfNeeded: st.st_ino))
    }

    private static func readUncached(file: URL, maxLines: Int, maxBytes: Int) -> [String] {
        guard let handle = try? FileHandle(forReadingFrom: file) else { return [] }
        defer { try? handle.close() }
        do {
            let size = try handle.seekToEnd()
            let length = min(size, UInt64(maxBytes))
            guard length > 0 else { return [] }
            let start = size - length
            // 多读前一个字节，区分恰好从行首开始和从行内开始。
            try handle.seek(toOffset: start > 0 ? start - 1 : 0)
            guard var data = try handle.read(upToCount: Int(length) + (start > 0 ? 1 : 0)) else { return [] }
            if start > 0 {
                guard !data.isEmpty else { return [] }
                let preceding = data.removeFirst()
                if preceding != 10 && preceding != 13 {
                    guard let boundary = data.firstIndex(where: { $0 == 10 || $0 == 13 }) else { return [] }
                    data = Data(data.suffix(from: data.index(after: boundary)))
                }
            }
            // 按行解码：正在写入或损坏的单行不应遮蔽其他完整行。
            let lines = data.split(whereSeparator: { $0 == 10 || $0 == 13 })
                .compactMap { String(data: $0, encoding: .utf8) }
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
            return Array(lines.suffix(maxLines))
        } catch {
            return []
        }
    }

    /// 会话树中最近修改的常规文件。
    /// AgentActionInspector（主线程每拍）与 AgentLogStreamer（流水页）共用——
    /// 此前两处各持一份逐字相同的实现，且都不防符号链接循环、无条目上限。
    /// - Parameters:
    ///   - maxEntries: 条目预算。会话树由工具/用户创建，结构不可全信；到预算即止，
    ///     防失控枚举长时间占用调用线程（与 FileMonitor.scanTree 的 maxDepth 同思路）。
    static func newestFile(in dir: URL, maxAge: TimeInterval, maxEntries: Int = 20_000) -> URL? {
        let fm = FileManager.default
        let keys: [URLResourceKey] = [.contentModificationDateKey, .isRegularFileKey,
                                      .isSymbolicLinkKey, .isDirectoryKey]
        guard let en = fm.enumerator(at: dir, includingPropertiesForKeys: keys,
                                     options: [.skipsHiddenFiles]) else { return nil }
        var newestURL: URL?
        var newestDate = Date.distantPast
        let threshold = Date().addingTimeInterval(-maxAge)
        var visited = 0
        while let item = en.nextObject() as? URL {
            visited += 1
            if visited > maxEntries { break }
            guard let v = try? item.resourceValues(forKeys: Set(keys)) else { continue }
            if v.isSymbolicLink == true, v.isDirectory == true {
                // 符号链接目录跳过整个子树，防循环（对齐 FileMonitor.scanTree）。
                // 注意 skipDescendants 只能对目录调用：对文件符号链接调用会破坏
                // 枚举器状态、丢弃后续所有条目（实测复现），故必须带 isDirectory 条件；
                // 文件符号链接经 lstat 语义 isRegularFile=false，自然排除
                en.skipDescendants()
                continue
            }
            guard v.isRegularFile == true,
                  let mtime = v.contentModificationDate,
                  mtime >= threshold,
                  mtime > newestDate else { continue }
            newestDate = mtime
            newestURL = item
        }
        return newestURL
    }
}

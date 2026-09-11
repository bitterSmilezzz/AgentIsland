import Foundation

/// 有界尾读：不追随持续增长的日志；截断首行在解码前丢弃。
enum LogTailReader {
    static func read(from file: URL, maxLines: Int, maxBytes: Int) -> [String] {
        guard maxLines > 0, maxBytes > 0,
              let handle = try? FileHandle(forReadingFrom: file) else { return [] }
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

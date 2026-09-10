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
}

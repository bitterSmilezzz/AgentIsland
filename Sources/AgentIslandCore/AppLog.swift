import Foundation
import os

// MARK: - 统一日志通道（R18）

/// 统一日志出口：发布构建里数据源异常此前散落在 print/debugPrint（均不可见）。
/// 常态走 os.Logger（Console.app / `log show --predicate 'subsystem == "com.agentisland.app"'` 可查）；
/// `AGENTISLAND_DEBUG=1` 启动时镜像落 /tmp/agentisland.log（README 调试指南的真实出口）。
/// 注意：Probe / Selftest 的 stdout 表格输出是 CLI 交互目的本身，不经过此通道。
public enum AppLog {
    public static let subsystem = "com.agentisland.app"
    private static let logger = Logger(subsystem: subsystem, category: "app")
    /// 调试镜像文件路径（仅 AGENTISLAND_DEBUG=1 时启用；按调用读取环境，
    /// 便于测试与运行期开关——日志频率低，读取开销可忽略）
    private static var mirrorPath: String? {
        guard ProcessInfo.processInfo.environment["AGENTISLAND_DEBUG"] == "1" else { return nil }
        return "/tmp/agentisland.log"
    }
    private static let mirrorLock = NSLock()

    public static func debug(_ message: String) {
        logger.debug("\(message, privacy: .public)")
        mirror(message, level: "DEBUG")
    }

    public static func warn(_ message: String) {
        logger.notice("\(message, privacy: .public)")
        mirror(message, level: "WARN")
    }

    public static func error(_ message: String) {
        logger.error("\(message, privacy: .public)")
        mirror(message, level: "ERROR")
    }

    /// 镜像落盘：追加写（懒创建），行格式 `日期 [级别] 消息`
    private static func mirror(_ message: String, level: String) {
        guard let path = mirrorPath else { return }
        let line = "\(Date()) [\(level)] \(message)\n"
        mirrorLock.lock()
        defer { mirrorLock.unlock() }
        if let handle = FileHandle(forWritingAtPath: path) {
            defer { try? handle.close() }
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: Data(line.utf8))
        } else {
            try? Data(line.utf8).write(to: URL(fileURLWithPath: path))
        }
    }
}

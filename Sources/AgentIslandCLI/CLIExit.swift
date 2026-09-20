import Foundation
import AgentIslandCore

// MARK: - CLI 退出码（对外契约）

/// 「没找到」与「办砸了」在退出码上必须是两件事——脚本与 CI 只看得到码和 stdout。
/// 此前所有失败分支都是「印一行红字然后 return」，进程仍以 0 退出：
/// `report -o /不存在的目录/x.md` 实测打印失败却 exit=0，CI 会认为报表已导出。
public enum CLIExit {
    /// 成功（默认，不调用即为 0）
    static let success: Int32 = 0
    /// 执行了但失败（写盘失败、投递失败、被要求终止的进程没死）
    static let failure: Int32 = 1
    /// 调用方式不对（未知 flag、参数值非法、缺参数）——与 1 区分，便于脚本分别处理
    static let badUsage: Int32 = 2

    /// 打印到 stderr 并以非零码退出。诊断文本一律走 stderr，绝不污染 `--json` 的 stdout
    static func fail(_ message: String, code: Int32 = failure) -> Never {
        FileHandle.standardError.write(Data((message + "\n").utf8))
        exit(code)
    }
}

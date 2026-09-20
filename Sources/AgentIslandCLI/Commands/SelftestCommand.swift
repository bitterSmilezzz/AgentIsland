import Foundation
import AgentIslandCore

/// `agentisland selftest` —— 核心逻辑自检（假数据断言，不碰真实机器状态）。
///
/// 与 `doctor` 是两件事：doctor 回答「这台机器上的监控数据可不可信」，
/// selftest 回答「这个构建本身的判定逻辑还对不对」。两者此前都只能靠 `.app` 的
/// 隐藏参数 `--selftest` / `--probe` 触发，用户与脚本无从得知入口存在。
public enum SelftestCommand {
    @MainActor
    public static func run(args: [String]) async {
        let code = Selftest.run()
        if code == 0 {
            print(CLIColor.green("\n✅ 核心逻辑自检全部通过"))
        } else {
            print(CLIColor.red("\n❌ 核心逻辑自检存在 \(code) 项失败"))
        }
        exit(code)
    }
}

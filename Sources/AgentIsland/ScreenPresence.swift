import CoreGraphics
import Foundation

// MARK: - 「人不在机器前」的感知

/// 只回答一个问题：现在这台 Mac 前有没有人。给远程外发的 `onlyWhenAway` 用，
/// 岛内的显示逻辑不看它。
///
/// 判定放在 `RemoteNotifier`（纯函数、可离线测），这里只负责取信号——
/// 取信号碰窗口服务器，测不了，所以一层薄薄的、不含判断。
enum ScreenPresence {
    /// 会话锁屏标记。窗口服务器只在**已锁定**时往会话字典里写这个键，
    /// 未锁定时它根本不存在（实测：未锁屏的字典里有 11 个键，无此项）。
    /// 头文件里没有对应常量，键名以运行时字典为准。
    static var isScreenLocked: Bool {
        guard let dict = CGSessionCopyCurrentDictionary() else { return false }
        let locked = (dict as? [String: Any])?["CGSSessionScreenIsLocked"]
        switch locked {
        case let n as Int: return n != 0
        case let n as NSNumber: return n.intValue != 0
        case let b as Bool: return b
        default: return false
        }
    }

    /// 主显示器是否已熄屏。人在机器前但显示器睡着，同样不该被手机再打扰一次；
    /// 而熄屏是本仓已经用过多次的判定（窗口动画在睡眠期会被挂起）
    static var isDisplayAsleep: Bool {
        CGDisplayIsAsleep(CGMainDisplayID()) == 1
    }

    static var isAway: Bool { isScreenLocked || isDisplayAsleep }
}

//! 电源状态与「人不在机器前」的信号采集。
//!
//! **这里只取信号，不做判断。** 判断是纯函数（[`crate::remote::Policy::is_away`]），
//! 可以离线测；取信号要碰系统 API，测不了——两层混在一起就等于把可测的那层也测不了。
//!
//! 与 Swift 侧 `ScreenPresence` / `PowerSourceMonitor` 同口径，逐条对齐：
//! · 锁屏读会话字典里那个**运行时才有**的键（未锁屏时它根本不存在）
//! · 输入时长取各输入类型里**最近**的那一个，而不是平均
//! · 取不到就给 `None`，让上层按 fail-open 处理——绝不拿 0 冒充「刚刚动过」

use crate::remote::PresenceSignals;

// MARK: - 「人不在机器前」

#[cfg(target_os = "macos")]
mod sys {
    //! 只保留**两个返回标量、无所有权、无生命周期**的查询。
    //!
    //! 手写 `extern "C"` 而不是引一个 crate：它们无状态、无回调、无所有权转移，
    //! 引一个 crate 为此要背上的依赖与构建成本比这两行声明大得多。
    //!
    //! **这里刻意不碰会话字典**（锁屏标记就在里面）：字典的键要用 CFString、
    //! 值可能是 `Bool` 也可能是 `NSNumber`，从裸指针读对它们需要
    //! `core-foundation`。曾按 Swift 的形状手写过一版并**段错误**——
    //! unsafe 路径宁可没有，也不要有一份会崩的。
    extern "C" {
        pub fn CGMainDisplayID() -> u32;
        pub fn CGDisplayIsAsleep(display: u32) -> i32;
        pub fn CGEventSourceSecondsSinceLastEventType(
            state: u32,
            event_type: u32,
        ) -> f64;
    }

    /// `kCGEventSourceStateCombinedSessionState`
    pub const COMBINED_SESSION_STATE: u32 = 0;
    pub const EVENT_KEY_DOWN: u32 = 10;
    pub const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    pub const EVENT_RIGHT_MOUSE_DOWN: u32 = 2;
    pub const EVENT_MOUSE_MOVED: u32 = 5;
    pub const EVENT_SCROLL_WHEEL: u32 = 22;

}

/// 会话是否已锁屏。
///
/// **本平台暂未实现，返回 false。**（这一条要写清楚，别让它看起来是「取不到」：
/// 上层按 fail-open 处理，判成「人还在」⇒ 只是少发一条外发通知；
/// 而返回一个猜的 true 会让「只在人不在时发」永远不发，比少发糟得多。）
///
/// 为什么不实现：锁屏标记在会话字典里，值类型随系统版本变（`Bool` / `NSNumber`），
/// 从裸 C 指针把它读对需要引入 `core-foundation` 这样的 crate——为一个布尔值
/// 多一个依赖不划算，而**手写 extern 去做这件事已经试过一次并段错误了**
/// （把字典值当成 `CFBoolean` 调 `CFBooleanGetValue`）。留在文档里比留一段
/// 会崩的 unsafe 强。
#[cfg(target_os = "macos")]
pub fn is_screen_locked() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn is_screen_locked() -> bool {
    false
}

/// 主显示器是否已熄屏。人在机器前但屏睡着，同样不该再打扰一次。
#[cfg(target_os = "macos")]
pub fn is_display_asleep() -> bool {
    unsafe { sys::CGDisplayIsAsleep(sys::CGMainDisplayID()) == 1 }
}

#[cfg(not(target_os = "macos"))]
pub fn is_display_asleep() -> bool {
    false
}

/// 距上一次「任意输入」的秒数。
///
/// 取各输入类型里**最近**的那一个：某类输入从没发生过时该函数返回一个极大值，
/// 取 min 正好把它压掉。全取不到（或都大得离谱）时返回 `None`——
/// 上层按 fail-open 处理，**不会**拿 0 冒充「刚刚动过」。
pub fn idle_seconds() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let types = [
            sys::EVENT_KEY_DOWN,
            sys::EVENT_LEFT_MOUSE_DOWN,
            sys::EVENT_RIGHT_MOUSE_DOWN,
            sys::EVENT_MOUSE_MOVED,
            sys::EVENT_SCROLL_WHEEL,
        ];
        let values: Vec<f64> = types
            .iter()
            .map(|t| unsafe { sys::CGEventSourceSecondsSinceLastEventType(sys::COMBINED_SESSION_STATE, *t) })
            .collect();
        let least = values.iter().copied().fold(f64::INFINITY, f64::min);
        // 24 小时 = 一天内没动过键盘鼠标，那不是「取不到」，是「一整天没碰」。
        // 与 Swift 同一条界线：过了它就当信号不可信。
        (least < 24.0 * 3600.0).then_some(least)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// 一次判定要用的完整信号。
pub fn presence_signals() -> PresenceSignals {
    PresenceSignals {
        screen_locked: is_screen_locked(),
        display_asleep: is_display_asleep(),
        idle_seconds: idle_seconds(),
    }
}

// MARK: - 电源状态

/// 是否处于系统「低电量模式」。
///
/// 走 `pmset -g` 而不是 IOKit 的 `ProcessInfo.isLowPowerModeEnabled`：
/// 后者在 Rust 侧没有等价导出，而 `pmset` 是系统自带、稳定输出的只读查询。
/// **取不到时返回 false**（按「没有低电量模式」处理）——
/// 这只会让降频不生效，而反过来会把用户的机器一直按在降频里。
pub fn is_low_power_mode() -> bool {
    parse_low_power(&run_pmset(&["-g"]))
}

/// `pmset -g` 的输出里找 `lowpowermode 0|1`。
///
/// 纯函数：pmset 的输出形状在 macOS 各版本间基本稳定，而**解析错了不会崩**——
/// 所以这里要能离线钉住，包括「输出里没有这一行」时怎么办。
pub fn parse_low_power(text: &str) -> bool {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("lowpowermode"))
        .and_then(|rest| rest.split_whitespace().next())
        .map(|value| value == "1")
        .unwrap_or(false)
}

/// 是否使用电池供电。
pub fn is_on_battery() -> bool {
    parse_on_battery(&run_pmset(&["-g", "batt"]))
}

/// `pmset -g batt` 里读电源来源。
///
/// **真实形状**（本机实测）：
/// ```
/// Now drawing from 'AC Power'
///  -InternalBattery-0 (id=35520611)  80%; AC attached; not charging present: true
/// ```
/// 状态**嵌在 `Now drawing from '…'` 那一句里**，没有独立的 `Battery Power` 行——
/// 我最初按「找状态行」写，测试里自己编了个形状，跑真机才发现对不上。
/// 所以这里锚定这一句再取引号内容，而不是全文 `contains`：
/// 后者会被下面那行的 `AC attached` 之类的文字蹭到。
pub fn parse_on_battery(text: &str) -> bool {
    text.lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("Now drawing from")?.trim();
            rest.strip_prefix('\'').and_then(|r| r.split_once('\'')).map(|(src, _)| src)
        })
        .is_some_and(|source| source.eq_ignore_ascii_case("Battery Power"))
}

fn run_pmset(args: &[&str]) -> String {
    std::process::Command::new("pmset")
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// 是否应当进入节能降频。**纯函数**（与 Swift `PowerSourceMonitor.shouldThrottle` 同规则）：
/// 低电量模式**优先于**开关——那是系统已经替用户做出的决定，用户关掉本应用的开关
/// 也不该把它顶掉；反过来，没在电池上时开关打开也不降频。
pub fn should_throttle(battery_saver_enabled: bool, is_low_power: bool, on_battery: bool) -> bool {
    if is_low_power {
        return true;
    }
    if !battery_saver_enabled {
        return false;
    }
    on_battery
}

/// 电池供电时的降频系数。Swift 侧是三档（`BatterySaver` 的调用方按电量分档），
/// Rust 侧先落**一档**：分档的收益在本机实测不到，而多一档就多一处可配错的数。
pub const BATTERY_THROTTLE_FACTOR: f64 = 2.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_power_mode_parsing_survives_shapes_it_has_never_seen() {
        // 真实形状
        assert!(parse_low_power(" pmset -g custom\nlowpowermode 1\n"));
        assert!(!parse_low_power(" pmset -g custom\nlowpowermode 0\n"));
        // 取不到这一行：按「没有低电量模式」处理，**不猜 true**
        assert!(!parse_low_power(""));
        assert!(!parse_low_power("pmset -g custom\n"));
        // 形状变了（多了别的行 / 值不是 0|1）：同样是 false，不 panic
        assert!(!parse_low_power("lowpowermode maybe\n"));
        assert!(!parse_low_power("lowpowermode\n"));
    }

    /// 解析用的是**本机实测的输出形状**，不是自己编的。
    /// 第一版按「找独立的 Battery Power 行」写，测试里也照着编了形状——
    /// 跑真机才发现对不上，于是这份用例改用实拍文本。
    #[test]
    fn on_battery_parsing_reads_the_real_pmset_shape() {
        // 本机 2026-09-28 实拍（插电）
        assert!(!parse_on_battery(
            "Now drawing from 'AC Power'\n -InternalBattery-0 (id=35520611)\t80%; AC attached; not charging present: true\n"
        ));
        // 同一形状、换成电池
        assert!(parse_on_battery(
            "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=35520611)\t80%; discharging; 3:12 remaining\n"
        ));
        // 锚定那一句 ⇒ **第二行**蹭到的关键词不会造成误判
        // （这一条就是「别用全文 contains」要防的东西）
        assert!(!parse_on_battery(
            "Now drawing from 'AC Power'\n -InternalBattery-0\t80%; was Battery Power earlier\n"
        ));
        // 反过来同样成立：权威行说电池就算电池，第二行说什么不算数
        assert!(parse_on_battery(
            "Now drawing from 'Battery Power'\n -InternalBattery-0\t80%; AC attached\n"
        ));
        // 空输出（pmset 失败 / 非 macOS）：false —— 按「在插电」处理，只是不降频
        assert!(!parse_on_battery(""));
        // 形状变了（没有那一句）：false，不 panic
        assert!(!parse_on_battery("garbage"));
    }

    /// 节电判定三条规则逐条钉住。**低电量模式优先于开关**是这一条的全部要点：
    /// 那条规则的存在是防「用户关了本应用的降频，系统低电量模式却失效」。
    #[test]
    fn low_power_beats_the_switch_and_the_switch_beats_plugged_in() {
        // ① 低电量模式：无论开关如何都降频
        assert!(should_throttle(false, true, false));
        assert!(should_throttle(true, true, false));
        // ② 开关关：一律不降频
        assert!(!should_throttle(false, false, true));
        // ③ 开关开 + 在电池上：降频
        assert!(should_throttle(true, false, true));
        // ④ 开关开 + 在插电：不降频
        assert!(!should_throttle(true, false, false));
    }

    /// 真机冒烟：只断言「能跑完、形状自洽」，不断言这台机器此刻在不在电池上——
    /// 那是环境，不是产品行为。
    #[test]
    fn the_real_queries_run_without_panicking() {
        let _ = is_low_power_mode();
        let _ = is_on_battery();
        let _ = presence_signals();
    }
}

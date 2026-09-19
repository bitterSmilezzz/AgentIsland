import Foundation
import IOKit.ps

// MARK: - 电池与硬件电源自适应监听 (v0.0.75)

public enum PowerSourceMonitor {

    /// 是否处于系统「低电量模式」
    public static var isLowPowerModeEnabled: Bool {
        if #available(macOS 12.0, *) {
            return ProcessInfo.processInfo.isLowPowerModeEnabled
        }
        return false
    }

    /// 是否使用电池供电（未接外接交流电源）
    public static var isOnBatteryPower: Bool {
        guard let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue() else {
            return false
        }
        guard let sources = IOPSCopyPowerSourcesList(snapshot)?.takeRetainedValue() as? [CFTypeRef] else {
            return false
        }
        for ps in sources {
            if let desc = IOPSGetPowerSourceDescription(snapshot, ps)?.takeUnretainedValue() as? [String: Any] {
                if let state = desc[kIOPSPowerSourceStateKey as String] as? String,
                   state == (kIOPSBatteryPowerValue as String) {
                    return true
                }
            }
        }
        return false
    }

    /// 综合判断当前是否应当进入节能降频模式
    public static func shouldThrottle(batterySaverEnabled: Bool) -> Bool {
        shouldThrottle(batterySaverEnabled: batterySaverEnabled, isLowPower: isLowPowerModeEnabled, onBattery: isOnBatteryPower)
    }

    /// 纯函数判定（供测试与逻辑复用）
    public static func shouldThrottle(batterySaverEnabled: Bool, isLowPower: Bool, onBattery: Bool) -> Bool {
        if isLowPower { return true }
        guard batterySaverEnabled else { return false }
        return onBattery
    }
}

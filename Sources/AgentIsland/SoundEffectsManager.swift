import AppKit
import AgentIslandCore

// MARK: - 原生交互音效与触觉反馈管理器 (v0.0.74)

@MainActor
public enum SoundEffectsManager {

    /// 播放任务完成提示音
    public static func playCompletionSound() {
        let soundEnabled = UserDefaults.standard.object(forKey: SettingKey.playCompletionSound) as? Bool ?? true
        guard soundEnabled else { return }

        let raw = UserDefaults.standard.string(forKey: SettingKey.completionSoundOption) ?? CompletionSoundOption.glass.rawValue
        guard let option = CompletionSoundOption(rawValue: raw),
              let soundName = option.systemSoundName else { return }

        NSSound(named: soundName)?.play()
        performHapticClick()
    }

    /// 播放熔断与严重异常告警音效
    public static func playAlertSound() {
        let raw = UserDefaults.standard.string(forKey: SettingKey.alertSoundOption) ?? AlertSoundOption.sosumi.rawValue
        guard let option = AlertSoundOption(rawValue: raw),
              let soundName = option.systemSoundName else { return }

        NSSound(named: soundName)?.play()
        performHapticClick()
    }

    /// 试听音效
    public static func previewSound(named soundName: String?) {
        guard let soundName, soundName != "mute" else { return }
        NSSound(named: soundName)?.play()
        performHapticClick()
    }

    /// 触发微触觉反馈（如果开启）。开关判定收在 `HapticFeedback.perform` 那一处，
    /// 这里只负责选节律——两处各读一次设置就会再长出「同一个键两种真相」。
    public static func performHapticClick() {
        HapticFeedback.perform(.alignment, at: .now)
    }
}

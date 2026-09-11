import Foundation
import UserNotifications
import AgentIslandCore

/// macOS 通知中心桥接：任务完成、需要关注和资源告警都同步投递。
@MainActor
enum CompletionNotification {
    static func requestAuthorization() {
        // 只要 .alert：通知固定静音投递（sound = nil），声音由岛内 NSSound 单独负责，
        // 不再为一个用不到的能力向用户申请权限。
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { _, _ in }
    }

    /// 投递系统通知。
    /// - Parameter policy: 通知策略裁决投递与否：完全静默不投递、专注免打扰只放行熔断/死循环
    ///   告警、标准模式全部投递。此前 `post` 在策略判断之前无条件调用，导致「完全静默」下
    ///   仍弹通知并响铃，直接违反该模式的文案承诺。
    static func post(for event: AgentTaskEvent, policy: NotificationPolicy) {
        // 与 NotificationPolicy.shouldPeek 同一套分级口径；之所以不用 shouldPeek 那个名字，
        // 是因为系统通知与岛内微窥是两个独立通道，共用命名会让调用点误以为二者必须同步。
        switch policy {
        case .silent:
            return
        case .focus:
            guard event.eventType == .costSpike else { return }
        case .standard:
            break
        }

        let content = UNMutableNotificationContent()
        switch event.eventType {
        case .completed:
            content.title = "\(event.agentName) 任务完成"
            content.body = event.message ?? "任务已完成，用时 \(durationText(event.duration))"
        case .attention:
            content.title = "\(event.agentName) 需要关注"
            content.body = event.message ?? event.detail ?? "有操作等待确认"
        case .costSpike:
            content.title = "\(event.agentName) 资源告警"
            content.body = event.message ?? event.detail ?? "检测到 Token 或 CPU 消耗异常"
        }
        // 固定不带声音：提示音统一由岛内的 NSSound 承担（见 IslandPanel.handleTaskEvent）。
        // 反过来让通知带声音会与 NSSound 叠加成双重提示音；而把声音交给通知中心还有个副作用——
        // 用户若关闭了 AgentIsland 的通知权限（或系统层面静音），熔断告警就会彻底失声。
        // 岛内 NSSound 不依赖通知权限，且已由通知策略与「任务完成提示音」开关共同裁决，
        // 因此它是唯一声音来源，通知只负责横幅可见性。
        content.sound = nil
        if #available(macOS 13.0, *) {
            // 任务完成属于用户应立即知道的主动事件，避免被系统当作被动更新而静音。
            content.interruptionLevel = .active
        }

        let request = UNNotificationRequest(
            identifier: "agentisland.event.\(event.id.uuidString)",
            content: content,
            trigger: nil
        )
        UNUserNotificationCenter.current().add(request)
    }

    private static func durationText(_ duration: TimeInterval) -> String {
        let seconds = max(0, Int(duration.rounded()))
        return seconds >= 60 ? "\(seconds / 60)分\(seconds % 60)秒" : "\(seconds)秒"
    }
}

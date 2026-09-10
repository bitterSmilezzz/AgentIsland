import Foundation
import UserNotifications
import AgentIslandCore

/// macOS 通知中心桥接：任务完成、需要关注和资源告警都同步投递。
@MainActor
enum CompletionNotification {
    static func requestAuthorization() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    static func post(for event: AgentTaskEvent) {
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
        // 明确指定默认通知音。`.default` 是静音通知的关键区别，不能省略。
        content.sound = UNNotificationSound.default
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

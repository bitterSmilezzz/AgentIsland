import Foundation

// MARK: - 远程通知的落盘与读取（非密钥部分走 UserDefaults，密钥走钥匙串）

/// 通道配置与本仓其他设置不同：它是**结构体**而不是散落的键。
/// 存法是每个通道一份 JSON（`remote.notify.channel.<kind>.v1`），切换通道时
/// 各自的填写内容互不覆盖——否则用户试完邮箱再试 ntfy，回来发现 SMTP 全空了。
///
/// 红线：这里**只存非密钥字段**。钥匙串条目名由通道唯一决定
/// （`RemoteChannelKind.defaultSecretName`），真实 sendkey/token/授权码只进 `RemoteSecret`——
/// UserDefaults 是明文 plist，会进 Time Machine 与任何 dump 工具的视野，
/// 而 webhook 里的 key 本身就是通行证。
public enum RemoteNotifyStore {
    public static let policyKey = "remote.notify.policy.v1"
    public static let kindKey = "remote.notify.kind.v1"

    public static func configKey(_ kind: RemoteChannelKind) -> String {
        "remote.notify.channel.\(kind.rawValue).v1"
    }

    public static func loadPolicy(defaults: UserDefaults = .standard) -> RemoteNotifyPolicy {
        let raw = decode(RemoteNotifyPolicy.self, key: policyKey, defaults: defaults)
        // 读出即归一化：损坏/越界的值在落盘时可能已经发生过，读这条路是唯一防线
        return (raw ?? RemoteNotifyPolicy()).normalized()
    }

    public static func save(_ policy: RemoteNotifyPolicy, defaults: UserDefaults = .standard) {
        encode(policy.normalized(), key: policyKey, defaults: defaults)
    }

    public static func loadKind(defaults: UserDefaults = .standard) -> RemoteChannelKind {
        loadKindDetailed(defaults: defaults).kind
    }

    /// 存档里的通道名不认识时（版本回退、手改 plist、将来加新通道后降级）不能只是
    /// 「当没发生过」：回落成 ntfy 之后，用户在界面上敲的每个字符都会写进 ntfy 的键，
    /// 而他真正的配置还留在原来那个键下——两份并存，且没人说过。
    /// 所以把不认识的原文一并回吐，让界面能明说「现在显示的是 ntfy，改动会写 ntfy」。
    public static func loadKindDetailed(defaults: UserDefaults = .standard)
        -> (kind: RemoteChannelKind, unrecognized: String?) {
        guard let raw = defaults.string(forKey: kindKey) else { return (.ntfy, nil) }
        guard let kind = RemoteChannelKind(rawValue: raw) else {
            AppLog.error("远程通知通道 \(raw) 无法识别，界面按 \(RemoteChannelKind.ntfy.rawValue) 显示；"
                         + "此后的改动会写进该通道的键，原通道配置保持不动")
            return (.ntfy, raw)
        }
        return (kind, nil)
    }

    public static func save(_ kind: RemoteChannelKind, defaults: UserDefaults = .standard) {
        defaults.set(kind.rawValue, forKey: kindKey)
    }

    public static func loadConfig(for kind: RemoteChannelKind,
                                  defaults: UserDefaults = .standard) -> RemoteChannelConfig {
        var config = decode(RemoteChannelConfig.self, key: configKey(kind), defaults: defaults)
            ?? RemoteChannelConfig()
        // 读盘也要收口（与 loadPolicy 同口径）：手改 plist 留下 smtpPort = 0 这种值，
        // 每次都连向一个注定不存在的端口，比退回默认更难查
        if config.smtpPort <= 0 || config.smtpPort > 65_535 { config.smtpPort = 465 }
        return config
    }

    public static func save(_ config: RemoteChannelConfig, for kind: RemoteChannelKind,
                            defaults: UserDefaults = .standard) {
        encode(config, key: configKey(kind), defaults: defaults)
    }

    /// 通道是否至少填到了「可能发得出去」——设置页用它决定要不要显示红字提示，
    /// 判据与发送前的检查共用 `missingField`，避免两处口径漂移
    public static func readiness(for kind: RemoteChannelKind, config: RemoteChannelConfig) -> String? {
        kind.missingField(config: config, hasSecret: RemoteSecret.exists(kind.defaultSecretName))
    }

    private static func decode<T: Decodable>(_ type: T.Type, key: String,
                                             defaults: UserDefaults) -> T? {
        guard let data = defaults.data(forKey: key) else { return nil }
        if let value = try? JSONDecoder().decode(type, from: data) { return value }
        // 键在、解不开 ⇒ 这不是「用户没配」，是存档坏了。它今天的路径是回落默认值，
        // 而策略的默认值就是总开关关——于是外发**静默停止**、零日志，设置页还显示成
        // 「你没配置」。至少要留一笔，并把原字节挪到备份键：用户随手改一次设置
        // 就不会把最后一点现场冲掉。
        AppLog.error("远程通知配置 \(key) 解码失败，本项按默认值运行（外发可能因此静默关闭）")
        let backup = corruptBackupKey(key)
        if defaults.data(forKey: backup) == nil {
            defaults.set(data, forKey: backup)
        }
        return nil
    }

    /// 坏存档的备份键。测试与手工恢复都按这个名字找。
    public static func corruptBackupKey(_ key: String) -> String { key + ".corrupt" }

    private static func encode<T: Encodable>(_ value: T, key: String, defaults: UserDefaults) {
        guard let data = try? JSONEncoder().encode(value) else { return }
        defaults.set(data, forKey: key)
    }
}

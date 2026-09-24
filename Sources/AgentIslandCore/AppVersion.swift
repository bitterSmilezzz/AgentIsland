import Foundation

/// 版本号的唯一声明处。
///
/// 曾经 CLI 横幅、Raycast 清单与设置页各写一份：发版脚本只校验 CHANGELOG 与 README，
/// 于是应用已经到 v0.0.84 而 CLI 还在打印 v0.0.80，导出的 Raycast 清单也停在旧版——
/// 对外报错误的版本号比没有版本号更糟。现在：
/// - 三处消费方一律读这里；
/// - `scripts/build-app.sh` 在打包前把它与 CHANGELOG 首条版本对齐，不一致即拒绝发版；
/// - 测试 `版本单一来源` 用同一口径再守一遍（防止有人绕过脚本手工构建）。
public enum AppVersion {
    /// 语义化版本号，与 CHANGELOG 首条 `## [x.y.z]` 必须一致
    public static let string = "0.0.128"

    /// 运行期可见的版本：从 .app 的 Info.plist 取（打包产物由 build-app.sh 写入同一值），
    /// 独立运行的 CLI 没有 bundle 信息时回落到上面的常量。
    public static var display: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? string
    }
}

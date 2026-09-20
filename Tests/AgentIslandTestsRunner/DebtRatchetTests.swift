import Foundation
@testable import AgentIslandCore

// MARK: - 债务棘轮（只准降不准升）
//
// 架构审计量出两处长期债务：Theme 之外的硬编码色值 186 处、`UserDefaults.standard` 直读
// 37 处（散在 7 个文件，SettingsStore 本该是唯一入口）。两者都没有缺陷史，做大面积清扫
// 是拿 186 处视觉回归风险换整洁，不划算；真正划算的是**先止住增长**——
// （注意计数口径：`grep -c` 数的是**行数**，一行两处会漏计，基线按实际出现次数定）
// 于是把当前数量钉成基线，新增一处就让测试失败。
//
// 基线只能往下改：修完一批就把数字改小，绝不往上调。
enum DebtRatchetTests {
    /// Theme.swift 之外的 `Color(hex:` 字面量数量基线
    private static let hardcodedColorBaseline = 186
    /// `UserDefaults.standard` 直读点数量基线（SettingsStore 是合法入口，不计入）
    private static let defaultsBaseline = 37

    @MainActor
    static func register() {
        TestKit.test("债务棘轮: 硬编码色值与 UserDefaults 直读点不得增长") {
            guard let sources = repoSourcesDirectory() else {
                return   // 非源码树环境（打包产物里跑测试）不做强断言
            }
            let files = swiftFiles(in: sources)
            var colors = 0
            var defaults = 0
            for url in files {
                // Theme.swift 是令牌定义处，硬编码色值只此一处合法
                let isTheme = url.lastPathComponent == "Theme.swift"
                // SettingsStore 是设置的唯一读写入口
                let isSettingsStore = url.lastPathComponent == "SettingsStore.swift"
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { continue }
                if !isTheme {
                    colors += text.components(separatedBy: "Color(hex:").count - 1
                }
                if !isSettingsStore {
                    defaults += text.components(separatedBy: "UserDefaults.standard").count - 1
                }
            }
            try expectTrue(colors <= hardcodedColorBaseline,
                           "Theme 外的 Color(hex:) 从 \(hardcodedColorBaseline) 涨到 \(colors)："
                           + "新界面请用 Theme 的动态浅/深成对色（见 Theme.swift 的令牌约定）。"
                           + "若是收敛了旧值，请把基线数字改小。")
            try expectTrue(defaults <= defaultsBaseline,
                           "UserDefaults.standard 直读从 \(defaultsBaseline) 涨到 \(defaults)："
                           + "设置读写应经 SettingsStore/SettingKey（含 SettingLimits 归一化），"
                           + "否则新增项会绕过区间校验与启动自愈。收敛后请把基线改小。")
        }
    }

    /// 从测试源文件位置回溯到仓库的 Sources 目录（与「版本单一来源」测试同一手法）
    private static func repoSourcesDirectory() -> URL? {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // Tests/AgentIslandTestsRunner
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // repo root
            .appendingPathComponent("Sources")
        return FileManager.default.fileExists(atPath: root.path) ? root : nil
    }

    private static func swiftFiles(in dir: URL) -> [URL] {
        let fm = FileManager.default
        guard let en = fm.enumerator(at: dir, includingPropertiesForKeys: nil) else { return [] }
        return en.compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" }
    }
}

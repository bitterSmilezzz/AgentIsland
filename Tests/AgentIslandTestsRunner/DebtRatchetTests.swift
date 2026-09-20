import Foundation
@testable import AgentIslandCore

// MARK: - 债务棘轮（只准降不准升）
//
// 架构审计量出两处长期债务：Theme 之外的硬编码色值 186 处、`UserDefaults.standard` 直读
// 37 处（散在 7 个文件，SettingsStore 本该是唯一入口）。
//
// 色值这一项 v0.0.92 已实际收敛而非仅止住增长：186 → 36。做法是把被反复抄写的 Tailwind
// 基色收进 `Theme.swift` 的 `Ramp`（slate-200 一种描边色此前抄了 45 处），并把
// AgentIslandApp / AgentRowView / AgentHoverTooltip 三份一模一样的 ActivityLevel 浅色
// 色阶并成 `Theme` 上的一处。剩的 36 处是各写一次、无从重复的一次性强调色与
// 黑/白半透明蒙层，收进 `Ramp` 只会让色板多出十几支没人复用的色。
//
// 基线只能往下改：修完一批就把数字改小，绝不往上调。
enum DebtRatchetTests {
    /// Theme.swift 之外的 `Color(hex:` 字面量数量基线（v0.0.92 收敛自 186）
    private static let hardcodedColorBaseline = 36
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
                           + "新界面请用 Theme 的动态浅/深成对色或 Ramp 基色（见 Theme.swift 的令牌约定）。"
                           + "若是收敛了旧值，请把基线数字改小。")
            try expectTrue(defaults <= defaultsBaseline,
                           "UserDefaults.standard 直读从 \(defaultsBaseline) 涨到 \(defaults)："
                           + "设置读写应经 SettingsStore/SettingKey（含 SettingLimits 归一化），"
                           + "否则新增项会绕过区间校验与启动自愈。收敛后请把基线改小。")
        }

        TestKit.test("语义色单点: Ramp 基色与 ActivityLevel 浅色色阶各只定义一处") {
            guard let sources = repoSourcesDirectory() else { return }
            let files = swiftFiles(in: sources)
            guard let theme = files.first(where: { $0.lastPathComponent == "Theme.swift" }),
                  let themeText = try? String(contentsOf: theme, encoding: .utf8) else {
                return
            }
            // 从 Ramp 块里取基色整数：以后往 Ramp 加色，本测试自动跟着守，不必改这里
            let rampHexes = rampBaseHexes(themeText)
            try expectTrue(rampHexes.count >= 15,
                           "Theme.swift 的 Ramp 块只解析出 \(rampHexes.count) 支基色，"
                           + "多半是块结构被改坏（测试靠 `enum Ramp {` … `}` 取整数）")

            var leaked: [String] = []
            var reladder: [String] = []
            let colorReturns = ["Ramp.", "Theme.status", "Theme.warning", "Color("]
            for url in files where url.lastPathComponent != "Theme.swift" {
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { continue }
                let lowered = text.lowercased()
                for hex in rampHexes where lowered.contains("0x" + hex) {
                    leaked.append("\(url.lastPathComponent) 又写了 0x\(hex)")
                }
                // 抄一份等级色阶：同一文件里 working 与 attention 两支都以「return 某个色令牌」
                // 出现。事件类型（EventType）的梯子只有 attention，不会误伤；
                // Core 里 return 中文文案的 label 梯子也不算。
                var colorCases = Set<String>()
                for line in text.split(separator: "\n") {
                    let parts = line.split(separator: ":", maxSplits: 1,
                                           omittingEmptySubsequences: false)
                    guard parts.count == 2 else { continue }
                    let value = parts[1].trimmingCharacters(in: .whitespaces)
                    guard value.hasPrefix("return"),
                          colorReturns.contains(where: { value.contains($0) }) else { continue }
                    let label = parts[0].trimmingCharacters(in: .whitespaces)
                    guard label.hasPrefix("case ") else { continue }
                    // 一支 case 可以并列多个等级（`case .working, .completed:`）
                    let tokens = label.dropFirst(5).split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespaces) }
                    for level in [".working", ".attention"] where tokens.contains(level) {
                        colorCases.insert(level)
                    }
                }
                if colorCases.count == 2 { reladder.append(url.lastPathComponent) }
            }
            try expectTrue(leaked.isEmpty,
                           "Ramp 基色被抄回视图层（改一处色板就得改多处）：" + leaked.joined(separator: "、"))
            try expectTrue(reladder.isEmpty,
                           "ActivityLevel 浅色色阶又出现了一份完整副本：" + reladder.joined(separator: "、")
                           + "；色阶只在 Theme.swift 的 lightText/lightFill/lightBorder 定义，"
                           + "调用点只保留自己的外观分支与透明度。")
        }
    }

    /// 取 `enum Ramp { … }` 块内出现过的 6 位十六进制基色（小写、去掉 `0x` 前缀）
    private static func rampBaseHexes(_ themeText: String) -> Set<String> {
        guard let start = themeText.range(of: "enum Ramp {") else { return [] }
        let tail = themeText[start.upperBound...]
        var block = Substring(tail.range(of: "\n}").map { tail[..<$0.lowerBound] } ?? tail)
        var out = Set<String>()
        while let r = block.range(of: "0x") {
            let digits = block[r.upperBound...].prefix(while: { $0.isHexDigit })
            if digits.count == 6 { out.insert(String(digits).lowercased()) }
            block = block[r.upperBound...]
        }
        return out
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

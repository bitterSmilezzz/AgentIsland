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
    /// `UserDefaults.standard` 直读点数量基线（SettingsStore 是合法入口，不计入）。
    /// 37 → 29：v0.0.108 把触觉反馈收进 `HapticFeedback` 一处，顺带清掉了设置读写
    /// 绕过 SettingsStore 的那批散点（其中 `SoundEffectsManager` 与 `IslandView`
    /// 对同一个键用了两套容错，见 SettingBool 的注释）。
    private static let defaultsBaseline = 29

    @MainActor
    static func register() {
        TestKit.test("债务棘轮: 硬编码色值与 UserDefaults 直读点不得增长") {
            let files = try SourceTree.requireSwiftFiles()
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

        TestKit.test("触觉反馈只有一个出口: 设置里的开关必须管到每一次震动") {
            // 「触控板微触觉反馈」这个开关此前只管 8 个调用点里的 2 个：其余 6 处直接
            // 调 NSHapticFeedbackManager，关掉后展开/收起/停靠/Esc/工具箱照样震。
            // 症状是「设置不生效」，而代码里两处都写着「（如果开启）」——谁都没错到看得见。
            let files = try SourceTree.requireSwiftFiles()
            var direct = 0
            var holders: [URL] = []
            for url in files {
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { continue }
                // 注释里提这个 API 是合法的（解释为什么不能直调），只数真正的调用
                let calls = text.components(separatedBy: "\n").filter { line in
                    let trimmed = line.trimmingCharacters(in: .whitespaces)
                    guard trimmed.contains("NSHapticFeedbackManager.defaultPerformer.perform"),
                          !trimmed.hasPrefix("//") else { return false }
                    return true
                }
                direct += calls.count
                if !calls.isEmpty { holders.append(url) }
            }
            try expectEqual(direct, 1, "NSHapticFeedbackManager 直调点应为 1（HapticFeedback.perform），"
                                    + "实际 \(direct)：\(holders.map(\.lastPathComponent))")
            // 光数出口数量不够：把开关那一行删掉，出口仍然只有 1 个，而设置又不管事了。
            // 唯一的出口必须**查**这个键——所以要读进 perform 的函数体确认 guard 还在。
            // 路径也从 SourceTree 取：上一版这里用 "Sources/AgentIsland/" + 文件名的**相对**
            // 路径，cwd 不是仓库根时读不到文件、直接 return，把真正的断言整段跳过。
            guard let holder = holders.first else {
                throw TestError(message: "数到了直调点却没记下文件，计数逻辑有洞")
            }
            let text = try SourceTree.text(
                relativePath: "Sources/" + holder.deletingLastPathComponent()
                    .lastPathComponent + "/" + holder.lastPathComponent)
            guard text.contains("static func perform(") else {
                throw TestError(message: "\(holder.lastPathComponent) 里找不到 HapticFeedback.perform 的函数头")
            }
            guard let start = text.range(of: "static func perform(") else {
                throw TestError(message: "函数头检查与上面的 contains 自相矛盾")
            }
            // 从函数头起做括号配平，取出整个 perform 的函数体
            var depth = 0
            var body = ""
            var cursor = text[start.lowerBound...].startIndex
            while cursor < text.endIndex {
                let ch = text[cursor]
                if ch == "{" { depth += 1 }
                if ch == "}" { depth -= 1 }
                body.append(ch)
                if depth == 0, ch == "}" { break }
                cursor = text.index(after: cursor)
            }
            try expectTrue(body.contains("guard isEnabled"),
                           "HapticFeedback.perform 必须查 isEnabled，实际函数体：\(body.prefix(200))")
        }

        TestKit.test("语义色单点: Ramp 基色与 ActivityLevel 浅色色阶各只定义一处") {
            let files = try SourceTree.requireSwiftFiles()
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

        TestKit.test("债务棘轮: UI 不得在 body 里就地构造 onReceive 的 publisher") {
            // onReceive 的第一个实参在每次 body 求值时都会重新求值。就地写
            // `Timer.publish(...).autoconnect()` 等于每次重绘新建一个计时器并重新订阅，
            // 于是「每 5 秒刷新」在界面持续变化时（用户在输入框里逐字改地址就是这种）
            // 永远走不满 5 秒——看着实时，其实冻在上一次空闲刷新上。
            // publisher 必须是只构造一次的存储属性（static let / 实例常量）。
            var offenders: [String] = []
            for url in try SourceTree.requireSwiftFiles(under: "AgentIsland", atLeast: 10) {
                guard let text = try? String(contentsOf: url, encoding: .utf8) else { continue }
                // 去掉全部空白再匹配：跨行写的就地构造（onReceive( 换行 Timer.publish…）也算
                let dense = text.components(separatedBy: .whitespacesAndNewlines).joined()
                if dense.contains(".onReceive(Timer.publish(") {
                    offenders.append(url.lastPathComponent)
                }
            }
            try expectTrue(offenders.isEmpty,
                           "onReceive 里就地构造了 publisher（每次重绘都重置计时，定时刷新会长期不触发）："
                           + offenders.joined(separator: "、"))
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
}

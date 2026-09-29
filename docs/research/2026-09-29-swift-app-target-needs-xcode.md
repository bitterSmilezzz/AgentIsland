# Swift 应用 target 现在编不出来（直接原因是缺 SwiftUIMacros 插件）

> 一份**环境**记录。2026-09-29 撞到。
>
> ⚠️ **本文档在 v0.0.244 被更正过一次**：v0.0.243 首版断言「这台机器从来编不出
> Swift 应用」，那是**错的**——同机在 2026-09-28 20:32 确实编译成功过。
> 下面是更正后的版本，已把「验到」与「没验到」分开写。

## 一、验到的

### 现象

`./scripts/build-app.sh`（不传 `SKIP_SWIFT=1`）必然失败，而且报错长得完全不像环境问题：

```text
error: external macro implementation type 'SwiftUIMacros.StateMacro'
       could not be found for macro 'State()'
```

然后**连锁出几十条**误导性错误：

```text
cannot find '$range' in scope
cannot find '$showingTooltip' in scope
cannot find '$selectedFilter' in scope
cannot assign to property: 'self' is immutable
cannot assign through subscript: 'self' is immutable
left side of mutating operator isn't mutable: 'self' is immutable
the compiler is unable to type-check this expression in reasonable time
```

第一条报错的行号落在 `ToolboxView.swift:636`：

```swift
verifyCleanup { remaining in
    guard case .toolbox = self.controller.route else { return }
    self.anomalies = remaining          // ← error: cannot assign to property: 'self' is immutable
```

`verifyCleanup` 内部是 `DispatchQueue.main.asyncAfter { self.scan { … } }`，
闭包里捕获的 `self` 成了不可变快照——**看起来是个标准的「闭包捕获 self 变只读」真 bug**。
按那个方向去改会改错地方。

### 直接原因

`@State` / `@Binding` / `@Environment` 这些 SwiftUI 属性包装器是**宏**，
由 `SwiftUIMacros` 插件实现。而它**现在**在任何可达位置都不存在：

```sh
$ xcode-select -p
/Library/Developer/CommandLineTools          # /Applications 下没有 Xcode.app

$ ls /Library/Developer/CommandLineTools/usr/lib/swift/host/plugins/
libObservationMacros.dylib
libSwiftMacros.dylib
testing
# —— 没有 SwiftUIMacros

$ ls ~/Library/Developer/Toolchains
（无备选工具链）
```

`AgentIsland` 应用 target 里几乎每个视图都用 `@State`，于是整棵视图层编不出来。
`AgentIslandCore` 与 `AgentIslandCLI` 不用 SwiftUI 宏，照常编译——
**这也是它一直没被发现的原因**：Swift 测试门禁只编 Core + CLI，应用 target 不在覆盖范围内。

### 它**曾经**编译成功过

这是本文档要更正的地方。app target 的全部目标文件都是**真实编译**出来的，
不是残留缓存：

```sh
$ ls -lT .build/out/Intermediates.noindex/AgentIsland.build/Release/AgentIsland-p.build/Objects-normal/arm64/*.o
Sep 28 20:32:31 2026  ActivitySearchFilter.o / DiagnosticsSnapshot.o
Sep 28 20:32:34 2026  IslandView.o / ToolboxView.o
Sep 28 20:32:35 2026  SettingsView.o / TokenAnalyticsView.o   ← 约 40 个文件，前后 4 秒
```

`dist/AgentIsland-Swift.app` 与 `.build/out/Products/Release/AgentIsland` 也都是
`Sep 28 20:32`。也就是说**同机、同 SDK（`MacOSX27.0.sdk`，8 月 31 日就在），
2026-09-28 20:32 编译成功，2026-09-29 编译失败**。

## 二、没验到的（别当结论用）

**为什么 09-28 能编、09-29 不能，我没有查清。**

已排除的：`SDK` 选择（`build-app.sh` 的候选顺序在 09-28 就已指向 27.0，且该 SDK 日期更早）、
`SDKROOT` 显式设置（不设也会失败）、备用 toolchain（没有）、
源码（`Sources/AgentIsland/` 最后一次改动是 v0.0.135 / 09-25，在两次编译之间没动过）。

**推断（标注为推断，未证实）**：`pkgutil --pkgs` 里仍有 `com.apple.pkg.Xcode` 收据，
而 `/Applications` 下已无 Xcode.app——**Xcode 曾经装过、后来被移除**。
`SwiftUIMacros` 是 Xcode 闭源插件，CLT 不带；Xcode 在时能编，移除后就编不出来。
这能解释全部现象，但**我没有证据证明 Xcode 是何时、被谁移除的**。

## 三、处理

`build-app.sh` 加了**前置检查**：进 Swift 构建段之前先看当前工具链
（`$(xcode-select -p)`，**不能写死 CommandLineTools**——写死的话，装好 Xcode 并
`xcode-select -s` 之后仍然找不到插件，会误报成「没装 Xcode」）里有没有 `SwiftUIMacros`，
没有就立刻停，并给三条出路：

1. 装 Xcode（`xcode-select -s /Applications/Xcode.app`）后重跑；
2. `SKIP_SWIFT=1` 跳过——**代价是没有 `dist/AgentIsland-Swift.app` 这条回退路**；
3. 确认不再需要回退路，把这一步从脚本里删掉。

**回退路在第 ② 条下是不存在的。** 这一点必须说出来，不能让人以为「跳过也一样」。

⚠️ 顺带一提：`dist/AgentIsland-Swift.app` 里现存的仍是 09-28 那份产物。
它**不是**当前源码能重新构建出来的——别把它当成「随时可以退回去」的保证。

## 四、两条教训

1. **报错的下游症状可以完全不像根因。** `@State` 宏缺失 → 投影值找不到 →
   `self` 读不到可变成员 → 报「self 不可变」。顺着症状改代码会改错地方。
2. **「这个 target 能不能编」与「我的测试过没过」是两回事。**
   测试门禁只编 Core + CLI 时，应用 target 不在覆盖范围内——
   而「能不能编」正是回退产物存在与否的前提。

## 取证命令

```sh
xcode-select -p
ls "$(xcode-select -p)/usr/lib/swift/host/plugins" | grep -i macros
# 完整错误清单（别看 tail，会被那条 4000 字符的命令行盖掉）
swift build -c release --product AgentIsland 2>&1 | grep -E "error:" | sort -u
# 目标文件时间戳——判断「是真编译过」还是「只是链接了旧缓存」
ls -lT .build/out/Intermediates.noindex/AgentIsland.build/Release/AgentIsland-p.build/Objects-normal/arm64/*.o
```

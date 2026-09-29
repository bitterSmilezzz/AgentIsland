# Swift 应用 target 在只装 CommandLineTools 的机器上编不出来

> 一份**环境**记录，不是代码问题。2026-09-29 撞到并查清。

## 现象

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

最后把一整条 **4000 字符的 `swift-frontend` 命令行**糊在脸上。

第一眼看到 `ToolboxView.swift:636: error: cannot assign to property: 'self' is immutable`
加上「复核回调在 `DispatchQueue.main.asyncAfter` 里捕获 `self`」，非常像是一个
**真代码 bug**：闭包里 `self` 成了不可变快照，所以 `self.anomalies = remaining` 非法。
按那个方向去改会改错东西。

## 真因

`@State` / `@Binding` / `@Environment` 这些 SwiftUI 属性包装器是**宏**，
由 `SwiftUIMacros` 插件实现。而 `SwiftUIMacros` 是 **Xcode 闭源提供的**。

这台机器上：

```sh
$ xcode-select -p
/Library/Developer/CommandLineTools      # 没有 /Applications/Xcode.app

$ ls /Library/Developer/CommandLineTools/usr/lib/swift/host/plugins/
libObservationMacros.dylib
libSwiftMacros.dylib
testing
# —— 没有 SwiftUIMacros
```

`AgentIsland` 应用 target 里几乎每个视图都用 `@State`，于是**整棵视图层编不出来**。
`AgentIslandCore` 与 `AgentIslandCLI` 不用 SwiftUI 宏，所以照常编译——
这也是它一直没被发现的原因：Swift 测试门禁只编 Core + CLI。

## 与代码新旧无关

**推断（标注为推断）**：这台机器大概从装好 CommandLineTools 起就没有 Xcode，
所以 Swift 应用 target 在**本机**从来没构建成功过。之所以没暴露，是因为
v0.0.233 之后交付物换成了 Rust 端，而发版一直带着 `SKIP_SWIFT=1`（或脚本的 Swift 段被跳过）。

⚠️ 这一点**没有独立证据**——我没有查这台机器的安装历史。
可确证的是：**当前**工具链缺 `SwiftUIMacros`，因此当前编不出来。

## 处理

`build-app.sh` 加了**前置检查**：进 Swift 构建段之前先看一眼 host/plugins 里有没有
`SwiftUIMacros`，没有就立刻停，并给出三条出路：

1. 装 Xcode（`xcode-select -s /Applications/Xcode.app`）后重跑；
2. `SKIP_SWIFT=1` 跳过——**代价是没有 `dist/AgentIsland-Swift.app` 这条回退路**；
3. 确认不再需要回退路，把这一步从脚本里删掉。

**回退路在第 ② 条下是不存在的。** 这一点必须说出来，不能让人以为「跳过也一样」。

## 两条教训

1. **报错的下游症状可以完全不像根因。** `@State` 宏缺失 → 投影值找不到 →
   `self` 读不到可变成员 → 报「self 不可变」。顺着症状改代码会改错地方。
2. **「这个 target 能不能编」与「我的测试过没过」是两回事。**
   测试门禁只编 Core + CLI 时，应用 target 是不在覆盖范围内的——
   而「能不能编」正是回退产物存在与否的前提。

## 取证命令

```sh
xcode-select -p
ls /Library/Developer/CommandLineTools/usr/lib/swift/host/plugins/ | grep -i macros
# 完整错误清单（别看 tail，会被那条巨型命令行盖掉）
swift build -c release --product AgentIsland 2>&1 | grep -E "error:" | sort -u
```

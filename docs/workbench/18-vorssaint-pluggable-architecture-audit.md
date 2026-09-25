# Vorssaint 一手调研

> 一手核实记录。来源：GitHub API 元数据（2026-09-26）、`main` 分支最新提交
> `71b7dfe`（2026-09-25 20:33:31 -0300，`docs(changelog): note clipboard history pasteboard restart fix`）
> 的**浅克隆全量源码**（841 个 blob / 745 个 Swift 文件 / Sources 610 文件 255,139 行 /
> Tests 131 文件 55,836 行）、`README.md`、`docs/PERMISSIONS.md`、`docs/PRIVACY.md`、
> `CONTRIBUTING.md`、`TRADEMARKS.md`、`Package.swift`、`build.sh`、`Tools/*.sh`、
> `.github/workflows/{ci,release}.yml` 的逐行原文。
> GitHub API 元数据（2026-09-26）：**21355 star / 799 fork / Swift / GPL-3.0 /
> 创建于 2026-06-12T15:25:43Z / 最后 push 2026-09-25T23:33:34Z / 548 open issues /
> repo size 73745 KB / homepage vorssaint.com / default_branch main**。
> 本文只记录「它是什么」「它怎么做到的」「和我们已有能力的关系」「我们要不要抄」。
> 引用按 `path:line` 给出，读不到的写「未获取」。**全文不含任何真实 token / key 值。**
>
> 本地素材：`/tmp/vorclone/`（浅克隆，含全部 Sources/Tests/docs/CI/build.sh）；
> 早前抓取的 `/tmp/uibatch/vorssaint/{README.md,repo.html}` 已与之核对，README 内容一致。

---

## 1. 一句话结论

**Vorssaint 是本文档写到这里为止形态最接近「macOS 常驻工具台」的一个：73 个可卸载 feature、
13 项系统权限、4800+ 用户可见开关，全部由一个 `AppFeature` 枚举 + 一张 `[AppFeature: () -> Void]`
闭包表串起来。它回答了我们侧边栏的核心问题——「一个入口怎么承载多个可插拔模块而不变成杂物抽屉」——
答案是「availability 是 enable 之上的独立一层，卸载 = 服务根本不实例化，设置全部留在 UserDefaults 里等它回来」。
**它的一整套 `Services/AgentUsage/` 是本文最硬的发现：`AgentProvider` 只有 `claude` 与 `codex` 两个 case
（`Sources/Vorssaint/Services/AgentUsage/AgentUsageModels.swift:8-9`）——一个 21355 star / 255139 行的
成熟产品，agent 用量这块**也只做两家**。这与我们「Phase 2 只做 Codex 一家」是同一判断的独立佐证。**
而对 Claude 侧，它走的是 `AgentClaudeAppUsage.swift`：只读
`~/Library/Application Support/Claude/plan-usage-history.json`（Claude Desktop 自己每 5–15 分钟
记一次的百分比采样），**零凭据、零网络、零子进程、零写入**，且 `version == 1 || version == 2`
之外的格式整体返回 nil（"left out rather than guessed"）——比 codenotch 那套三级回退更干净的一条路径。
它还从历史样本反推 session 重置时刻（"a session renews five hours after the hour its use began"）。

它同时给出了一套教科书级的权限 UX：`activeFeatures(using:)` 一个纯函数同时喂「这条权限谁在用」（反查）
与「这条权限现在没人用，你可以撤销」（提示），我们把 Phase 2 配置文件授权照抄这套即可。
**但两条红线必须先说清：GPL-3.0 是强传染，源码一行不能抄（我们只抄做法与结构）；
它的 AI Agents 那一节已经证明「读 Claude Desktop 的 plan-usage-history.json」是一条零凭据零网络的路径，
这与我们 Phase 2 的档位卡是同一个问题的两种解法，且比 codenotch 的三级回退更干净。**

---

## 2. 规模与工程事实（先钉住数字，后文引用）

| 维度 | 实测值 | 取证 |
| :--- | :--- | :--- |
| Feature 数 | **73 个 `AppFeature` case**（`enum AppFeature: String, CaseIterable`，`Sources/Vorssaint/Core/FeatureCatalog.swift:15`；逐个 case 计数） | 克隆全量 + case 清单 |
| 分组 | 8 个 `FeatureGroup`（windowsDock / mouseKeyboard / clipboardFiles / sound / energyDisplay / tools / dynamicIsland / monitor，`:42`） | 同上 |
| 权限 | 13 项 `AppPermission`（accessibility / screenRecording / fullDiskAccess / filesAndFolders / notifications / automationFinder / automationTerminal / automationPlayback / audioCapture / microphone / camera / appManagement / calendar，`:46`） | 同上 |
| Swift 源码 | **610 文件 / 255,139 行**（`Sources/Vorssaint/**/*.swift`） | `find` + `wc -l` |
| Swift 测试 | **131 文件 / 55,836 行** | 同上 |
| 测试/实现行数比 | **约 1 : 4.6**（我们的 Swift 端是 548 个 `test(...)` 调用，口径不同不可直接比） | 实测 |
| 语言 | 13 门本地化（`Resources/*.lproj` 13 个 + `Core/Localizations/Strings+*.swift` 13 个），README 自称「more than a dozen languages」 | 克隆 |
| CI | 2 个 workflow：`ci.yml`（macos-15 Swift 6.0.3 兼容 + macos-26 构建/selftest/打包/测试）与 `release.yml`（受保护签名环境） | `.github/workflows/ci.yml`、`release.yml` |
| 测试入口 | `./build.sh --test`（自建 runner，编译一个选定源集跑 `./build/metrics-tests`，无 XCTest）；`./build.sh --list-tests` / `--test-suite=NAME` | `build.sh:519-530`、`CONTRIBUTING.md` |
| 运行期自检 | `./build/Vorssaint --selftest`（`Sources/Vorssaint/Support/SelfTest.swift`，IOKit 电源断言/内存/SMC 均实查） | 克隆 |
| 构建 | **只要 CLT**：`swiftc` 直编，无 xcodebuild、无 SPM 依赖；`Package.swift` 只为编辑器索引（注释原话「supports editor indexing; it does not assemble or sign the app bundle」，`CONTRIBUTING.md:15-17`） | `build.sh:550-575`、`Package.swift` |
| 目标平台 | macOS 14+，**仅 arm64**（`TARGET="arm64-apple-macosx14.0"`，`build.sh:65`） | `build.sh` |
| 发布纪律 | tag 形如 `vX.Y.Z`；release.yml 三重门：preflight 验签 + 验 commit 在 main 上 + 验 `Info.plist`/`CHANGELOG` 版本一致 → 受保护环境签名/公证 → publish 时复核 SHA256 与 release 元数据 | `.github/workflows/release.yml` 全文 |
| 版权/商标分离 | `LICENSE` = GPL-3.0；`TRADEMARKS.md`（746 B，全文已读）：源码归 GPL，**名字/logo/icon/bundle id/trade dress/签名身份都不归 GPL**，fork 必须换名换图标换 bundle id 换签名身份换更新源 | `TRADEMARKS.md:1-14` |

---

## 3. A. 可插拔架构（本文重点）

### 3.1 三层模型：availability ⊃ enable ⊃ 资源

它的 catalog 文件头把设计说死了，值得整段引（`Core/FeatureCatalog.swift:6-14`）：

> Availability is a layer ABOVE each feature's own enable key: an unavailable
> feature disappears from Settings, the menu panel and the menu bar, and its
> service tears down (and never instantiates on the next launch). Turning a
> feature back on restores saved enable choices. A first install turns on its
> primary control when no enable choice was saved before.

三层是：

1. **availability（安装层）**：`UserDefaults` 一个布尔键，键名 `featureAvailable.<rawValue>`
   （`var availabilityKey: String { DefaultsKey.featureAvailable(rawValue) }`，`:207`；
   `static func featureAvailable(_ id: String) -> String { "featureAvailable.\(id)" }`，
   `Core/Defaults.swift:847`）。**默认值不是写死的，是 `availabilityDefaults` 现算的**
   （`:404-411`）：除 8 个显式 opt-in（focusFollowsMouse / fanControl / diskImageInstaller /
   killProcess / scrollHorizontal / portManager / wallpaper / audioPriority）外全部 `true`，
   「Existing features stay available on update; new opt-in features and explicit betas
   ship uninstalled.」（`:403` 注释原话）
2. **enable（行为层）**：每个 feature 的 `enabledKeys`（`:225-283`）——**一个 feature 可以有多把钥匙**，
   例如 `.dockClick` 有三把（minimize / hide / cycleWindows），`.scrollInverter` 有两把
   （纵轴 / 横轴），`.finderCutPaste` 有两把。**空数组表示「按需工具」**（一个面板磁贴、一个菜单动作），
   「being available already counts as engaged for the permissions portal」（`:220-222` 注释）。
3. **资源层**：真正的服务单例。

**关键点：装卸是「运行时开关」，不是编译期开关。** 证据链：
`setAvailable(_:_:)` 写 availability 键 → 立刻跑 `Self.bindings[feature]?()` → 再 bump `revision`
（`App/FeatureRuntime.swift:100-123`）。`bindings` 是 `private static let bindings: [AppFeature: () -> Void]`
（`:202-361`），**160 行的闭包表**，每个入口是 `XxxService.shared.syncWithPreferences()`。
服务侧的 `syncWithPreferences()` 一律是「wanted && 已安装 && 有权限 && 会话在前台 → start()，else stop()」
（例：`Services/SmoothScrollService.swift:75-85`，判据抽成了
`SessionActivitySupport.tapShouldRun(featureWanted:accessibilityGranted:sessionIsActive:)`，
`Services/SessionActivitySupport.swift:35-39`）。

所以 README 那句「Uninstalled features stop loading and disappear from the interface」
**在源码里有字面对应，而且比 README 说的更强**：

- 「stop loading」= 启动路径 `syncAtLaunch()` 只遍历 available 的 feature
  （`FeatureRuntime.swift:177-181`，注释「Only available features get their binding run,
  so nothing else even instantates.」），**未安装 feature 的单例连构造都不发生**；
- 「disappear from the interface」= `FeatureVisibilitySupport.isPageVisible(page:)`
  按 `features(for:)` 的**门集合**判整页去留（`UI/Settings/FeatureVisibilitySupport.swift:349-357`），
  且「A page with several features only disappears when ALL of them are switched off」
  （`:341-343` 注释）。菜单面板同理：`PanelSectionID.featureGate`（`UI/MenuPanel/PanelLayout.swift:80-108`）
  + `isVisibleInPanel`（`:170-173`）。

### 3.2 设置持久化：一套 UserDefaults，不删只藏

这是「reinstalling restores their settings」的实现，也是**对我们最直接可抄的一条**。

- **一切存在 `UserDefaults.standard`**：`Defaults.swift` 是一个 140 KB 的 `enum DefaultsKey`
  （实测 1743 行），`Defaults.register()` 一次性 `register(defaults:)` 全部键
  （`Core/Defaults.swift:1734-1758`），**含 `AppFeature.availabilityDefaults`**（`:1743`）——
  注意这是 `register`（注册默认值）而不是 `set`，所以 `object(forKey:)` 能区分
  「全新安装」与「用户存过一个 false」（`FeatureCatalog.swift:218-221` 的
  `enableOnFirstInstall` 正是靠这个区分）。
- **卸载从不删键**：`setAvailable` 只 `set(available)`，不 `removeObject`
  （`FeatureRuntime.swift:114-116`）；反过来 `enableOnFirstInstall` 也只在
  「持久域里这个键从没出现过」时才写入（`FeatureCatalog.swift:293-298`），
  **所以「卸了再装」不会把人关掉的开关又打开**。
- **权限授予用的文件夹书签单独隔离**：`Files and Folders` 用的是 security-scoped bookmark，
  且 PRIVACY.md 明说「does not travel in a settings backup」（`docs/PRIVACY.md:92`）。
- **一个 counter-example 值得记**：`clipboardHistory` 的 binding 里写着
  「Auto clear rides the clipboard feature's availability but not its capture toggle:
  uninstalling the feature stops it, turning history off does not.」（`FeatureRuntime.swift:235-237`）
  ——即**一个 availability 键可以搭载多个服务**，它把依赖关系写成了显式注释。
- **对照我们的 `SettingsStore`**：`Sources/AgentIslandCore/SettingsStore.swift`（490 行）是
  `enum SettingKey { static let ... }` 字面量集中管理，同样是 UserDefaults、同样有
  `customAgents.corruptBackup` 一类派生键。**我们的形态与它同构，缺的只是「availability 层」**：
  我们的键全是 enable 语义，没有「这个模块装没装」这一层。这是 §6.1 第一条改造点的根据。

### 3.3 装卸闸门：三道互相校验的规则

| 规则 | 实现 | 为什么这么写（原注释） |
| :--- | :--- | :--- |
| **硬件不支持不许装** | `mayFlip(_:to:)`（`FeatureRuntime.swift:88-91`）：`return !available \|\| feature.isHardwareSupported` | 「A feature the Mac cannot run never installs, so a disabled row cannot be walked around from the button above it.」（`:79-84`） |
| **已装的不许被吊销** | 同函数：`guard feature.isAvailable != available else { return false }` | 「Uninstalls are never refused and an existing install is never revoked: the check reads hardware and can be wrong, and a wrong answer that strands someone's settings costs far more…」（`:84-87`） |
| **每行都读同一处闸门** | `installBlockedReason`（`FeatureCatalog.swift:394-396`）被 hub 行与首次运行选择器共用 | 「Both the hub and the first-run picker read this, so neither can drift from the gate in `FeatureRuntime`.」（`:392-393`） |

**硬件不支持的理由也要能显示**：`hardwareUnsupportedReason`（`FeatureCatalog.swift:377-385`）
注释「One switch answers both questions, so a feature can never be unsupported without
a reason to show for it」。目前只有 `fanControl` 一条（问 `FanControlHardware.hasControllableFan`）。

### 3.4 「立即卸载」做不到的事，它老实承认

**这是我认为它最值得学的一点：诚实边界写进 UI。** `needsRestartToUnload`（`FeatureRuntime.swift:31-33`）：

> A feature uninstalled mid-session stops working immediately, but its (inert) singleton
> only leaves memory on the next launch — this set is what the hub's restart banner keys off,
> including the install-then-uninstall-again case.

且 `loadedThisSession` 的初始化是 `Set(AppFeature.allCases.filter(\.isAvailable))`
（`:24`）——**启动时就没装的 feature 不进这个集合，所以不需要重启提示**（`:29-30` 注释）。
重启走 `relaunchApp()`：spawn 一个 `/bin/sh` 轮询等本进程消失再 `open`，等进程而不是等固定时长，
「quitting flushes the clipboard history and every other pending write first」（`:36-42`），
20 秒收工（`:47-48`）。

### 3.5 一个入口怎么不变成杂物抽屉（正面回答）

它的 Settings 是 30 个 `SettingsPage`（`FeatureVisibilitySupport.swift:9-12`），但** indexing 是一张静态表而不是散落的 `if`**：

- `AppFeature.settingsDestination`（`:237-347`）**穷尽 switch，73 个 case 全覆盖**，
  注释「Exhaustive by design: adding an AppFeature requires choosing its Settings
  destination before the project compiles.」（`:230-231`）。实测：**73 个 feature 落到 29 个 page**
  （notch 11 个共用一页、mouse 9 个、monitor 8 个、quickTools 7 个…），
  另 `diskImageInstaller` 的 destination 就是 `.features`（hub 本身），
  `hasNavigableSettingsDestination` 于是为 false，行上不画 chevron（`:231-234`，注释
  「The hub itself is the honest fallback … but linking a row back to its current page would
  present a chevron that appears to do nothing.」）。
- **hub 是唯一的总表**：`FeatureHubSettings`（`UI/Settings/FeatureHubSettings.swift:12-15`）
  两 tab——「功能」+「权限」。功能 tab 里有 summary 卡（已装 N/可装 M + 一条 `ProgressView`）、
  preset 卡、8 个 group 的 DisclosureGroup，组头是 `X/Y` + 份额条
  （`groupHeader(_:installed:total:)`，`:315-330`）。**「Nobody arrives wanting 67 decisions;
  a preset shapes the app in one move and everything else stays one click away」
  （`presetsCard` 注释，`:204-206`）。**
- 每行给三个东西：`hubTitle` + `hubDescription`（长文）、一行 metadata（权限名 + 能耗档）、
  一个安装开关；装了且有页面才画 chevron 并可点进设置页（`FeatureHubRow`，`:417-470`）。
- 从 Command Bar / 搜索跳进来时带 `SettingsFeatureTargetRequest`，落到 hub 会**展开对应 group
  并高亮那一行**（`revealPendingFeatureTarget`，`:105-118`），且请求 id 一次性消费，
  「a stale target from an earlier search can never leak into a later, unrelated navigation」
  （`:140-142`）。

**能耗是显式声明的静态档，不是现测的**——这一点它的注释很硬（`Core/FeaturePresets.swift:69-75`）：

> The honest, curated cost label each feature earns in the hub: what the feature keeps alive
> WHILE IT IS ON. Uninstalled features load nothing at all, which is the hub's own promise.
> **Static by design — pretending to measure per-feature cost live would be theater.**

六档：`idle`（按需工具，静止零成本）/ `mouse`（鼠标事件 tap）/ `pointer` / `keyboard` /
`inputs` / `periodic`（定时采样）（`:78-89`）。hub 行上显示成一个词 + 一组小图标
（`energySymbols`，`FeatureHubSettings.swift:459-471`）。

### 3.6 我们要改什么（侧边栏四块的可插拔改造点）

```text
今天                                 改造后
─────────────────────────────────────────────────────────────────
sidebar 四块（监控 / Provider /       SidebarModule enum（5 个 case：
待办 / 高级设置）写死在 UI 里            monitor / provider / todos / advanced
                     ──►             + 预留 agents）
                                      availabilityKey = "moduleAvailable.<id>"
                                      availabilityDefaults 现算（默认全部 true）
                                      bindings 表：module → 重渲染/启停闭包
                                      isPageVisible 按 module 门集合判块去留
                                      卸载不删键，重装恢复 enable
                                      enableOnFirstInstall 只在新键时写
                                      needsRestartToUnload → 侧边栏内横幅
```

四条最小改动（Phase 1 可做，逐条都在 Vorssaint 有对应实现）：

1. **加一个 `SidebarModule` 枚举 + `moduleAvailable.<id>` 键层**（照 `FeatureCatalog.swift:15`、
   `:207`、`Defaults.swift:847`）。我们今天只有 enable 语义（`compactView` / `menuBarBadgeMode`
   这种），没有「这块装没装」。四块的增删今天是改代码，改后是改一行枚举。
2. **设置全部留在 `SettingsStore`，卸载只翻 availability**（照 `FeatureRuntime.swift:114-116`）。
   我们 Phase 2 的 CC Switch 档位文件尤其需要这条：卸载一个 provider 不该删用户的档位。
3. **hub 式总表替换「高级设置」里的平铺列表**：一个「模块」页，四块各一行（名 + 一句话 +
   用到哪些权限 + 一个开关），未装的整块从侧边栏消失。**注意我们不需要它的 73 行规模**——
   我们 4–5 个模块，preset/preset 卡可以不抄，但「已装 N/可装 M + 一条份额条」值得抄。
4. **侧边栏导航的显式请求 + 一次性消费**（照 `SettingsRouter`，`FeatureVisibilitySupport.swift:113-215`）。
   我们从待办跳分析页、从 Provider 跳档位编辑时，用同样的 `requestID` + `pendingTarget`
   防止旧请求落到新导航上——它注释里那个 bug（迟到的旧请求覆盖了新导航）是真实会发生的。

**一条它没做但我们要做的**：它的 hub 行「装了且有页面」才可点。我们侧边栏四块里
「高级设置」本身就是入口，不能对它再用 chevron 语义，得照 `hasNavigableSettingsDestination=false`
那条处理（不画箭头、不像链接）。

---

## 4. B. 权限 UX（本文重点）

### 4.1 四条逐条对源码

README 那句（`README.md:228`）：「Every one is optional, the app explains each in plain words,
shows which features actually use it, and even tells you when a permission you granted is
no longer needed by anything, with a shortcut to revoke it.」

**① 可选 —— 静态映射 + 一票否决**

- 每个 feature 声明 `permissions: [AppPermission]`（`FeatureCatalog.swift:315-373`），
  **switch 穷尽且带注释解释为什么**，例：
  「The bar reads other apps' menus and windows and types at the caret, all of it through
  Accessibility.」（`.commandBar`，`:329-331`）、
  「Only emptying the Trash asks the Finder; every other quick toggle (dark mode included)
  works without a permission.」（`.quickToggles`，`:325-327`）。
- **反例清单同样显式**：`.notchAgents: return []` 带注释
  「Session logs and the saved limits sit in the home folder, outside every protected
  location, and no sign-in or keychain item is used.」（`:320-322`）——
  **它把「不需要权限」也写进了表**，这是我们该抄的：我们 Phase 2 读 agent 配置文件同样零权限，
  应该在表里写成一条显式的声明而不是留空。
- `onboardingPermissions`（`:376-386`）是 permissions 的**子集**，只留 accessibility 与
  screenRecording，注释「Broad grants worth explaining during first run. Permissions used only
  by an optional sub-feature stay contextual, at the moment that control is actually used.」
- 每个权限在 `docs/PERMISSIONS.md` 有一段「Why it comes up / What uses it / **If you say no** /
  Optional」四段式（`PERMISSIONS.md:22-44` 等），「If you say no」**逐条写降级行为**
  （例 Screen Recording：switcher 退回 app 图标，Dock Preview/截图/OCR 直接不可用，
  `:52`）。**这张表本身是可核对的**（见 §7）。

**② 平白解释 —— 每权限一句 explainer，13 门语言齐全**

- `permission.explainer(hub)`（`FeatureHubSettings.swift:1074-1090`）逐权限取一句。
- 中文本地化实测（`Core/FeatureHubStrings.swift:1450-1457`）：
  「让功能响应点按和按键，并移动窗口。」/「让功能显示窗口缩略图并读取屏幕上的文字。」/
  「让清理器和卸载器在任何位置找到残留文件。」—— **一句一到两行，不含术语**。
- **首次运行 onboarding 的 permissions 步骤直接复用同一张 portal**：
  `PermissionsPortalSections` 出现在 onboarding 的 `DisclosureGroup` 里
  （`UI/Onboarding/OnboardingView.swift:472-476`），两张皮一份数据。
- 权限请求路径带一个**不抢焦点的浮动卡**：`PermissionGuideOverlay`（`UI/PermissionGuideOverlay.swift`），
  三步指引 + 自动等授权（订阅 `Permissions.shared.$accessibility` / `.$screenRecording`，
  `.filter { $0 }`，`:87-105`），12 秒还没授就显示「stale」提示 + 「重新开始」按钮
  （`staleAfter = 12`，`:30`；`:196-206`）。卡片 `NSPanel` 是 `.nonactivatingPanel`
  + `canJoinAllSpaces`，所以**系统设置在它后面保持焦点**（`:69-71` 注释）。

**③ 显示谁在用 —— 一个纯函数反查**

这是整套的核心，值得整段引（`FeatureCatalog.swift:414-437`，节选）：

```swift
static func activeFeatures(using permission: AppPermission,
                           isAvailable: (AppFeature) -> Bool,
                           boolFor: (String) -> Bool,
                           stringFor: (String) -> String?,
                           dataFor: (String) -> Data? = { _ in nil }) -> [AppFeature] {
    allCases.filter { feature in
        guard feature.permissions.contains(permission), isAvailable(feature) else { return false }
        let keys = feature.enabledKeys
        guard keys.isEmpty || keys.contains(where: boolFor) else { return false }
        switch (feature, permission) { ... default: return true }
    }
}
```

**它同时是「声明」与「判定」**：静态部分看 `permissions` + availability + enable 键，
动态部分是一个 `(feature, permission)` 的 switch，**每条都是一句业务规则**：

| (feature, permission) | 判据 | 规则原文 |
| :--- | :--- | :--- |
| `.switcher, .screenRecording` | `!boolFor(switcherSimpleMode)` | 简化模式不需要录屏（`:428-429`） |
| `.notch, .automationPlayback` | `notchHiddenModules` 不含 "music" | 音乐模块被藏了就不算用（`:426-427`） |
| `.notchAccessories, .accessibility` | 从未列出 | 手势类 notch 扩展不需要辅助功能（`:327`） |
| `.keepAwake, .accessibility` | `boolFor(keepAwakeMouseJiggleEnabled)` | 只有开鼠标抖动才用（`:452-453`） |
| `.mixer, .accessibility` | `boolFor(preciseVolumeRollerEnabled)` | 只有精细滚轮用（`:454-455`） |
| `.brightness, .accessibility` | keys 或 OSD 任一开（`:456-458`） |
| `.monitorCPU, .notifications` | `monitorAlertCPU \|\| monitorAlertCPUTemperature`（`:461-462`） | 只有警报开着才用通知 |
| `.cleaner, .filesAndFolders` | `whatsAppDownloadsEnabled`（`:466`） | 与 WhatsApp 下载相关的独立子功能 |
| `.screenRecorder, .microphone` | `recorderMicrophone`（`:474`） | 录制时才问麦克风 |

**它把这函数做成纯函数 + 注入读取器，注释给了理由**：「Readers are injectable so the logic
stays testable without touching real UserDefaults.」（`:409-411`）。
测试侧确实验了这套（`Tests/FeatureCatalogTests.swift:1113-1120` 的 `activeSet` 辅助，
`:1186-1187` 的「with nothing enabled only on-demand features use accessibility」，
`:1171-1174` 的 windowLayout 分区规则）。

**④ 不再需要时可撤销 —— 就是 `activeFeatures.isEmpty`**

`FeatureHubSettings.swift:766-770`：

```swift
if status == .granted, activeFeatures.isEmpty {
    unusedCard
}
```

`unusedCard`（`:821-833`）渲染 `hub.unusedBanner`。中文本地化（`FeatureHubStrings.swift:1441`）：

> 你已授予此权限，但没有已开启的功能需要它。如果愿意，可以在系统设置中撤销。

`usedByLine`（`:788-792`）非空时渲染「使用者：A、B、C」（`:1440`），空时
`hub.usedByNone` = 「目前没有已开启的功能在使用此权限。」（`:1440`）。

**这就是答案：「不再需要」= 纯函数求值为空，不是引用计数，不是静态映射表对比。**
它没有「曾经用过现在没用」的历史，也没有计数——**只有当下这一刻谁在用**。
撤销通道是 `openSystemSettings()`（`:865-881`）逐权限 `x-apple.systempreferences:` pane URL，
外加 `startOver(_:)` 用 `tccutil reset <service> <bundleID>` 抹掉 TCC 条目让系统重新问
（`Core/Permissions.swift:280-296`，注释解释为什么必须这么做：macOS 把权限绑到代码签名，
自建/换签名的构建会看到「开关是开的但 app 不被信任」，「nothing short of removing the entry
makes the system ask afresh」）。

### 4.2 两条我们直接能用的工程细节

**(a) 权限轮询不是定时器，是三段式按需（`Permissions.swift:56-119`）**

```text
visibleSurfaceCount > 0 或有功能需要却没授  → 2.5s
有功能需要且已授（守「撤销」）            → 60s
没人需要                                  → 无定时器
```

- 纯函数版 `PermissionPollingSupport.interval(...)`（`FeatureCatalog.swift:51-65`），
  四个输入全是注入的，测试四条覆盖（`FeatureCatalogTests.swift:1121-1144`）。
- 「谁需要」= `activeFeatures(using:).contains { $0.monitorsPermissionChanges }`
  （`Permissions.swift:93-104`）。`monitorsPermissionChanges`（`FeatureCatalog.swift:69-97`）
  **区分「常驻功能」与「一次性工具」**：注释「One-shot tools ask and refresh at the moment they
  run; polling for those just because their tile is installed wastes wakeups.」——
  screenOCR / cleaningMode / screenshot / commandBar / screenRecorder / wallpaper 显式 `false`。
- `setActivePermissionSurface(_:visible:)` 用 UUID 记「可见的权限 UI 占了几张需求票」
  （`Permissions.swift:123-131`），保证 SwiftUI 反复 onAppear 不会把定时器泄漏。
- **Full Disk Access 故意不轮询**（`Permissions.swift:68-71` 注释）：它只能跨重启变，
  且探测本身要碰受保护路径，「polling it would just be repeated denied accesses for no gain」。

**(b) 「撤销 Accessibility」前必须先停 event tap，否则整机输入冻结**

`SelfUninstall.swift:132-139` 注释原文：

> Tears down every Accessibility-backed input interceptor. MUST run on the main thread and
> BEFORE permissions are reset: otherwise revoking Accessibility while an event tap is still
> live makes the tap's callback block on an AX call, which stalls the OS input queue and
> freezes the keyboard and clicks (only the mouse cursor keeps moving).

`clearPermissions` 同理先 `suspendInputInterceptors()` 再 `resetTCC()`（`:22-50`），
失败路径会把 fan helper 注册态恢复回去（`:107-116`）。**这条对我们直接有效**——
我们 Swift 端目前**没有任何 Accessibility/Screen Recording 调用**（本机 grep：`AXIsProcessTrusted` /
`CGPreflightScreenCaptureAccess` / `CGRequestScreenCaptureAccess` 在 `Sources/` 与 `app/` 零命中；
我们的 Computer Use 能力是靠外部授权完成的，见 `CHANGELOG.md:2894`）。
我们一旦要接「窗口预览 / 会话聚焦抬 app」，就会装上第一条 event tap，这条顺序就得跟着写下。

### 4.3 我们 Phase 2 的照做清单

按优先级排，全部有 Vorssaint 对应物：

1. **一张静态权限映射表**：`[Module: [Permission]]`，开关穷尽，注释写「为什么需要 / 为什么不需要」。
   **Phase 2 真实清单只有两条**：读 `~/.claude.json` / `~/.codex/` 或类似 → **零系统权限**
   （照 `.notchAgents: return []` 那条显式写出来）；若 Phase 2 要做「把 Codex 窗口带到前台」
   → Accessibility。
2. **一个 `activeUsers(permission:)` 纯函数**，输入注入（`isAvailable` / `boolFor` / `stringFor`），
   同时喂两个 UI：权限行上的「使用者：X、Y」与 `isEmpty` 时的「已授权但无人使用，可在系统设置撤销」。
   **我们只会有 1–2 条权限，但这套结构不变，且它是可单测的**（我们的自建 runner 正合适）。
3. **按需轮询**：侧边栏可见（= visibleSurfaceCount>0）时 2.5s，已授且守撤销 60s，否则不轮询。
   我们今天的微细条/侧边栏没有权限轮询需求，但 Phase 2 一旦接入就要按这个来，不要起一个常驻 1s timer。
4. **请求路径带一张不抢焦点的卡**（`PermissionGuideOverlay` 的形状：三步 + 自动等 + stale 兜底）。
   我们的 372×520 卡片装不下，灵动岛形态下尤其装不下——**但 sidebar 形态装得下**，
   且它是「让用户自己去系统设置再回来」这个流程里唯一不让人迷失的东西。
5. **「撤销前先停用依赖它的能力」**写成硬规则（`SelfUninstall` 那条）。
6. **每权限一段「If you say no」**：我们的产品口径是「不碰网络、不持有密钥」，
   所以更该写的是「若拒绝，哪个功能降级成什么样」。Phase 2 至少覆盖「拒绝 Accessibility = 不能抬窗口，
   清单与 token 统计不受影响」。

---

## 5. C. 与我们的对照

| 维度 | Vorssaint | AgentIsland | 它强在哪 | 我们强在哪 | 该不该跟进 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **形态** | 一个菜单栏图标 + 下拉面板 + 独立 Settings 窗口（30 page）+ Dynamic Island  notch 形态。菜单栏读数**是 NSStatusItem 的一项** | 灵动岛微细条 + 可展开卡（Swift `cardWidth` 330 / Tauri `372×520`）+ 规划中的侧边栏 | 形态三件套齐全且互相独立；**同一信息三种密度** | 微细条 6pt 宽是刻意选择；三级导航 | **只抄「读数可选、可排序、可隐藏」的机制**，不抄形态 |
| **菜单栏读数** | **`MenuBarMetric` 15 个 case**（cpu/gpu/memory/温度/电池/外设电池/网络/磁盘/功耗/风扇/已连设备，`App/MenuBarRenderer.swift:7-9`）。三层控制：**顺序**（`menuBarMetricOrder` 逗号串 + 缺省补位插入而非追加，`PanelLayout.swift:118-138` 同款算法，`MenuBarRenderer.swift:80-87`）+ **显隐**（每题一个 defaultsKey）+ **形态**（`values`/`bars`，`bars` 把 CPU/GPU/RAM/磁盘换成竖条，`MenuBarSpacingSupport.swift:22-34`）。三层过滤：**装了 + 显了 + 硬件有**（`enabled(in:)` 三重 `&&`：读数键真 + `feature.availabilityKey` 真 + `isAvailableOnCurrentHardware` 真，`MenuBarRenderer.swift:117-123`；所属 feature 被卸掉的 pinned 读数**键留着、不渲染也不采样**，`:93-96`）。revoke 一条读数 = 删一个 `NSStatusItem`，读数**连续 5 次**空渲染才收（`MenuBarSpacingSupport.swift:261-281`） | 微细条本身（呼吸灯 + 五态 + 活跃数）；Tauri `tray` 图标 + 菜单 | **「选哪些 / 什么顺序 / 数字还是条」三层正交，且硬件不存在的读数自动不渲染** | 微细条是**唯一形态**，不占多条 status item | **抄三条小的**：① 顺序持久化用「逗号串 + 缺省补位插入」（新增读数不能把它挤到末尾）；② 读数停更 N 拍才收，避免闪断；③ 一条读数消失 ≠ 整个图块消失 |
| **可插拔模块** | **73 feature / 8 group / 13 权限**，availability⊃enable⊃资源三层；一处穷尽 switch 定 page；卸载不删设置；硬件门三道校验；明确的「需重启才真正卸载」横幅 + 原地重启 | 侧边栏四块**写死在 UI**（监控 / Provider / 待办 / 高级设置）；`SettingsStore` 有约 40 个键但**全是 enable 语义** | 装卸是运行时开关 + 一处索引 + 不删设置 + 诚实横幅 | 规模小意味着我们不需要 hub；`SettingsStore` 已是集中键表 | **跟进（Phase 1 必做）**：加 `SidebarModule` + `moduleAvailable.<id>` 层 + 静态 page 表 + 卸载不删键。详见 §3.6 |
| **权限 UX** | 13 权限 × 静态映射 + 纯函数反查 + 「 granted 但无人用」卡 + 三步浮动引导 + 按需轮询 + 撤销前先停 tap | **零系统权限申请**（grep 证实）。Computer Use 靠外部授权 | 全套 | 我们不碰权限，就没有 TCC 债务 | **Phase 2 照抄清单**（§4.3）；**今天不用建** |
| **配置管理** | **导出/导入整套设置**（`SettingsBackupSupport`，438 行）：`exportKeys()` = 注册默认值 ∪ availability ∪ 未注册偏好 − `machineStateKeys`；导入时 `sanitizedSettings` 只放行本 build 认识且导出的键，「a tampered or future file can never write outside the allowed set」；`valueLooksRight` 逐个键校验类型；`omitsDynamicIslandSettings` 处理「旧备份没有 notch 键，导入不能把本机 notch 设置擦掉」 | 无设置导入导出 | **白名单式导出 + 类型校验 + 新增 key 的向后兼容** | Phase 2 的 CC Switch 档位是我们独有 | **抄机制不抄范围**：Phase 2 档位文件导出时显式列白名单 + 版本号信封 |
| **跨平台** | 仅 macOS arm64（`build.sh:65`） | Swift/macOS 本体 + Rust/Tauri `app/`（构建不通） | 单品做到极深 | 我们有 `app/` 的同构壳 | 无关 |
| **测试纪律** | 131 测试文件 55,836 行；`build.sh --test` 自建 runner + `--selftest` 运行期自检；CI 两个 runner（macos-15 Swift 6.0.3 兼容 + macos-26） | 548 个 `test(...)` 自建 runner；Rust 0 测试；**无 CI** | 「新行为有测试」进了 CONTRIBUTING；**CI 有 Swift 版本兼容矩阵双跑** | release.sh 把扫描→commit→tag→release 钉死 | **跟进一条**：Swift 版本兼容 CI（我们 `Package.swift` 是 5.9，值得知道 6.x 会不会炸） |
| **隐私口径** | PRIVACY.md **列了 9 类网络连接**，逐类写「只在你做什么时才发生 + 发什么 + 对方 IP 怎么处理」；并声明「there are no hidden beacons or background uploads」 | `CONTEXT.md:196` 口径「不碰网络、不转发流量、不持有密钥」（远程外发本身即主动出网，见 codenotch 调研 §5.1） | **逐类可核对**，不只是口号 | 我们的口径更严（我们连更新检查都没有） | **抄文体**：README 的隐私段落写成逐类清单 |
| **许可** | **GPL-3.0 + TRADEMARKS.md 双轨** | MIT | 商标不随源码走，fork 需自有身份 | MIT 允许我们抄做法不抄代码 | 见 §6.3 |

### 5.1 menu bar readouts vs 我们的微细条：它答的是「选读数」，我们答的是「agent 在干什么」

- 它的 15 个读数全是**系统指标**，无一是 agent 相关；AI Agents 是 notch 里的一个 section
  （`notchAgents` 一个 `AppFeature` case，权限空，见 §7）。
- **形态上它是「多条独立 status item」，我们是「一条 6pt 微细条」**。它一个读数一个 item
  （`metricStatusItems: [String: NSStatusItem]`，`StatusItemController.swift:23`），
  可 `autosaveName` 记住位置（`:688-690`），⌘拖动可重排——**这就是 menu bar 读数的原生自由度**。
  我们的微细条不做这个，也不该做（它不是菜单栏项的竞品）。
- **它有一条我们没有的能力：读数的顺序、显隐、形态三层解耦**。我们的微细条内容是固定的一份。
  若 Phase 3 我们想「微细条显示 token 速率而不是活跃数」，**该抄的是它的三层分离结构**
  （顺序串 / 显隐键 / 形态枚举），不是它的条目集。

### 5.2 「一个入口怎么承载多个可插拔模块」——它的答案对我们的可用性

它的答案是**「hub 是唯一的安装面，page 是唯一的配置面，两者用一张穷尽 switch 连起来」**。
四块模块（哪怕将来加第五块）在我们这里的映射：

```text
Vorssaint                       我们
AppFeature (73)            →    SidebarModule (~5)
availabilityKey            →    moduleAvailable.<id>
FeatureGroup (8)           →    不需要（我们四块本身就是四个顶级块）
settingsDestination (29)   →    module → 已有路由（分析页/详情页/Provider 页/待办页）
FeatureVisibilitySupport   →    侧边栏 nav 的可见性判据（模块卸了，导航项消失）
FeatureHubSettings         →    「高级设置 → 模块」页
SettingsBackupSupport      →    Phase 2 档位导出的白名单机制
```

**关键差异：它 73 行是因为它真的有 73 个功能；我们四块是因为我们只有四块。
该抄的是机制（键分层 + 一张静态表 + 卸载不删设置），不是规模。**

---

## 6. D. 可抄的三件 / 不可抄的两件

### 6.1 可抄（做法与结构，零源码）

1. **availability ⊃ enable ⊃ 资源三层 + 卸载不删键**（`FeatureCatalog.swift:6-14`、
   `:404-411`；`FeatureRuntime.swift:100-123`；`Defaults.swift:1743`）。
   **Phase 1 做，成本一行枚举 + 一个键函数 + 一张 bindings 表。**
2. **一个 `activeUsers(permission:)` 纯函数同时喂「谁在用」与「没人用了可撤销」**
   （`FeatureCatalog.swift:414-437`、`FeatureHubSettings.swift:766-770`、`:788-792`）。
   **Phase 2 做；今天 0 条权限，纯函数仍可先写好并单测。**
3. **穷尽 switch 做模块→配置面索引，让新增模块逼一次显式选择**
   （`FeatureVisibilitySupport.swift:230-231`、`:237-347`；实测 73→29）。
   **Phase 1 做，防的是「加了模块忘了接入口」这类静默遗漏。**

另外三条小的、同属「做法」：**顺序持久化的「逗号串 + 缺省补位插入」**
（`PanelLayout.swift:118-138`）、**读数停更 N 拍才收**（`MenuBarSpacingSupport.swift:261-281`）、
**设置导出的白名单 + 类型校验 + 版本信封**（`SettingsBackupSupport.swift:18-24`、`:99-170`、`:197-204`）。

### 6.2 不可抄（源码）

**GPL-3.0 是强传染，`Sources/` 一行都不能进我们的仓。**

- 事实：`LICENSE` 是 GPL-3.0（35149 B，未逐行读全文）；`Package.swift:2` 每个文件头都带
  `// SPDX-License-Identifier: GPL-3.0-or-later`；`TRADEMARKS.md:1-14` 明写
  「That license covers copyright in the source code only」，**商标/图标/bundle id/签名身份
  另行覆盖**。
- 我们**没有 LICENSE 文件**（`find . -maxdepth 2 -iname "LICENSE*"` 零命中，README/CONTEXT
  也零命中 MIT 字样）——**这条本身是个待办，不是既有既定事实**。任务书说「我们是 MIT」，
  但本仓当前看不到任何 license 载明。**这条要用户拍板（§8 问题 3）。**
- 结论（不论我们最终 license 是什么）：**不抄任何 `.swift` 源码，不 vendored 它的文件，
  不做「改写注释后照搬」。抄的只能是算法思想、数据形状、命名与流程**，且要在我们自己的文件里
  用自己的话重写、用自己的注释解释。**这一条写死在本文档，也建议写进 `CONTEXT.md` 或 ADR。**

### 6.3 不可抄（形态与规模）

1. **不抄它的 hub 规模与分组。** 73 feature / 8 group 是它自己的产品形态，我们四块不需要
   preset 卡、份额条、分组 DisclosureGroup 这些为 73 行服务的东西。
2. **不抄它的菜单栏多 status item 方案。** 我们的微细条不是菜单栏项的替代品
   （见 `docs/workbench/04-plan.md:49-62` 的分工：island 形态装不下的东西交给 sidebar）。
3. **不抄 Dynamic Island / notch 那一整套 38 文件的子系统**（`Services/Notch/` 38 文件 +
   `UI/Notch/` 31 文件，合计约 50 万字节）——那是它的主形态，不是我们的。

---

## 7. 隐私口径的可验证性（README 声明 vs 源码）

**方法**：全量 grep `https://` 字面量 + 逐个 `URLSession` 使用点。Sources 下 `URLSession` 只出现在
**13 个文件**（`AgentPriceSource` / `AppUpdateFeedLoader` / `AppUpdatesService` / `FeedbackService` /
`HomebrewManager` / `SpeedTest` / `NotchLyricsService` / `ScreenshotShareService` /
`RecordingShareService` / `RadialMenuSupport` / `SelfUninstall` / `UpdateInstallerSupport` /
`UpdateService` / `UpdateShowcaseMedia`——14 个，含 `SelfUninstall` 一处只在注释里）。

**逐条对 `docs/PRIVACY.md` 的 9 类声明**：

| # | 声明 | 源码证据 | 对得上？ |
| :--- | :--- | :--- | :--- |
| 1 | 更新检查 `api.github.com` | `Update/UpdateService.swift:122-130`：`https://api.github.com/repos/<repo>/releases/latest`（或 `?per_page=10`）；`User-Agent: Vorssaint/<version>`；`cachePolicy = .reloadIgnoringLocalCacheData` | ✅ |
| 2 | 测速 `speed.cloudflare.com` | `Metrics/SpeedTest.swift:36` host 常量；`:109` `/__down?bytes=0`、`:154` `/__down?bytes=N`、`:156` `POST /__up` | ✅ |
| 3 | Homebrew actions | `Services/Homebrew/HomebrewManager.swift`（本地 `brew` 子进程）+ `formulae.brew.sh` 字面量；PRIVACY.md:52-54 明说会连 Homebrew/GitHub/厂商 host | ✅（子进程网络，源码内不可逐条列） |
| 4 | App updates App Store 源 | `uclient-api.itunes.apple.com/WebObjects/MZStorePlatform.woa/wa/lookup`、`itunes.apple.com/lookup` 字面量；Online 源读 app 自带 URL + `formulae.brew.sh` 全量目录 | ✅ |
| 5 | 临时截图链接 `screenshots.vorssaint.com` | `QuickTools/ScreenshotSharingSupport.swift:42`：`static let productionEndpoint = URL(string: "https://screenshots.vorssaint.com")!`；上传 `v1/screenshots?expiresIn=`（`:70-79`）；PNG magic number 校验 + 25 MB 上限（`ScreenshotShareService.swift:79-88`） | ✅ |
| 6 | 临时录制链接（同一服务） | `Recorder/RecordingShareService.swift` + `RecordingSharingSupport.swift:65-74`，`v1/recordings` | ✅ |
| 7 | 反馈 `screenshots.vorssaint.com/v1/feedback` | `Services/Feedback/FeedbackService.swift:61`；`:66-73` ephemeral session + 禁 cookie + 20/30s 超时 + 10–2000 字符 + 8 KB body 上限 | ✅ |
| 8 | 在线歌词 `lrclib.net` | `Services/Notch/NotchLyricsService.swift:121`：`URLComponents(string: "https://lrclib.net/api/get")` | ✅ |
| 9 | AI 价格表 `raw.githubusercontent.com` | `Services/AgentUsage/AgentPriceSource.swift:11`：`.../vorssaint/vorssaint-utils/main/Resources/agent-prices.json`；`:12-13` 24h 刷新 / 失败 6h 重试；PRIVACY.md:73 明说可关 | ✅ |

**结论：声明与源码一致，无隐藏出网点。**

三条值得记的加固细节：
- 所有出网 session 都是 `URLSessionConfiguration.ephemeral` + `httpShouldSetCookies = false`
  （`FeedbackService.swift:66-73`、`AppUpdateFeedLoader.swift:23-29`、`ScreenshotShareService.swift:29-37`）。
- **分享链接的删除 token 有形状校验**：`id` 必须 `^[A-Za-z0-9_-]{32}$`、`deleteToken` 必须
  `^[A-Za-z0-9_-]{43}$`、`viewPath` 必须等于 `/s/<id>`、过期时间必须在
  (now, now+24h+5min] 内，否则不入库（`ScreenshotSharingSupport.swift:81-92`）——
  **响应体被当不可信输入校验，这是好纪律。**
- `RadialMenuSupport.swift:1050` 的 `FaviconDownload` 是第 10 类出网（给径向菜单的网址抓 favicon），
  **`docs/PRIVACY.md` 的 9 类清单里没有它**（`grep favicon docs/PRIVACY.md` 零命中）。
  它是一个「只在用户在设置里显式点击时触发」的按需抓取，有 2 MB 传输上限与 64 KB 存储上限
  （`RadialMenuSupport.swift:947-955`），**风险低，但这是一个声明与实现的缝隙**——
  它去抓的是**用户自己输入的任意 URL 的 host**。按本文档的纪律记下：**这是它的问题，不是我们的**，
  但它提醒我们：我们 Phase 2 若要做「给档位卡抓提供商 logo」，必须先把这类出网写进隐私清单。

**另两条它自己写在 PRIVACY.md 的诚实边界**（值得学的文体）：
- Online 更新源：「The server receives your public IP address and the requested URL,
  **which can reveal which app is being checked**.」（`PRIVACY.md:56-57`）
- 分享链接：「Anyone with the link can view, download, save or redistribute the image,
  and **active links are available to the service operator for abuse moderation**.」（`:65-67`）

---

## 8. E. 需用户拍板的

### 8.1 三条我(agent)建议直接采纳、不必投票

1. **GPL 红线写死**：不抄 `vorssaint-utils` 任何源码；做法与结构可抄，且在我们自己的文件里重写。
   建议顺手写进 `CONTEXT.md` 或一条 ADR。
2. **Phase 1 的 availability 层**照 §3.6 改造点做（这是本文档最确定的一条收益）。
3. **`activeUsers(permission:)` 纯函数**在 Phase 2 一开始就建，含单测——即便我们只有 0–1 条权限。

### 8.2 需要你(用户)拍板的三条

1. **侧边栏要不要为「模块可插拔」付一次结构成本？**
   Phase 1 现在做（加 `SidebarModule` + `moduleAvailable.<id>` + bindings 表）
   还是等真的出现第五个模块再做？
   （推荐：**Phase 1 顺手做**。四块时建，成本是「一个枚举 + 一个键函数 + 一张五个元素的表」；
   等第五块再建，就要回头改三处已经写死的 UI。）

2. **Phase 2 的「配置文件授权」要不要照 Vorssaint 建一条显式的「零权限」声明？**
   即：`Phase2Permissions` 表里显式写一条「读 agent 配置文件 = 不需要任何系统权限」，
   并在 UI 上显示「本功能不请求任何权限」。
   （推荐：**建**。这条比 Vorssaint 的 `.notchAgents: return []` 更有价值——
   我们的产品口径是不碰网络不持密钥，把「零权限」和「零网络」一起显示出来是差异化。
   但它要求 Phase 2 的配置文件读路径**真的**零权限零网络，别在 CC Switch 解析 key 时联网。
   **这条需先与 codenotch 调研 §5.1 的口径结论对齐**：那篇提出的「借 CLI 登录态 vs 自己持钥」
   边界问题在这里同样适用。）

3. **本仓的 license 到底是什么，要不要落一个 `LICENSE` 文件？**
   任务书说「我们是 MIT」，但本仓当前**没有任何 LICENSE 文件**，README / CONTEXT / AGENTS
   也零命中「MIT」字样（唯一命中是 AGENTS.md 里描述 mattpocock/skills 的上游 license）。
   **在 license 未落定前，「GPL 不能抄」这条结论不变**；但反过来，**MIT 本身不阻止我们抄 GPL
   源码——真正阻止我们的是「我们的分发物会被 GPL 传染」**。这一条需要先定 license 再谈复用边界。
   （推荐：**先补一个 `LICENSE` 文件把 MIT 落定**，然后按 MIT 处理：GPL 源码仍然不抄，
   但理由从「我们 license 未知」变成「GPL 会传染我们的分发物」。）

---

## 9. 未核实清单

1. **`docs/PRIVACY.md` 与 README 的「网络只被你能看见的东西碰」在 9 类之外的全部出网已核**，
   但 `Sources/` 下**是否有非 `https://` 字面量的 host**（配置驱动、字符串拼接、`NWConnection`）
   只核到「grep `NWConnection` 零命中」——**没有逐一读 14 个 `URLSession` 文件的全部构造点**。
   `RadialMenuSupport.swift:1050` 的 favicon fetcher 是**唯一发现的第 10 类**（README/PRIVACY 未列）。
2. **`AppUpdatesService`（37216 B）、`AppUpdatesSupport`（31826 B）、`HomebrewManager`（35798 B）
   未逐行读**：App Store / Online / Homebrew 三源的**实际**开关逻辑与 host 列表是从字面量 + PRIVACY.md
   反推的，未逐行确认。
3. **`NotchService`（121470 B）、`NotchSupport`（79789 B）、`NotchWindowHost`（76917 B）未读**：
   notch/灵动岛形态的窗口管理、动画、触摸手势**未核实**。本文对它的动效纪律**不下任何结论**。
4. **`MenuPanelView`（131854 B）未读**：面板的滚动/布局/空态细节只从 `PanelLayout` 与 README 推出。
5. **`CommandBarService`（154004 B）、`ScreenshotSupport`（117158 B）等大文件未读**：与本文四个问题无关，跳过。
6. **73 个 `AppFeature` 的 `permissions` 表已整表读过**（`FeatureCatalog.swift:315-373`），
   但 `activeFeatures` 的动态 switch 只有 15 条 `case` 被逐条列出（`:424-476`），
   **`default: return true` 覆盖的其余组合未逐一枚举其业务含义**。
7. **`SettingsBackupSupport.swift` 只读到 250/438 行**：`portableNotchDisplay` /
   `portableMediaSettings` / `portableMouseExceptions` / `restoredExceptionList` 的细节读到，
   但 `valueLooksRight` 的实现、导入写回 `UserDefaults` 的调用点（`AdvancedSettings.swift` 附近）
   **未读**。
8. **它的 `--selftest` 在本机未运行**（未尝试构建，本文档纯文档任务、不跑 build）。
   它能否在无 Xcode 的机器上真正构建，**只从 `build.sh` 与 `ci.yml` 的 `run:` 行与 CONTRIBUTING
   的自述推定，未实测**。
9. **548 个 open issues 的标题与内容未读**（只从 API 拿计数）。它的已知限制清单因此**只有 README /
   PRIVACY / CONTRIBUTING 自陈那几条**（换签名会孤立权限、Full Disk Access 无预检 API、
   App Management 无预检 API、`tccutil` 是唯一的重新提问手段、AI 价格表可关）。
10. **`CHANGELOG.md`（198911 B）未读**：版本节奏（各版本日期与频率）未统计。当前版本直读
    `Resources/Info.plist` 的 `CFBundleShortVersionString` = **3.4.0-beta.5**（同时印证 release.yml
    的 showcase 特判写的 3.1.4 是过去版本）。Homebrew cask 与 release 数**未拉取**。
11. **本仓的 license 状态未核实到底**（§8.2 问题 3）：`LICENSE` 文件不存在是事实（`find` 证实），
    但「项目意图是 MIT」只是任务书口径，非仓内证据。
12. **我们的 `SettingsStore.swift` 只读了结构（490 行的键清单与 `SettingBool`）**，
    未逐行读写入/迁移逻辑；Phase 1 改造点的可行性判断基于「键全部集中在 `SettingKey`」这一事实
    （`SettingsStore.swift:5-7` 注释亦自称「键名字面量不得散落在调用方」）。

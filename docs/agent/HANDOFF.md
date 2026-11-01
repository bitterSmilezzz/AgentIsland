# 交接说明（Hand-off）

> **给下一个 Agent**：这份文件回答「我该从哪里开始、当前处于什么状态、有哪些坑别踩」。
> 与它配套的还有：[`AGENTS.md`](../../AGENTS.md)（工作约定，Agent 无关）、
> [`docs/README.md`](../README.md)（文档地图）、[`需求状态.md`](../需求状态.md)（做到哪了）。

**交接时点**：2026-11-01
**本轮起始基线**：`main` at `4021fd3`（v0.0.426）；本轮改造为 TASK-05 本地产出正文持久化。
**本轮开始时最新发布**：`v0.0.426`（DMG / ZIP / updater 包 / `latest.json`）。
**本轮完成发布**：`v0.0.427`，提交 `b861578bc37f979f132ac2d418a258d5e831ca9c`，tag 与 GitHub Release 已创建；详见第 21 节。
**基线状态**：最终 `verify-framework.sh` exit 0；本机 Windows 整 crate 检查因缺少 ring 所需 Windows CRT/SDK `assert.h` 标记未验证。发布源码 CI、包身份检查与隔离 VM 原生门禁均通过。

> 🟢 **DMG 分发 + 自动更新已上线**（第 8 节），自动更新界面层见第 9、12、13 节。
> v0.0.427 完成本地产出正文第五片；TASK-05 仍缺产出版本演进、比较与恢复的任务页接线。NOT-06 的 v0.0.416 真实升级仍未验，旧版 `agentisland-update-e2e` VM 保持原样。

> ⚠️ 本文件由**多个会话接力**：第 1–6 节是 2026-10-17 晚（NOT-05 交付与三轮复审），
> 第 7 节是 CI 身份规则修复（**不是本次交接者写的**，但记录的是真实修复，保留不删），
> 第 8 节是 DMG 与自动更新，第 9 节是自动更新界面层，第 10 节是 Windows 修复复核，
> 第 11 节是槽位租约。

---

## 1. 五分钟上手

```sh
git clone https://github.com/bitterSmilezzz/AgentIsland.git && cd AgentIsland
npm i playwright                                    # 浏览器 UI 回归需要（缺则 release.sh 明确失败）
bash scripts/verify-framework.sh                    # 确认基线是绿的
```

读的顺序：`AGENTS.md` → `CONTEXT.md`（领域口径）→ `docs/需求.md`（要做什么）
→ `docs/需求状态.md`（**做到哪了 + 未修 bug**）→ `docs/README.md`（要查具体文档时）。

---

## 2. 当前状态

> 本节保留 2026-10-17 的历史快照；截至 2026-11-01 的当前状态与下一步见第 17 节。

### 已完成（本轮相关）

| 需求 | 状态 | 证据 |
| :--- | :--- | :--- |
| **NOT-05** 外发交付状态与故障升级 | **已完成** | 8 项需求逐条有测试；真实二进制端到端 `not05.remotedelivery`；三轮独立复审「能定案」 |

**本轮（2026-10-17）交付**：
- NOT-05 实现 + 三轮独立复审整改（详见 [`code-review/2026-10-17-not05-review-remediation.md`](../code-review/2026-10-17-not05-review-remediation.md)，585 行）
- **外发链路的真实二进制端到端验收**（此前只有注入假传输的单测）
- 文档分门别类（`docs/{spec,testing,ops}/`）+ 需求状态/bug 登记自动化
- `v0.0.324` 发布（含入队延迟、入站账本、自动化框架、NOT-05）

### 进行中

无。上一轮工作已全部提交推送，工作区干净。

### 下一步建议（按价值排序）

1. **`docs/需求状态.md` 里 19 个「部分完成」需求**——每个的 `coverage_note` 都列了缺口，
   挑一个按「要做什么 → 怎么验 → 门禁」走完，然后把 `status` 改成 `done`。
2. **3 条未修 bug**（同文档「未修 bug 登记」表）——`BUG-001` 语义未明需先查清，
   `BUG-002`（release.sh 断点续跑）是明确的工程改进。
3. `docs/需求.md` 末尾的 `https://t3.codes/` 是「后续分析项」，只核实过官方定位。

---

## 3. 环境前置（缺了就发不了版）

| 依赖 | 现状 | 缺了会怎样 |
| :--- | :--- | :--- |
| `playwright` | 已装（`npm i playwright`） | `release.sh` **明确失败**，不跳过 |
| 隔离 VM | `tart` 有 `agentisland-iso`（本轮已启动，来宾 Aqua/SSH 前置通过） | `release.sh` 直接 `die`「未运行原生门禁，不发布」 |
| VM 来宾账号 | `NATIVE_TEST_USER=admin`（历史命令里找到的，非文档固定） | 同上 |
| `gh` | 已认证（bitterSmilezzz） | 建不了 Release |

发版命令（**必须显式文件清单**，脚本拒绝自动收取工作区改动）：

```sh
RELEASE_FILES_FILE=<清单文件> NATIVE_TEST_VM=agentisland-iso NATIVE_TEST_USER=admin \
  bash scripts/release.sh <X.Y.Z> "<CHANGELOG 首条那句话>"
```

清单生成（**不要**用 `unicode_escape` 还原中文路径，会坏）：

```sh
git status --porcelain -z --untracked-files=all | python3 -c "
import sys
data=sys.stdin.buffer.read().decode('utf-8','surrogateescape')
entries=[e for e in data.split('\0') if e]
paths=[]; i=0
while i < len(entries):
    e=entries[i]; status=e[:2]; path=e[3:]
    if status[0] in 'RC': i+=1
    paths.append(path); i+=1
open('.scratch/release-files.txt','w',encoding='utf-8').write('\n'.join(sorted(set(paths)))+'\n')
"
```

---

## 4. 本轮踩过的坑（**别重复**）

完整版见 [`agent/session-2026-10-17-problems.md`](session-2026-10-17-problems.md)。最该记住的：

1. **「断言存在」≠「断言有效」。** 本轮三处「已通过」的断言实测抓不住变异
   （阈值测试没断言边界、渠道独立测试用了另一个实例、断言的 `Accepted` 来自脚本耗尽后的兜底默认值）。
   **每次修复都要配变异注入，并核对 `Compiling` 行**。
2. **`cp -c` 恢复文件会保留 mtime**，cargo 跳过重编 → 变异验证得到**假阴性**。用 `touch` 强制重编。
3. **文案/键名的「唯一来源」要连成一条链路。** 夹具是唯一来源，但新增链路极易再抄一份
   （本轮 N2 → P1 同一形态重复两次）。新链路一律从夹具读。
4. **渲染断言要带可见性判据。** 故障条曾渲染在默认收起的 `<details>` 里，
   而 `textContent` 断言对不可见完全无感（实测整块 `display:none` 照样通过）。
5. **状态机两侧都要断言。** 只测「故障条出现」不测「清除后消失」，else 分支写错会让陈旧故障永久挂着。
6. **非 ASCII 路径不能用 git 默认输出。** `git diff --cached --name-only` 会给中文路径加引号并八进制转义，
   下游 `test -e` 判定不存在 → 扫描器报「什么都没扫」并**恒拒提交**。
   一律用 `-z`（NUL），且**不要用 awk 的 `RS="\0"`**（macOS awk 支持不完整，实测 4 条只输出 1 条）。
7. **改扫描器的输出约定会连带破坏测试 stub。** 本轮把 `file_list` 改成 `-z` 后，
   `test-scan-secrets.sh` 里模拟 git 的 stub 仍在输出换行分隔，导致并发用例红。stub 要同步改。
8. **活仓库上跑全量门禁，`source.integrity` 必然红**（指纹覆盖全部未忽略内容）。
   要一份可定案的全绿，必须在编辑停止后跑。**运行期间不碰任何文件**。
9. **发版闸门按设计工作**：脱敏复扫在 **push 之前**，拦下时提交已建、未推送，可修。
   但 `release.sh` **不支持断点续跑**（见 BUG-002），需手工接续 打包→tag→push→release。
10. **`AskUserQuestion` 弹窗超时 = 未授权。** 发布类动作不能拿推荐项当用户的选择。

---

## 5. 硬边界（不要碰）

- **用户真实状态只读**：不得向运行中的应用投递探测/合成事件。验证走隔离实例
  （`--background-test`/`--ui-smoke`）或临时 fixture 探针。真实账本只读。
  确需写入时：停应用 → 备份整份 → 按**能逐条指向的精确 id 集合**删 → 重启 → 逐条 diff 复验。
  **禁止用时间窗口当判据**（会随后续投递变宽，误删真实数据）。
- **脱敏红线**：commit 前必过 `scripts/scan-secrets.sh`；`cred_prefix`/`private_key` 两类**绝对零命中**，
  不许进 baseline；豁免用行内 `nosec: <理由>`。
- **不整文件回滚共享文件**（其它会话可能有改动）；只用显式路径暂存本任务的文件。
- **发版必须走 `release.sh`**（顺序钉死：扫描先于 commit；tag 与 release 是必须发生的步骤）。

---

## 6. 我(agent)留下的、你可能想改的

- `docs/需求状态.md` 是**生成物**，改 `registry.json` 后要跑
  `render-matrix.py --write` **和** `render-status.py --write`（漏跑任一个门禁报漂移）。
- 需求状态机只有三态（未开始/部分完成/已完成），靠 `coverage_note` 前缀判定。
  若觉得不够用（如需要「阻塞」态），改 `render-status.py` 的 `validate()` 与状态词表。
- `BUG-003` 是「机制问题」而非已修 bug：探针的日志短路靠每个探针自己登记 `ACTIVE` 标记，
  没有统一入口——同类遗漏还会再发生。想根治可以做一个统一的 headless 模式判定。
- `requirements[].gates` 与矩阵「层」列不一致（`BUG-001`），且该字段**全仓无消费者**、
  语义未明。我(agent)查过后**先改又回退**，没有擅自统一——要动它请先写清语义。

---

## 7. 2026-10-18 追加：CI 身份规则修复（本轮）

- **CI 的 `$USER` 是通用账号名**：macOS runner 上 `USER=runner`。`scan-secrets.sh` 的
  `username` 规则曾把它当敏感词，命中源码里正常的英文词 **287 条**，macOS job 卡在
  「脱敏扫描（工作区）」，后面的 ci profile、构建与 package 验收**一条都没跑**。
  已改为 `SCAN_USER` 优先、通用名**显式停用**两条身份规则；要恢复 CI 判别力就配仓库变量
  `AGENTISLAND_SCAN_USER`（开发者本机用户名）。细节与证据见
  [`docs/testing/测试自动化交付.md`](../testing/测试自动化交付.md) 第二十四轮。
- **Windows job 仍红**：编译已过，但 `cargo test` 两次分别是 23 / 25 个失败
  （952 passed / 8 ignored 量级），已登记 `BUG-004`。它仍是 `continue-on-error`，
  不挡 macOS 门禁；总 run 在 macOS 通过时为 success。
- **推送后实测**（run `39056405805`，SHA `12f56b6`）：macOS job **15/15 步全绿**
  （扫描、ci profile、Tauri CLI、构建签名、package），约 29 分钟。途中还发现 runner 的
  CLT 27 SDK 与默认 Xcode 26.6 的 ld 错配会让任何链接失败（`actions/runner-images#14940`），
  workflow 已改用默认 Xcode 自带 SDK（`xcrun --sdk macosx --show-sdk-path`），
  本机 `build-app.sh` 的最新一代 SDK 选择逻辑未动。

---

## 8. 2026-10-18：DMG 分发 + 自动更新基础设施（**已随 v0.0.325 发布**）

> ✅ 改动随 `118b84d` 提交，并已由 `release.sh` 正式发布：
> 发版提交 `e79f056`、tag `v0.0.325`、GitHub Release 附件
> `AgentIsland-0.0.325.dmg`（6,482,244 B）/ `.zip` / `.app.tar.gz` / `.sig` / `latest.json`。

### 已实测可用（线上）

| 项 | 证据 |
| :--- | :--- |
| DMG 已上线 | `releases/download/v0.0.325/AgentIsland-0.0.325.dmg` HTTP 200，字节数与本地构建一致（sha256 `d4d876ed…b0e5`） |
| DMG 内 app 签名完整 | 构建日志 `✓ DMG 内 app 签名完整`；残缺即构建失败 |
| updater 更新包已签名 | `.app.tar.gz` + `.sig`（412 B），真实 minisign 签名 |
| 更新清单端到端 | `releases/latest/download/latest.json` HTTP 200：version 0.0.325 / `darwin-aarch64` / signature / 下载 URL |
| 站点直链 | `site/` 四个按钮都是 DMG 直链，点击不经过中间页 |
| 全量门禁 | `bash scripts/verify-framework.sh` → **ALL VERIFIED**（rc=0） |

### 发版命令（本轮实际用过的那条）

```sh
# 1) 生成显式文件清单（不要用 unicode_escape 还原中文路径，会坏）
git status --porcelain -z --untracked-files=all | python3 -c "
import sys
data=sys.stdin.buffer.read().decode('utf-8','surrogateescape')
e=[x for x in data.split('\0') if x]; p=[]; i=0
while i<len(e):
    it=e[i]; st=it[:2]; path=it[3:]
    if st[0] in 'RC': i+=1
    p.append(path); i+=1
open('.scratch/release-files.txt','w',encoding='utf-8').write('\n'.join(sorted(set(p)))+'\n')
"

# 2) 发版（私钥路径必须给，否则只出 DMG 不出更新包）
RELEASE_FILES_FILE=.scratch/release-files.txt \
NATIVE_TEST_VM=agentisland-iso NATIVE_TEST_USER=admin \
TAURI_SIGNING_PRIVATE_KEY_PATH=~/.local/share/agentisland-updater/updater.key \
  bash scripts/release.sh <X.Y.Z> "<CHANGELOG 首条那句话>"
```

VM 前置：`tart run agentisland-iso --no-graphics`（脚本发现 VM 未运行会直接 `die`「未运行原生门禁，不发布」）。

### ⚠️ `release.sh` 要求「有未提交改动」（BUG-005，未修）

`release.sh` 的设计是**把本轮改动一起提交**（清单 → 暂存 → commit → tag），
所以它对「工作区干净」是**不成立**的：清单由 `git status` 生成，干净时为空的清单第一步就退出；
手填已提交的文件则走到「提交」一步因无 staged 改动而 die。

推论：**发版那一轮必须先有未提交的改动**（版本号 / CHANGELOG / 站点直链就是这批改动）。
v0.0.325 与 v0.0.326 都是这么发的。已登记为 BUG-005，是否给脚本加「已提交树」模式尚未决定。

### 站点 DMG 直链的维护约定

`site/index.html` 的 4 个下载按钮指向
`releases/download/<tag>/AgentIsland-<版本>.dmg`，**没有 `latest` 这种稳定写法**
（GitHub 不提供 `latest.dmg`）。代价是每次发版都要同步这一行版本号。
这是有意取舍：换来用户点击后**不经过任何中间页面**直接下载。

### ⚠️ updater 私钥（不可逆的那部分）

- 位置 `~/.local/share/agentisland-updater/updater.key`，权限 **600**，**仓库外**，内容从未打印或提交。
- **这把钥匙丢了 = 已装用户永远收不到更新**（官方原话：lose the key ⇒ cannot publish new updates）。
  **建议在别处再备份一份**（离线介质或另一台机器的密码管理器）。
- 公钥在 `app/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，可以公开。
- 无口令生成（`signer generate --ci`），所以 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 必须是**空串**。
- **只认内容不认路径**：传 `TAURI_SIGNING_PRIVATE_KEY_PATH` 给 tauri 会去找钥匙串密码
  （实测报 `incorrect updater private key password`）。`build-app.sh` 已处理：读进内容再 export。

### 本轮修掉的 4 个真实缺陷（下个 agent 别改回去）

1. **updater 的 `default` 特征引入 `ring`** —— 它带 `rustls-tls`，而 `ring` 要编译 C 代码；
   本机做 Windows 交叉检查没有 Windows SDK 头文件，直接断在 `fatal error: 'assert.h' file not found`，
   **让 `windows crate` 门禁从 passed 变 failed**。已改为
   `default-features = false, features = ["native-tls", "zip"]`，`ring` 彻底移出依赖树。
   **别把它改回 `default`**。
2. **密钥必须在主构建之前注入** —— `createUpdaterArtifacts: true` 对**每一次** `cargo tauri build` 都生效，
   不只是打 DMG 那次。原先只在 DMG 段注入，主构建先失败报
   `A public key has been found, but no private key`。
3. **`--check` 区分「无产物」与「漂移」** —— 两者曾被混为一谈，导致任何没跑过带签名构建的干净克隆上
   `verify-framework.sh` 恒红（`test-framework-verifier.py` 20 项里 5 项红）。
   现在无产物安静通过，`--write` 仍失败（没有产物却写出清单 = 伪造发布信息）。
4. **测试桩件要复制真实生成器** —— `test-framework-verifier.py` 用 `.touch()` 造空脚本，
   python 跑空文件返回 exit 2。生成器类脚本必须 `shutil.copy2`。

### 我(agent)这次犯的两个错（写下来免得重犯）

- **为了让测试过而放松生产校验**：我一度把 `[[ -d "$app" ]]` 从 DMG 自校验里去掉。
  发现后立刻回退——**生产代码不为迁就测试而放松**。正确解法是让桩件造出真实形状。
- **错误结论被 `&&` 掩盖**：曾输出「✓ DMG 已修好」，实际那次 `codesign` 是**失败**的
  （DMG 只读、挂载后无法重签名），但前一条命令 rc=0 导致 `&& echo ✓` 执行。
  **校验必须看 `codesign --verify` 的判据，不能看前一条命令的退出码。**

### 仍未做（如实）

- **自动更新的界面层已在 v0.0.326 补上**（见下一节）；下面这些仍然没有：
- `release.sh` 里更新包附件那段，v0.0.325 是它**第一次真跑**——资产名与「缺件即失败」都已按实跑结果修正。
- **只 arm64**（Apple Silicon）；Intel Mac 需要 universal binary。
- 未公证，用户首次打开仍需右键 → 打开（要免这步需 Apple Developer Program）。
- 自动更新的**端到端验收**（客户端真去 GitHub 拉、验签、替换）仍未做。
  **签名不匹配的负例（错误公钥必须被拒）也没做**——这是安全判据，不是可选项。

---

## 9. 2026-10-18 追加：自动更新界面层（v0.0.326）

上一节列的「下一步建议 2」已完成：设置页有完整的
**检查更新 → 看到新版本与说明 → 下载并安装（带进度）→ 立即重启**。

- Rust 侧 `app/src-tauri/src/update.rs`：`update_current_version` / `update_check` /
  `update_install` / `update_restart` 四条命令，进度走 `update://progress` 事件。
- 界面侧 `app/ui/js/update-state.js`（纯状态机，node 可直接测）+
  `update-panel.js`（渲染与绑定），挂进 `views.js` 的 `pageSettings()`。
- 三条硬规则有门禁兜底：只有点击才出网、只有 available 才可安装、安装完**不**自动重启。
- 门禁：`not06.update-state`（变异 3/3）、`not06.update-panel`（真 DOM + 全长样式表，变异 2/2），
  都进 fast/ci/automation/release 四档；`browser-check.mjs` 允许名单已登记 `test-update-panel.mjs`。

### 这一节踩过的坑（下个 agent 别重犯）

1. **`createUpdateController` 解构的是单个对象**：写成 `(依赖, render)` 会把 render 当第二个
   位置参数丢掉——面板挂载成功、状态也对，只是**永远不重绘 DOM**。非常难查。
2. **Tauri `listen` 回调是 `{event, id, payload}`**：进度在 `payload` 里。读成 `event.downloaded`
   不抛错，进度条永远停在「下载中…」。
3. **`bindUpdatePanel` 会被 `bindSettings` 以元素调用**：元素没有 `querySelector`，
   直接调用会把整个设置页打不开。这条是既有的 `test-functional-ui` 抓到的。
4. **新 UI 文件必须进 `EMBEDDED_ASSET_SAMPLE`**：`embedded_assets_sentinel` 要求 `app/ui` 下的
   文件与资源表**逐一对上**；漏了就等于这个文件没嵌进二进制，而症状是**零报错**。
5. **浏览器测试要加载真实样式表**：不加载 CSS 时按钮显隐断言全部成立，结论未必能外推。

### 下一步建议

1. **端到端升级验收**（最有价值）：留一个旧版客户端 → 发新版 → 确认它真能发现并升上去。
2. **负例**：用错误的公钥验证「签名不匹配被拒绝」，且界面给出的是「已拒绝安装」而不是可重试的错误。
3. Intel / universal 路径。

---

## 10. 2026-10-19 追加：Windows 修复复核（v0.0.327，交付中）

更正：run 39186309579 的 979 passed / 0 failed 排除了两个套接字用例及文件锁竞争用例。
它只能说明剩余集合通过；此前把 BUG-004 关闭以及“Winsock 不支持取消”的解释均撤回。
不通过增加等待时间或删测试完成验收。

当前修复：持久化恢复所有平台的写锁，并通过同一持锁句柄读回修订，避免 Windows
强制锁阻止另一个句柄读取。文件身份使用 Unix dev/inode 或 Win32
GetFileInformationByHandle，不调用尚未稳定的 MetadataExt::file_index。

HTTP 层用 Arc 共享实际系统句柄，读取、超时、关闭不再分散到 Windows 复制句柄；
解析器必须等前一个 SequentialReader 释放后才能解除空闲读取超时。本地新增的
keep-alive 大包体回归在修复前失败、修复后通过；Windows 全套结果仍需单独取证。

保持 CRLF 指针偏移、JS 文件 URL、SVG 换行归一与 Path::is_absolute 修复。
更新门禁不再读取旧 dist 作为普通 Rust 测试输入；发布时另外核对实际签名包版本与签名。

验证入口：cargo test（全套及 webhook/dedup/notify_ledger 定向）、
vendor/tiny_http 的 timeout-cancel 回归、独立签名脚本与 release --published 验签。
真旧版客户端安装升级、Intel/universal 路径仍未验。

---

## 11. 2026-10-27 追加：槽位租约口径与三处真实缺陷（v0.0.417）

上一节列的「下一步建议 1（端到端升级验收）」本轮**未做**；本轮做的是 TASK-02/TASK-09 的
**槽位租约**收窄——它是那些「真实执行链路」的前置判据。

### 做了什么

1. **新模块 `app/src-tauri/src/slot_lease.rs`（纯函数）**：一条统一的槽位租约口径。
   租约**有版本**（释放必须带版本）、**只认当前持有者**、**占用幂等**、**超限拒绝不动账**；
   版本号**不复用**（租约清空后退回 1 会让旧版本的迟到释放撞上新版本）。
2. **`agent_resources.rs`**：运行时槽位从**裸计数**换成**版本化租约**。
   裸计数不知道「谁持有这个槽位」⇒ 迟到释放会减掉别人的占用 ⇒ 并发超卖；
   重复派发还会把计数抬高 ⇒ 看起来满了其实没满。
3. **`machine_node.rs`**：修掉三处真实缺陷（见下）。

### 三处真实缺陷（先在隔离副本复现，再改）

| 缺陷 | 根因 | 后果 |
| :--- | :--- | :--- |
| **迟到释放踢掉当前持有者的租约** | 释放只按 `task_id` 删租约、不核对版本 | **脑裂**：两台机器同时执行同一任务 |
| **抢占失败却已扣掉旧机器的槽位** | 先减旧机器再校验新机器 | **超卖**：实际并发超上限 |
| **重复派发同一任务** | 允许对已租约任务再次派发 | **槽位泄漏**：账实不符，可用槽位虚高 |

修法：释放**必须带版本**（不符 ⇒ 拒绝且账不动）；抢占**先校验目标再一并提交**；
已持有租约的任务**不重复派发**。

### 验证

- 新增 13 条用例（`slot_lease` 7 + `agent_resources` 3 + `machine_node` 3）；
  **变异 7/7 全部被拒**（含「摘版本核对」「摘先校验后提交」「摘重复派发检查」
  「摘占用幂等」「版本可复用」「超限拒绝仍动账」「绕过租约表」）。
- 全量 Rust：**1377 passed / 0 failed / 15 ignored**（基线 1364 + 13）。
- 新增门禁检查项 `task02.slotlease`（进 fast / ci / release 三档）。
- 证据：[`code-review/2026-10-27-1612-v0.0.417-independent.md`](../code-review/2026-10-27-1612-v0.0.417-independent.md)。

### 顺带修掉的两处**门禁自身**缺陷（都曾把发版卡住）

1. **VM 复位夹具的路径引号**（`scripts/test-native-gates-vm.py`）：
   夹具把解释器路径**未转义**地传给 `reset_guest_acceptance`，而该函数会把它拼进
   shell 命令串 ⇒ 本机 python 位于 `.../Application Support/...`（含空格）⇒ 命令被切开
   ⇒ **夹具恒红**。生产路径本来就 `shlex.quote` 转义（来宾 python 不含空格），所以只有夹具红。
   现夹具与生产同口径。
   ⚠️ **同类坑**：任何「拼进 shell 命令串」的参数都要先转义；
   `sh -c` 会**二次解析**，已经引号包裹的串也会被再切一次。
2. **Ed25519 门禁缺依赖**：`scripts/test-update-signature.py` 需要 `cryptography`
   （`scripts/requirements-test.txt` 已钉 `cryptography==50.0.2`），本机未装 ⇒
   7 条用例全部报 `ModuleNotFoundError` 而非真失败。
   **装法**：`python3 -m pip install -r scripts/requirements-test.txt`。

### 下一步建议

1. **真实执行链路接线**（最有价值）：本片只是**判据层**——
   `slot_lease.rs` 还没有接入任何真实进程编排。TASK-02 的剩余缺口是
   **可信能力与授权**（真实能力来源 + 授权策略验证器）、主控与工人的真实进程编排。
   ⚠️ AUTO-01 / TASK-06 的真实 driver 也卡在同一处：
   `pipeline.py` 的 `trusted_authorization_policy_unconfigured` 硬门
   （`scripts/automation/pipeline.py:1285`）——**这需要一次显式的安全决策**，不要绕过。
2. **NOT-06 端到端升级验收**：留一个旧版客户端 → 发新版 → 确认它真能发现并升上去；
   负例「签名不匹配被拒」仍未做（安全判据，不是可选项）。
3. **Intel / universal 路径**（当前只 arm64）。

### 本轮未验项（如实）

- `windows crate` 交叉类型检查失败（BUG-011：`ring` 需要 Windows SDK 头文件，
  本机无 Windows SDK；**基线同样红**，与本轮改动无关）。
- 原生动效逐边取证、官方 Claude 客户端取证（本轮未运行，需外部环境）。
- 真实多机环境、真实进程编排、端到端验收：均未涉及。

---

## 12. 2026-10-31 追加：NOT-06 更新命令边界与跨页恢复（v0.0.418，待发版）

### 改动

- Rust 更新命令现在执行进程级状态门禁：检查成功后才可安装，安装成功后才可由用户显式重启；重启状态只消费一次。
- 设置页重进恢复检查、下载、待重启、签名拒绝与普通失败状态。签名拒绝须用户关闭说明后才能重试；失败原因沿用已脱敏文案。
- 离页时清理进度监听与轮询；工作台缓存页恢复时重新读取后端状态，并用挂载代次丢弃旧页面的迟到响应。

### 验证

- `cargo test --locked --manifest-path app/src-tauri/Cargo.toml`：**1380 passed / 0 failed / 15 ignored**。
- `node scripts/test-update-state.mjs`：通过。
- `node scripts/automation/browser-check.mjs test-update-panel.mjs`：通过，覆盖检查/下载/错误/待重启恢复、签名拒绝、缓存页恢复、旧挂载迟到回包与监听清理。
- `scripts/common.sh` 已优先选 Python 3.15，再检查 `python3` 与较旧兼容版本；`scripts/test-framework-verifier.py` 新增优先级回归。VM 原生门禁单独保留 macOS 系统 Python：Tart ARP 元数据可能受本地网络隐私限制，其他解释器读取为空。
- `scripts/check-version.py 0.0.418` 与状态文档生成校验通过。
- `/opt/homebrew/bin/python3 scripts/native-gates-vm.py --vm agentisland-update-e2e --user admin --check-only`：通过，克隆 VM 的 Aqua 会话与 SSH 前置已验证。

### 尚未验收与环境

- 本轮尚未跑真实客户端点击升级或真实 updater 错签拒绝。隔离克隆 `agentisland-update-e2e` 中已从公开 v0.0.416 DMG 安装并启动旧客户端，基础 VM 未改动；当前没有可用的来宾 UI 控制回执，需在后续验收时完成检查、安装、显式重启与版本核对。
- 跳过版本的 UI/持久化、Intel/universal 架构仍未覆盖，NOT-06 继续标为 partial。
- 本轮代码审查通过；两位审查者确认的剩余范围是上述跳过版本 UI 与真实 updater 端到端验收。

---

## 13. 2026-11-01 追加：NOT-06 跳过版本持久化与恢复入口（v0.0.419）

### 改动

- 更新页允许跳过最近检查确认的具体版本；选择写入本地设置，重进页面仍可恢复，也可以显式恢复更新提示。更高版本仍会出现。
- 新增更新专用读/写/恢复命令；普通设置补丁不能改跳过记录。Settings 归一化版本号并原子写入；旧设置缺少字段时默认为未跳过。
- 检查、跳过与恢复操作的异步回包绑定面板代次；跨页重进后的旧结果不覆盖新状态。失败文案持续可见，只有成功重读设置才清除读取错误。

### 验证

- `cargo test --locked --manifest-path app/src-tauri/Cargo.toml skipped_version`：**7 passed / 0 failed**。
- `node scripts/test-update-state.mjs`：通过，包含检查、跳过写入与恢复提示的跨重挂载迟到回包用例。
- `node scripts/automation/browser-check.mjs test-update-panel.mjs`：通过，覆盖跳过按钮、跨页恢复、更高版本提示、恢复失败与设置读取失败。
- `python3 scripts/check-version.py 0.0.419`、状态文档生成器与 `git diff --check`：通过。
- 独立规格与工程规范复核均通过；finding、修复和证据见 [`2026-11-01-0116-v0.0.419-independent.md`](../code-review/2026-11-01-0116-v0.0.419-independent.md)。

### 未验项与环境

- NOT-06 仍为 partial：v0.0.416 客户端真实升级、真实 updater/UI 错签拒绝及 Intel/universal 架构路径未验收。
- BUG-016 仍未复现。自动检查核实原 VM 安装包为 v0.0.416，但正在运行的是验收构建；克隆副本启动了确切 .416 应用后，主机锁屏阻止桌面自动化，未触发检查，也没有 updater 请求或日志证据。原 VM 未改动，临时克隆已清理。
- `agentisland-iso` 已重新启动，来宾 Aqua/SSH 只读前置通过；`agentisland-update-e2e` 保持运行，未结束其中的验收进程。
- `scripts/release.sh 0.0.419 "NOT-06：跳过版本可保存并恢复提示"`：成功；GitHub [v0.0.419 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.419) 含 DMG、ZIP、updater 包、签名和 `latest.json`。CI 125/125 通过、package 7/7、原生门禁全部通过、真实产物签名验证 7/7；提交前及提交后脱敏扫描均通过。
- 发布未验项：原生动效逐边取证、真实 Claude 客户端取证。它们已由 `release.sh` 明确登记，不能据本次全绿推断覆盖。
- 后续优先安排旧版真实升级、签名拒绝和 Intel/universal 验收；BUG-016 需获得可自动操作的来宾桌面或独立的应用 updater 请求证据后继续定因。原 VM 的公开 updater manifest 有界 GET 为 HTTP 200、1.91 秒，但这不能证明 v0.0.416 应用本身完成了检查。

## 14. 2026-11-01 追加：TASK-01 项目依赖持久化与管理页接线（v0.0.420）

### 改动

- `Project.dependencies` 持久化在 `tasks.v1.json`；旧记录缺字段时默认为空。Store 在完整依赖图验证通过后沿用聚合 revision 与原子写契约。
- 项目依赖更新及项目删除已接入 Tauri 命令和管理页。界面可以选择项目、编辑依赖、保存或撤销草稿；共享 revision 写入前会拦截未保存依赖草稿，冲突后保留草稿并提示重新载入。
- 完整图校验对项目和依赖边线性遍历；未知项目、重复、自依赖、循环和超过 8 层的图均拒绝。删除守卫同时检查仍被依赖及仍有任务归属的项目。
- TASK-01 仍为 partial：看板拖拽界面接线与 actor-based authorization 未实现。

### 验证与发布

- 全量 Rust：**1391 passed / 0 failed / 15 ignored**。
- `node scripts/automation/browser-check.mjs test-task-dependencies.mjs`：通过；覆盖空项目控件隐藏/旧处理器清除、Tauri 命令注册与 Store 转发、共享 revision 草稿保护、依赖增删、循环拒绝、过期 revision 草稿保留和删除守卫。
- `python3 scripts/test-test-registry.py`：**23 passed**；`check-version.py 0.0.420`、需求矩阵和状态生成器通过。
- 规格与工程规范两项独立审查最终均 PASS；finding 修复和复测证据见 [`2026-11-01-0226-v0.0.420-independent.md`](../code-review/2026-11-01-0226-v0.0.420-independent.md)。
- `scripts/release.sh 0.0.420 "TASK-01：项目依赖持久化与管理页接线"`：CI **126 passed / 0 failed**，package **7 passed / 0 failed**；实际签名错误负例 7/7 通过，隔离 VM 原生门禁报告所有门禁均通过并核对了传输产物哈希。提交前、提交后脱敏扫描均通过。
- 提交 `f946d24818ea773ac1fa174f01d2606881f39620`、tag `v0.0.420` 已推送，GitHub [v0.0.420 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.420) 已核实含 DMG、ZIP、updater 包、签名及 `latest.json`。脚本两次确认重启成功，最终进程 PID `88859`，运行 `dist/AgentIsland.app`。

### 未验项与下一步

- Release 明确保留 2 项未验：原生逐边动效取证（需 `NATIVE_MOTION=1`）与真实官方 Claude 客户端取证（需 pinned CLI）；本次不能据此推断两项通过。
- NOT-06 仍为 partial：v0.0.416 客户端真实升级、真实 updater/UI 错签拒绝、Intel/universal 路径未验；BUG-016 仍未复现。`agentisland-update-e2e` 保持运行，本轮原生门禁使用的是独立 `agentisland-iso`，没有操作旧版升级实例。
- 可脱离来宾 UI 的下一片优先完成 TASK-01 看板拖拽界面接线；NOT-06 真实升级与错签验收待隔离来宾桌面自动控制链可用时继续。

## 15. 2026-11-01 追加：TASK-01 看板项目归属拖放（v0.0.421）

### 改动

- 未归档任务卡可拖到项目或“未分组”目标修改归属。项目目标栏在真实 `dragstart` 后显示、`dragend` 后收起，避免窄窗口常驻目标区挤压任务列表；详情页的原生项目选择控件保留键盘编辑路径。
- Tauri 命令将写入交给 Store；Store 在同一快照下核对任务、归档、来源列、目标项目和聚合 revision。成功只改归属与任务/聚合 revision，不碰运行历史、attention、artifact 或 receipt。同项目 no-op 不写文件也不发变化事件。
- 拖动期间刷新排队；取消不写入；过期 revision 和未保存的详情/项目依赖草稿拒绝提交并保留当前内容。

### 验证与更正

- 全量 Rust：**1395 passed / 0 failed / 15 ignored**。
- `node scripts/automation/browser-check.mjs test-task-board-drag.mjs`：通过；真实 DOM 拖放、键盘类型搜索及 Tab/Enter 保存、历史保持、同项目 no-op、草稿保护、归档任务、刷新竞争与取消均覆盖。
- `node scripts/automation/browser-check.mjs test-task-detail-navigation.mjs`：通过，**12 组**尺寸/主题/动效组合。首次 CI 发现项目目标常驻导致 880 px 窗口只剩 59 px 列表高度；目标区现改为仅拖动时出现，该回归复测通过。
- 最终稳定源码 CI run `20261031T193956Z-abc279c27993`：**127 passed / 0 failed / 0 blocked**。更早 run `20261031T191334Z-62b9d67e1308` 保留了第一次布局回归失败与源在验证期间变动导致的 `source.integrity` 失败；随后修复并完整复跑通过。
- 规格与工程规范两项最终复核均 PASS。审查发现键盘测试误称、短窗口目标栏挤压列表、drop 清理依赖已清空的 `activeDrag`，均已修复并加入浏览器断言。前端与 Rust 独立推导来源列是服务端信任边界所需；建议未来增加共享状态夹具，不作为当前阻断项。

### 发布结果与未验项

- `scripts/release.sh 0.0.421 "TASK-01：看板任务卡拖放修改项目归属"` 成功：源码 CI run `20261031T200751Z-abc708b7f024` 为 **127 passed / 0 failed / 0 blocked**；打包身份验证 **7/7**；实际签名正例和错签/篡改负例 **7/7**；提交前与提交后历史脱敏扫描通过。
- 隔离 VM 原生验收最终通过，证据目录 `.build/vm/acceptance-36e06f9340d74b5c9ca2366980a79212`，包含 UI、Dock、单实例、主包与 helper CLI 投递、窗口生命周期五项，并验证传输产物哈希。Release 脚本明确保留原生动效逐边与真实官方 Claude 客户端两项未验。
- 首次发布尝试的原生门禁失败样本保留在 `.build/vm/acceptance-b406668df05c4ed9a4fc373ff1c9dac0`：`test-cli-native.py` 停在 `primary-hide-delivery`，状态为 incomplete。随后在同一隔离 VM 对已注册同名副本场景做受控重放，主包和 helper 的收起/展开均成功；独立完整原生验收 `.build/vm/acceptance-757be60d2e544651a82938c82ad11697` 及发布流程中的完整重跑均通过。失败原因尚未确认，因此保留为一次未复现的间歇性失败，不据重跑结果抹除。
- 提交 `56e3989`，tag `v0.0.421` 已推送；GitHub [v0.0.421 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.421) 已核实包含 DMG、ZIP、updater 包、签名与 `latest.json`。脚本最终确认应用重启成功，PID `32835` 运行 `dist/AgentIsland.app`。本轮只操作发布用 `agentisland-iso`，没有操作 `agentisland-update-e2e`；升级签名密钥从受限权限文件读取，内容未查看或输出。
- `verify-framework.sh` 的 Windows 交叉类型检查仍受既有 BUG-011（本机缺 Windows SDK 头文件）影响；release CI profile 独立通过。
- TASK-01 仍为 partial：派生状态请求消费链、actor-based authorization 未完成。NOT-06 的 v0.0.416 真实升级、真实 updater/UI 错签拒绝及 Intel/universal 路径未验；BUG-016 仍未复现。没有要求用户进行手工 VM 操作。
- 独立审查报告：[`2026-11-01-0345-v0.0.421-independent.md`](../code-review/2026-11-01-0345-v0.0.421-independent.md)。

## 16. 2026-11-01 追加：TASK-01 派生状态列拖放消费链（v0.0.422）

### 改动

- 任务卡拖到派生状态列后先打开可键盘操作的确认框，用户需选择具体的本地进度；确认写入 `observed=false`，不会启动、停止或批准外部智能体，也不能通过拖放设置“来源已验收”。
- 新 Tauri 命令由 Store 按聚合 revision、任务当前来源列、归档状态和目标/状态映射复核；取消、同列 no-op、无效请求、草稿保护和过期 revision 均不写入。普通写入失败保留选择并显示后端原因。
- Rust 与真实 DOM 回归进入 CI/release；TASK-01 仍为 partial，actor-based authorization 仍在当前单用户界面之外。

### 验证与更正

- 定向 Rust 拖拽用例：**13 passed / 0 failed**。新增的 Store 表驱动用例覆盖全部合法目标、人工关口语义、Accepted/映射/来源/revision/归档/缺失拒绝及拒绝时文件字节保持。
- `node scripts/automation/browser-check.mjs test-task-board-status-drag.mjs` 与既有 `test-task-board-drag.mjs` 均通过；前者覆盖真实 DOM 确认/取消、所有选项、原始 revision、草稿保护、过期和普通写失败时的选择保留。
- 第一次源码 CI 留存失败样本 `.scratch/automation-runs/20261031T205818Z-9f5a831aa080`：**127 passed / 1 failed**，`automation.registry` 报同一 Rust 测试文件被两个 check id 重复认领。修正后 Rust 源文件仍由既有唯一登记项和全量 Rust 门禁覆盖，新浏览器门禁只认领自己的回归脚本。完整复跑 `20261031T211209Z-5686f464a6d0` 为 **128 passed / 0 failed / 0 blocked**。
- 首次发布尝试在打包前因当前 shell 未注入 updater 私钥路径而安全退出；随后按本机既有受限权限文件路径重跑，文件权限核对为 600，私钥内容没有读取或输出。构建产物、打包身份/随包 CLI 检查 **7/7**，错签/篡改签名用例 **7/7**。
- 隔离 VM 原生门禁全部通过，证据目录 `.build/vm/acceptance-0035de07622d4dd4b3f4dbfc770ef4f3`，并核对传输产物哈希。Release 明确保留两项未验：原生动效逐边取证、真实官方 Claude 客户端取证。
- 提交前与提交后历史脱敏扫描均通过。`scripts/release.sh 0.0.422 "TASK-01：看板任务卡拖放记录本地进度"` 成功：提交 `0baf09cbb180`、tag `v0.0.422` 已推送，GitHub [v0.0.422 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.422) 已核实含 DMG、ZIP、updater 包、签名与 `latest.json`；脚本确认应用重启成功，PID `89634` 运行 `dist/AgentIsland.app`。
- 全仓 `cargo fmt --check` 仍会报大量既有文件格式差异；本轮改动 Rust 文件的定向 `rustfmt --check` 通过。`verify-framework.sh` 受既有 BUG-011（本机无 Windows SDK 头文件）影响的记录见 §15；本轮 release CI 独立通过。

### 未验项与下一步

- NOT-06 继续为 partial。自动续查只读核实隔离 VM 安装包是 v0.0.416、当前进程是另一份验收构建；主机处于锁屏，Tart Guest Agent 的系统查询无响应，来宾辅助功能查询超时且无法抓屏。未触发 updater 检查/安装/重启，没有 updater 请求证据，BUG-016 仍未复现；证据已追加到 registry。没有要求用户进行手工操作，也没有改动该旧包或验收进程。
- v0.0.416 客户端真实升级、真实 updater/UI 错签拒绝、Intel/universal 路径仍未验；不将公开 updater manifest 可达性或隔离 VM 原生门禁通过解读为真实升级通过。
- 后续开发按 [`04 · 实施计划`](../workbench/04-plan.md) 和当前需求缺口选取，不把单用户场景里没有 actor 表面的 authorization 作为伪实现；NOT-06 桌面交互验收继续使用可自动操作的隔离环境，不要求用户代为点按。
- 独立规格与工程规范复审均 PASS；过程、初审修复与范围见 [`2026-11-01-0454-v0.0.422-independent.md`](../code-review/2026-11-01-0454-v0.0.422-independent.md)。

## 17. 2026-11-01 追加：NOT-06 更新检查有界等待（v0.0.423）

### 改动

- `update_check` 对一次用户显式发起的检查设置 30 秒总时限。超时会丢弃未完成的检查 future，把共享会话写为可重试失败态，并显示固定脱敏文案“连接更新服务超时，请稍后重试”。超时归类与检查超时共用 `UPDATE_TIMEOUT_REASON`。
- Rust 回归用永久 pending future 复现无界结算会使会话停在 `checking`；加入限时后验证会话进入 `failed`、原因固定且允许再次检查。真实 DOM 回归验证超时原因可见、“重试”恢复、安装入口不出现。
- 安装仍需用户单独点击；安装完成仍需用户再次显式重启。此保护不自动重试、安装或重启，也不改变隔离 VM。

### 验证与发布

- 定向 Rust updater 测试 **14 passed / 0 failed**；浏览器更新面板回归与 `test-update-state.mjs` 通过；registry 测试 **23 passed**，版本检查与需求矩阵/状态文档生成检查通过。改动文件的 rustfmt、Node 语法和 `git diff --check` 均通过。
- 完整源码 CI run `20261031T214307Z-36d6da5f08b9`：**128 passed / 0 failed / 0 blocked**。构建身份与随包 CLI 检查 run `20261031T215231Z-668f8cb0827e`：**7/7**；真实签名正例与错签/篡改负例 **7/7**。
- 隔离发布 VM 原生门禁全部通过，证据目录 `.build/vm/acceptance-c98d06dabaca48ea9ffc5214038cf94a`，并核对传输产物哈希。发布明确保留两项未验：原生动效逐边取证、真实官方 Claude 客户端取证。
- 提交前与提交后历史脱敏扫描均通过。`scripts/release.sh 0.0.423 "NOT-06：更新检查超时后可重试"` 成功：提交 `7427c0a7d468`、tag `v0.0.423` 已推送；GitHub [v0.0.423 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.423) 已核实包含 DMG、ZIP、updater 包、签名和 `latest.json`。脚本确认应用重启成功，PID `36928` 运行 `dist/AgentIsland.app`。
- 独立规格与工程规范审查均 PASS；报告见 [`2026-11-01-0541-v0.0.423-independent.md`](../code-review/2026-11-01-0541-v0.0.423-independent.md)。

### 未验项与下一步

- NOT-06 仍为 partial：v0.0.416 客户端真实升级、真实 updater/UI 错签拒绝、Intel/universal 架构未验。
- BUG-016 保持 open。代码级 pending future 证明该类未完成请求会造成相同的 `checking` 症状，也证明新版本 30 秒保护可恢复；这不能确认 v0.0.416 VM 的原始原因。隔离 VM 的图形界面自动化仍受主机锁屏和来宾辅助功能限制，本轮没有触发其检查、安装或重启，也没有要求用户手工操作。
- 后续如无需 VM 的开发继续按 [`04 · 实施计划`](../workbench/04-plan.md) 选择；BUG-016 的根因确认仍需来自旧版客户端本身的 updater 请求/错误证据。

## 18. 2026-11-01 追加：GW-03 工具事件 JSON 防御解析（v0.0.424）

### 改动

- `ToolCallAggregator` 对畸形 JSON、非对象 payload 和 serde_json 默认递归深度拒绝统一返回固定 `MalformedPayload`；合法对象缺少或空 `call_id` 仍返回 `MissingCallId`。
- 解析拒绝发生在写入聚合状态前。回归建立一条已有工具调用，再分别提交畸形、非对象和 130 层嵌套输入，并确认调用列表、索引和分片计数保持不变。
- 不增加真实网关传输、工具执行或凭据读取；不把这片描述为四类协议的真实端到端验收。矩阵中的 SSE 字节/事件数上限仍由既有 `sse_parser` 明确拒绝。

### 验证与发布

- 定向 `tool_call_aggregate` 测试 **9 passed / 0 failed**；测试注册表 **23 passed**；版本、需求矩阵、状态生成、rustfmt 与 `git diff --check` 均通过。
- 完整 CI run `20261031T220911Z-fc66899ba6cd`：**128 passed / 0 failed / 0 blocked**。构建身份与随包 CLI run `20261031T221822Z-013ad46d2632`：**7/7**；真实签名正例和错签/篡改负例 **7/7**。
- 隔离发布 VM 原生门禁全部通过，证据目录 `.build/vm/acceptance-472c760fc19740ad8b8aea6af0815cc8`，并核对发布包哈希。仍未验：原生动效逐边取证、真实官方 Claude 客户端取证。
- 提交前和提交后历史脱敏扫描均通过。发布提交 `c6074dd`、tag `v0.0.424` 已推送；GitHub [v0.0.424 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.424) 已核实含 DMG、ZIP、updater 包、签名与 `latest.json`。脚本确认本机新构建重启成功，PID `77347`。
- 独立规格与工程规范审查均 PASS；报告见 [`2026-11-01-0607-v0.0.424-independent.md`](../code-review/2026-11-01-0607-v0.0.424-independent.md)。

### 未验项与下一步

- GW-03 仍为 partial：流中失败状态机与四类协议真实端到端验收未完成；SSE parser 的单行 64 KiB、单事件 256 KiB 和每次解析 10,000 个事件上限已由现有解析器拒绝并有回归覆盖。
- BUG-016 仍为 open。v0.0.416 真实客户端检查、安装、显式重启与真实错签拒绝仍未验；旧版 `agentisland-update-e2e` VM 保持原样，本轮没有要求用户手工操作。
- TASK-01 的项目归属拖放与派生状态列拖放已分别在 v0.0.421 / v0.0.422 发布，下一片按当前 [`需求状态`](../需求状态.md)与[`需求测试矩阵`](../spec/需求测试矩阵.md)中的未完成缺口选择，不重复排已完成的拖放项。

## 19. 2026-11-01 追加：GW-06 首字节边界与流中失败状态机（v0.0.425）

### 改动

- 新增 `gateway_stream_failure.rs` 纯状态机，复用 `gateway_retry` 的失败资格/每候选重试预算与 `gateway_fallback` 的候选校验及顺序。非空上游字节块越过首字节提交边界；首字节前 transient 按预算重试，耗尽后才切下一个候选；首字节后任何失败都终止部分响应，不重试、不降级。
- Throttled 进入显式 Retry-After 等待态；调用方确认条件满足后可在同一请求、同一候选恢复，恢复消耗原有每候选与总预算。等待期拒绝写入、失败结算和完成，允许取消。
- 每次上游尝试都有内存终态；最多候选数 × 3 次、硬上限 24。该记录不是费用账本。GW-06 仍为 partial：没有接入真实转发、provider、工具执行或费用账本。

### 验证与发布

- 定向测试：`gateway_stream_failure` **7 passed**，`gateway_retry` **3 passed**；测试注册表 **23 passed**；版本、矩阵/状态生成、rustfmt 与 `git diff --check` 通过。变异测试在隔离 worktree 中将首字节后判据关闭，回归按预期以 `RetrySameCandidate` 对比期望 `AbortPartialResponse` 失败。
- Standards 与 Spec 独立复审均 PASS；Spec 初审曾指出 Retry-After 不能保留请求预算，修正后复审确认关闭。报告 [`2026-11-01-0640-v0.0.425-independent.md`](../code-review/2026-11-01-0640-v0.0.425-independent.md)。
- 本轮框架基线的 Rust 全量套件 **1409 passed / 15 ignored**，统一 CI **passed**；不过 `verify-framework.sh` 总结果非绿：旧本地签名 updater 包为 v0.0.424，与本轮配置不一致导致预构建只读漂移检查失败（v0.0.425 打包后真实 manifest 校验通过）；Windows 整 crate 类型检查在 `ring v0.17.14` 自定义构建步骤失败，与 registry 已登记的 **BUG-011** 相同：本机没有 Windows SDK 头文件，导致 `ring` 的 Windows 目标 C 构建不能交叉完成。该本机门禁仍为 open；权威 Windows CI 曾在 v0.0.360 / v0.0.361 含 `ring` 的依赖树下通过。本轮不能据此声称本机 Windows 整 crate 类型检查通过。
- 发布 CI run `20261031T225110Z-770fa7b2720a`：**129 passed / 0 failed / 0 blocked**；包内 CLI run `20261031T230215Z-acaa1b9832a9`：**7/7**。隔离发布 VM 原生门禁全部通过，证据 `.build/vm/acceptance-ab21f6e37d134fca8fbab187bd847ecf`；updater 签名正例/错签与篡改负例 **7/7**。提交前与提交后历史脱敏扫描通过。
- `v0.0.425` 发布提交 `1b9d9c4e1fe7932d6a5dee647c3ca81c585e2278`、tag 与 GitHub [v0.0.425 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.425) 已核实；附件含 DMG、ZIP、updater 包、签名与 `latest.json`。脚本确认宿主应用已重启，PID `72057`。明确未验：原生动效逐边取证、真实 pinned Claude 客户端取证。

### 未验项与下一步

- BUG-016 仍为 open：v0.0.416 真实客户端检查、安装、显式重启与真实错签拒绝尚未完成；旧版 `agentisland-update-e2e` VM 保持原样。本轮没有要求用户手工操作，也未把 v0.0.418/0.0.425 宿主或隔离发布 VM 的结果冒充旧版复现。
- 下一片从当前 [`需求状态`](../需求状态.md)与[`需求测试矩阵`](../spec/需求测试矩阵.md)的未完成缺口选取；若继续 GW-06，必须先接入消费该状态机的转发/工具/账本子系统并按 ADR 0040 另行定义授权与真实验收，不能把纯状态判据升级描述为端到端完成。

## 20. 2026-11-01 追加：Windows 本地时钟编译与本机交叉门禁分类（v0.0.426）

### 改动与验证

- `localclock.rs` 的 Windows 反向墙钟转换改用 Windows CRT `_mktime64`，在窄化到目标 `libc::time_t` 前检查范围；Unix 继续走 `libc::mktime`，系统 DST 规则和读回校验不变。
- 本机整 crate Windows 交叉检查现在只在唯一失败根因是 `ring` 且 ring 明确缺少 Windows CRT `assert.h` 时返回 exit 3。ring 自身额外 clang 错误、其他 crate 的错误、多重 C 编译错误与普通测试 suite 的 exit 3 都仍然失败；框架只把 Windows crate 检查的这个精确结果显示为 blocked / unverified。
- classifier 的真实 ring 日志正例、其他 crate 缺头文件、跨 crate 混合错误、ring 自身混合 clang 错误、普通 suite exit 3 均有回归。新加 ring 混合 clang 错误用例先按预期失败，再在修正后通过。完整 verifier 单测 **29/29**，Windows classifier 用例 **7/7**；registry 测试 **23/23**。
- 全量 Rust suite **1,409 passed / 0 failed / 15 ignored**；最终 `scripts/verify-framework.sh` exit 0，统一 CI profile passed。Windows 整 crate 本机交叉检查因本机没有 Windows CRT/SDK 头文件仍明确 blocked；5 个 suite/profile 列为 blocked / unverified（Windows crate、native、manual、client、soak）。不能把它们写成通过。
- Release 源码 CI run `20261101T000945Z-7171479e841e` **129 passed / 0 failed / 0 blocked**；随包 CLI 与构建身份验收 `20261101T001709Z-1b088dc99d3c` **7/7**；更新包错签、篡改与正例 **7/7**；隔离 VM 五项原生门禁通过，证据 `.build/vm/acceptance-35f166bbbe1d40988b4f4e82e08063c2`。GitHub Actions [run 41256233179](https://github.com/bitterSmilezzz/AgentIsland/actions/runs/41256233179) 的 **Windows 与 macOS jobs 均 success**；Windows 编译、HTTP 超时/取消回归、Rust 测试全部通过，因此 **BUG-017 已关闭**。独立审查报告 [`2026-11-01-0807-v0.0.426-independent.md`](../code-review/2026-11-01-0807-v0.0.426-independent.md) 的复审时点结论为 Standards / Spec 均 PASS。
- `scripts/release.sh` 完成脱敏扫描、签名构建、原生验收、提交后历史复扫与附件校验；提交 `4021fd300cbe802641adcbeec13c04fc810bd56b`，tag `v0.0.426` 已推送，GitHub [v0.0.426 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.426) 含 DMG、ZIP、updater 包、签名与 `latest.json`。应用已重启，PID `90057` 运行 `dist/AgentIsland.app`。本轮另有两个明确未验项：原生动效逐边取证、真实 pinned Claude 客户端取证。

### 失败与更正记录

- 第一次全框架复验因源码已升为 0.0.426、磁盘签名 updater 包仍是 0.0.425 而在只读清单漂移检查失败；随后按实际构建路径生成 0.0.426 包、复制签名副本并重写清单，完整复验 exit 0。一次独立 `test-update-signature.py --published` 在复制更新包/签名到 `dist/` 前运行，7 项均在版本一致性准备断言处失败；按发布脚本顺序复制后 7/7 通过。
- 首次发版文件清单预检含一条误写路径，脚本在暂存前拒绝，未提交或推送；修正显式清单后才继续。签名内容始终由构建脚本读取，没有查看或输出。

### 未验项与下一步

- BUG-011 的“本机环境前置误报为门禁失败”已关闭；本机整 crate Windows 交叉编译依然因缺 SDK 头文件未验证，Windows 权威 CI 已通过。
- BUG-016 仍 open：没有在 `agentisland-update-e2e` 中触发检查或变更旧版应用。代码级超时回归证明当前版本不会无限停在 checking，但不能证明 v0.0.416 的原始根因。全程没有要求用户做手工 VM 操作。
- 后续按当前 [`需求状态`](../需求状态.md)、[`需求测试矩阵`](../spec/需求测试矩阵.md) 与 [`04 · 实施计划`](../workbench/04-plan.md)选择未完成项；若继续网关方向，按 ADR 0040 接入真实消费子系统并单独定义授权和验收，不把现有纯状态逻辑描述成端到端能力。

## 21. 2026-11-01 追加：TASK-05 本地产出正文持久化（v0.0.427）

### 改动

- 用户在“记录进度”中填写的产出正文以 SHA-256 文件独立、原子保存；任务 JSON 仅保留摘要和字节数。任务、运行、产出与 revision 精确绑定，重开 Store 后仍可读回。
- 单条正文上限 64 KiB、被引用正文总量上限 64 MiB；凭据形态、软链接、非普通文件、陈旧 revision、损坏或摘要不符内容均拒绝。任务页支持草稿撤销与纯文本只读查看；关闭阅读器或离页后清空正文。
- 来源事件正文仍按原只读路径瞬时读取，不进入任务文件或 blob 存储。未接入产出版本追加、比较/恢复 UI 或自动任务执行。
- 用户本轮确认的看板拖拽语义与既有 `task01.dragstatus` 实现一致：选择具体本地进度并确认；不写“来源已验收”，也不操作外部智能体，因此没有重复改动该项。

### 验证与发布

- 定向 `artifact_store` **9/9**、`local_artifact` **4/4**；本地产出浏览器回归通过；automation registry **23/23**；需求矩阵、状态生成、版本检查与 `git diff --check` 通过。
- 最终 `scripts/verify-framework.sh` **exit 0**：Rust **1417 passed / 0 failed / 15 ignored**；Windows 四片段 **4/4**；工作区脱敏扫描零新增命中。Windows 整 crate 交叉类型检查因本机缺 `ring` 所需 Windows CRT/SDK `assert.h` 明确 blocked / unverified；该结果不是 Windows 构建通过。
- 同次统一 CI run `20261101T015604Z-6a6ebd4591d7` **131 passed / 0 failed / 0 blocked**；package profile run `20261101T020539Z-a1961bc63747` **7/7**。首次复验因本地 updater 包仍为 v0.0.426、随后因 `dist/latest.json` 未重生成而失败；按发布构建路径重建 v0.0.427 签名包并运行 `render-update-manifest.py --write` 后，最终总验证 exit 0。失败原因与更正均保留在本段。
- `release.sh` 源码 CI run `20261101T020818Z-02ffe983807f` **131/131**；构建身份与随包 CLI run `20261101T021509Z-c0c0b851e0cb` **7/7**。隔离 VM 五项原生门禁全部通过，证据 `.build/vm/acceptance-5a4fcce6f0ac40369763dc94bf0edd7b`；updater 正签、错签和篡改测试 **7/7**。
- 提交前/提交后历史脱敏扫描通过。提交 `b861578bc37f979f132ac2d418a258d5e831ca9c`、tag `v0.0.427` 已推送；GitHub [v0.0.427 Release](https://github.com/bitterSmilezzz/AgentIsland/releases/tag/v0.0.427) 已核实包含 DMG、ZIP、updater 包、签名与 `latest.json`。脚本重启 `dist/AgentIsland.app` 成功，PID `28348`。
- 本轮未验：原生动效逐边取证、真实 pinned Claude 客户端取证；真实 v0.0.416 updater 升级也未触发。未要求用户手动操作，旧 `agentisland-update-e2e` VM 未触碰。

### 未验项与下一步

- TASK-05 仍为 partial：既有版本判据尚未接入任务页生产流程，下一片继续完成产出版本演进、比较与恢复 UI；外部单版本来源不伪造历史。
- BUG-016 仍为 open：v0.0.416 卡在“检查更新”的原始请求/错误证据未取得，不能用宿主 v0.0.418、发布 VM 或代码级超时测试冒充旧版复现。
- TASK-01 `task01.dragstatus` 已满足本轮确认的本地进度选择与确认语义；不要重复排期。后续按当前 [`需求状态`](../需求状态.md)和 [`需求测试矩阵`](../spec/需求测试矩阵.md)选择未完成项。

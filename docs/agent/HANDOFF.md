# 交接说明（Hand-off）

> **给下一个 Agent**：这份文件回答「我该从哪里开始、当前处于什么状态、有哪些坑别踩」。
> 与它配套的还有：[`AGENTS.md`](../../AGENTS.md)（工作约定，Agent 无关）、
> [`docs/README.md`](../README.md)（文档地图）、[`需求状态.md`](../需求状态.md)（做到哪了）。

**交接时点**：2026-10-18
**基线**：`main` = `origin/main` = `118b84d`，**工作区干净**
**最新发布**：`v0.0.324`（**DMG 与自动更新已提交但未发版**，见下）
**基线是否绿**：门禁 `verify-framework.sh` 通过（`ALL VERIFIED`，rc=0）；4 个 profile 因外部前置 blocked（见下）

> 🟡 **DMG 分发 + 自动更新基础设施已提交（`118b84d`）但没有发版**。
> 最新 release 仍是 `v0.0.324`，**用户还拿不到 DMG**。
> **接手第一件事**：读 [第 8 节](#8-2026-10-18-追加dmg-分发--自动更新基础设施未提交未发版)，
> 那里有一条可直接跑的发版命令。

> ⚠️ 本文件由**两个会话接力**：第 1–6 节是 2026-10-17 晚（NOT-05 交付与三轮复审），
> 第 7 节是 CI 身份规则修复（**不是本次交接者写的**，但记录的是真实修复，保留不删），
> 第 8 节是 DMG 与自动更新。

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
| 隔离 VM | `tart` 有 `agentisland-iso`（**stopped**，需 `tart run` 启动） | `release.sh` 直接 `die`「未运行原生门禁，不发布」 |
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

## 8. 2026-10-18 追加：DMG 分发 + 自动更新基础设施（**已提交、未发版**）

> ✅ 这 19 项改动**已提交并推送**（`118b84d`），已通过全量门禁（`ALL VERIFIED`，rc=0）。
> **但没有发版**——`v0.0.325` 未上线，用户仍拿不到 DMG、自动更新也没启用。

### 已实测可用（已提交，未发版）

| 项 | 证据 |
| :--- | :--- |
| DMG 打出 | `AgentIsland-0.0.325.dmg`（6.2 MB） |
| DMG 内 app 签名完整 | 构建日志 `✓ DMG 内 app 签名完整`；残缺即构建失败 |
| updater 更新包已签名 | `.app.tar.gz` + `.sig`（412 B），真实 minisign 签名 |
| 更新清单端到端 | `dist/latest.json` 字段正确：version / `darwin-aarch64` / signature / 下载 URL |
| 全量门禁 | `bash scripts/verify-framework.sh` → **ALL VERIFIED**（rc=0） |

### 发版只差一条命令

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
  bash scripts/release.sh 0.0.325 "DMG 安装包与自动更新基础设施"
```

### ⚠️ 发版前必读：`release.sh` 要提交本轮改动

`release.sh` 的设计是**把本轮改动一起提交**（清单 → 暂存 → commit → tag）。
所以它对「工作区已经干净」是**不成立**的：

- 清单由 `git status` 生成，工作区干净时清单为空 → 第一步就 `清单为空` 退出；
- 即使手填已提交的文件，走到「提交」一步也会因为没有 staged 改动而 die。

本轮（v0.0.325）的改动已经随 `118b84d` 推上去了，所以**现在直接跑上面的命令会失败**。
正确做法是：先带着本轮要发版的修复做一次提交，让 `release.sh` 把它作为发版提交。
（这也是为什么上面那步先写「先提交并发布」，而不是「一条命令」。）

**推论：站点改动也别提前提交。** 展示页的下载按钮是 DMG 直链，而直链里的 tag
（`v0.0.325`）只有发版之后才存在——提交站点**必须**发生在发版之后或同一次发版里。
提前提交会出现「仓库里已经写着 v0.0.325.dmg，但该链接 404」的窗口期。
所以顺序是：改站点 → 保持未提交 → 跑 `release.sh`（它会一并提交并打 tag）。

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

- **自动更新的界面层**：检查 / 提示 / 下载 / 安装，一行都没写。`views.js` 与新命令都还没动。
- `release.sh` 里更新包附件那段**尚未真跑过一次**（要真发一次才知道对不对）。
- **只 arm64**（Apple Silicon）；Intel Mac 需要 universal binary。
- 未公证，用户首次打开仍需右键 → 打开（要免这步需 Apple Developer Program，本轮不含）。
- 自动更新的**端到端验收**（客户端真去 GitHub 拉、验签、替换）完全没做。
  **签名不匹配的负例（错误公钥必须被拒）也没做**——这是安全判据，不是可选项。

### 下一步建议

1. 先提交本轮修复（CI 无密钥构建 + 资产名 + 能力标签 + 文档），**再**走 `release.sh 0.0.325`
   —— 注意 `release.sh` 要自己提交，所以清单不能是空的（见上面「发版前必读」）。
2. 再做界面层：`检查更新` 按钮 → 调 updater → 提示 → 下载 → 重启安装。
   按已定策略：**手动检查 + 确认后安装**，不在用户不知情时重启应用。
3. 补端到端验收：装一个旧版 → 发新版 → 确认客户端能发现并升级；
   **负例必须做**——用错误的公钥验证签名不匹配时被拒绝。

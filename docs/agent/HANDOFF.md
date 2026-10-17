# 交接说明（Hand-off）

> **给下一个 Agent**：这份文件回答「我该从哪里开始、当前处于什么状态、有哪些坑别踩」。
> 与它配套的还有：[`AGENTS.md`](../AGENTS.md)（工作约定，Agent 无关）、
> [`docs/README.md`](README.md)（文档地图）、[`需求状态.md`](需求状态.md)（做到哪了）。

**交接时点**：2026-10-17 晚
**基线**：`main` = `origin/main`，工作区干净
**最新发布**：`v0.0.324`
**基线是否绿**：门禁 `verify-framework.sh` 通过（`ALL VERIFIED`，rc=0）；4 个 profile 因外部前置 blocked（见下）

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
- NOT-05 实现 + 三轮独立复审整改（详见 [`code-review/2026-10-17-not05-review-remediation.md`](code-review/2026-10-17-not05-review-remediation.md)，585 行）
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

完整版见 [`agent/session-2026-10-17-problems.md`](agent/session-2026-10-17-problems.md)。最该记住的：

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
- **Windows job 仍红**：编译已过，但 `cargo test` 952 passed / 23 failed / 8 ignored，
  已登记 `BUG-004`。它仍是 `continue-on-error`，不挡 macOS 门禁。
- 下一次 push 后要盯 Actions：macOS job 会**第一次**真正跑到「构建并验证 ad-hoc 签名包」
  与 package profile；若那两步红，失败点在扫描之后，别再按扫描问题排查。

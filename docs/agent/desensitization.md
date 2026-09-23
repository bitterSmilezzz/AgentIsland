# 数据脱敏清单（提交、发版与对外交付前执行）

本项目为公开仓库。凭据类内容**必须**为零；个人身份类信息（本机用户名、绝对路径、真实邮箱）
历史上确实漏进过入库文档一次（见第三节），所以这条不靠记性、靠 `scripts/scan-secrets.sh`。
另一侧的风险面是**未入库的本地工作区**（`.scratch/`、`artifacts/`、`dist/`、`.build/`、
根目录 `*.zip`）——它们记录本机路径、开发者代号与真实屏幕内容，交给会收集数据的第三方工具前必须先剔除。

## 一、清理动作（删除即可，无需改写）

| 目录/文件 | 内容 | 处理 |
|---|---|---|
| `.scratch/` | issue/spec 草稿、review 记录，含 macOS 用户名、个人代号、本机工具清单 | 删除 |
| `artifacts/` | UI 验证截图与 OCR，捕捉真实屏幕（AI 会话片段、浏览网址、消费/token 数字） | 删除，仅可保留 `artifacts/999-club/` 宣传图 |
| `dist/` | 历次 `.app` 与发布 zip，二进制内嵌构建期本机绝对路径 | 删除 |
| `.build/` | SwiftPM 构建缓存，`manifest.pif` / `workspace-state.json` 含本机绝对路径 | 删除 |
| 根目录 `*.zip` | 遗留发布包，同上 | 删除 |

删除后工作区仅剩 `.gitignore` / `AGENTS.md` / `CHANGELOG.md` / `CONTEXT.md` / `Package.swift` /
`README.md` / `Sources/` / `Tests/` / `docs/` / `scripts/`。这些都在库里——**库里的东西要按
第二节扫过才算可公开**，因为删掉工作区文件并不会让已经 commit 的内容消失。

## 二、复扫（每次提交与发版前，一条命令）

```sh
scripts/scan-secrets.sh            # 工作区：git 认得的全部文件（.gitignore 排除的不扫）
scripts/scan-secrets.sh --staged   # 只扫已暂存内容（pre-commit 走这条）
scripts/scan-secrets.sh --release  # 工作区 + 全部 git 对象，发版前置（release.sh 内置）
```

此前这一节是一份手工 grep 清单，写着「凭据/密码赋值/个人身份三类应零命中」——**那个前提不成立**：
测试夹具里有形如 `password: "…"` 的假口令与占位邮箱，调研文档里有示例收件地址，手工清单要么天天红
要么被人调瞎。现在改成一档一档可执行的棘轮：

- 规则分两类：`cred_prefix`/`private_key` 是**绝对零命中**（真凭据的形状，没有「假值」这一说）；
  其余（假口令、URL 里的 token、本机用户名与绝对路径、邮箱、手机号）命中必须在
  `scripts/secrets-baseline.txt` 里逐条备过案，**新增即失败**。
- 备案的是 `(规则, 文件, 行内容校验和)` 三元组，不是文件也不是目录——把某棵子树整体放行
  等于把门禁关掉。
- 就地豁免 `nosec: <理由>` 只认写在命中行上的理由；`SKIP_SCAN=1` 仅供本地试跑，
  用它产出的产物不得发布（脚本会在输出里标出来）。
- 命中内容一律打码后才打印，密钥不会进终端历史或 CI 日志。

## 三、已知情况说明

- **git 作者**：全局 `user.name`/`user.email` 为 `bitterSmilezzz` /
  `38044568+bitterSmilezzz@users.noreply.github.com`，即该公开仓库的 GitHub 账号本身，
  属公开信息，无需处理。
- **`docs/agents/triage-labels.md`** 两处提到 `GH_TOKEN` / `fine-grained PAT`，只描述该
  token 无 labels 写权限，**不含任何 token 值**。
- **源码 `home()` 调用**（`AgentRegistry.swift` 等处）均使用 `FileManager.default.homeDirectoryForCurrentUser`
  拼系统相对路径，无硬编码用户名。
- **扫描器第一次跑就抓到一处真泄漏**：`docs/research/qoder-monitoring.md` 粘了一条本机实况，
  里面有 `/Users/<开发者账号>/workspace/...`。当前版本已改成占位路径；
  **旧版本仍在 git 历史里**（blob `eaad946e…`），改写公开历史要 force push 已发布的
  全部 tag，代价大于收益，故记入 `scripts/secrets-history-allow.txt` 并写明理由。
  这条是「工作区改了不等于历史干净」的实例——所以扫描分工作区与 git 对象两档，发版走两档。
- **备份**：如需在交付后恢复本地开发状态，备份包放仓库外，且**不要**放进被交付的目录。

## 四、给 agent 的执行顺序

提交 → 打 tag → 发 release 是一条链，**扫描必须在第一步之前**：commit 一旦落地，泄漏内容
就进了 git 对象，删文件也拿不回来。整条链由 `scripts/release.sh <X.Y.Z> "<标题>"` 串起来，
手工分步执行时按 `scan-secrets.sh --release` → commit → tag → push → `gh release create`
的顺序，不要跳步。

# 数据脱敏清单（交付给第三方工具前执行）

本项目为公开仓库，代码与文档本身**不含任何个人凭据**。风险集中在**未入库的本地工作区**
（`.scratch/`、`artifacts/`、`dist/`、`.build/`、根目录 `*.zip`）——它们记录了本机路径、
开发者代号、真实屏幕内容，交给会收集数据的第三方工具时必须先剔除。

## 一、清理动作（删除即可，无需改写）

| 目录/文件 | 内容 | 处理 |
|---|---|---|
| `.scratch/` | issue/spec 草稿、review 记录，含 macOS 用户名、个人代号、本机工具清单 | 删除 |
| `artifacts/` | UI 验证截图与 OCR，捕捉真实屏幕（AI 会话片段、浏览网址、消费/token 数字） | 删除，仅可保留 `artifacts/999-club/` 宣传图 |
| `dist/` | 历次 `.app` 与发布 zip，二进制内嵌构建期本机绝对路径 | 删除 |
| `.build/` | SwiftPM 构建缓存，`manifest.pif` / `workspace-state.json` 含本机绝对路径 | 删除 |
| 根目录 `*.zip` | 遗留发布包，同上 | 删除 |

删除后工作区仅剩 `.gitignore` / `AGENTS.md` / `CHANGELOG.md` / `CONTEXT.md` / `Package.swift` /
`README.md` / `Sources/` / `Tests/` / `docs/` / `scripts/`——全部内容已入库，可公开。

## 二、复扫清单（每次交付前跑一遍）

```sh
# 1) 凭据模式（应零命中）
grep -rInE '(sk-[A-Za-z0-9]{16,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-|xox[baprs]-|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{25,}|-----BEGIN [A-Z ]*PRIVATE KEY-----)' \
  --exclude-dir=.git --exclude-dir=.build --exclude-dir=dist .

# 2) 账号密码赋值（应零命中）
grep -rInE '(password|passwd|secret|api[_-]?key|access[_-]?token)"?[[:space:]]*[:=]' \
  --exclude-dir=.git --exclude-dir=.build --exclude-dir=dist .

# 3) 个人身份：本机用户名 / 绝对路径 / 邮箱 / 手机号（应零命中，$USER 替换为本机用户名）
grep -rInE "/Users/\$USER|$USER|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}|1[3-9][0-9]{9}" \
  --exclude-dir=.git --exclude-dir=.build --exclude-dir=dist .

# 4) git 全对象（含悬空对象、提交信息），逐对象 dump 后匹配
git rev-list --objects --all | awk '{print $1}' > /tmp/o.txt
git fsck --unreachable --dangling 2>/dev/null | awk '{print $3}' >> /tmp/o.txt
sort -u /tmp/o.txt | git cat-file --batch-check | awk '$2=="blob"||$2=="commit"||$2=="tag"{print $1}' > /tmp/c.txt
while read o; do git cat-file blob $o 2>/dev/null || git cat-file commit $o 2>/dev/null || git cat-file tag $o 2>/dev/null; printf '\n\0'; done < /tmp/c.txt > /tmp/d.txt
grep -aoiE '<第 1、2、3 步同样的模式>' /tmp/d.txt   # 应零命中

# 5) 本地目录是否还存在
ls -d .scratch artifacts dist .build 2>/dev/null; ls *.zip 2>/dev/null   # 应只有需要保留的
```

## 三、已知情况说明

- **git 作者**：全局 `user.name`/`user.email` 为 `bitterSmilezzz` /
  `38044568+bitterSmilezzz@users.noreply.github.com`，即该公开仓库的 GitHub 账号本身，
  属公开信息，无需处理。
- **`docs/agents/triage-labels.md`** 两处提到 `GH_TOKEN` / `fine-grained PAT`，只描述该
  token 无 labels 写权限，**不含任何 token 值**。
- **源码 `home()` 调用**（`AgentRegistry.swift` 等处）均使用 `FileManager.default.homeDirectoryForCurrentUser`
  拼系统相对路径，无硬编码用户名。
- **备份**：如需在交付后恢复本地开发状态，备份包放仓库外，且**不要**放进被交付的目录。

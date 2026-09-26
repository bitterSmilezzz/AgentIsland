# graphify 一手调研

> 一手核实记录。来源：GitHub API 元数据（2026-09-26）、`v8` 分支（**默认分支就是 `v8`，
> 不是 `main`**，README:1175 `git checkout v8  # active development branch`）
> 全量 tree（`git/trees/v8?recursive=1`，331 条路径）、`README.md`（1251 行全读）、
> `docs/how-it-works.md`、`BENCHMARKS.md`、`LICENSE` / `LICENSE-MIT` / `NOTICE`，
> 以及逐字原文抓取的源码：`graphify/cli.py`（239,092 B，含 `_run_hook_guard` 全文）、
> `graphify/install.py`（108,574 B）、`graphify/hooks.py`（43,511 B）、
> `graphify/build.py`、`graphify/llm.py`、`graphify/report.py`、`graphify/validate.py`、
> `graphify/skill.md`（41,733 B）、`graphify/always_on/claude-md.md`、`graphify/skills/claude/references/hooks.md`、
> `tools/skillgen/gen.py`（61,959 B / 1383 行）、`tools/skillgen/platforms.toml`、
> `tests/test_hook_strict.py`（202 行全读）、`tests/test_confidence.py`（196 行全读）。
> GitHub API 元数据（2026-09-26）：**121,455 star / 11,705 fork / Python / Apache-2.0 /
> 创建于 2026-04-03 / 1477 open issues / homepage graphify.com / default_branch `v8`**。
> 本地素材：`/tmp/uibatch/graphify/`（README.md、BENCHMARKS.md、how-it-works.md、
> cli_full.py、install.py、hooks.py.raw、skill.md.raw、test_hook_strict.py、test_confidence.py、
> gen.py、platforms.toml、NOTICE、LICENSE、LICENSE-MIT）。
> 引用按 `path:line` 给出，读不到的写「未获取」。**全文不含任何真实 token / key 值。**

---

## 1. 一句话结论

**graphify 是「用机制而不是用文档保证 agent 行为」这条路目前最完整的一个公开实现，
而它的机制只有两层：一个 fail-open 的 PreToolUse guard（默认只 nudges），
加一个 opt-in 的 strict deny（第一次 raw read 被拒，然后永久退回 soft nudge）。**
它把「fires at most once per session and never gets stuck」的一句话 README 承诺
（README:214）落成了 `_mark_session_denied` 的 `O_CREAT|O_EXCL` 单次占有 + 24h GC
（`cli_full.py:710-732`），并且用 14 个 pytest 逐条钉住这个性质
（`tests/test_hook_strict.py:64-167`）。**这是本文最该抄的一处，抄的是形状不是内容。**

**它的第二等价值是 EXTRACTED/INFERRED/AMBIGUOUS 三档——但必须先说清一个事实：
这三档里 AMBIGUOUS 只由 LLM 语义 pass 产生（`llm.py:485`、`:515`、extraction-spec.md:59），
AST 确定性 pass 一辈子只产出 EXTRACTED 与 INFERRED 两档。**也就是说它「AMBIGUOUS 是第三档」
的全部含金量在于给 LLM 的 rubric，不在工程实现。对我们的启示因此是反的：
**我们的 `provenance` 是确定性枚举（不是置信度打分），加第三档要么照 INFERRED 的做法
（一个真正的「本该观测到但没观测到」的状态），要么不加。**

**Apache-2.0 与 Vorssaint 的 GPL-3.0 是本文的第三条结论：Apache-2.0 不传染
（连修改后的分发也不强制回馈），允许 embed / 派生 / 闭源再分发；代价是要保留
`NOTICE`、`LICENSE` 与版权声明、以及被改动的文件要标注。NOTICE 现读为 5 行
（"Portions of this software were contributed under the MIT License prior to the
relicensing and remain available under those terms"）——**它有一段 MIT 历史，
如果我们真要复用某个文件，得逐文件判断它落在哪个 license 下。****

---

## 2. 规模与工程事实（先钉住数字，后文引用）

| 维度 | 实测值 | 取证 |
| :--- | :--- | :--- |
| Star/fork/issues | **121,455 / 11,705 / 1477 open** | GitHub API repo（2026-09-26） |
| 语言 / license | Python / **Apache-2.0**（`LICENSE` 首行 "Apache License Version 2.0"） | `LICENSE`、`NOTICE` |
| 默认分支 | **`v8`**（README:1175 `git checkout v8 # active development branch`） | README |
| 包名与命令 | PyPI 包 **`graphifyy`（双 y）**，CLI 命令 `graphify`（README:172） | README:172 |
| 源码规模 | `graphify/` 下 **54 个 `.py` 文件**；最大的三个：`extract.py` 405,818 B、`extractors/engine.py` 375,572 B、`cli.py` 239,092 B、`install.py` 108,574 B | `git/trees/v8?recursive=1` |
| 语言支持 | README 自称 **37 tree-sitter grammars**（README:510），另有 Apex / Terraform / OCaml / Lisp / Robot 走 extra | README:510-527 |
| 测试 | `tests/` 下 **140+ 个 `test_*.py`**；与本文直接相关的：`test_hook_strict.py`、`test_hook_guard.py`、`test_confidence.py`、`test_hook_chain_survives_skip.py`、`test_hook_out_of_project_paths.py`、`test_claude_md.py` | `contents/tests` 目录列表 |
| CI | `.github/workflows/` 三个：`ci.yml`、`publish.yml`、`release-graph.yml`；README 自陈 CI 在 Ubuntu 跑 Python 3.10/3.12/3.13/3.14 四版本 | `git/trees` + README:1196、:1222 |
| CI 里的 skill 自检 | `python -m tools.skillgen --check` / `--audit-coverage` / `--schema-singleton` / `--monolith-roundtrip` / `--always-on-roundtrip`（README:1200-1204） | README + `gen.py:1-20` |
| 测试 runner | `uv run pytest tests/ -q`，无 XCTest 类比物（Python 项目） | README:1189 |
| 平台矩阵 | 20+ 助手，每个一个 `graphify <platform> install` 子命令 | README:216-294、:1041-1079 |
| skill 产物 | **16 个 `skill-*.md` monolith + 13 个平台 `skills/<p>/references/` 目录**（每目录 8 个 reference），全部由 `tools/skillgen` 生成为 committed artifact | `tools/skillgen/expected/` 目录列表 |

**一个规模判断**：121k star / 1477 open issues，与 Vorssaint 的 21k star / Swift 单品不同量级。
但它的代码组织方式对我们最有参考价值的部分（hook guard、skillgen、置信度 rubric）
都不在最大的文件里——`cli.py` 的 guard 只有约 120 行，`always_on/claude-md.md` 只有 772 字节。
**规模与可抄性在这里是脱钩的。**

---

## 3. A. 它是什么（产品层）

### 3.1 形态：一个 skill + 一个 CLI + 一组 hook

它**自己发 skill**，装法三步（README:36-41）：

```text
uv tool install graphifyy     # 包名 graphifyy，命令 graphify
graphify install              # 注册 skill 到 ~/.claude/skills/graphify/
/graphify .                   # 在 IDE 里调用
```

project-scoped 安装写 `.claude/skills/graphify/SKILL.md` 或 `.agents/skills/graphify/SKILL.md`
外加一个按需加载的 `references/` sidecar（README:197-202）。

**`graphify install --platform agents`（README:292）值得记**：它显式指向跨框架标准位置
`~/.agents/skills/` 与 `./.agents/skills/`——**与我们本机 `.agents/skills/` + `.claude/skills/` 软链
是同一个约定**（见 [AGENTS.md:78](../../AGENTS.md#L78)）。README 原话：`--platform agents` targets
the spec's user-global `~/.agents/skills/`（read by npx skills and spec-compliant frameworks）。
这说明 Agent Skills 已经是它愿意投入的平台之一，不是边角料。

### 3.2 本地 deterministic 的边界（哪部分用 LLM 哪部分不用）

`docs/how-it-works.md:3-22` 把三 passes 写得很清楚：

| Pass | 内容 | LLM？ | 离机？ |
| :--- | :--- | :--- | :--- |
| 1 | tree-sitter AST 提 classes/functions/imports/calls/comments | **不用** | **不离机** |
| 2 | 视频/音频 faster-whisper 转录 | 不用 | 不离机 |
| 3 | docs / PDFs / 图片 / 转录稿的语义抽取 | **用** | **离机** |

关键细节：**代码不进语义 pass**。README:570 原话 "Code is extracted locally with no API
calls (AST via tree-sitter). Everything else goes through your AI assistant's model API."
`skill.md:157-175` 把这件事说成一条硬指令：
`Code files are not sent to the LLM semantic extractor in the normal pipeline...
semantic extraction is reserved for docs, papers, images, and transcripts.`

**「兜底 LLM = 宿主 agent 自己」这个设计很妙**（`skill.md:161-173`）：
没有 API key 时，语义 pass 由「当前 IDE 会话里那个 agent」通过 Task 工具并行分派完成。
skill 里明写 `**MANDATORY: You MUST use the Agent tool here. Reading files yourself
one-by-one is forbidden - it is 5-10x slower.**`（`skill.md:246`）——这是**用文档强制机制**的
又一个例子，而且它连失败信号都定义好了：chunk 文件不存在 → 说明 subagent 被派成只读的
Explore 类型，打印警告而不是静默跳过（`skill.md:288-291`）。

**对我们的关系**：我们读 agent 会话日志、`~/.claude.json`、`~/.codex/` 全是确定性本地读，
连 Pass 3 都不需要。**graphify 的本地/离机分界线画在「代码 vs 非代码」上，
我们的画在「本机文件 vs 网络」上，且我们这边整条线都不越界。**

### 3.3 三档置信度

README:503 原话：`Confidence tags — every inferred relationship is marked EXTRACTED,
INFERRED, or AMBIGUOUS. You always know what was found vs guessed.`

`docs/how-it-works.md:36-50` 是它最完整的一处口径表：

| Tag | 含义 | confidence_score |
| :--- | :--- | :--- |
| `EXTRACTED` | 源码里直接找到（函数调用、import） | 恒 **1.0** |
| `INFERRED` | 合理推断 | 离散 rubric：0.95 / 0.85 / 0.75 / 0.65 / 0.55 |
| `AMBIGUOUS` | 不确定——标记出来给人审 | **0.1–0.3** |

INFERRED 的 rubric 是**离散枚举不是连续区间**，理由它写在 `extraction-spec.md:55-58`：

> Models follow discrete rubrics better than continuous ranges; the bimodal
> distribution observed in production (>50% at 0.5, >40% at 0.85+) shows the
> range guidance is being collapsed to a binary. If no value above fits, mark
> the edge AMBIGUOUS rather than picking 0.4 or below.

**这是全文我最喜欢的一条工程观察：它观察到「给模型连续区间，模型会塌成两点」，
于是改成枚举，并且把「没有一个值合适」显式路由到 AMBIGUOUS 而不是让模型硬给 0.4。**
它甚至禁掉 0.5（`extraction-spec.md:47` "never use 0.5 as a default"），
而缺省值取 0.55——注释理由是「absence of evidence 应该取 rubric 允许的最弱值，
而不是中点」（`tests/test_confidence.py:133-137`）。

---

## 4. B. EXTRACTED / INFERRED / AMBIGUOUS 与我们 `provenance` 的对照

### 4.1 我们的现状

`Sources/AgentIslandCore/Models.swift:11-19`：`AgentProvenance` 四个 case：
`selfReported` / `observed` / `inferred` / `conflict`。注释把加这一维的动机写死了：
「**同一张卡片上会混着两种可信度**」——带令牌自报的「我在等确认」与 CPU/写入兜底猜出来的
「看起来在干活」，此前都印成同一行字，用户没法分辨那句是谁给的。

**注意它是确定的四态，不是连续置信度**：`selfReported` = 有令牌且在 TTL 内；
`observed` = 会话日志有强语义；`inferred` = CPU/写入双信号兜底；`conflict` = 两条对不上且
**两条都要显示，不许悄悄选一个**（`Models.swift:16` 注释原话）。

### 4.2 逐维对照表

| 维度 | graphify 三档 | 我们 `AgentProvenance` | 谁细 | 判断 |
| :--- | :--- | :--- | :--- | :--- |
| **来源分档** | EXTRACTED（源码显式）/ INFERRED（解析推断）/ AMBIGUOUS（不确定） | observed（会话强语义）/ inferred（CPU+写入兜底） | 它多一档 | **我们有它没有的 `selfReported`**（它没有「当事人自己说的」这个概念） |
| **冲突态** | **无** | `conflict`（自报与观测对不上，两条都显示） | **我们** | 它三档里没有「两个来源互相矛盾」这一格 |
| **置信度量纲** | `confidence_score` 浮点（INFERRED 用离散枚举） | **无量纲**——纯来源标记 | 它细 | 我们不需要分数：我们只有 2-3 个确定的观测源 |
| **强度档位** | INFERRED 细分 0.55/0.65/0.75/0.85/0.95 五档 | inferred 一档（CPU 与写入双信号） | 它细 | **这里我们其实可以学**：我们「CPU 高但进程在跑构建」与「CPU 高且无已知工作负载」的可信度显然不同 |
| **对读者的呈现** | 报告里逐条列 `[AMBIGUOUS]` 让人审（`report.py:298-306`）；`amb_pct > 20` 时告警（`report.py:361`） | `badgeText` 只给 selfReported/conflict 挂标签，observed/inferred 挂 nil——「**给它们都挂个标签等于把噪声当信息**」（`Models.swift:36-38` 注释） | **我们更克制** | 这条我站我们：卡片上 6pt 宽微细条挂不起五档标签 |
| **score 的身份** | EXTRACTED 恒 1.0；AMBIGUOUS ≤0.4（测试钉住，`tests/test_confidence.py:65-77`） | 不适用 | — | 它用**测试固定 score 与 tag 的对应关系**，这是可抄的纪律 |

### 4.3 **「我们该不该加第三档」——判断：不加，但要做一件不同的事**

先说清楚 graphify 的 AMBIGUOUS **是什么**：

1. **它只由语义 pass 产生**。全仓 grep `AMBIGUOUS` 的出处只有四处有语义：`llm.py:485`
   （system prompt 里写 "AMBIGUOUS: uncertain — flag for review, do not omit"）、
   `llm.py:515`（deep mode 后缀 "Mark uncertain ones AMBIGUOUS instead of omitting"）、
   `extraction-spec.md:59`（"AMBIGUOUS edges: 0.1-0.3"）、
   `extraction-spec.md:27`（图片手写/白板 "mark uncertain readings AMBIGUOUS"）。
   **AST 侧 39 处 `"confidence": "INFERRED"`、多处 `EXTRACTED`，零处 `AMBIGUOUS`**
   （`extract.py` grep 实测）。
2. **它是 LLM 的出口阀，不是算法判定**。触发条件不是「两个证据打架」，而是
   「模型觉得说不好，且离散 rubric 里没有一个值合适」。
3. **它在图里的排序最低**：`_CONFIDENCE_RANK = {"EXTRACTED": 3, "INFERRED": 2, "AMBIGUOUS": 1}`
   （`build.py:65`），用于同一对节点重复边时保留高置信度那一条（`build.py:1450-1464`）。

**所以直接照搬 AMBIGUOUS 对我们没有意义**——我们没有 LLM 在生成边，也没有五分制置信度。
但 graphify 在这个问题上有一个**真正可迁移的洞察**，值得单独说：

> **「conflict」与「ambiguous」是两种不同的东西，而我们只有前者。**
> graphify 没有 conflict 格（它的三档描述的是**单条边**的可信度，不是多源一致性）；
> 我们有 conflict 格（`Models.swift:16`）。这是两个正交维度，谁都不该被谁替代。

**结论：不加 AMBIGUOUS 这一档。要动就动 `inferred` 的细分——把「CPU/写入兜底」
拆成至少两档**：一个是我们确实知道有工作负载在跑（编译、测试、已知 CLI）所以 CPU 高是预期的，
一个是什么已知负载都没有、CPU 却高。前者几乎等于 observed，后者才是真推断。
**这条改进的依据不是「信息更全」，而是它命中 `Models.swift:6-8` 已写明的动机：
同一张卡片上混着两种可信度。** 若要加，形态应该是新 case（如 `inferredExplained` /
`inferredUnexplained`），**不是加一个 confidence score 字段**——后者会把我们拖进
「UI 上要不要显示 0.75」这种我们明确不想回答的问题（`Models.swift:36-38` 注释）。

**明确不做的**：给 `provenance` 加浮点分、加五档 rubric、在微细条上挂更多标签。

---

## 5. C. strict 模式：机制细节 + 我们能照做什么

这是本文的重点，`cli_full.py` + `install.py` + `tests/test_hook_strict.py` 三份原文能把它完整复原。

### 5.1 它挂在哪个机制上

Claude Code 的 **PreToolUse hook**，写进 `.claude/settings.json`（`install.py:1855-1877`）：

```python
read_cmd = f"{exe} hook-guard read" + (" --strict" if strict else "")
[
  {"matcher": "Bash|Grep", "hooks": [{"type":"command","command": f"{exe} hook-guard search","timeout":10}]},
  {"matcher": "Read|Glob", "hooks": [{"type":"command","command": read_cmd,"timeout":10}]},
]
```

（`install.py:356-362`）三个实现细节值得记：

- **`Grep` 在 search matcher 里**，注释解释了为什么：「current Claude Code routes content
  search through its dedicated Grep tool, not Bash (#1986) — a Bash-only matcher never
  fired on the agent's primary search path」（`install.py:337-340`）。
- **每条 hook 都带 `timeout: 10`**，理由是 Claude Code 对 command hook 的默认超时是 **600 秒**，
  「a single wedged guard ... stalls the surrounding tool call for ten minutes on every
  Bash/Grep/Read/Glob call — the four highest-frequency tools an agent uses」
  （`install.py:352-356`）。**这一条对我们直接适用：我们不在 `.claude/settings.json` 里装 hook，
  但任何我们写的 hook 都必须带 timeout，默认 600s 是陷阱。**
- **project-scoped 装的时候用裸 `graphify` 而不是绝对路径**，因为配置会被提交，
  装的那台机器的路径是对的、别人 clone 下来是错的（`install.py:334-336`，issue #3129）。

### 5.2 guard 的行为（`_run_hook_guard`，`cli_full.py:814-955`）

它读 stdin 的 tool-call JSON，分 `search` / `read` / `gemini` 三种 kind：

**search kind（Bash|Grep）——永远只 nudge**：

```python
if (is_grep_tool or is_bash_search) and out_path("graph.json").is_file():
    sys.stdout.write(_SEARCH_NUDGE)
```

（`cli_full.py:868-870`）注意它**用一个真正的 shell 命令解析判断 Bash 是否在跑搜索工具**
（`_bash_invokes_search`，`_SEARCH_COMMANDS` / `_COMMAND_WRAPPERS`，`cli_full.py:745-813`），
而不是全串子串匹配——注释给了理由：「the old test was a plain substring scan over the whole
command string, including quoted arguments and heredoc bodies - so
`git commit -m "add flag support"` fired ("flag " contains "ag ")」
（`cli_full.py:758-762`）。**「截 `ag `」这个 bug 类别我们太熟了——文档里凡提到 grep/find
的地方都可能被误触。**

**read kind（Read|Glob）——五道闸门，然后才是 strict deny**：

```python
# 1. 落在 graphify-out/ 里或不带源文件扩展名 → 直接 return（不 nudge）
under_out = "graphify-out/" in j or (GRAPHIFY_OUT_NAME.lower() + "/") in j
if under_out or not any(tl in _HOOK_SOURCE_EXTS for tl in tails): return
# 2. #1840: 解析不出项目根 → out-of-project → return
root = Path(os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd())
... if not in_project: return
# 3. #1840: 图的 mtime 比目标文件旧 → stale → 软 nudge，绝不 deny
stale = os.stat(fp).st_mtime > gmtime
if stale: sys.stdout.write(_READ_NUDGE_STALE); return
# 4. strict deny：只有 Read 工具、本 session 第一次、最近没 query 过、文件确实被索引
tool_name = d.get("tool_name")
if _hook_strict_enabled(strict) and tool_name in (None, "Read") \
        and not _query_stamp_fresh() \
        and _target_is_indexed(fp, root) \
        and _mark_session_denied(str(d.get("session_id") or "")):
    sys.stdout.write(_READ_DENY); return
# 5. 其余 → 软 nudge
sys.stdout.write(_READ_NUDGE)
```

（`cli_full.py:881-955`）

**fail-open 写在 docstring 里当契约**：「Fails open everywhere: any error, or a non-matching
tool call, prints nothing and the caller exits 0, so a legitimate tool call is never
blocked by a bug.」（`cli_full.py:830-832`）整个函数体包在一个 `try/except Exception: pass` 里
（`cli_full.py:952-954`）。测试逐条钉住：`test_fail_open_on_malformed_stdin`（`tests/test_hook_strict.py:169-171`）。

### 5.3 「fires at most once per session and never gets stuck」怎么保证

三个独立的机制叠起来，这是最漂亮的部分：

**(a) `_mark_session_denied`：用文件系统做一次性互斥，不是内存状态**

```python
fd = os.open(str(d / f"{sid}.denied"), os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o644)
os.close(fd)
...
return True
except FileExistsError: return False
except Exception: return False
```

（`cli_full.py:710-732`）**`O_CREAT|O_EXCL` 第一次创建成功返回 True，之后同一 session id
一律 FileExistsError → False。** 附带两个好处：跨进程有效（hook 是每次新起一个进程）、
失败即降级（任何异常都 return False，即「这次不拦」）。sid 先过 `re.sub(r"[^A-Za-z0-9_-]","_")`
再截断到 64 字符，防路径注入。**并且顺手 GC 掉 24h 前的 marker**（`cli_full.py:720-728`）。

**(b) `_query_stamp_fresh`：query 过一次就自动解锁**

```python
ttl = float(os.environ.get("GRAPHIFY_HOOK_STRICT_TTL", "1800"))
return (time.time() - out_path("cache", "last_query_stamp").stat().st_mtime) < ttl
```

（`cli_full.py:699-707`）`graphify query` / `path` / `explain` 三个命令成功后都会
`_touch_query_stamp(gp)` 写一个 mtime 戳（`cli_full.py:1321`、`:1714`、`:1851`）。
**这是「redirect 到图」的真正闭环：deny 消息里给的是 `graphify query "<question>"`，
agent 跑了它就自解锁。** 默认 TTL 1800s。

**(c) 环境变量随时可关**：`_hook_strict_enabled`（`cli_full.py:675-684`）——
`GRAPHIFY_HOOK_STRICT=1/0` 覆盖安装时烘焙的 flag，**不用重装 hook**。

**「never gets stuck」还有两道语义闸门**（`cli_full.py:833-839` docstring 原话）：
`Search (Bash) and Glob stay nudge-only: a compound shell command has no single
parseable target and blocking file listing would strand navigation.` 加
`reads of out-of-project files are ignored, and a graph that is stale for the target file
softens to a non-mandatory nudge`。测试逐条对应：`test_glob_never_denies`、
`test_search_never_denies`、`test_out_of_project_read_silenced`、`test_stale_graph_softens_never_denies`、
`test_needs_update_flag_softens`（`tests/test_hook_strict.py:121-154`）。

`_target_is_indexed`（`cli_full.py:972-1000）也值得一提：**只在目标文件真的在图里才拦**，
读 manifest.json（cap 2MB），missing/corrupt/oversized/unresolvable 一律 return True——
注释：「that block is self-limiting, so erring toward it is safe」。

### 5.4 我们能照做什么（具体到本仓文件）

我们**没有 `.claude/settings.json` hook 机制在跑**（本仓的 hook 只有 `.git/hooks/pre-commit`，
见 §6.1），所以不能照搬 PreToolUse。但**「用机制保证文档纪律」这个思路有四条立刻可落地**：

**① 把「先读 `docs/workbench/01-current-state.md` 再动侧边栏」从文档约定变成 pre-commit 检查。**
我们 `AGENTS.md` 已有一条纯文档纪律：「给 agent 的操作指令必须点名工具」
（[AGENTS.md:87](../../AGENTS.md#L87) 那条 mattpocock 元规范的落地）。它现在**无机制保证**。
可做：`.git/hooks/pre-commit` 里加一段——**`Sources/**` 的变更若触及侧边栏四块
（`Sources/AgentIsland/...Sidebar*` 之类）而同一次 commit 没碰 `docs/workbench/01-current-state.md`
也没有 `.scratch/` 说明，打印一条硬警告。** 注意这个形态要照 graphify 的 fail-open 与
at-most-once：**不要 exit 1**（会挡住正常提交），要 stdout 一条醒目警告即可。
**这是 strict 的最小可抄版：把「should」变成「会被提醒」，而不是变成「会被拦」。**

**② 我们的 pre-commit hook 缺 timeout 与「已有别人的 hook 不覆盖」的兜底。**
现状：`scripts/install-git-hooks.sh:19-24` 发现 `$HOOK` 已有别人的钩子就**退出 1 拒绝装**
（不追加），而实际 `.git/hooks/pre-commit` 只跑了扫敏。graphify 的做法是
**marker 包围 + append**（`hooks.py:628-653` `_install_hook`：marker 已在就原地替换块，
不在就 `content + "\n\n" + script` 追加，卸载按 marker 精确删），
且整块包在 `( ... )` 子 shell 里使每个 skip 条件的 `exit 0` 只结束自己那段
（`hooks.py:372-380` 注释原话：bare `exit 0` in a flat script ends the WHOLE hook）。
**这条直接相关且立刻可抄**（详见 §6.1）。

**③ 「先查再改」这条若能机制化，最该用在脱敏扫描上。** graphify 的 guard 有一条我们
完全没做的：**stale 就降级**（目标文件比产物新 → 从「必须」降成「建议」）。
对应我们：`scripts/scan-secrets.sh --staged` 今天只看暂存内容。可以加一条等价的软化——
**当 `.graphify` 风格基线文件 `scripts/secrets-baseline.txt` 比本次变更的文件还旧时，
提示「基线可能过期，建议 `--rebaseline` 核对」而不是直接放行警告**。这是我看到的最自然的
一处迁移，但**属于新功能，不在本次任务范围**。

**④ 文档纪律的机制化上限要说清**：graphify 能这么做是因为它自己的 skill 明确知道自己
读的是什么（`graphify-out/graph.json`）。**我们的纪律大多是「先读哪份文档」，
没有唯一可判定的源，机制化只能做到「提醒」做不到「拦截」。** 这是它和我们处境的结构性差别，
硬抄 deny 会把人和 agent 都卡住。

---

## 6. D. 工程上可抄的三件（含 git hook 那条）

按「立刻能做 + 有对应源码 + 不引入依赖」排：

### 6.1 **git hook：解释器路径内嵌 + marker 追加 + 子 shell 隔离**（最该抄，且直接相关）

**它解的问题（README:212 原话）**：`graphify hook install` embeds the current interpreter
path directly into the hook scripts at install time, so the post-commit hook fires correctly
even in GUI git clients and CI runners where `~/.local/bin` is not on PATH.

**为什么这是个真问题**：`uv tool install graphifyy` 把可执行文件放进 `~/.local/bin`
（README:206 自陈 "common on a fresh macOS + zsh setup — that dir isn't on your PATH yet"）。
GUI git 客户端（GitHub Desktop、Sourcetree）和 CI runner **不读你的 `.zshrc`**，
export 给 hook 的 PATH 是系统最小集 → `command -v graphify` 找不到 → hook 静默失败。

**它的解分三层，第一层最值得抄**：

1. **安装时把绝对解释器路径烤进脚本**：`__PINNED_PYTHON__` 占位符在 `hook install` 时
   替换成当时那个 `sys.executable`（`hooks.py:13-17` 注释：「Pinning sys.executable at
   install time makes the hook work regardless of PATH at git-trigger time」）。
   然后 hook 先试这个 pinned 路径：

   ```sh
   _PINNED='__PINNED_PYTHON__'
   if [ -n "$_PINNED" ] && [ -x "$_PINNED" ] && "$_PINNED" -c "$_GFY_PROBE" 2>/dev/null; then
       GRAPHIFY_PYTHON="$_PINNED"
   fi
   ```

   （`hooks.py:35-40`）**探针用 `importlib.util.find_spec` 而不是 `import graphify`**，
   注释给了硬理由：「a probe that imports graphify wholesale executes the full package
   import (10s+ cold on machines with AV-scanned or large site-packages)」
   （`hooks.py:27-30`）。

2. **四级 fallback**：pinned → `graphify-out/.graphify_python` 文件 → PATH 上的 launcher
   解析 shebang → 扫 uv tool 环境 → `python3`/`python`。每一级都过同一套字符白名单
   （`*[!a-zA-Z0-9/_.@: \\-]`），**防 shell 注入**（`hooks.py:19-135`）。

3. **pinned 路径若会过期就不 pin**：`_is_rotating_prefix` 检测 `/snap/<app>/<revision>/`，
  命中就返回空字符串走降级路径——注释：「observed across 15 repositories at once when
  an editor snap moved past its revision」（`hooks.py:709-717`）。

**它的 hook 骨架还有三条对我们的 pre-commit 直接有用**：

| 它的做法 | 源码 | 抄给我们什么 |
| :--- | :--- | :--- |
| **marker 包围，append 不覆盖** | `_HOOK_MARKER = "# graphify-hook-start"` / `_END`（`hooks.py:8-11`）；已装在原地替换、不在就追加（`hooks.py:639-652`） | 我们的 `install-git-hooks.sh:19-22` 现在是「有别人的 hook 就 exit 1」。**改成 marker + append，就能和别人共存在同一个 `pre-commit` 里** |
| **整块包在 `( ... )` 子 shell** | `hooks.py:372-380` 注释：flat script 里 graphify 每个 skip 条件的 `exit 0` 会结束**整个** hook，「silently dropping anything after graphify's end marker」 | 我们的 pre-commit 若加了第二个逻辑，**第一个逻辑的 `exit 0`（例如 rebase 中跳过）会吞掉扫敏**。我们今天只有一个 hook 所以没暴露，加第二个那天就会炸 |
| **rebase/merge/worktree 一律跳过** | `_WORKTREE_GUARD`（`hooks.py:364-370`，用 `git rev-parse --git-dir` vs `--git-common-dir` 判 linked worktree）；rebase-merge / rebase-apply / MERGE_HEAD / CHERRY_PICK_HEAD 四个目录/文件探测（`hooks.py:405-410`） | **我们的 `pre-commit` 今天在 rebase 中会跑扫敏**。rebase 里的中间提交常常缺上下文（例如某次 rebase 后暂存集变了），这是假拒绝源头 |
| **`GRAPHIFY_SKIP_HOOK=1` 逃生门 + `--allow-partial`** | `hooks.py:412`、README:940 | 我们已有 `SKIP_SCAN=1`（AGENTS.md:66），口径一致 |
| **后台化不阻塞 commit** | `_detached_launch` 用 Python `subprocess.Popen(start_new_session=True)`（`hooks.py:342-356`），POSIX setsid / Windows `CREATE_NO_WINDOW\|CREATE_NEW_PROCESS_GROUP`；回滚了旧的 `nohup ... &`（Git for Windows 的 MSYS 没 nohup，#1161） | **我们不后台化**（扫敏只要几秒），但若要加「提交后自动干点什么」，**这条形状值得照抄：用宿主语言做 detach，别依赖 coreutils** |

### 6.2 **skillgen：把 skill 当生成产物，用 CI 钉住不漂移**

这是我们 `AGENTS.md` 的 Installed skills 一节**完全没有**的一条纪律。

它把 16 个 monolith + 13 平台的 references 全部视为**由 `tools/skillgen/fragments/` 生成的
committed artifact**，人只编辑 fragment，跑 `python -m tools.skillgen` 重渲染，
`--check` 字节级比对（`gen.py:9-20`、`gen.py:686-700`）。**漂移即 CI 失败 + pre-commit 失败**
（`.pre-commit-config.yaml:10-17`，注释原话 "a hand-edit to a generated file fails this
check the same way CI does"）。

更漂亮的是四个 roundtrip 校验（README:1200-1204，`gen.py:1291-1350`）：

```text
--check               字节级比对 render vs committed + expected/
--audit-coverage      每个 host：它自己 v8 body 的每个 heading 都单一归属在自己的 render 里
--schema-singleton    file_type 六值枚举在所有产物里逐字节相同
--monolith-roundtrip  每个 monolith == v8  modulo  enum 统一
--always-on-roundtrip 每个 always_on/*.md 逐字节复现它曾经的常量
```

**`--audit-coverage` 的注释解释了为什么必须 per-host**（`gen.py:46-52`）：
"a drop that only hits one host (e.g. trae losing its AGENTS.md integration section) is
invisible when every host is checked against claude's monolith"。**这是「一个 host 悄悄少了一段」
的真实 bug 类别。** 它甚至把 baseline 固定到一个不可变 commit SHA 而不是 `origin/v8`
（`gen.py:60-61`，因为一旦 split 落地 `origin/v8` 就变成自我比较）。

**抄给我们**：我们的 `.agents/skills/libraries-dev/`（单份源 + `.claude/skills/` 软链）
已经是「单一来源」形态，但没有「生成物不许手改」的检查。**可抄的最小版**：
在 `.git/hooks/pre-commit` 或 CI 里加一条 `git diff --quiet HEAD -- .claude/skills/libraries-dev`
——**因为它是软链指向 `.agents/skills/`，任何对它内容的改动必然要落到源那边；发现软链目标被改
就用 diff 检出。** 但注意这条需要先确认我们的 skill 分发形态（见 §8 未核实清单第 4 条）。

### 6.3 **「多形态产物分层」：三个文件各对一个读者**

`graphify-out/` 三个文件，README:43-49：

```text
graph.html       人看（可点、可筛选、可搜索）
GRAPH_REPORT.md  人读摘要（god nodes / surprising connections / suggested questions）
graph.json       agent 与机器读（不重读文件就能查）
```

**为什么给三种而不是一种 markdown**——`skill.md:645-655` 给了答案：跑完管线后**只把
GRAPH_REPORT.md 的三个 section 粘进聊天**（"Do NOT paste the full report - just those
three sections. Keep it concise."）。即**三种形态对应三种消费长度**：浏览器里人愿意看几千节点；
聊天窗口只容得下一屏摘要；agent 要的是可遍历结构。

**对照我们的文档分层**（[AGENTS.md:42](../../AGENTS.md#L42) 那张表）：README（功能）/ CONTEXT（口径）/
workbench（作战）/ research（一手证据）/ CHANGELOG（版本）。我们的分层**按时间与确定性切**，
graphify 的**按消费者切**。两条轴不冲突，但它提醒我们一件事：
**research 那批一手调研（含本文）其实只有 agent 读，写成人读的长文是有价值的
（证据要能被人核对），但引用它的时候应该像 graphify 的 query 那样给 scoped 引用，
不是让人通读全文。**

---

## 7. benchmark 表：方法论比数字重要

README:105-132 的表与 [BENCHMARKS.md](https://github.com/Graphify-Labs/graphify/blob/v8/BENCHMARKS.md) 全文：

| Suite | Dataset | 指标 | graphify | 对照 |
| :--- | :--- | :--- | :--- | :--- |
| Memory | LOCOMO (n=300) | recall@10 | **0.497** | bm25 0.362 / mem0 0.048 / supermemory 0.149* |
| Memory | LOCOMO (n=300) | QA accuracy | 45.3% | supermemory 49.7% / bm25 31.3% / mem0 27.3% |
| Memory | LongMemEval-S (n=50) | QA accuracy | 76% | dense RAG 76% / mem0 70% |
| Cost | LOCOMO ingest | USD | ~$1.40 | supermemory $15.67 / mem0 $3.48 |
| Code | ERPNext (~1M LOC, n=6) | key-fact coverage | **82.0%** | grep+read floor 70.8% |

**它赢的三处**：recall 最高、建图零 LLM 成本、ingest 便宜 11 倍。
**它输的一处**：QA accuracy 输 supermemory 4.4 分——**它自己把这个写进了表**
（README:118、BENCHMARKS.md:30），然后用「but at about 11x the ingest cost」把它框回来。

**方法论上真正值得学的四条**：

1. **一个 harness，对手作为 adapter 跑在里面**（BENCHMARKS.md:36-40）：
   "Competing systems (mem0, supermemory) are run as adapters inside it, so every system
   sees the same model, token budget, and grader." **不信任对手自报的数字。**
2. **judge 自身被验证并公开**（BENCHMARKS.md:79-82）：
   "the judge was blind-validated against a second, independent judge on a sampled set
   at 90.6% agreement, Cohen's kappa 0.81 ... Most published memory benchmarks disclose
   no judge validation at all; we publish ours so the grading itself can be audited."
   它甚至公布了「大多数基准不公布 judge 验证」这个事实作为自己的加分项。
3. **承认 embedder 混淆**（BENCHMARKS.md:114-117）：
   supermemory 的 recall 是 `*`，理由是它自带 768-d 英文专用 embedder 而 harness 共享 BGE-m3——
   **它主动标记这个维度不可比，而不是偷偷把优势数字摆在最显眼处。**
4. **deliberately 指出 seed-only ablation 也还不错**（BENCHMARKS.md:125-126）：
   "A seed-only ablation (no graph expansion) still scores 42.7% at $1.40 ingest, so most
   of the accuracy holds at the cheapest setting." **自己给自己的核心卖点做了消融。**

**对我们的用法**：我们不会建图也不需要 benchmark。但**第 2、3、4 条是给我们的
「怎么证明自己」立的样板**——特别是第 3 条（主动标出不可比维度）与第 4 条（给自己的卖点做消融）。
我们外发 README 时若写性能数字，该按这四条办。

---

## 8. Apache-2.0 对我们意味着什么（与 Vorssaint GPL-3.0 的关键区别）

### 8.1 两者差异（写准）

| 维度 | Apache-2.0（graphify） | GPL-3.0（Vorssaint） |
| :--- | :--- | :--- |
| **传染性** | **不传染**。embed / 派生 / 修改 / 闭源再分发都允许，无需开源你的代码 | **强传染**。分发派生作品必须以 GPL-3.0 提供源码 |
| **我们能做的事** | 可以读源码学实现；可以复制**代码片段**进我们的项目；可以闭源分发含它的二进/派生 | 只能学**做法与结构**，源码一行不能进我们的项目 |
| **必须履行的义务** | 保留 `LICENSE` 副本、保留 `NOTICE`、保留版权与归属声明；**改动过的文件要标注 "changed"**；若原作品带 NOTICE 且你改过它，NOTICE 里的归属信息要传递 | 提供源码、同 license、声明修改 |
| **专利** | 有明确的**专利授权条款**（贡献者授予用户其贡献的专利许可） | 有配套的专利条款（GPLv3 加了反专利迫害条款） |
| **商标** | **不授予商标权**——名字/logo 不能用 | 同 |

**所以对 graphify：源码可以抄，这是与 Vorssaint 那篇的根本区别。**
Vorssaint 那篇的结论是「做法可抄、代码不可抄」；graphify 这篇**没有这条禁令**。

### 8.2 但有三个必须写进决策的限定

1. **NOTICE 里有一段 MIT 历史**（`NOTICE` 原文 5 行）："Portions of this software were
   contributed under the MIT License prior to the relicensing and remain available under
   those terms. The original MIT license text is retained in LICENSE-MIT."
   **即逐文件判断：某个具体文件可能落在 MIT 下而不是 Apache-2.0 下。** 如果我们真要
   复用某个文件，得查它进仓时的 license。这在「抄几个函数」这种粒度上通常无所谓
   （MIT 与 Apache-2.0 都允许），但**抄整段/整文件时是实的**。
2. **它自带 `TRADEMARKS` 性质的分离我们不抄**：Vorssaint 有独立 `TRADEMARKS.md`
   （fork 必须改名换图标换 bundle id）。graphify 没有同等文件，但那是因为 Apache-2.0
   **默认就不给商标权**——所以「叫 graphify」依然不行，与 Vorssaint 同理。
3. **它 121k star 是 2026-04-03 创建的**，到 2026-09-26 只有不到 6 个月。
   **这意味着它的 API 与 skill 格式都在剧烈变动**（tree 里的 `BENCHMARKS.md` 标注
   "Last updated: 2026-07-05"，README 里的 issue 引用已到 #3511、#3558）。
   **抄它必须抄「形状」而不是抄「具体哪一行」**——我们抄 hook guard 的五道闸门形状是可以的，
   抄它的 `_HOOK_SOURCE_EXTS` 元组是没有意义的（它的清单会变）。

---

## 9. E. 明确不抄的

1. **「把代码库建成图」这件事本身，对我们过度。**
   我们的产品是**监控 agent 运行状态**，Session 类信息来自日志与进程，用不到符号图。
   graphify 的核心价值（god nodes / Leiden 社区 / shortest path 查询）建立在
   「人要在陌生大代码库里导航」这个场景上——我们自己的仓库规模
   （`swift build` 项目，未见 monorepo 化）根本不触发这个场景。**建图只是它的手段，
   它真正值得我们学的是「用机制保证 agent 行为」与「诚实标注推断」两件事，
   这两件事都可以脱离图单独存在。**
2. **不给我们的 skill 加 `references/` sidecar。** graphify 的 13 平台矩阵 + skillgen
   是因为它要同时服务 20+ 个 IDE；我们只有 Claude Code 一个 host，一份 `SKILL.md` 足够。
   加机器生成层只有成本没有收益。
3. **不引 `confidence_score` 浮点 rubric。** 见 §4.3——我们的 provenance 是确定性来源标记，
   不是置信度。加了就要回答「UI 上怎么显示 0.75」。
4. **不抄它的并行 subagent 抽取形态。** `skill.md:246` 的 `MANDATORY: You MUST use the
   Agent tool` 是为 LLM 建图服务的。我们没有 LLM 抽取阶段。
5. **不抄 `graphify reflect` / work-memory overlay。** README:1025-1031 那套
   （save-result → reflect → `.graphify_learning.json` 的 preferred/tentative/contested 标记）
   是「让 agent 记住哪些查询有用」的长期记忆层。**它的 `contested` 标记有意思
   （与我们的 conflict 遥相呼应），但那是产品级功能，我们今天的 workbench 文档已在做
   同样的事（人写的，不靠机制）。**
6. **不引它的任何依赖。** 它是 Python 包（uv/pip/pipx），我们是 SwiftPM。即使 Apache-2.0
   允许，跨语言 embed 一个 239KB 的 CLI 也不成立。

---

## 10. 应用建议清单（与实现解耦，尚未排期）

按优先级，全部有 graphify 对应实现：

1. **改 [scripts/install-git-hooks.sh](../../scripts/install-git-hooks.sh) 为 marker + append**
   （照 `hooks.py:628-653`）：有别人的 hook 就追加而不是 `exit 1`；
   marker 用 `# agentisland:scan-secrets`（现在已经是 marker，但只用做检测不用做定位替换）。
   **同时把两块逻辑都包进 `( ... )` 子 shell**（照 `hooks.py:372-380`），
   否则未来加第二个 hook 时第一个的 `exit 0` 会吞掉扫敏。
2. **给 `.git/hooks/pre-commit` 加 rebase/merge/worktree 短路**（照 `hooks.py:405-410`）：
   `rebase-merge` / `rebase-apply` / `MERGE_HEAD` / `CHERRY_PICK_HEAD` 存在就跳过扫敏。
3. **把一条纯文档纪律变成 at-most-once 的提醒**（照 strict 的最小版）：
   在 pre-commit 或 CI 里检「改了侧边栏相关源码但没动 `docs/workbench/01-current-state.md`」，
   只打印警告不 exit 1。**判据要 fail-open**。
4. **`.claude/settings.json` 里若将来装 hook，一律带 `timeout: 10`**（照 `install.py:352-356`）
   ——默认 600s 是 Claude Code 的陷阱。
5. **skill 生成物防手改检查**（照 `.pre-commit-config.yaml` + `gen.py --check`）：
   至少加一条 `.claude/skills/libraries-dev` 软链目标一致的检查。
6. **写外发性能数字时按 BENCHMARKS 四条办**：不信任对手自报、judge 自身要验证并公开、
主动标出不可比维度、给自己的卖点做消融。
7. **长期（Phase 2）**：若 `provenance` 要细分 `inferred`，加 case 不加 score，
且细分依据是「有没有已知负载能解释这个 CPU」而不是任何浮点模型。

---

## 11. 未核实清单

1. **AMBIGUOUS 的实际产量未核实**。全仓 grep 确认它**只由 LLM 语义 pass 产生**
   （`llm.py` 的 prompt + `extraction-spec.md`），**AST 侧零产出**；但
   「一个真实 corpus 跑下来 AMBIGUOUS 占多少比例」未实测——`report.py:361` 的
   `amb_pct > 20` 阈值暗示它在实践中可能很高，但这只是阈值不是观察数据。
   BENCHMARKS 与 how-it-works 都没给分布数字。
2. **strict 模式在真实 Claude Code 会话中的体感未实测**：本文的全部结论来自
   `cli_full.py` 源码 + 202 行 pytest，**没有真跑过一次 `/graphify` + strict hook**。
   `deny` 后 agent 是否真的会照 `permissionDecisionReason` 去跑 `graphify query`，
   只有间接证据（`_query_stamp_fresh` 的存在说明它预期会跑）。
3. **`graphify install --project` 在 `--platform agents` 下写什么、与 mattpocock 的
   `.agents/invocation.md` 规范是否冲突，未逐字核对**。README:292 说它写
   `./.agents/skills/`，但**skill frontmatter 是否带 `disable-model-invocation`
   或等价声明没读**（本文读了 `skill.md` frontmatter，只有 `name` + `description` 两行，
   没有 invocation 元数据）。**它与 mattpocock 上游规范的一致性因此未判定。**
4. **我们的 skill 分发形态细节未核实**：`.claude/skills/libraries-dev` 是软链这一点
   来自 [AGENTS.md:78](../../AGENTS.md#L78) 自述，**未直连 `ls -l` 验证**；
   §10.5 那条检查的可行性依赖这个事实。
5. **`docs/how-it-works.md` 自称 "25 languages supported"**，而 README:510 自称
   "37 tree-sitter grammars"，README:511 的扩展名清单更长（含 `.f90` 等 Fortran 系列）。
   **三处口径不一致，以源码 `extractors/` 目录实测 30 个 extractor 文件为准，但每个
   extractor 覆盖的语言数未逐个统计。**
6. **BENCHMARKS 的可复现性未验证**：它说 `python memory/runner.py ...` 可以复现，
   但 `memory/` 与 `crosstool/` **不在 `git/trees/v8?recursive=1` 的 331 条路径里**
   ——即 benchmark harness 的代码要么在别的分支/仓库，要么未推送。
   **这是它 benchmark 章最大的一个洞：命令给了，代码不在这个 repo 里。**
7. **CI 三个 workflow 未逐行读**：`ci.yml` / `publish.yml` / `release-graph.yml` 只从
   README:1196、:1222 与 `gen.py` 的实际检查点反推内容。
8. **graphify 自己的 `.git/hooks/` 实践未读**（它用 pre-commit framework，
   `.pre-commit-config.yaml` 读了全文只 22 行，但 `graphify hook install` 生成的
   post-commit / post-checkout 脚本本文读了全文，两者是不同东西，不矛盾）。
9. **它 1477 个 open issue 的标题与内容未读**（只从 API 拿计数）。
   它的已知限制清单因此**只有 README Troubleshooting 一节 + 源码注释里那些
   `#<issue>` 引用**（#1161 nohup、#1840 stale、#1939 prompt 归属、#2166 空格路径、
    #3129 committed config 路径、#3511 unclassified、#3558 shown+thin）。
   这些 issue 编号密集（本文引到的最高 #3558）说明**它在快速修 bug，
    但对我们的启示只有形状，没有可直接抄的具体补丁。**
10. **LICENSE / LICENSE-MIT / NOTICE 三份只读了头部与 NOTICE 全文**：
    Apache-2.0 正文 11,358 字节只核对了首行，**专利条款与 NOTICE 传递义务的精确措辞
    未逐条引**。§8.1 的表格依据 Apache-2.0 的通用性质与 NOTICE 原文（5 行）写成。
11. **我们没有真机跑过 graphify**（未安装、未试 `graphify query`）。
    本文是纯文档任务，不跑 build/test。

---

## 12. 取证命令

```sh
# GitHub API 元数据（2026-09-26）
curl -sS "https://api.github.com/repos/Graphify-Labs/graphify"
# 全量 tree（默认分支 v8）
curl -sS "https://api.github.com/repos/Graphify-Labs/graphify/git/trees/v8?recursive=1"
# 源码原文（raw 对 >200KB 文件会截断，必须用 git/blobs + Accept: raw）
curl -sS -H "Accept: application/vnd.github.raw" \
  "https://api.github.com/repos/Graphify-Labs/graphify/contents/graphify/cli.py?ref=v8"
curl -sS -H "Accept: application/vnd.github.raw" \
  "https://api.github.com/repos/Graphify-Labs/graphify/contents/tools/skillgen/gen.py?ref=v8"
```

**一条可复现的教训记在这里**：`raw.githubusercontent.com` 对 `cli.py`（239,092 B）
与 `gen.py`（61,959 B）**返回的是截断后的 39,709 字节且 HTTP 200、无截断标记**——
我第一次 `curl` 到的 `cli.py` 只有 949 行、缺 `_target_is_indexed` 的函数体，
而 `contents` API 报的 size 是 239,092。**换 `git/blobs/<sha>` + `Accept: application/vnd.github.raw`
才拿到全量。** 大文件只信 `contents` API 的 `size` 字段。

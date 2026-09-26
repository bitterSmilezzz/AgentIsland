# 04 · 实施计划

> **当前执行门禁**以 [ADR 0010](../adr/0010-swift-freeze-and-rust-prerequisites.md) 的 M1–M5 为准。
> 下列 Phase 1–4 是最初的功能实施草案，不能跳过 Rust 测试与能力迁移直接开侧边栏。
> 当前：Phase 0 的扫描器及七条守护已入库，Rust 五态、三方言 fixture 与
> `provider.rs` 原子写/掩码测试使 M1 完成；M2 本机工作区/打包已达成。
> M3 已从已有档案的口径差异着手；ZCode 会话路径经本机实样核实并修正，
> Trae/Windsurf 待实样。12 个缺失模块仍待逐个迁入。
> `scripts/release.sh` 现在执行脱敏守护、`cargo test --locked` 和 Swift 测试。

## Phase 0 — 修复不可信的发版门禁（✅ 已完成）

脱敏闸门 `scripts/scan-secrets.sh` 有两处失败静默，症状都是「什么都没扫却打印 ✓」。
详见 [01-current-state.md](01-current-state.md) §6.1。

**改动**

1. 新增 `put()` 作为所有中间文件的写入出口：先写 `.new` 再 `mv`，每步失败 `exit 3`。
2. 新增 `load_file_list()`：接住 git 退出码、拒绝空列表。
3. `sort -u "$HITS" -o "$HITS"` 改走临时文件（in-place 排序会触发 SIGBUS）。
4. 基线文件缺失不再等价于「零命中」，改为拒绝放行。
5. 命中行的掩码前移到写盘前，`HITS` 里永不出现密钥原文。

**新增测试**：[scripts/test-scan-secrets.sh](../../scripts/test-scan-secrets.sh)，7 条守卫用例：
干净工作区必须通过（对照基准）／git 退出码非零／扫描中途 SIGKILL／结果文件不可写／
文件列表为空／基线缺失／新增未备案命中仍要拦住。

**验收结果**

| 项 | 结果 |
| :--- | :--- |
| 干净工作区 | ✓ 18 条命中零新增，返回 0 |
| 544 项 Swift 测试 | 544 通过, 0 失败 |
| 闸门守卫测试 | 7 通过, 0 失败 |
| 反向验证 | 把被测脚本换回修复前版本，第 1 条用例即 `Bus error: 10` 崩溃 |

**文档**：README 构建段补入 `./scripts/test-scan-secrets.sh` 入口。

---

## Phase 1 — 侧边栏壳 + 双形态并存

1. `settings.rs` 增 `shell_mode: "island" | "sidebar"`，默认 `island`。
2. `tauri.conf.json`：保留现有 island 窗口配置不动，**新增** `sidebar` 窗口。
3. 新增 `app/ui/css/sidebar.css`：两栏骨架 + 侧栏导航 + 极简首层；复用 `tokens.css`，不新增色值。
4. 新增 `js/main.js` 的形态分派与侧栏导航；模块按需 `import()` 懒加载。
5. `views.js` 重构：`renderCard`/`pageAnalytics`/`pageAgentDetail` 抽成不依赖容器尺寸的
   **监控模块**，island 形态包一层卡片、sidebar 形态直接铺进内容区。
6. `placement.rs` / `main.rs` 按 `shell_mode` 分派定位策略：island 分支零改动，
   sidebar 分支新增「贴左/右 + 记忆宽度」。
7. `island.css` 拆成 island 专属（notch / sliver / edge-top）与共用组件两部分，
   专属部分只在 island 壳下挂载。

**验收**

- 默认启动仍是灵动岛且行为零变化
- 切到 sidebar 后为侧边栏，首层极简
- 「高级设置」能进到既有分析页与详情页，功能不变
- 既有 544 项测试无回归
- 两种形态反复切换 10 次：无窗口残留、无样式串味

---

## Phase 2 — Codex 配置档位（原 CC Switch 草案收窄）

1. `provider.rs`：`scan_tools()` / `list_profiles()` / `save_profile()` /
   `apply_profile()`（原子写 + 备份）/ `delete_profile()`。
2. `main.rs` 注册 command，DTO 强制掩码。
3. 前端页面：档位列表 + 当前生效标记 + 切换确认 + 备份还原。
   **仅在 sidebar 形态下可达**（island 形态的 372×520 卡片装不下）。
4. 第一阶段只实现 `Tool::Codex`；其他工具待单独核实并立项。
5. 测试：原子写失败不留半截文件、密钥绝不出现在 DTO、备份-还原往返一致。
6. **界面必须写明能力边界**：「切换同厂商多账号，不含跨厂商模型」——否则用户切了档
   以为能用 Kimi，实际不能用。这是 Magpie 调研的直接结论，不能只写在文档里。
7. 学习 Magpie 被点名的一项工程实践：**原子写时保留配置文件的注释、顺序与缩进**
   （`settings.json` / `config.toml` / `config.yaml`）。朴素的 JSON 序列化会把用户手写的
   格式冲掉，这是会被用户立刻察觉的破坏。

**验收**

- 改一个 Codex 档位的配置片段 → 应用 → `~/.codex/config.toml`
  内容正确，且原始内容可从备份还原；生效需重启 Codex 进程并给用户提示
- DTO 里搜不到任何 4 位以上连续密钥形状串
- 只做 Codex 一家，不为其他工具画未实现的档位 UI
- 配置文件的注释与缩写在切换后仍然保留
- 界面上能看到能力边界说明

---

## Phase 3 — ToDos 模块

1. `todos.rs`：`list` / `add` / `toggle` / `remove` / `clear_done`，原子写。
2. 前端：极简列表，回车加条，勾选划掉，无分组无日期。
3. 侧栏图标带未完成计数。
4. 测试：损坏 JSON 不 panic（降级为空清单）、原子写、计数口径。

**验收**：增删改查往返一致；把 json 写坏后重启 App 不崩且列表为空。

---

## Phase 4 — 收尾

1. 文档同步：README / CONTEXT / CHANGELOG。README 里所有「灵动岛」表述从
   「唯一形态」改成「形态之一」，逐条核对，不留已不成立的描述
   （AGENTS.md：过期的限制比没有限制更误导）。
2. `docs/adr/0009-sidebar-alongside-island.md`：记录为何两种形态并存而非二选一、
   各自适用面（island = 不打断注意力的被动感知；sidebar = 键盘可达与信息容量）、
   以及 `shell_mode` 默认 island 的理由。
3. `scripts/scan-secrets.sh --release`，按 AGENTS.md 走 扫描 → commit → tag → release。
4. `docs/code-review/` 写本轮独立 review。

---

## 风险

| 风险 | 影响 | 对策 |
| :--- | :--- | :--- |
| **泻密**：Provider payload 含真实 key | 最高危 | DTO 掩码层做成结构断言（有测试盯着）；密钥只走文件不走 invoke 响应 |
| **写坏用户配置** | 高 | 切换前必备份 + 原子 rename；失败即回滚 |
| **扫描器假绿持续存在** | 中 | Phase 0 已修并有 7 条守卫测试盯住 |
| **双形态导致 CSS / 定位代码翻倍** | 中高 | island 分支零改动；共用组件抽到独立文件；两壳不共根容器 |
| **`views.js` 抽取时破坏 island 首屏** | 中 | 默认 `shell_mode=island`；重构后先在 island 下跑通 544 项再开 sidebar 开关 |
| **CC Switch 工具集扩张失控** | 中 | 第一阶段只做 Codex；其他工具待独立核实 |
| **构建门槛** | 低 | README 前置条件已写 `SDKROOT=MacOSX26.5.sdk` |

## 开放项

1. **macOS 本体迁移方向已定**：[ADR 0010](../adr/0010-swift-freeze-and-rust-prerequisites.md)
   要求前置门齐备后逐模块迁到 Rust；Swift 仅保留 Bug、安全与必要兼容修复。
2. **`shell_mode` 默认值是否要为「新用户默认 sidebar」加版本迁移逻辑**：
   当前一律默认 island，最安全但不能让新用户直接看到侧边栏。
3. **灵动岛形态下 Provider / 待办不可达**：这是有意的形态分工（372×520 装不下），
   但需在文档里写明，否则会被当成缺陷报上来。
4. **是否与 Magpie 共存**：第一版只做配置切换层。做完后再评估让工作台检测/启动本机
   Magpie 并显示各 agent 当前模型（见 [05-magpie-research.md](05-magpie-research.md) §4.1）。
   前提是核实 Magpie 的 CLI 输出是否稳定可解析、配置与令牌位置、端口是否可配——
   **这些尚未核实**，要读它的源码才能定。
5. **若将来要做网关**：那是独立的大改造立项，不是当前 Phase 的延伸。
   它会让本产品从「监控 Agent」变成「转发 Agent 的全部流量」——产品性质变了，
   凭据口径与资源占用预算都要重新论证。

## 待办（来自 OpenSquilla 调研，不阻塞 Phase 1）

见 [06-opensquilla-research.md](06-opensquilla-research.md) §4。

1. **`LocalEventServer` 补 Guest 降权三原则**：认证失败与未认证给同一套权限（不给"差一点的
   凭据"留半开的门）；远端连接不进审批队列（待审批项不能成为提权通道）；
   边界在所有执行面一致生效。这是本项目已知的安全短板
   （`Sources/AgentIsland/LocalEventServer.swift:10-15`：`/notify` 无鉴权、
   不设 `requiredLocalEndpoint` 时实际监听通配）。
2. **脱敏清单补上下文敏感项**：现有规则覆盖密钥 / 本机路径 / 邮箱 / 手机号，
   **不含客户名、项目名、channel 标识**。分享诊断或导出 session 前，这类信息同样要清。

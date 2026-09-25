# 03 · 技术方案

## 1. 分层

```
app/src-tauri/src/
├── 既有（零改动）   engine.rs  filemon.rs  session.rs  tokens.rs
│                    procmon.rs webhook.rs registry.rs models.rs
├── 新增             provider.rs   CC Switch：配置档读写 / 切换 / 原子写 / 备份
│                    todos.rs      ToDos：本地清单存储
└── 改写              main.rs      窗口壳 + command 注册 + 模块编排
                      placement.rs 按 shell_mode 分派定位策略
                      settings.rs  增 shell_mode 字段

app/ui/
├── 新增             css/sidebar.css  侧边栏骨架、导航、极简首层
│                    js/provider.js   CC Switch 页面
│                    js/todos.js      待办页面
├── 复用             css/tokens.css   设计令牌（零改动）
├── 拆分             css/island.css   → island 专属 + 两边共用组件
└── 改写             js/views.js      卡片渲染抽成「监控模块」，双形态共用
                      js/main.js      读 shell_mode，装载对应壳
```

## 2. 双形态并存（核心设计）

### 2.1 对照

| | 灵动岛（保留） | 侧边栏（新增） |
| :--- | :--- | :--- |
| 窗口 | 372×520，`focus: false`，透明无边框 | 窄高可拉伸，`focus: true`，半透明无边框 |
| 定位 | `placement.rs` 四边吸附 + 轴向锚点 | 贴左 / 贴右 + 记忆宽度 |
| 收起态 | 6pt 微细条 + 呼吸绿灯 | 折叠为纯竖条图标轨 |
| 键盘输入 | 不接 | 接 |
| 模块 | 仅监控（列表 / 分析 / 详情） | 监控 + Provider + 待办 + 高级设置 |
| 样式 | `island.css` | `sidebar.css` + 共用组件 |

### 2.2 切换点

`settings.rs` 增 `shell_mode: "island" | "sidebar"`，**默认 `island`**。
`main.js` 启动时读取，装载对应壳。模块页（`views.js` 抽出的部分）两边共用。

> **为什么默认 island**：老用户首屏零感知。若希望新用户直接看到侧边栏，
> 需要额外的「版本迁移」逻辑（老用户读旧设置、新用户默认 sidebar）——列为开放项。

**模式名只用一代**（借自 OpenSquilla 文档的做法：*"Older CLI mode names remain accepted only
as an upgrade compatibility shim and are not shown in the current UI"*）。
对外只呈现当前这一代的模式名；将来若出现第三种形态，旧名降级为后台兼容读取，
**不许同时挂在 UI 上**——否则界面上三代叫法并存，用户无从判断哪个有效。

### 2.3 CSS 隔离（硬要求）

`.edge-top`、`--notch-inset`、sliver 等 island 专属规则**只在 island 形态下挂载**。
做法：两个壳各自持有独立根容器与样式表，**不共用 `#root`**。
否则侧边栏会出现 notch 预留缩进、顶部倒角等串味布局。

## 3. 侧边栏信息架构

```
┌──────────────┬─────────────────────────┐
│ ◆ AgentIsland│  [极简首层]              │
│              │                          │
│ ▸ 监控   3   │  当前活动 Agent 一览       │
│ ▸ Provider   │  + 一行数摘要             │
│ ▸ 待办   5   │  + 「高级设置 ›」入口      │
│              │                          │
│ ⚙ 高级设置    │  ← 详情入口              │
└──────────────┴─────────────────────────┘
```

- **首层（默认）**：只放监控摘要。Agent 不工作时连列表都收起，只留呼吸灯。
- **侧栏图标带计数**：监控 = 活跃数，待办 = 未完成数，Provider = 当前档位名。
- **高级设置**：朴素入口，点进去才有 token 分析、异常扫描、各 Agent 详情、远程通知、
  自定义档案等既有重页面。

## 4. CC Switch 模块（`provider.rs`）

> **能力边界（先说清）**：这一层只读写 agent 的本地配置文件，**不架网关、不做协议翻译**。
> 所以它能切的是**同厂商的多套 key/账号**；想让 Claude Code 用上 DeepSeek/Kimi 这类跨厂商模型，
> 需要的是会说 Anthropic Messages 的本地网关（Magpie 做的那件事），本版不做。
> 完整调研与代价对比见 [05-magpie-research.md](05-magpie-research.md)。

### 4.1 档位模型

```rust
pub struct ProviderProfile {
    pub id: String,                 // "claude-kimi" / "codex-official"
    pub tool: Tool,                 // Claude | Codex | (预留 Gemini | OpenCode)
    pub name: String,               // 展示名
    pub payload: serde_json::Value, // 该工具的配置片段
    pub source_path: PathBuf,       // 来源文件
    pub created_at: i64,
}

pub enum Tool { Claude, Codex, Gemini, OpenCode }  // 后两者仅作档案预留
```

### 4.2 命令面

`scan_tools()`（探测本机有哪些工具存在配置文件）、`list_profiles()`、`save_profile()`、
`apply_profile()`（原子写 + 备份）、`delete_profile()`。

### 4.3 安全约束（最高优先级）

继承项目既有凭据口径（`CONTEXT.md`「凭据口径」一节）：

- `payload` 里若含 key / token / 授权码，**一律不进返回给前端的 DTO**，只回掩码
- `auth.json` / `settings.json` 的密钥字段只写盘读盘，**零透传到 UI、零进日志**
- 切换前**原子写**（临时文件 + `rename`），失败不改动原文件
- 切换前自动备份当前配置为 `<name>.bak`，可一键还原
- 所有 Provider 命令走同一 invoke 通道，无网络访问

> **机械守卫**：掩码层要做成**结构断言**（像本仓已有的 `?? 0` 禁用写法那样有测试盯着），
> 而不是靠「记得加」。测试断言 DTO 里搜不到任何 4 位以上连续密钥形状串。

### 4.4 第一阶段范围

只实现 `Tool::Claude` 与 `Tool::Codex`。枚举里留 `Gemini` / `OpenCode`，
但 `scan_tools()` 对它们返回「本机未发现配置」，**不宣称支持**。

## 5. ToDos 模块（`todos.rs`）

- 存储：`dirs::data_dir()/agentisland/todos.json`（与 `settings.rs` 同源路径策略）
- 命令：`list` / `add` / `toggle` / `remove` / `clear_done`
- 条目只含标题 + 完成态 + 创建时间
- **不做**项目分组、标签、截止日期（轻量优先，需要再说）
- 原子写；损坏 JSON 时降级为空清单，不 panic

## 6. 性能预算（本版硬约束）

**判据不是 CPU 数字，而是成本-质量前沿。**「省一次」必须按它可能漏掉的那次状态转移的
下游成本计价，而不是按省下的账面值——这条口径见 `CONTEXT.md`「省一次要按下游计价」
（借自 OpenSquilla 的 harness-native routing 思想，见 [06-opensquilla-research.md](06-opensquilla-research.md)）。
另一条相邻思想来自 [08-chendahuang-cloudflare-research.md](08-chendahuang-cloudflare-research.md)：
**容量曲线是脉冲的，就把峰值交给有弹性的底座，业务代码自己不写守护进程**——我们这边没有
边缘云可交，等价物是分档轮询 + 只在真正需要时启动引擎，而不是自己养一个常驻调度器。

| 项 | 预算 | 依据 |
| :--- | :--- | :--- |
| 侧栏收起时 CPU | ≤ 0.1% | 现状 mac 端 Docked 态 0.0–0.1% |
| Token 轮询 | 仅在「监控页展开」时运行 | 复用既有 `presentation active` 口径 |
| Provider / ToDos | **惰性装载**，切到该页才 invoke | 纯静态文件操作，无常驻必要 |
| 引擎循环 | 维持 2s 有活动 / 5s 闲置降频，不因新增模块提速 | `main.rs` 现有逻辑 |
| 任何新增的"省" | 必须说明漏一次状态转移的代价 | 上述口径；漏答即不该做 |

## 7. 关键取舍

| 取舍 | 选择 | 代价 |
| :--- | :--- | :--- |
| 形态 | 双形态并存 | `placement.rs` / CSS / 壳代码需按形态分派，总量上升 |
| CC Switch 范围 | 只做 2 家 | 另外 7 家暂不可用 |
| ToDos 字段 | 最小集 | 无分组无日期 |
| 默认形态 | island | 新用户看不到侧边栏，需手动切 |
| Rust 核心 | 不改 | 新模块须适配既有模型，而非让模型适配新模块 |

# Magpie 融合 — 调研与取舍

> 一手核实记录。来源：[usemagpie.ai](https://usemagpie.ai/) 官方首页全文（2026-09-25 抓取），
> 开源仓库 [github.com/yetone/magpie](https://github.com/yetone/magpie)（MIT）。
> 本文只记录「它是什么」「和我们已有能力的关系」「融合的代价」，不记录它每一版改了什么。

## 1. 它是什么（核实过的部分）

Magpie 的自我定义是 *"Every agent's model. One place."* —— 让任意 agent 用任意模型。
它不是「配置文件切换器」，而是**跑在本机的一个 LLM 网关**。

### 1.1 架构

```
用户的 agent（Codex / Claude Code / Gemini CLI / OpenCode / Goose …）
        │  各说各的协议：Responses / Messages / Gemini / Chat
        ▼
magpie gateway  127.0.0.1:3425
        │  双向翻译：OpenAI Chat·Responses、Anthropic Messages、Google Gemini
        │  含流式、tool calls、reasoning
        ▼
任意 provider（DeepSeek / Kimi / GLM / Qwen / OpenRouter / Ollama / 你自己的订阅）
```

关键点：**它拦在所有请求中间**。改写 agent 配置只是入口，真正的活儿是协议翻译与请求路由。

### 1.2 五块能力

| 能力 | 站点原文口径 |
| :--- | :--- |
| **协议网关** | 在 `127.0.0.1:3425` 起服务，代理 OpenAI Responses / Anthropic Messages / Google Gemini，双向翻译，含流式、tool call、reasoning |
| **模型选择器** | 把第三方模型注入 Codex 自己的 `/model` 列表；把 Kimi/GLM/GPT 映射到 Claude Code 的 `opus`/`sonnet`/`haiku` 后面 |
| **订阅共享** | 已登录 agent 的认证变成 provider，供其他 agent 使用，无需复制密钥 |
| **智能路由** | 多账号多 key，按「重置时间最近者优先 / 余额耗尽者退场 / 按厂商 Retry-After 与重置头跳过 / 失败退避重试」调度 |
| **用量统计** | 按 agent 与模型统计 token、cache 命中与成本 |

### 1.3 工程特征（站点自述）

- 原生、小于 15 MB、用系统 webview，无 Electron，MIT 开源
- **原子写配置**：`settings.json` / `config.toml` / `config.yaml`的注释、顺序、缩进都保留
- **不读环境变量里的密钥**，只用手动添加的
- 任何有 base-URL 设置的对象都能接：`OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` / `GOOGLE_GEMINI_BASE_URL`
- 同时提供 GUI、TUI 与 CLI

## 2. 和我们已有能力的关系

### 2.1 重叠的部分：只占它的表层

| 我们的计划（`provider.rs`） | Magpie |
| :--- | :--- |
| 保存 / 切换 / 还原**本地配置文件** | 也做，且**保留注释与缩进**地原子写 |
| 支持 Claude Code + Codex 两家 | 15 个 agent、14 个 provider、20 个预设 |
| 无网络，纯本地文件操作 | **拦全部请求**，做协议翻译与路由 |

### 2.2 我们已有的、它没有的

本项目独有、且与网关**没有冲突**的资产：

- **Agent 运行状态监控**（五态状态机、会话方言尾读、CPU 差分）
- **Token 净消耗统计**——但口径不同：我们读 agent **写在本机**的会话日志；
  Magpie 统计的是**流经它自己**的请求。前者不依赖用户走网关，后者只有走了网关才有数。
- **可观测性五类结论**、死锁三态、「没查到 ≠ 零」
- 远程通知、在场判定、异常扫描

### 2.3 我们已有的、但和它**重叠且会冲突**的

**`LocalEventServer`（127.0.0.1:41999）** 已经在跑一个本机 HTTP 服务。
Magpie 的网关在 `127.0.0.1:3425`。两者端口不同、职责不同，但——
我们已有一次因为「只监听回环」收紧过的安全记录（`LocalEventServer.swift:10-15`：
`NWParameters` 不设 `requiredLocalEndpoint` 实际监听通配，局域网可伪造事件）。
**再加一个 HTTP 服务，就是把这条已知的坑再挖一遍。**

## 3. 融合的真实代价（这是决策依据）

### 3.1 工作量差一个数量级

| 方案 | 内容 | 量级估计 |
| :--- | :--- | :--- |
| **A. 只做配置切换**（原计划） | 读写 `settings.json` / `config.toml`，存档位、切档位 | 数百行 Rust + 一个前端页 |
| **B. 做完整网关** | 三种 API 协议的双向翻译（含流式、tool call、reasoning）+ 多账号调度 + 失败退避 + 用量统计 + 17 家 agent 的配置适配 | **数千行 Rust，且是长跑服务** |

### 3.2 A 方案的根本局限（必须写清楚）

**只改配置，无法让 agent 用上第三方模型。**

因为 agent 只会说自家的协议：Codex 只说 OpenAI Responses，Claude Code 只说 Anthropic Messages。
不架网关做翻译，就没有任何路径把一个 DeepSeek/Kimi 的 key 接到 Claude Code 上。

`ANTHROPIC_BASE_URL` 这类开关指向的**必须是一个会说 Anthropic Messages 的东西**——那就是网关本身。
所以「配置切换」最终只能切**同一厂商的多套 key/账号**，跨厂商模型这条路走不通。

### 3.3 B 方案的风险

| 风险 | 说明 |
| :--- | :--- |
| **它会代理用户的全部 API 流量** | 密钥、请求内容、模型响应都流经我们的进程。本项目 `CONTEXT.md` 的凭据口径是「钥匙串 + 零日志」，一旦架网关，凭据就必须在内存里参与签名计算 |
| **它是常驻在线服务，与「低资源占用」目标直接冲突** | 现在的引擎是全静默时 5s 一拍；网关只要有 agent 在跑就持续处理流式响应 |
| **协议翻译是深水区** | tool call 与 reasoning 的语义在三种协议间**不对等**。翻错的症状是 agent 静默丢工具调用、或 reasoning 块错位——且只在特定模型组合下出现 |
| **与 `--help` 声明的定位漂移** | 从「监控 Agent」变成「转发 Agent 的流量」，产品性质变了 |
| **Windows WPF 对照端无法承担** | `windows/` 保留作功能偏离对照，网关这种长跑服务 WPF 端做不到 |

## 4. 三个可选路径

| 路径 | 做法 | 代价 | 得到什么 |
| :--- | :--- | :--- | :--- |
| **① 只做配置切换** | 原计划的 `provider.rs` | 小 | 多账号/多 key 一键切换，**不能**跨厂商用模型 |
| **② 架完整网关** | 自实现三种协议翻译 + 调度 | 大，且改变产品性质 | 真正的「任意 agent 用任意模型」 |
| **③ 与 Magpie 共存** | 工作台检测 / 启动 / 显示本机 Magpie 的状态与当前各 agent 的模型，网关本身交给 Magpie | 小到中 | 「一个面板看全」，且不重写协议翻译 |

### 4.1 关于路径 ③ 的可行性核实

Magpie 是 MIT 开源、原生小应用（<15 MB）。要「共存」需要确认：它的 CLI 能否被外部查询状态
（`magpie ls` 的输出是否稳定可解析）、配置与令牌放哪、端口是否可配。
**这些尚未核实**，需要读它的仓库源码后才能定。

## 5. 建议

~~**倾向 ③，但先做 ①。**~~

**已定（用户拍板）：先做 ①，之后评估 ③。**

理由：

1. ① 是 ③ 的前置——无论共不共存，「保存/切换 agent 配置」这层都是要的，且它能独立交付价值
   （多账号切换是真实痛点）。
2. ① 做完再评估 ③ 的接入成本，那时对「配置怎么写」已经有第一手经验，判断更准。
3. ② 不做：它改变产品性质、与低占用目标冲突、且协议翻译的失败模式难以排查。
   若未来要做，应作为独立的大改造立项，而不是塞进当前 Phase。

### 5.1 ①的局限必须对用户讲明

只改配置，**无法让 agent 用上第三方模型**。因为 agent 只会说自家的协议。
`ANTHROPIC_BASE_URL` 这类开关指向的**必须是一个会说 Anthropic Messages 的东西**——那就是网关。
所以配置切换层的实际能力是**同厂商多账号/多 key 切换**，不是跨厂商模型切换。
这一句要写进界面文案，不能让人以为切了档位就能用 Kimi。

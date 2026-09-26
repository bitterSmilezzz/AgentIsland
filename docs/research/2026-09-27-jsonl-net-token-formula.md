# JSONL 净 token 口径：本机实测两端口径差 29 倍

2026-09-27 核实。起因是准备把 `StructuredTokenUsageIndex` 的保留口径搬到 Rust 时，
逐行读 Swift 的 `netTokens` 发现两边公式不同，于是拿本机真实日志量了一次。
**结论：Rust 旧口径把缓存命中的上下文当新输入全额计，整批虚高 29 倍；已按 Swift 口径修好。**

## 取证实样

本机 `~/.codex/sessions/` 下 26 份 `*.jsonl`。两种记录族**并存**：

| 记录族 | 判定 | 本机文件数 |
| :--- | :--- | ---: |
| `type == "token_usage_record"`（Swift 认这一族） | 21 | |
| `event_msg.payload.type == "token_count"`（Rust 旧实现认这一族） | 22 | |
| **两者都有的文件** | **21** | |

即「换一族」并不能避开重复计数问题：同一份日志里两族同时存在，各认一族才是当前状态。

一条真实记录的字段值（token 计数不是敏感内容，记录在这里作为公式的证据）：

```
token_count.last_token_usage:
  { cache_write_input_tokens: 0, cached_input_tokens: 41728,
    input_tokens: 43449, output_tokens: 200, reasoning_output_tokens: 38,
    total_tokens: 43649 }
```

`input_tokens` 里 41728/43449 = **96% 是缓存命中**。

`token_usage_record.usage` 的键：`cache_write_input_tokens` `cached_input_tokens`
`input_tokens` `output_tokens` `reasoning_output_tokens` `total_tokens`（与上同形）。

## 四种组合分离「记录族」与「公式」

同一批 26 份文件，按 `(记录族 × 公式)` 四种组合求和：

| 记录族 | 公式 | 合计 tokens |
| :--- | :--- | ---: |
| `token_usage_record` | **Swift**：`max(input − min(cached, input), 0) + max(output, 0)` | **3,905,535** |
| `token_usage_record` | Rust 旧：`input + output + cache_write` | 114,006,271 |
| `token_count.last` | **Swift 公式** | 3,893,007 |
| `token_count.last` | Rust 旧公式 | 113,676,687 |

**记录族的影响只有 0.3%**（3,905,535 vs 3,893,007，条数几乎一一对应），
**公式的影响是 29 倍**。所以差别不在「读哪一族」，而在「怎么算净额」。

逐文件对照（差值最大的几个，均为同一文件内两族各自求和）：

```
tur=  758,343 (340条)   tc_last=45,155,740 (345条)   差=+44,397,397
tur=1,211,246 (257条)   tc_last=29,905,233 (257条)   差=+28,693,987
tur=  650,153 (169条)   tc_last=19,469,957 (169条)   差=+18,819,804
```

条数一一对应、数值差 20–60 倍 —— 与「同一条记录、两种算法」的形状一致。

## 取证命令（可复跑）

```sh
# 1) 两族各覆盖多少文件、是否并存
grep -rl '"type":"token_usage_record"' ~/.codex/sessions | wc -l
grep -rl '"type":"token_count"'         ~/.codex/sessions | wc -l

# 2) 四种组合求和（只打印数值与键名，不打印任何日志正文）
python3 - <<'PY'
import json, glob, os
files = glob.glob(os.path.expanduser('~/.codex/sessions/**/*.jsonl'), recursive=True)
def n(d,k):
    v=d.get(k); return int(v) if isinstance(v,(int,float)) else 0
swift_f = lambda u: max(n(u,'input_tokens') - min(n(u,'cached_input_tokens'), n(u,'input_tokens')), 0) + max(n(u,'output_tokens'),0)
rust_f  = lambda u: n(u,'input_tokens') + n(u,'output_tokens') + n(u,'cache_write_input_tokens')
tot = {('tur','swift'):0, ('tur','rust'):0, ('tc','swift'):0, ('tc','rust'):0}
for path in files:
    for line in open(path, encoding='utf-8', errors='replace'):
        if '"usage"' not in line and 'token_count' not in line: continue
        try: d=json.loads(line)
        except Exception: continue
        t=d.get('type'); p=d.get('payload') or {}
        u=None; which=None
        if t=='token_usage_record' and isinstance(p.get('usage'),dict):
            u=p['usage']; which='tur'
        elif t=='event_msg' and p.get('type')=='token_count':
            info=p.get('info') or {}
            if isinstance(info.get('last_token_usage'),dict):
                u=info['last_token_usage']; which='tc'
        if u is None: continue
        tot[(which,'swift')] += swift_f(u); tot[(which,'rust')] += rust_f(u)
for k,v in tot.items(): print(k, f"{v:,}")
PY
```

## 修好之后的端到端复核

Rust 实现改为「认 `token_usage_record` 族 + Swift 公式」后，
用同一个 monitor 直接跑本机真实目录（临时 `#[ignore]` 探针，验完即删）：

```
RUST tokens_total = 3905535
```

与上表 `token_usage_record × Swift 公式` 的 **3,905,535 逐字相同**；
修前同一路径会得到 114,006,271 那一档。这条对拍证明的是「实现与独立脚本同值」，
不是「公式本身更对」——后者是 Swift 侧既定的净消耗口径（`CONTEXT.md`「Token 用量」：
净消耗不含缓存读取），本次只是把 Rust 对齐过去。

## 附带确认的两个形状（同一批取证）

- **Codex 的 `token_usage_record` payload 里没有模型名**：键为
  `response_id` `root_turn_id` `session_id` `thread_id` `thread_token_usage`
  `turn_id` `turn_token_usage` `usage`。所以 Rust 侧模型名落 `unknown` 是如实的，
  此前写死的 `"gpt-5"` 是编的。
- **ZCode rollout 的真实形状**（`~/.zcode/cli/rollout`，19MB）：
  顶层键 `attempt` `completedAt` `durationMs` `model` `querySource` `request`
  `requestId` `response` `sessionId` `startedAt` `traceId` `turnId` `type`；
  `model.modelId` / `response.responseId` / `response.usage` 的键为
  `cacheReadTokens` `cacheWriteTokens` `inputTokens` `outputTokens` `totalTokens`。
  去重键取 `requestId`（兜底 `response.responseId`）。
- **本机没有 Claude Code 的 JSONL**：`~/.claude/projects` 下 `"cache_read_input_tokens"`
  零命中。所以 anthropic 方言这一路只有合成夹具覆盖，**没有本机实样**。

> 数据边界：本文件记录的是**数值与键名**，不含任何日志正文；取证命令一律只打印计数与字段名。
> 路径一律写 `~`，不写绝对家目录。

# R37 验证记录：状态真实性——打开应用没动不再误报完成与响铃（v0.0.54）

日期：2026-09-12 · 执行：主 agent
触发：用户报告「现在状态跟踪一直不对。有些软件打开了没动也算一次完成，会响铃通知」

## 取证（真机，两条独立成因）

### 成因 A：纯 CPU 区间补发完成事件

用真引擎探针（`ActivityEngine` + 真实 provider，每 2s 打印等级与事件）复现：

```
[t6]  chatgpt: idle → working  cpu=63.2%  活动=1m 前    ← 启动 CPU 尖峰，全程无写入信号
[t43] chatgpt: working → idle  cpu=0.9%
[t43] ★事件 completed chatgpt: ChatGPT 任务完成 (1分15秒) ← 用户听到的响铃
```

安安静静放着的 ChatGPT 反复出现 CPU 尖峰，每次尖峰都走完「working → idle」并补发完成事件与提示音：

```
[t71] chatgpt: idle → working  cpu=20.6%  活动=7m 前
[t76] chatgpt: working → idle  cpu=6.4%
[t76] ★事件 completed chatgpt: ChatGPT 任务完成 (10秒)
[t79] chatgpt: idle → working  cpu=43.4%
```

结论：`desktopCPUFloor=20` 挡不住桌面应用的启动/后台尖峰（实测 20.6%~93.1%），而完成事件此前只看「信号消失」不看「信号来源」。

### 成因 B：与应用任务无关的文件写入

1. **SQLite `-shm` 空转触碰**：Antigravity 空闲时 10 个会话库的 `*.db-shm` 在**同一秒**被批量触碰、size 恒为 32768 字节，而**主库 mtime 停在 22 小时前**——`conversations` 目录 22 小时内没有任何真实任务写入，引擎却因此判「刚刚写入 → working」（探针实测 `antigravity: idle → working  cpu=0.0%  活动=刚刚`）。`-shm` 是连接共享的 mmap 索引页，任何进程打开数据库（即使只读）都会刷新其 mtime
2. **浏览器内核用户数据目录**：Antigravity 仅被打开（无任何任务）时，20 分钟内 36 次写入全部落在 `Cache/Cache_Data`、`Code Cache/js`、`Local Storage/leveldb`、`Session Storage/*`、`GPUCache`、`DIPS`、`SharedStorage-wal`、`Network Persistent State`、`DevToolsActivePort`、`oauth_credentials.json`、`app_storage.json` 一类路径上——此前 `ignoredActivityPathComponents` 只有 `caches`（复数），单数的 `Cache` 与其余内核目录全部漏网，还会虚报「活跃会话」
3. **Sparkle 更新器**：ChatGPT 档案监控目录里只有 `production-appcast-bootstrap.json` 在动（更新检查），同样被当作活动

### 过滤口径的反向验证（避免误杀真实信号）

- `-wal` 保留：600 秒探针窗口内 `*.db-wal` 与主库**零变化**（真实写入才落 `-wal`），而 `-shm` 的触碰与任务无关
- 任务产物反例断言：`transcript.jsonl`、`state.json`、`session.sqlite`、`history.jsonl`、`*.db-wal`、`messages.json` 一律不得判为噪声

## 修复

1. **完成事件需写入证据**（`ActivityEngine.workingPeriodHadWrite`）：区间起点由写入驱动、或区间内出现过写入，才在收尾时发完成事件。纯 CPU 区间仍照常显示 working（**双信号判定契约不变**），但静默收尾——不响铃、不弹横幅/Peek、不进事件流
2. **文件噪声剥离**（`FileMonitor`）：`-shm` 侧车 + Sparkle appcast + 账号/状态文件 + 内核状态文件名（27 项）+ `BrowserMetrics*` 前缀族；忽略目录集扩充 45 项内核子树（Cache / Code Cache / GPUCache / Dawn 系列 / Session Storage / Local Storage / IndexedDB / Crashpad / blob_storage / DIPS / Trust Tokens / Singleton* 等）
3. **档案配置**：Antigravity 不再监控 `Library/Application Support/Antigravity`（内核用户数据目录），保留 `conversations` + `brain`
4. 证据集合在所有清理点（离线、睡眠断点、停止、终止/清理、启用集裁剪）同步清除，不跨区间泄漏

## 测试与变异验证

- 新增 6 用例：噪声名单（含反例）、`-shm` 与缓存子树不顶起 newest 且不计活跃会话、纯 CPU 区间不报完成、区间内有写入照常报完成（反向守卫）、证据不跨区间泄漏、档案不得把内核目录当会话目录（通用哨兵）
- **变异验证**：临时还原过滤与准入条件 → `196 通过 / 6 失败`（4 个新用例 + 2 个既有忽略集用例全红，反向守卫保持绿）；恢复后 **202/0 全绿**（连跑多次）

## 活体验证（真机）

| 场景 | 结果 |
|------|------|
| 假 Agent 进程「CPU 99.9% 烧 22s → 静置 40s」（无任何写入） | `idle → working 驱动=CPU(99.9%≥20)` → `working → idle` **静默收尾**，全程 `★事件` 数 = 0（修复前同形态补发完成事件） |
| 触碰全部 `*.db-shm`（模拟空闲刷新） | 无任何状态变化，Antigravity 保持 idle |
| 触碰全部 `*.db-wal`（模拟真实写入） | `antigravity: idle → working 驱动=写入(4s) 最新=*.db-wal` |
| 真实写入 `~/.vibe-usage/<probe>.json` | `idle → working 驱动=写入(1s)` → `★事件 completed vibe-usage`（真实工作照常响铃；探针文件已清理） |

## 发布

- 测试 202/0；打包重启（0.0.54）；CHANGELOG / README / version-mapping 三处版本哨兵通过

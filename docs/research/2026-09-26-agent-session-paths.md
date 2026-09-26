# ZCode / Trae / Windsurf 会话路径核实（2026-09-26）

## 本机实样与取证方法

只检查目录是否存在、文件类型与 JSON 键名；不复制文件名、会话正文、请求、密钥或工作区路径。
在本机用 `Path.home()` 拼下表的 `~` 路径，`Path.exists()` / `Path.iterdir()` 看目录，
对 ZCode 两个 JSONL 文件逐行 `json.loads()`，只累计顶层和 `response` 的键名及计数；
SQLite 用 `sqlite3.connect("file:…?mode=ro", uri=True)` 只查 `sqlite_master`、
`pragma table_info(tasks)` 和 `count(*)`。没有向原数据写入。

| 候选位置 | 2026-09-26 本机结果 |
| :--- | :--- |
| `~/.zcode/v2/checkpoints` | 不存在 |
| `~/.zcode/v2/tasks-index.sqlite` | 存在；`tasks` 表 1 行，含 `task_status` / `updated_at` 列 |
| `~/.zcode/cli/rollout` | 存在；1 个 JSONL、89 条合法记录；89 条都有 `response`、`response.toolCalls`、`completedAt`、`requestId`，也都有正数净用量 |
| `~/.zcode/cli/log` | 存在；1 个 JSONL、2221 条合法记录；顶层是 `event` / `level` / `message` / `timestamp` 等日志字段，没有 `response`；文件修改时间晚于 rollout |
| Trae / Trae CN 的 Application Support 根及 `/Applications` 应用 | 均不存在 |
| Windsurf 的 Application Support、`~/.windsurf`、`~/.codeium` 根及 `/Applications` 应用 | 均不存在 |

ZCode 的 rollout 顶层 `type` 为 `model_io`，`response` 含 `usage`、`toolCalls` 等键。
复查时可运行如下只输出汇总的取证命令；路径与键名是代码常量，绝不打印记录值：

```sh
python3 - <<'PY'
from pathlib import Path
import json, sqlite3
h = Path.home()
for rel in ['.zcode/v2/checkpoints', '.zcode/cli/rollout', '.zcode/cli/log',
            'Library/Application Support/Trae CN/User/workspaceStorage',
            'Library/Application Support/Trae/User/workspaceStorage',
            'Library/Application Support/Windsurf/User/workspaceStorage',
            '.codeium/windsurf']:
    p = h / rel
    print(rel, 'present' if p.exists() else 'absent')
for rel in ['.zcode/cli/rollout', '.zcode/cli/log']:
    for p in (h / rel).glob('*.jsonl'):
        records = responses = positive_net = 0
        with p.open(errors='replace') as stream:
            for line in stream:
                row = json.loads(line)
                records += 1
                response = row.get('response')
                responses += isinstance(response, dict)
                usage = response.get('usage', {}) if isinstance(response, dict) else {}
                if isinstance(usage, dict):
                    positive_net += sum(usage.get(k, 0) for k in
                                        ('inputTokens', 'outputTokens', 'cacheWriteTokens')) > 0
        print(rel, 'records', records, 'with_response', responses,
              'positive_net', positive_net)
db = h / '.zcode/v2/tasks-index.sqlite'
if db.exists():
    con = sqlite3.connect(f'file:{db}?mode=ro', uri=True)
    print('zcode_tasks_rows', con.execute('select count(*) from tasks').fetchone()[0])
    con.close()
PY
```

这与 [Rust `probe_zcode`](../../app/src-tauri/src/session.rs) 和
[token 解析器](../../app/src-tauri/src/tokens.rs) 所读结构相符。CLI log 比 rollout 新，
但没有该解析器需要的 `response`；把整个 log 目录当工作写入信号会让无关运行日志
有机会触发 `Working`。这是依据字段与当前监控规则得出的推断，未做持续空闲采样。

## 外部一手证据与边界

当前 [Cascade Hooks 官方文档](https://docs.devin.ai/desktop/cascade/hooks)
（旧 Windsurf 文档地址自动跳转至此）把 `~/.codeium/windsurf/hooks.json` 列为
IDE 用户级 **hook 配置**位置，并保留旧 Windsurf 路径兼容说明。
因此 Rust 目前监听整个 `~/.codeium/windsurf` 不能仅凭目录名证明是在监听会话；
这份文档也没有给出 Cascade 普通会话的工作区落盘位置。Trae 官方文档搜索未找到
可核对 `workspaceStorage` 路径的一手说明。两款应用在检查的标准位置均无实样，
本轮不猜路径。

## 对代码的结论

- ZCode：Swift 保留旧 `checkpoints` 路径，同时加入已验证的 `cli/rollout`；
  Rust 只以 rollout 作会话写入与 token 来源，去掉日志目录。Swift 的只读任务库不变。
- Trae：Rust 同时列 Trae 与 Trae CN 工作区，Swift 只列 Trae CN；缺实样，维持现状。
- Windsurf：Swift 列 Application Support 工作区，Rust 列 `.codeium/windsurf`；
  官方只证实后者有配置文件，尚不能裁定哪边是会话数据。维持现状并继续列入 M3 差异。

以上是单机当前安装状态，不代表所有平台、安装渠道和上游版本；不得把“目录不存在”
误写成厂商从未使用过它。

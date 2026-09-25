# OpenCode 一手调研（被监控对象专项）

> 一手核实记录。来源：[github.com/anomalyco/opencode](https://github.com/anomalyco/opencode)（MIT，TypeScript，
> 默认分支 **`dev`**）。GitHub API 元数据（2026-09-26）：**210,053 star / 27,765 fork / 创建于 2025-04-30 /
> 6288 open issues / homepage opencode.ai / 最新 release `v1.18.32`（2026-09-21）/ 仓库体积 538,821 KB**。
> 描述 "The open source coding agent."（README 正文写 "The open source AI coding agent."）。
> **本文与 `12-monocode-competitor-audit.md` 性质不同**：opencode 不是竞品参照，是**本仓要监控的头号对象**
> （`Sources/AgentIslandCore/AgentRegistry.swift:218-231` 已适配）。所以本文回答的是工程问题：
> 它把东西落在哪、什么格式、我们今天读到哪一层、还差什么。
> 所有引用按 `path:line` 给出，读不到的文件写「未获取」。**不记录它每一版改了什么。**

---

## 1. 一句话结论

**opencode 是一个 Bun 编译的原生 CLI（下游是同一个 SQLite 库）＋ 一个 Electron Desktop App（BETA）+ 一个后台
HTTP server，三者共用同一份落盘数据**：会话在 `~/.local/share/opencode/opencode.db`（WAL 模式），
消息/片段/用量分列在 `message` / `part` / `session` 三张表里。**我们今天对它的监控覆盖度判定：
"token 全、状态对、动作半对、进程对一半"**——四类里 token 一类完全命中（`message.data` 的
`tokens.*` 与 `cost` 字段名与我们的 SQL 逐字一致），状态机命中（`time.completed` 语义成立），
但**「当前动作」与「实时流水」两处读的 `part.data` 字段名 `type == "tool-call"` 与 `toolName`
在今天 1.18.x 的 schema 里已改名为 `type == "tool"` 与 `tool`**——这不是猜测，是逐版本对着源码核出来的
（见 §2.3、§7.2）。**该 bug 已于 v0.0.145 修复**：解析逻辑抽成纯函数
`AgentActionInspector.openCodeAction(fromPartJSON:nowMs:timeUpdatedMs:sessionTitle:)`，
同时接受新旧两种字段名（上游三个版本 `v1.0.180` / `v1.16.0` / `dev` 都是 `tool` + `tool`；
兼容旧名只为防上游改名），并补了 `state` 四态措辞与 4 条测试（548 全绿）。
**修复是从源码推出的，仍未经本机真库复现**——本机未装 opencode。**本机未装 opencode（`~/.local/share/opencode/` 不存在），因此这条判定是从源码推出的，
未在本机真库上复现。**

---

## 2. 「我们能监控到什么」

### 2.1 进程：CLI 二进制名对得上，Desktop 是 Electron、我们漏了引擎进程

**(1) CLI 的二进制名就是 `opencode`。**

- npm 包名 `opencode`（`packages/opencode/package.json`：`name: "opencode"`），
  `bin` 明确只有一个入口：`"opencode": "./bin/opencode"`（同文件）。
- 安装脚本 `install`（根目录，460 行 bash）把二进制装到 `~/.opencode/bin/opencode`
  （`install:68` `INSTALL_DIR=$HOME/.opencode/bin`；`install:343` `mv "$tmp_dir/opencode" "$INSTALL_DIR"`；
  `install:344` `chmod 755`）。**注意装的是"opencode"这个名字，仓库里的构建产物其实叫 `lildax`**
  （`packages/cli/script/build.ts`：`const binary = "lildax"`，产物 `./dist/<name>/bin/lildax`，
  其中 `<name>` 是 `lildax` → `cli` 替换后的名字。
  安装时被改名成 `opencode`，所以进程名是稳定的）。
- **`install` 脚本本身与 README 的说明不一致**：README 第 86–94 行列了四级优先
  （`$OPENCODE_INSTALL_DIR` → `$XDG_BIN_DIR` → `$HOME/bin` → `$HOME/.opencode/bin`），
  但 `install.sh` 全文 grep 不到 `OPENCODE_INSTALL_DIR` / `XDG_BIN_DIR` 这两个变量，
  `INSTALL_DIR` 是无条件写死的 `$HOME/.opencode/bin`（`install:68`）。
  **README 这一节描述的是未来的行为，不是今天脚本的行为。**
  （顺带：仓库根的 `install` 文件名没有 `.sh` 后缀，`install:1` 才是 shebang。）

**(2) Desktop App 是 Electron 42，而且它不是 CLI 的外壳——它自己 fork 一个 server 子进程。**

- `packages/desktop/package.json`：`name: "@opencode-ai/desktop"`，`version: "1.18.32"`，
  devDependencies 里 `electron 42.3.3` + `electron-builder 26.15.2` + `electron-vite ^5`。
  渲染层是 **SolidJS**（`solid-js`、`@solidjs/router`、`@kobalte/core`、`vite-plugin-solid`），
  不是 Tauri。**它的前身是 Tauri**——`packages/desktop/src/main/migrate.ts` 里有一整段
  "Migrate a Tauri .dat file into the corresponding electron-store"，`TAURI_MIGRATED_KEY = "tauriMigrated"`
  （`migrate.ts:8`），说明是从 Tauri 迁到 Electron 的，迁移代码至今还在。
- bundle id 按渠道三套：`ai.opencode.desktop` / `.beta` / `.dev`
  （`packages/desktop/electron-builder.config.ts`：`APP_IDS = { dev, beta, prod }`）。
  **我们的档案 `AgentRegistry.swift:222` 只写了 `bundleIDs: ["ai.opencode.desktop"]`——beta/dev 渠道装上后按 bundle 认不出来。**
- **关键：真正干活的是 desktop 主进程 fork 出来的 utility process + 一个独立二进制。**
  - `SIDECAR_VERSION === "v1"`（默认，由 `OPENCODE_SIDECAR_V2=1` 才切 v2，
    `packages/desktop/src/main/index.ts:64`）：`spawnLocalServer()` 用 Electron 的
    `utilityProcess.fork(sidecar.js)` 起一个 utility process，`serviceName: "opencode server"`
    （`packages/desktop/src/main/server.ts:31`，`server.ts:59-65`）。
    这个 sidecar 的代码是 `virtual:opencode-server` 虚拟模块，实际指向
    `packages/opencode/dist/node.js`（`packages/desktop/electron.vite.config.ts:5`
    `const OPENCODE_SERVER_DIST = "../opencode/dist/node"`）。
  - `SIDECAR_VERSION === "v2"`：`startBackgroundCli()` 先 `execFile` 一个名叫
    `opencode-cli`（Windows 下 `opencode-cli.exe`）的二进制跑 `--version` 与
    `service start` / `service get password`（`packages/desktop/src/main/background-cli.ts:19-56`、
    `background-cli.ts:120-122` `executableName()`），连本地 server 都在这个 CLI 手里。
  - 两条路都设 `OPENCODE_CLIENT: "desktop"`（`server.ts` 的 `preferAppEnv`）。
- **对我们的结论**：`AgentRegistry.swift:223` 的 `processNames: ["opencode"]` 够覆盖 CLI，
  但 Desktop 这一形态**只靠 `ai.opencode.desktop` 这个 bundle id 命中主进程**（Electron 主进程的
  basename 通常是 `Electron`，helper 叫 `Electron Helper`）。
  对照我们给 MiMo 档案写的那段注释（`AgentRegistry.swift:244-246`：按路径排除
  `frameworks`/`helper`，因为「CPU 会跨条目求和，把渲染器的空闲抖动累成高负载工作中」）——
  **opencode 档案今天没有 `pathExcludes`**，也就是说装上 Desktop 后，它的 GPU/渲染/网络 helper
  的 CPU 会被一起累加。**这是今天一处已确认的适配缺口，且教训在 MiMo 档案的注释里已经写过了。**

**(3) 一个额外发现：Desktop 的 userData 目录与 CLI 的 XDG 目录是两套。**
`preferAppEnv` 只设 `XDG_STATE_HOME`（`server.ts`：`XDG_STATE_HOME: process.env.XDG_STATE_HOME ?? userDataPath`），
**`XDG_DATA_HOME` 不改**，所以 Desktop 与 CLI 仍然共用同一个 `~/.local/share/opencode/opencode.db`。
只有 onboarding 测试模式才会把 `XDG_DATA_HOME` 一起改掉（`index.ts:126-139`，
那里还顺手设 `OPENCODE_DB = ":memory:"`）。**这条对我们的意义是：用户只装 Desktop、从没在终端敲过
`opencode`，我们也照样能读到会话库——覆盖度不受影响。**

### 2.2 状态：今天的判据成立，且比我预期的更稳

**(1) 落盘位置（本文最重要的答案）。**

路径构造代码就一句话，`packages/core/src/global.ts:9-14`：

```ts
const app = "opencode"
const data = path.join(xdgData!, app)      // → ~/.local/share/opencode
const cache = path.join(xdgCache!, app)    // → ~/.cache/opencode
const config = path.join(xdgConfig!, app)  // → ~/.config/opencode
const state = path.join(xdgState!, app)    // → ~/.local/state/opencode
const tmp = path.join(os.tmpdir(), app)    // → /tmp/opencode
```

用的是 `xdg-basedir` 这个 npm 包（`global.ts:3` `import { xdgData, ... } from "xdg-basedir"`），
**不是自己拼 `$XDG_DATA_HOME`**——所以 `XDG_DATA_HOME` 未设时走包的默认 `~/.local/share`。
我们把档案写成 `home(".local/share/opencode")`（`AgentRegistry.swift:227`）**对**。
**注意 `Global.Path.bin` 指向的是 `cache/bin` 而不是 data/bin**（`global.ts:22`）。

数据库文件名在 `packages/core/src/database/database.ts:43-55`：

```ts
export function path() {
  if (Flag.OPENCODE_DB) {
    if (Flag.OPENCODE_DB === ":memory:" || isAbsolute(Flag.OPENCODE_DB)) return Flag.OPENCODE_DB
    return join(Global.Path.data, Flag.OPENCODE_DB)
  }
  if (["latest", "beta", "prod"].includes(InstallationChannel) || ...) return join(Global.Path.data, "opencode.db")
  return join(Global.Path.data, `opencode-${InstallationChannel.replace(...)}.db`)
}
```

也就是说：**`OPENCODE_DB` 环境变量可以把库挪到任何地方**（`:memory:` 或绝对路径都吃），
而渠道不是 latest/beta/prod 时（也就是本地 `dev` 构建）库名会是 `opencode-<channel>.db`。
**我们的档案写死 `opencode.db`（`AgentRegistry.swift:230`）对 release 渠道成立，
对 dev 渠道与自定义 `OPENCODE_DB` 不成立。** 这是一条可备案的已知边界，不是 bug。

打开时的 pragma 六个（`database.ts:27-33`）：`journal_mode = WAL`、`synchronous = NORMAL`、
`busy_timeout = 5000`、`cache_size = -64000`、`foreign_keys = ON`、`wal_checkpoint(PASSIVE)`。
**WAL 意味着我们会看到 `opencode.db-wal` 与 `-shm` 两个伴生文件；只读打开 SQLite 会自己读 WAL，
我们的 `ReadonlyDB` 走的就是这条路，这点无需改动。**

**(2) 表结构：`session` / `message` / `part` 三张表，与我们的 SQL 逐字对齐。**

`packages/core/src/session/sql.ts` 定义了 Drizzle schema，`packages/core/src/database/schema.gen.ts`
是它的 SQL 落地版。**19 张表**：`workspace` / `data_migration` / `account_state` / `account` /
`control_account` / `credential` / `event_sequence` / `event` / `permission` / `project_directory` /
`project` / `message` / `part` / `session_context_epoch` / `session_input` / `session_message` /
`session` / `todo` / `session_share`。

我们读的三张（`schema.gen.ts:128-148`、`:182-215`）：

```sql
CREATE TABLE `message` ( `id` text PRIMARY KEY, `session_id` text NOT NULL,
  `time_created` integer NOT NULL, `time_updated` integer NOT NULL, `data` text NOT NULL, ... );
CREATE TABLE `part` ( `id` text PRIMARY KEY, `message_id` text NOT NULL, `session_id` text NOT NULL,
  `time_created` integer NOT NULL, `time_updated` integer NOT NULL, `data` text NOT NULL, ... );
CREATE TABLE `session` ( `id` text PRIMARY KEY, ..., `cost` real DEFAULT 0 NOT NULL,
  `tokens_input` integer DEFAULT 0 NOT NULL, `tokens_output` integer DEFAULT 0 NOT NULL,
  `tokens_reasoning` integer DEFAULT 0 NOT NULL, `tokens_cache_read` integer DEFAULT 0 NOT NULL,
  `tokens_cache_write` integer DEFAULT 0 NOT NULL, ... `time_created` / `time_updated` integer NOT NULL, ... );
```

`Timestamps` 两个字段都是 `integer`、值是 `Date.now()`（`packages/core/src/database/schema.sql.ts:3-12`），
**即 epoch 毫秒——与我们在 `AgentSessionInspector.swift:1024` 用 `date(fromEpochMillis:)` 解读一致。**

**(3) 状态机的两个判据都对：**

- **`message.data` 是 JSON，`role` 为 `assistant` 时有 `time.completed`，没有就是还在生成。**
  schema 在 `packages/schema/src/session-message.ts:177-186`：
  ```ts
  type: Schema.Literal("assistant"), agent: Schema.String, model: Model.Ref,
  content: [...], finish: Schema.String.pipe(optional), cost: Schema.Finite.pipe(optional),
  tokens: Schema.Struct({ input, output, reasoning, cache: { read, write } }).pipe(optional),
  time: Schema.Struct({ created: DateTimeUtcFromMillis, completed: DateTimeUtcFromMillis.pipe(optional) }),
  ```
  （V1 投影表结构同形：`packages/schema/src/v1/session.ts:453-485`，`Assistant` 里
  `time: { created, completed? }`、`tokens: { total?, input, output, reasoning, cache: { read, write } }`。）
  **这与我们在 `AgentSessionInspector.swift:1028-1034` 写的判据（assistant 有 `time.completed` = 已完成，
  只有 `time.created` = 还在生成）逐字对得上。**
- **新消息写进 `message` 表的路径是投影器，不是直接写。** `packages/core/src/session/projector.ts:267-272`：
  ```ts
  yield* events.project(SessionV1.Event.MessageUpdated, (event) => { ...
    yield* db.insert(MessageTable).values({ id, session_id: sessionID, time_created, data })
      .onConflictDoUpdate({ target: MessageTable.id, set: { data } }).run() })
  ```
  `time_created` 取自 `event.data.info.time.created`（同文件:263）。**同 id 会 upsert**，
  所以「`time_created` 变化」不是可靠的新消息信号，我们的指纹用 `\(agentId)-msg-\(newest.id)`
  （`AgentSessionInspector.swift:1026`）是正确取舍。
- **`session.agent` 这一列会随 agent 切换被改写**（`projector.ts:332-338`，
  `SessionEvent.AgentSwitched` → `update(SessionTable).set({ agent })`）。
  **这就是我们缺的那块（§4）："当前在跑 build 还是 plan" 在库里是查得到的，就在
  `SELECT agent FROM session WHERE id = ...`。我们今天没读这一列。**

**(4) 一个新东西：`session_message` 表是 V2 的追加式消息日志。**
`session_message`（`sql.ts:144-152`）有 `type`（`SessionMessage.Type`）+ `seq` +
`data`，按 `(session_id, seq)` 唯一索引。`packages/core/src/session/projector.ts:197` 往里插，
`:113-127` 按 id 更新。它是 V2 的重投影日志，`message`/`part` 两表仍是投影目标。
**我们读 `message`/`part` 读的是同一个真源的两个视图，不受影响。**

### 2.3 动作：**这里是今天最大的缺口，且是从源码逐版本核出来的**

我们今天的 opencode 动作/流水读的是 `part.data` 里的这几个字段
（`Sources/AgentIslandCore/AgentActionInspector.swift:690-695` 与
`AgentLogStreamer.swift:443-448`）：

```swift
if let type = json["type"] as? String {
    if type == "reasoning" { return "思考规划中" }
    else if type == "tool-call", let toolName = json["toolName"] as? String { return "正在调用: \(toolName)"" }
}
```

**逐版本核对结果：`type == "tool-call"` 与 `toolName` 在今天的 1.18.x 里都不存在。**

- **今天（`dev`，即 1.18.32 及之后）**：`packages/schema/src/v1/session.ts:315-322`
  ```ts
  export const ToolPart = Schema.Struct({
    ...partBase,
    type: Schema.Literal("tool"),      // ← 不是 "tool-call"
    callID: Schema.String,
    tool: Schema.String,               // ← 不是 "toolName"
    state: ToolState,
    metadata: ...
  })
  ```
- **`v1.16.0`**：`packages/core/src/v1/session.ts:306-313` 同为 `type: "tool"` / `tool: Schema.String`。
- **`v1.0.180`（最早的 tag）**：`packages/opencode/src/session/message-v2.ts:274-282`
  同为 `type: z.literal("tool")` / `tool: z.string()`。

也就是说 **`part` 的判别字段一直是 `type: "tool"`，工具名字段一直是 `tool`（外加 `callID`）**。
我们代码里的 `tool-call` / `toolName` **从来不是 opencode 的形状**。
这两个名字更像 DimAgent 自己的形状（`AgentLogStreamer.swift:323` 读 dim 的 `toolMeta` 时用的就是
`meta["toolName"]`，那里是对的）。

**这条对四个消费点的具体影响：**

| 位置 | 今天的行为 | 后果 |
| :--- | :--- | :--- |
| `AgentActionInspector.swift:690-695` | `type == "tool-call"` 永不成立 | **"正在调用: xxx" 这条动作永远出不来**；只剩 `reasoning → "思考规划中"` 与标题兜底 |
| `AgentLogStreamer.swift:443-448` | 同上 | 流水的 `toolCall` kind 与 `message`/`thinking` 两类还在，**工具调用事件全部落空** |
| `AgentSessionInspector.swift:955-957` 的注释 | 写着「`part.data` 是 text/reasoning/step-start/step-finish 这类片段」 | 注释本身就漏了 `tool` 与 `snapshot`/`patch`/`subtask`/`agent`/`retry`/`compaction` |
| `AgentSessionInspector.swift:975` 的回落检测 | 与上同 | 回落路径能探到的东西比注释说的更少 |

**V1 part 的完整判别联合**（`packages/schema/src/v1/session.ts:357-372`，12 种）：
`text` / `subtask` / `reasoning` / `file` / `tool` / `step-start` / `step-finish` / `snapshot` /
`patch` / `agent` / `retry` / `compaction`。
**其中 `step-finish` 带了 `cost` + `tokens`**（`v1/session.ts:240-256`）——
这是**除 `message.data.tokens` 之外第二个能拿到用量的地方**，且它是逐 step 的（比 message 级更细）。
我们今天没有读它。

**`tool` 的 `state` 是个四态联合**（`v1/session.ts:259-313`）：`pending` / `running` /
`completed` / `error`。其中 `running` 带 `time.start`（`:262-268`），
`completed` 带 `time.start`/`time.end`（`:277-290`）。
**这意味着"某个工具正在跑"在库里是可判的**：最新一条 `type == "tool"` 的 `state == "running"`
即动作进行中。我们今天做不到（因为 type 名就错了）。

**还有一个附带发现**（来自 OpenViking 的 issue #5128，第三方对同类 fork 的实测）：
同族 fork 的库里 `part` 行有 `type == "text"` 且 `synthetic == true` 的 Host 注入上下文
（运行时备注 / MCP 工具清单 / skill 正文），以及 `message.data` 里塞了 ~28 KB 的系统 prompt
（`system` 字段）。**这条要盯住：我们的流水详情今天把整行 `data` 当 `detail` 透出
（`AgentLogStreamer.swift:459-464` `detail: dataStr`），若不做过滤，
一条 text part 可能就是几十 KB。**（注：这是 MiMo fork 的实测，opencode 本体的字段名
需在本机真库上复核，见 §7。）

### 2.4 token：完全命中，字段名逐字一致

我们今天的 opencode token SQL（`Sources/AgentIslandCore/TokenUsageMonitor.swift:979-989`）：

```sql
SELECT time_created,
       COALESCE(json_extract(data,'$.tokens.input'),0)
         + COALESCE(json_extract(data,'$.tokens.output'),0)
         + COALESCE(json_extract(data,'$.tokens.reasoning'),0),
       COALESCE(json_extract(data,'$.cost'),0)
FROM message
WHERE json_extract(data,'$.role')='assistant'
  AND time_created >= ? AND time_created <= ?
ORDER BY time_created
```

**与上游的字段名逐字对得上**：`session-message.ts:177-182` 的 `cost` 与
`tokens: { input, output, reasoning, cache: { read, write } }`，`role: "assistant"`。
**连它自己回填历史汇总用的 SQL 都是同一套字段**（`packages/core/src/database/migration/20260510033149_session_usage.ts`
里的 `sum(json_extract(message.data, '$.tokens.input'))` 等，逐字段同款）。

三个值得记的差异点：

1. **`message.data.tokens` 是 optional**（`session-message.ts:182` `.pipe(optional)`；V1 里
   `v1/session.ts:472-481` 也是 optional）——**我们的 `COALESCE(...,0)` 正好吃掉这种情况，写法是对的。**
2. **`cache.read` / `cache.write` 我们没读**。这两个在 schema 里是必填非 optional
   （`v1/session.ts:477-480`），**补进去是净收益**，而且能对齐 README 说的 "Prompt / Completion /
   Cache / Thoughts 细分" 那种口径。`session` 表上也有汇总好的
   `tokens_cache_read` / `tokens_cache_write` 两列（`sql.ts:46-47`、`schema.gen.ts:204-205`），
   **按 session 取比按 message 加更便宜**。
3. **`session` 表自带 `cost` / `tokens_*` 六列汇总**，由投影器增量维护：
   `applyUsage()`（`projector.ts:90-105`）在每次 `PartUpdated` 带 `step-finish` 时
   **按 sign 加减**（删除 part 时 `-1`）。**所以"累计 token"可以一条
   `SELECT tokens_input+... FROM session ORDER BY time_updated DESC LIMIT 1` 拿到，
   不必扫 message 全表。** 我们今天走全表扫（`TokenUsageMonitor.swift:999` `rawRows(openCodeSQL, ...)`），
   这是有优化空间的（但属于改造，不属于本轮调研范围）。

**顺带一个对照**：opencode 有一个官方的 `STATS.md`（仓库根，18 KB）就是下载量流水表，
`packages/stats/` 是一个 SST 部署的公开站点（`opencode.ai/stats`？未核实）。
**它的 "stats" 是市场统计，与我们的 token 统计同名不同物。**

---

## 3. Desktop App：它是 Electron，自己 fork 一个 server，不是 CLI 的外壳

### 3.1 它是什么

| 维度 | 事实 | 证据 |
| :--- | :--- | :--- |
| 技术栈 | **Electron 42.3.3 + SolidJS + Vite**，从 Tauri 迁来 | `packages/desktop/package.json` devDeps；`migrate.ts:8` `TAURI_MIGRATED_KEY` |
| 版本 | `1.18.32`，与 CLI 同号 | 同文件 `version` |
| 渠道化 | dev / beta / prod 三套 appId 与 productName | `electron-builder.config.ts` `APP_IDS`；`getConfig()` 的 switch |
| 产物名 | `opencode-desktop-mac-arm64.dmg` / `-x64` / `windows-x64.exe` / Linux `deb·rpm·AppImage` | `electron-builder.config.ts:43` `artifactName`；mac `target: ["dmg","zip"]`、win `["nsis"]`、linux `["AppImage","deb","rpm"]` |
| 分发 | GitHub Releases + brew cask + scoop extras + 自托管 `opencode-beta` 仓 | README:79-82；`electron-builder.config.ts` beta 分支 `repo: "opencode-beta"` |
| 安全面 | macOS **hardenedRuntime + notarize**，entitlements 显式声明；Windows Azure Trusted Signing | `electron-builder.config.ts` `mac: { hardenedRuntime: true, notarize: true, entitlements }`；`.github/workflows/publish.yml:158-159` |
| 协议 | 注册 `opencode://` scheme | `electron-builder.config.ts` `protocols: { name: "OpenCode", schemes: ["opencode"] }` |

### 3.2 与 CLI / server 的关系：三层，但共用一份库

```
opencode（Bun 编译的 CLI，进程名 opencode）
   ├── opencode serve / opencode service start  → 本地 HTTP server（loopback）
   │     密码落 ~/.local/state/opencode/password（0600，randomBytes(32).toString("base64url")）
   │     注册信息落 ~/.local/state/opencode/server.json（{id, version, url, pid}）
   └── opencode <TUI>  → 终端 UI（@opentui/solid）

OpenCode.app（Electron 主进程）
   ├── (默认 v1 sidecar) utilityProcess.fork(sidecar.js, serviceName "opencode server")
   │     代码 = packages/opencode/dist/node.js（虚拟模块 virtual:opencode-server）
   └── (v2) execFile("opencode-cli", ["service","start"]) + ["service","get","password"]
```

server 的启动细节：`packages/cli/src/services/daemon.ts` 里
`const file = path.join(directory, "server.json")`、`passwordFile = path.join(directory, "password")`，
`directory = Global.Path.state`（即 `~/.local/state/opencode`）；
`Daemon.start()` 会 spawn 一个子进程，
密码注释写得很明确：「Keep one private credential across server restarts so discovered clients
can reconnect without exposing a password flag or environment variable.」（`daemon.ts` 中段）。
**这是我们**绝不去读**的两个文件：`~/.local/state/opencode/password` 是明文凭据。**

本机只读核对 `~/.local/state/opencode/`：有 `kv.json` / `locks/` / `model.json` /
`prompt-history.jsonl`，**没有 `server.json`、没有 `password`**（本机未跑过 server）。
`~/.config/opencode/` 有 `opencode.json`（1983 字节，未读内容）、`AGENTS.md`、`skills/`（42 项）、
`plugins/`、`node_modules/`（30 项）、`tui.json`、`memory/`、`squeez/`——**我们档案注释
（`AgentRegistry.swift:225-226`）说这里「含 node_modules，占全量扫描 22-50ms / 2098 个条目」是实的**，
只监控 `.local/share/opencode` 这个取舍正确。

### 3.3 对我们监控的冲击（三条，两条轻一条实）

1. **轻：库里照样有数据。** Desktop 不改 `XDG_DATA_HOME`（`server.ts` 的 `preferAppEnv`），
   会话库还是同一个。**用户只用 Desktop 我们也能监控。**
2. **轻：进程覆盖面。** bundle id 覆盖主进程；但 beta/dev 渠道的 appId 我们没列，
   且 **Electron helper 的 CPU 会被累加**（MiMo 档案的注释已记过这个坑，opencode 档案没抄）。
3. **实：一个我们完全没覆盖的形态——本地 HTTP server。**
   Desktop 随时可能在 127.0.0.1 上跑一个 opencode server（`server.ts` 里 `hostname = "127.0.0.1"`，
   端口从 `OPENCODE_PORT` 环境变量或随机取）。**这个 server 进程的 basename 会是
   `node`（Bun 编译产物在 macOS 上仍表现为独立进程名），我们的 `processNames: ["opencode"]`
   大概率认不出它**——本机未装，无法实测进程名，列入未核实。
   更值得注意的是：**这个 server 是有密码的**，我们从设计上不去连它（§6）。

---

## 4. `plan` 只读 agent：我们今天的缺口

### 4.1 它是什么、怎么定义的

README:104-110：「plan - Read-only agent for analysis and code exploration / Denies file edits by default /
Asks permission before running bash commands」。

**这不是文档承诺，是代码里的硬 deny。** `packages/opencode/src/agent/agent.ts:156-180`：

```ts
plan: {
  name: "plan",
  description: "Plan mode. Disallows all edit tools.",
  permission: Permission.merge(
    defaults,
    Permission.fromConfig({
      question: "allow",
      plan_exit: "allow",
      task: { general: "deny" },                                  // 不许起 general 子代理
      external_directory: { [path.join(Global.Path.data, "plans", "*")]: "allow" },
      edit: {
        "*": "deny",                                              // ← 默认全部 deny
        [path.join(".opencode", "plans", "*.md")]: "allow",       // 只放开写计划文件
        [path.relative(ctx.worktree, path.join(Global.Path.data, "plans", "*.md"))]: "allow",
      },
    }),
    user,                                                         // 用户配置可再覆盖
  ),
  mode: "primary", native: true,
},
```

配套的默认权限里 `question: "deny"`、`plan_enter: "deny"`、`plan_exit: "deny"`
（`agent.ts:130-135`），**build agent 才显式 `question: "allow"` / `plan_enter: "allow"`**（:140-145）。
还有一个专门测这件事的用例：`packages/opencode/test/agent/plan-mode-subagent-bypass.test.ts`
断言 `Permission.evaluate("edit", "/some/file.ts", planAgent.permission).action === "deny"`，
并测「子代理权限优先于父 agent 限制」与「只读子代理的只读限制仍然有效」。

它的日志里也有原生的一手证据：`agent.ts:156` 上一行注释写
`satisfies Agent.Info`，其 `mode` 字段取值是 `Schema.Literals(["subagent", "primary", "all"])`（`agent.ts:41`）。
**所以 "agent 身份" 是一等公民：`build` / `plan` / `general` / `explore` / `compaction` / `title`
六个内置 agent**（`agent.ts:139-240`，另有两个 `hidden: true` 的内部 agent）。

### 4.2 我们今天的缺口

**"agent 在做只读探索" 与 "agent 在改文件" 今天我们分不出来。**

- 库里**有**这个信息：`session.agent` 这一列（`sql.ts` 的 `SessionTable.agent: text()`，
  `schema.gen.ts:213`）由 `SessionEvent.AgentSwitched` 事件实时维护（`projector.swift` 对应处
  `projector.ts:332-338`）。
- 库里**还有**更细的：`part.data` 里 `type == "tool"` 的 `tool` 字段与 `state`
  ——`edit` / `write` / `apply_patch` 被 deny 时**甚至不会产生 part**（上游注释明说
  `write` 与 `apply_patch` 路由到 `edit` 权限，见 `plan-mode-subagent-bypass.test.ts:19-22` 的注释），
  所以「plan agent 的会话里看不到写工具」本身就是可判的。
- 我们**一个都没读**：`AgentSessionInspector.swift:990-993` 的 SQL 只取 `id, data`，
  没取 `session.agent`；动作检测只看 `part.data.type`（且 type 名还是错的，§2.3）。

**结论：今天做不到。缺口具体是两处——(a) `part.data.type` 的判别名错了导致工具调用全落空；
(b) 没有任何一条查询读 `session.agent` 或 `message.data` 的 `agent` 字段。**
修 (a) 是改四个字符串；补 (b) 是新增一次查询。两者都不大，但都超出本轮"纯调研"范围。

---

## 5. 工程上可借鉴的三件事

### 5.1 「迁移即源码」：每次破坏性 schema 变更都写成一个可审计的 `.ts` 文件

`packages/core/src/database/migration/` 下 **31 个 TypeScript 迁移**，命名
`<UTC 时间戳>_<随机形容词_名词>.ts`（drizzle-kit 风格，如 `20260622170816_reset_v2_session_state.ts`）。
`migration.gen.ts` 是**生成的文件**，只做 `import` 清单（`AGENTS.md` 明令「Do not edit ... directly」
的位置只有 `src/generated`，迁移本身是手写源）。

最值得看的是它怎么处理「必须作废旧数据」：`20260622170816_reset_v2_session_state.ts` 与
`20260622202450_simplify_session_input.ts` 的 `up()` 都是**六条 `DELETE` + 一条 `UPDATE ... = NULL`**，
一条注释都没有，但文件 id 自己说清楚了。**它不写"兼容层"，直接删。**

`migration.ts` 的启动判定也很干脆（`migration.ts:19-23`）：

```ts
const tables = yield* db.all(sql`SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'`)
if (tables.some((table) => table.name === "session")) return yield* applyOnly(db, migrations)
if (tables.length > 0) return yield* Effect.die("Database is not empty and has no session table")
```

**「库非空但没有 session 表 → 直接 die，不猜」。** 这条对我们有用：我们的方言解析今天遇到
「库在但表不在」是静默返回 nil（`AgentSessionInspector.swift:995` 的注释还专门解释了这个取舍），
**opencode 的作法是"明确失败"而不是"静默回落"**。我们的取舍有理由（一个源不该让整拍失败），
但**可以借它的一手证据做一件我们今天没有的事：把「库在、`message` 表不在」记进
`SessionProbeHealth`，而不是当成"这一族没有这张表"静默放过**（`AgentSessionInspector.swift:1004-1006`
现在只报 `stepFailed`，`prepareFailed` 被 `return nil` 吞掉）。

### 5.2 `AGENTS.md` 是一份带反例的风格契约，不是散文

根目录 `AGENTS.md`（8748 字节）与 `packages/desktop/AGENTS.md`、`packages/stats/AGENTS.md`
（每个大包一份）。写法是**每条规则配 Good/Bad 代码对**，而且规则细到命名层：

- 「Never alias imports. Do not use `import { foo as bar }`」「Never use star imports.
  Do not use `import * as Foo from`」——**但它同时给了正路：`import { Project } from "@opencode-ai/core/project"`
  然后用 `Project.ID`**（即"模块自己导出 namespace"）。这是一条被完整闭环的规则，不是口号。
- 「Avoid `else` statements. Prefer early returns.」配 Good/Bad。
- 「Reduce total variable count by inlining when a value is only used once.」配 Good/Bad。
- 「Keep things in one function unless composable or reusable」「Do not extract single-use helpers preemptively」。
- Drizzle 字段名**强制 snake_case**，理由是「so column names don't need to be redefined as strings」
  （这一条直接影响了我们今天读到的列名 `time_created` / `session_id` / `project_id`）。
- 「Avoid `try`/`catch` where possible」「Avoid using the `any` type」「Use Bun APIs when possible」。
- 「Always run `bun typecheck` from package directories, never `tsc` directly.」
- 「Tests cannot run from repo root (guard: `do-not-run-tests-from-root`)」——**根 package.json 的
  `test` 脚本就是 `echo 'do not run tests from root' && exit 1`**（我们抓到的 `pkg.json` scripts 实证）。
- 「In Effect generators, bind services to named variables before calling methods. Do not use nested
  service yields such as `yield* (yield* Foo.Service).bar()`.」

**对我们的可搬之处**：我们的 `CONTEXT.md` / `docs/adr/` 已经承担了口径与长期决策，
但**"代码风格"今天散落在各文件注释里**（`AgentSessionInspector.swift` 的 SQL 注释、
`ProcessMonitor.swift:296-330` 的匹配规则注释，质量其实很高）。**值得借的是它"Good/Bad 成对 +
一条规则只讲一件事 + 违反会明确的失败（如根 test 直接 exit 1）"这三招，而不是把它的规则搬过来。**

### 5.3 `specs/` 是一次改造的作战文档，而且**每篇都写「Status: Completed」**

`specs/` 下 16 个文件，其中 `specs/storage/remove-opencode-db.md`（约 20 KB）是范本：
开头是「## Goal / ## Current Inventory / ## Current Inventory 精确到 22 个文件列名 + 65 处引用分四组」，
然后每组都有 `**Status: Completed.**` 与「Target shape:」「Why this group comes first:」
「Suggested order: / Suggested first step:」。

**这与我们的 `docs/workbench/` 是同一件东西，但它的纪律更硬：每篇的结论区会被回写成
"已完成 / 未完成"，而不是让文档与现实脱节。** 我们 workbench 的四件套分工已经写在
`docs/workbench/README.md`，可借的是「改造定案的结论要往回写状态」这一条。

另一处小但值得抄：**`specs/storage/effect-sqlite-package.md` 与 `remove-opencode-db.md`
把「换存储层」当成一场有 Goal / Inventory / Group / Order 的战役，而不是一个 commit。**

---

## 6. 明确不看 / 不做的事

1. **不读 `~/.local/state/opencode/password` 与 `server.json`。** 前者是 `randomBytes(32)` 生成的
   明文服务密码（`daemon.ts`），我们从设计上不碰任何凭据（本项目硬红线）。
2. **不连它的本地 HTTP server。** server 有 basic auth（`OPENCODE_SERVER_USERNAME` /
   `OPENCODE_SERVER_PASSWORD`），连它等于引入一条凭据通道。**我们走只读 SQLite 就够，
   而且这条路上我们不需要 agent 配合**（这正是我们对 MonoCode 的优势所在）。
3. **不读 `~/.config/opencode/` 的内容。** `opencode.json` 是用户配置（可能含 provider 设置）、
   `node_modules/` 是依赖安装。今天排除它的理由（`AgentRegistry.swift:225-226`）成立，维持。
4. **不给它做「会话宿主」类功能。** 与 `12-monocode-competitor-audit.md` §6.1 对 MonoCode 的判断同构：
   它 7407 个文件、6635 个 blob、121 MB packages，**光 console 一个包就 50 MB**。
5. **不跟进它的 v2 / channel DB 命名与 `OPENCODE_DB` 覆盖。** 记入未核实清单，不改造。
6. **不因为这次调研改任何项目代码或文档**——本轮是纯调研，产物只有本文。
7. **不把它的 `packages/console`（SST + Cloudflare 的账号/计费后端）当成监控对象。**
   那是服务端，与我们无关。
8. **本机 `~/.opencode/`（安装脚本的 `INSTALL_DIR`）在本机不存在；`~/.local/share/opencode/` 也不存在。**
   未运行任何 opencode 命令，未修改任何其数据。本机 `~/.config/opencode/` 只做了 `ls` 与看了文件名，
   **未 `cat` 任何可能含配置值的文件**（`opencode.json` 未打开）。

---

## 7. 未核实清单

1. **`part.data.type == "tool-call"` 在本机真库上确实取不到值——未实测。** 本机未装 opencode
   （`~/.local/share/opencode/` 不存在，`which opencode` 未执行以避免副作用）。
   §2.3 的结论是从三个版本（`v1.0.180` / `v1.16.0` / `dev`）的 schema 源码推出的，
   **三个版本一致，但仍是推断而非本机复现。装一次 opencode 跑一个会话即可证实/证伪，这是最高优先级的后续动作。**
2. **`synthetic == true` 的 text part 与 `message.data.system` 的 ~28 KB 系统 prompt 是 MiMo fork 的实测**
   （来自 OpenViking issue #5128 的第三方记录），**opencode 本体的字段名与体积未在本机复核。**
   若成立，我们的流水 `detail` 有内容泄露与体积风险（`AgentLogStreamer.swift:459-464`）。
3. **Desktop 形态下本地 server 进程的 basename 未获取。** 猜测是 `node` 或 Bun 编译产物的名字，
   **未实测。** 装上 Desktop 跑一次会话，`ps` 一下即可。
4. **`OPENCODE_DB` / dev 渠道 DB 名（`opencode-<channel>.db`）从未在我们档案里出现过**——这是源码结论
   （`database.ts:43-55`），但与用户实际装法的交互未测。
5. **`install` 脚本与 README 的安装目录说明不一致**（README 说四级优先，脚本写死
   `~/.opencode/bin`）——**是 README 落后还是脚本落后，未判定**（可能是未来 PR 改了 README 未改脚本，
   或反之。两者的 git 历史未查）。
6. **standalone `opencode serve` 的进程名。** `Daemon.start()` 会 spawn 子进程（`daemon.ts`），
   spawn 的是什么可执行文件未逐行读。
7. **CI 全貌只读了 `test.yml` 的前 90 行**（linux + windows 双矩阵、`bun turbo test`、
   `check:generated`、`test:httpapi`、Playwright e2e）。`typecheck.yml` / `publish.yml` 后半段
   （签名、公证、SST 部署）**未获取**。`.github/workflows/` 共 34 个文件，只读了 3 个的开头。
8. **测试规模只数了文件名**：`dev` 分支 `681` 个 `.test.ts(x)` / 902 个 test-ish 文件；
   `v1.16.0` 是 `487`。**未跑过，也不知道通过率。**
9. **`packages/web`（13.25 MB）与 `packages/app`（12.38 MB）的职责切分未读。**
   只知道 `packages/app` 是 SolidJS 前端（被 desktop 的 renderer 复用），`packages/web` 疑似 web 版。
10. **`STATS.md` 只读了前 4 KB**（下载量流水表），未读它的统计口径与更新机制。
11. **`CONTEXT.md` 只读了前 5 KB**（术语表：System Context / Session History / Context Epoch /
    Safe Provider-Turn Boundary / Session Drain 等）。32 KB 全量未读。
12. **`v1.0.180` 到 `v1.16.0` 之间"JSON 文件存储 → SQLite"的切换具体发生在哪个版本，未定位。**
    `v1.0.180` 用的是 `~/.local/share/opencode/storage/<key>.json` 文件存储
    （`packages/opencode/src/storage/storage.ts:144` `const dir = path.join(Global.Path.data, "storage")`）；
    `v1.16.0` 已有完整 `packages/core/src/database/`。中间的 `v1.5.0`–`v1.9.0`
    raw.githubusercontent 多数返回 000/404（网络或 tag 不存在），**未取到，故未定位。**
    **这条对旧版本用户的兼容有意义：若用户跑的是很老的 opencode，我们读 `opencode.db` 会什么都读不到，
    而数据其实在 `storage/` 下的一堆 JSON 里。**

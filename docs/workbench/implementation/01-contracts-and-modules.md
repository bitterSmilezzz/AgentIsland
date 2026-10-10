# 01 · 契约与模块边界

## 1. 存量与改动位置

当前 `models.rs` 的 `AgentSnapshot` 保存活动、可观测性、健康度、用量；`EngineState` 保存快照、事件和导航映射；`AgentTaskEvent` 是通知事件，不是持久任务。`session_navigation.rs::Target` 已区分精确会话与打开工具；`main.rs` 注册命令；前端 `tauri.js` 提供 transport。

保持旧 DTO 字段与旧命令。第一步只增量提供新契约，不将通知事件结构改成任务，不一次重写 engine。

| 目标模块 | 工作 | 初始边界 |
| --- | --- | --- |
| `contracts.rs`（新增） | 能力/来源/错误等跨模块 DTO | 纯类型，不做 IO |
| `task_store.rs`、`task_service.rs`（新增） | 任务落盘、状态/来源转换 | 不负责执行编码任务 |
| `window_layout.rs`、`platform/window_layout_macos.rs`（新增） | 纯布局与 macOS AX 操作 | 不操作未选中的窗口 |
| `connections.rs`、`connection_adapters/`（新增） | 连接配置与按服务能力适配 | 不存凭据值；首版只读 |
| `main.rs`（修改） | 参数校验、调用服务、命令注册 | 原生写操作不持业务锁 |
| `engine.rs`（增量） | 向任务服务提交可信来源事件 | 通知与健康规则原实现保持 |
| `app/ui/js/transport.js`（新增） | 封装现有 tauri.js 的命令调用与订阅 | UI 不直接感知平台 API |

路径均相对 `app/src-tauri/src/`，另有明确前端路径。现有模块逐处接入，新增文件只在实际使用时创建。

## 2. 身份与来源

本机进程身份由档案声明，CLI匹配保留argv边界，只读取入口文件名或完整路径后缀；内联脚本、预载文件和业务参数不形成运行证据。Node命令别名、受支持入口及原生名字/路径规则分别验证。PID不是不可变的程序身份，采样需刷新可执行文件与入口，同PID exec后不能保留旧所有者；入口在每拍解析一次并供全部档案复用，匹配层不另存参数全文，库中的原始参数随每拍刷新覆盖。未知启动形式不猜测，识别不等同会话能力或安装状态。

目标 `SourceRefV1`：`source_kind`（local_session / trusted_event / user_link / remote_service）、`agent_id`、可空 `session_id/task_id/run_id/event_id/connection_id`、`observed_at_ms`、`capability`。无证据的字段为 null，不制造 ID。来源路径留在后端，不把个人绝对路径带进远程导出。

外部 ID 不全局唯一：用 `(source_kind, connection_id, external_id)` 去重，分别保存内外部身份。项目路径是本机元数据，不作为跨设备唯一 ID。新实体用抗冲突 ID，选用实现须有并发/重启测试；旧 Todo 数字 ID 原样保留，禁止重编号。

`NavigationCapabilityV1`：`kind=exact_session|application|unavailable`、`label`、`reason`。旧 Target 由后端转换，不直接开放任意 URL/path 启动。来源导航请求带稳定对象 ID 和 `expected_revision`；后端重新检查来源、所属 Agent、事件有效性后打开。

## 3. 能力与错误

能力目标字段：`key`、`state=supported|unavailable|permission_required|unverified`、`reason`、`platform`、`verified_at_ms`。前端只按此展示入口；“未验证”不等于已支持，也不等于工具不存在。

新命令使用可序列化错误 `code/message/retryable/current_revision`。错误码至少包括 `invalid_input`、`not_found`、`stale_revision`、`source_expired`、`permission_required`、`unsupported`、`io_error`、`timeout`、`authentication_failed`。服务端错误和字段校验分别映射；原始响应不得直接输出凭据、完整会话或配置文件。

新DTO字段采用snake_case；旧Target的camelCase保持，由transport转换，不能无提示改旧序列化。所有时间字段明确 unix 毫秒，数值单位在 DTO 上记录。成本含 currency、estimate、source；缺值用 null，禁止转零。预算、资源健康、任务进度保持不同字段，前端不把健康度解释成任务分数。

## 4. 读写与并发

- 小配置使用既有原子替换；任务聚合一次写入同时更新 task/run/artifact 引用，见 03。
- 新持久对象带 `revision`。写请求与预览确认提交都带 expected revision，冲突拒绝并允许重读；不能静默覆盖。
- IO、网络、原生窗口操作移出引擎锁。锁内复制所需值，锁外执行，回锁提交前复核版本。
- 前端请求以 page/request ID 判定是否还有效；后端完成不代表当前页必须渲染。丢弃过期展示结果，不自动撤销已经成功的业务写入。
- 新订阅按 `subscribe → update → dispose` 管理；页面隐藏停止昂贵读取，监控核心按既有策略运行。

## 5. 接入步骤与验收

先定义 DTO 与序列化夹具，再封装 transport，接一个只读能力入口，最后逐个接任务/窗口/连接命令。旧页面与 CLI 在整个迁移过程中可用。前端枚举未知值降级展示“当前版本不支持”，不能执行未知动作。

验证：缺字段/未知枚举、旧 DTO 兼容、重复外部 ID、过期 revision、source 缺失、错误脱敏、GUI/CLI 同口径。需要原生启动的用例使用受控夹具或注入 launch seam，普通单测不能打开用户真实工具或终止进程。

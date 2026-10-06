# 04 · macOS GUI 多窗口布局

当前推进：macOS inspect/write 适配器、当前桌面元数据核验、apply/undo IPC 与工作台窗口页已接入；只读探针以 AX 对象成员、PID/launchDate、唯一几何对应及 CGWindowNumber 共同核验，不使用标题或私有窗口 ID API。没有当前桌面唯一对应、全屏/最小化状态不可读时明确限制。设置尺寸后再次核验身份和可见性，再设置位置；每步复核启动身份，最终限时等待连续两次目标读数并复核身份，保留 actual。撤销原位置所在屏幕断开则逐窗失败并保留重试。已通过模拟页面联调与本轮 799 项 Rust 回归；真实工具隔离验证、原生读取耗时、真实延迟响应与完整 UI 品质验收仍待完成。核心输入一次最多 16 个窗口，间距 0–64 点；左右并排严格 2 个、主窗与辅窗严格 3 个，不丢弃多选。

只读命令现状：`window_layout_capabilities` 不提示授权；`window_layout_candidates` 在主线程读取 NSScreen 工作区，在后台按登记 bundle ID 枚举 NSRunningApplication/AXWindows，保存临时 UUID 与 retained AX 对象、PID、launchDate；标题只进入本机 DTO，不持久化。AX 可写检查分别读取位置/尺寸，无法核实则限制；最小尺寸未核实保持未知。单工具最多读取 32 窗口，探测有消息超时和批次时间预算，部分读取明确警告。`window_layout_preview` 仅接受后台列表 ID 和 revision，不接受任意矩形；列表 60 秒后拒绝，生成 15 秒预览，刷新替换身份并清除旧预览。当前几何适用性不等于原生应用成功，C2 必须再次检查身份、当前屏幕、窗口几何、桌面可见性与权限。

原生接入依据（2026-10-04 核对 Xcode SDK）：`HIServices/Headers/AXAttributeConstants.h:609–641` 明确位置以菜单栏主屏左上为原点、Y 向下、尺寸单位为点；不得把 AppKit 的 Y 向上矩形直接传入。`AXUIElement.h:55–64` 明确 trust 检查的 prompt 是异步且不改变返回值，首次读取不得提示；`188–204` 的属性可写检查有独立错误结果，不能把读取成功当作可移动。在线出处：[Apple 属性可写检查](https://developer.apple.com/documentation/applicationservices/1459972-axuielementisattributesettable)、[Apple 属性读取](https://developer.apple.com/documentation/applicationservices/1462085-axuielementcopyattributevalue)。实际工具限制、身份与跨桌面行为仍需只读探针和隔离真机验证。

## 1. 接缝与范围

规则保存已接入独立 `window_layout_rules.rs` 与工作台常用布局区：保存、载入、删除都检查聚合 revision。后台从当前快照导出工具偏好；窗口标题/句柄/PID/屏幕坐标不进入文件。重新加载只填入当前可唯一核实的窗口，部分枚举或多候选时清空自动选择；主屏偏好重新指向当前主屏，副屏偏好要求手动选屏。长期口径见 [ADR 0021](../../adr/0021-window-layout-preferences.md)。

现有 placement.rs 只安排 AgentIsland 自身窗口。新增 window_layout 模块处理用户选择的外部 GUI 窗口，不能复用岛的尺寸状态机。首版支持左右并排、主窗+两个辅窗、均分网格；保存模板与工具选择偏好，不持久化AX对象或依赖窗口标题作身份。

目标平台接口：`capabilities/enumerate/preview/apply/undo`。纯几何计算用逻辑点；原生适配负责屏幕坐标转换、AX位置/尺寸与主线程要求。macOS AX API细节在实现前用只读探针验证，不能凭进程存在判断可移动。

## 2. 数据与权限

| DTO | 字段 |
| --- | --- |
| WindowCandidate | opaque window_id、agent_id、应用名、显示标题、screen_id、rect、movable/resizable、restriction；标题只在本机UI显示 |
| DisplayArea | screen_id、可用rect、scale；排除菜单栏/Dock的区域 |
| LayoutPreview | preview_id、revision、template、候选before/target rect、限制/冲突、created_ms、expires_ms |
| ApplyResult | operation_id、逐窗 applied/failed/skipped、原因、actual_rect、undo_available |
| UndoResult | 逐窗 restored/failed/skipped、原因 |

window_id 是后端临时句柄映射；申请应用时复核进程启动身份、对象仍存活及用户选择。原生对象不由前端构造。系统重启或应用重启后过期，保存布局只保存规则。

页面首次打开只读能力检查。需要AX权限时解释“用于调整你选择的工具窗口”，用户点击后再进入授权流程。拒绝仍可看模板，不循环弹窗。全屏、不可调整、其他桌面与最小尺寸不满足等返回明确限制。

## 3. 布局计算

输入：可用工作区、选中窗口、有界gap、模板、顺序。输出目标rect或constraint_error。左右各半；主窗初始约60%宽，两辅窗在其余区域上下；网格按数量选择行列。边界与最后一列/行剩余空间统一处理，避免舍入产生越界。

能力不允许缩放或模板小于已核实最小尺寸时，预览标注不可应用；不移动到屏幕外来假装成功。多屏一次预览指定一个目标屏；后续跨屏布局另扩展。首版窗口过多时建议网格，不静默忽略多选项。

## 4. 拟新增命令与执行协议

- `window_layout_capabilities()`：能力与权限，不触发申请。
- `window_layout_candidates()`：只枚举用户可选择的已识别工具窗口。
- `window_layout_preview(selection, screen_id, template, gap)`：后端计算、保存有时限快照，返回预览；不移动。
- `window_layout_apply(preview_id, expected_revision)`：复核选中身份/屏幕/原几何；发生漂移拒绝并要求重新预览。
- `window_layout_undo(operation_id)`：复核仍是原对象，恢复成功修改窗口的before rect。
- `window_layout_save_rule(name, selection_rules, template)`：只保存可复用规则；下次重新选择/解析窗口再预览。

执行顺序：获取快照 → 复核目标 → 针对每窗设置位置与尺寸 → 读取actual rect → 记录逐窗结果。原生API是否需要位置/尺寸两步及顺序由实际窗口验证决定，不承诺所有应用原子移动。部分失败不默认再次移动已成功窗口，也不隐式整体回滚；显示结果和撤销入口。

undo只针对有读回证据的实际改动，包括操作失败但位置或尺寸部分改变的窗口；不能因为后一步失败而遗漏前一步副作用。窗口已关闭/进程重启时skip，暂时不可读则保留撤销记录供重试。窗口之后由用户移动或缩放，撤销先检查actual是否仍匹配上次结果，冲突要求用户明确选择后再恢复，不悄悄覆盖用户新布局。未先观测到冲突不能提交强制恢复；恢复部分失败继续保存最后可读结果。首版只保留最近一次可撤销操作的内存快照，重启后不展示有效撤销。

## 5. UI 与验收

工作台窗口页：窗口选择列表 → 屏幕与模板 → 预览图/限制 → 应用；结果逐窗可见，撤销固定在结果区。灵动岛快捷动作仅在已保存规则且能力可用时考虑，仍先打开预览，不默认移动窗口。

纯几何测试覆盖负坐标显示器、不同缩放、菜单栏/Dock、舍入与过多窗口；适配测试用fake后端覆盖权限拒绝、身份过期、漂移、部分失败、实际rect不符与撤销冲突。真机在隔离桌面逐一验证代表工具，不为验证移动用户当前工作窗口。未核实AX限制列入执行issue，不在代码中硬编码猜测。

## 持久历史与逐窗恢复（2026-10-06）

`window_layout_journal` 在每次排列/历史恢复前预写 pending，执行后结算 finished 或无写入 failed；最大100条、每条16窗、文件512 KiB。只保留登记工具ID、before/target/actual、结果及操作关联，不保留标题、原生句柄或PID。记录错误阻止移动；执行后结算失败仍展示实际结果并保留本次撤销，不返回未确认的记录ID。历史读取按需展开，最近5条，其余折叠。删除记录需要行内确认，可取消/Esc；检查整文件revision，只删除元数据，不移动窗口，释放记录容量。

恢复每次绑定一条历史slot到新读取的同工具窗口，当前窗口需用户显式选择；不自动复用旧身份，不接受前端自造矩形。`window_layout_recovery_preview` 核对历史revision/slot、60秒窗口快照、工具一致性、限制与当前屏幕完整包含目标，生成15秒预览；共用apply再查记录/屏幕/窗口身份和几何。恢复也产生新记录并关联来源记录，最近一次恢复可撤销到恢复前的位置。待核对记录只能作为历史意图参考，不证明上次执行。

36项窗口模块测试含模拟重启、新身份绑定、错误工具、屏幕越界、预览后原生位置漂移、记录被删除、结果结算失败保留撤销及100项容量退出。文件测试证明记录落盘和重启读取；受控Web界面验证选择/预览/恢复/删除/焦点/失败禁用，不构成真实AX写入、多屏和客户端验证。跨进程文件内容CAS与全组合窗口历史关联尚未宣称完成。

组合窗口历史关联已接入：记录保存组合/布局上下文，应用期间复核当前组合引用和规则版本，普通预览须匹配规则模板、gap与工具顺序；历史恢复关联来源必须属于同组合/布局。工作空间重启后入口定位具体历史详情，新窗口选择获得焦点，恢复应用仍共用原生复核。新增实际文件/模拟重启测试与组合引用失效测试，全量801项通过，受控网页验证对应入口、准确记录定位及IPC上下文；真实原生验收仍待完成。

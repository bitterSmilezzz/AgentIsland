# R32 验证记录：状态机时序批（v0.0.49）

日期：2026-09-12 · 执行：主 agent + 独立验收官

## 改动
- F5：onStateChanged/handleTaskEvent 代际守卫（sink Task 跳跃乱序时旧态覆盖新态——验收官逐场景推演确认守卫不吞合法中间态、check-then-act 无撕裂窗口）
- F8：beginDrag 取消 peek 后复位 grace（拖拽后自动收起不再被压制最长 6s）
- F10：scheduleCollapse 执行判定实时读 collapseDelay（挂起期改设置不再按旧延迟）
- F1：resumeGapThreshold 钳 180s 上界（idle=600 脏配置时旧阈值 1800）；tokenSpikeAlerted private→internal
- F7：断点清理补 tokenSpikeAlerted（唤醒后激增告警重新武装）
- 测试 186 → 188

## 验收与更正
- 验收官推演 ①②：守卫不吞合法中间态；「任何时刻当前值必有匹配 Task 完整执行」不变量成立
- **验收官指出 F1 测试无判别力**（150s/idle=5 与 idle=60 构造均无效）——重构为 idle=100 + MutableProcessProvider（gap 200s 介于新旧阈值 300/180 之间，第三拍信号消失转 idle），变异验证闭环：旧代码 187/1 红、新代码 188/0 绿
- **F1 规格层决策落档**：任务书动机「idle=60 时 2-3 分钟 App Nap 逃过检测」未被钳 180 根治（idle=60 时新旧阈值同为 180）——根治需事件驱动（willSleep 锚点），规格决策记入 issue 供后续轮
- F10 调小延迟时收起时点最迟旧 delay（≤5s 有界），可接受

## 运行证据
- 188/0 ×3；selftest 全过；打包重启（0.0.49）

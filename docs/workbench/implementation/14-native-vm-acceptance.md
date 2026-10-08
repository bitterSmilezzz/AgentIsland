# 14 · 隔离原生验收

对应 DELIVERY-01、KEEP-01 与 UX-01。原生验收使用已有 Tart macOS VM；运行器不创建账户、不配置密码或登录桌面、不自动安装依赖。凭据不进入仓库、输出或传输包。

## 运行契约

宿主桌面不得被验收占用：用户已要求停止前台干扰，旧的宿主 AXRaise、frontmost、cliclick 与聚焦后截图驱动停用。现有发布门禁通过 SSH 在来宾执行；需要画面和输入的专项优先使用无宿主窗口的来宾通路。尚不能后台完成的场景保留未完成，既有桌面授权旗标不能抵消当前用户约束。

当前本地原生窗口专项采用 Tart 2.40.1 的无窗口实验 VNC，通过 loopback RFB 驱动来宾；[对应版本 Run.swift](https://github.com/cirruslabs/tart/blob/2.40.1/Sources/tart/Commands/Run.swift) 在 noGraphics 路径不打开 Screen Sharing，[FullFledgedVNC.swift](https://github.com/cirruslabs/tart/blob/2.40.1/Sources/tart/VNC/FullFledgedVNC.swift) 创建临时认证。启动输出中的连接凭据仅在控制器内存消费，禁止落日志、argv、文件或剪贴板；关闭宿主/来宾剪贴板共享。该驱动是本地诊断夹具，需独立验证帧积压、实际前台目标、几何与生命周期，不因连接成功宣布业务通过，也不替代每次发布的五门禁。

宿主提供已签名的 `dist/AgentIsland.app`、Swift 编译器及现有 SSH 公钥通路；来宾需要已登录 Aqua、与宿主相同系统 build、用户目录中的 Miniconda Python。用户名必须明确提供，不从报告猜测。可用 `NATIVE_TEST_IDENTITY` 指定既有 SSH 身份路径，身份文件不复制到来宾或证据包。

```sh
NATIVE_TEST_USER=<guest-user> /usr/bin/python3 scripts/native-gates-vm.py --vm <existing-vm> --check-only
NATIVE_TEST_USER=<guest-user> /usr/bin/python3 scripts/native-gates-vm.py --vm <existing-vm>
NATIVE_TEST_USER=<guest-user> NATIVE_TEST_VM=<existing-vm> scripts/release.sh <version> "<title>"
```

宿主使用系统 Python：本机已复现 Homebrew Python 子进程读取 ARP 返回空表，而系统 Python 可取得 Tart 地址；不通过放宽网络隐私授权或硬编码地址绕过。来宾继续使用已探测的 Miniconda Python。

`--check-only` 只核对环境，不能证明 UI 门禁通过。默认发布没有隔离环境时拒绝；`ALLOW_DESKTOP_UI=1` 是已有的明确桌面授权路径，不是 VM 验证的替代证据。

运行器按本地 VM 名与运行状态核对，再通过 Tart 的 DHCP/ARP 解析私网地址。SSH 使用公钥和忽略目录内的专用主机密钥记录，已有密钥变化拒绝连接；不关闭主机密钥验证。来宾 `kern.hv_vmm_present=1`、console UID≥500 且与 SSH 用户一致、Python 和系统 build 必须全部核实。

## 产物与互斥

应用签名先在宿主验证，二进制、原生测试及资源脚本与预编译 Dock helper 的 SHA256 随包传输，来宾重新校验文件身份及整个 bundle 签名。只执行传输来的正式产物，不回退 debug 或旧安装。

五个门禁顺序执行：三窗口 UI 冒烟、原生 Dock 五段显隐、正式应用冷启动与单实例、随包 CLI 原生收起/唤回、原生窗口生命周期。Dock 驱动在主线程完成每个动作后等待测试端核对 NSRunningApplication.activationPolicy，再确认进入下一步；创建耗时不压缩后续阶段，超时或原生状态不符即失败。CLI 门禁临时注册未修改的同名副本但不启动，核对原实例 PID 和 Dock policy 的实际变化；结束后撤销临时注册并恢复目标注册。来宾独占目录锁保护本运行器间互斥；残留锁须人工检查，不自动删除。旧临时脚本不认识此锁，长时内存采样与其他原生自动化必须先停止，不能并行扰动同一来宾。

证据留在忽略目录 `.build/vm/acceptance-*/`：manifest、完整 gates.log 与 receipt。只有全部测试正常退出且最终完成标记存在，receipt 才通过；连接、传输、身份验证或超时失败不能复用旧回执。发布脚本每次打包后重新执行，不接受历史绿色日志抵扣。

## 验收边界与当前证据

已核实既有 VM 的真实用户名、同 build、虚拟化与公钥认证；来宾路径以现场探测为准。未登录桌面时 console UID=0，运行器正确拒绝。隔离条件/地址解析六项回归通过，Rust 回归 861 通过、11 忽略；原账户桌面登录已恢复，本轮完整原生门禁通过：证据目录 `.build/vm/acceptance-0c0b0fb375a54720a5f0b6427a921789/`，receipt 的 passed=true，三项门禁退出为0。首次 Dock 固定时钟驱动曾失败，改为完成/观测确认后通过，不以旧产物暖启动结果抵扣。

单显示器空环境 VM 不证明多屏/DPI、真实工具 AX、用户会话深链、完整业务负载或获奖品质。UI 冒烟不证明逐帧无抖动；内存专项必须实际满 30 分钟并完成 20 次打开/回收，记录单调时间、目标启动身份与有效结构化样本。相关退出条件继续按 07/09 逐项验收。

## 长时资源协议

`scripts/memory-soak.py --pid <existing-pid> --binary <formal-bundle-binary> --log <application-log>` 只在已登录的 macOS VM 内运行，与原生门禁共用独占锁。前置是正式单实例门禁留下的同一应用，工作台已打开；不挑选同名 PID、不启动替代实例、不读取历史标记假冒新动作。

初次收起并取得新回收日志后开始空闲计时，按真实单调时间采集 0/5/10/15/20/25/30 分钟七拍。随后完成 20 次创建/采样/隐藏/真实回收；每次系统派发、目标启动身份与新日志标记都复核，失败停止，不追加完成结论。每行是独立 JSON，保留主进程/可归属成员的测量状态及缺失值，原始日志正文不输出。

完成只证明协议执行，`budget_assessed=false` 明示资源预算仍需分析；无法完整归属的样本不补零、不变为预算通过。空 VM 仍不能代表带用户会话和监控负载的常驻预算。六项协议回归覆盖满 30 分钟、完整 20 次、目标身份/采样失败、旧日志拒绝及新标记核对；不将虚拟时钟测试写成实际长时验收。

长时驱动的派发绑定被测 bundle（`open -a <exact-bundle> agentisland://…`），避免来宾保留多份测试包时全局 scheme 选错目标。每次仍核对同一 PID/启动身份与新原生日志，不接受其他包动作。此协议不证明全局 CLI 的 LaunchServices 选择正确；该项独立验收。驱动与被测应用版本不同步时，另记驱动哈希，不改写已通过门禁的原 manifest。

## 原生动画专项

```sh
NATIVE_TEST_USER=<guest-user> /usr/bin/python3 scripts/native-gates-vm.py --vm <existing-vm> --motion-only
```

专项使用同一身份/签名/哈希门禁和来宾互斥锁，依次启动四个显式 `--motion-smoke` 实例；只读 AppKit frame 与 WebKit 卡片几何配对，覆盖设置、用量、Agent 详情进返，快速逆向及 650ms 晚到报告。合成 Agent 不证明真实工具会话跳转；测试不恢复 hooks、不启用远程通知。正常启动拒绝探针命令，不动态加载动画驱动。

驱动须等待真实文档加载完成、Tauri接口与合成Agent卡片可用后单次导入；首次几何请求确认测试已开始，15秒未确认即失败，不以固定加载等待或eval返回成功代替就绪。导入拒绝、缺失结果与超时记录受控失败原因，不放宽场景断言或测试超时。

每个场景检查锚点漂移、裁切、非预期反向、中间帧与最终收敛。原生包络和 WebKit 视口尚未同步的帧不能混用坐标系，最终必须同步；探针带 IPC 开销，帧间隔只算仪器数据，不证明生产帧率或视觉品质。Tauri 全局接口只读，晚到报告通过经后端探针校验的薄封装延迟交付，不改写全局 invoke。

专项 receipt 的 gates 只有 `motion-four-edges`，不能抵扣默认 UI / Dock / 单实例 / CLI / 窗口生命周期门禁；四边逐帧 JSON 留在该次证据目录。失败、不完整场景或缺失结果不接受，不能用纯算法夹具宣布原生通过。

本轮正式产物的专项证据在 `.build/vm/acceptance-ac998be781e84c0bb6487c0f686c1bea/`，receipt passed=true，四边各9场景通过。先前驱动改写只读 Tauri API 导致未进入测试，已修正并以只读夹具回归；不将该次失败视为产品动画失败，也不以当前几何通过宣布全部 UX-01 或整版品质完成。

随包 CLI 的编译后参数夹具覆盖配套包、软链接、单独安装和破损包；原生多副本送达检查已纳入默认第四项门禁，核对同一正式实例的收起/唤回和 Dock policy。四项通过证据包含明确未锁屏前置，旧三门禁回执不追改。每次新发布仍须重新执行，资源协议运行期间不得抢占来宾。

进程枚举器自身会出现在 `ps` 快照中，等待其退出后再查 coalition 会产生无效的未知成员。测量器仅按实际创建的子进程 PID 排除自身观察者，不按名称排除其他 `ps`，也不忽略其他不可读 PID；超时杀掉并回收观察者后仍返回失败。此修正须使用新驱动另取证，正在运行的旧驱动与历史样本保持原口径，不据此追改 complete 或预算结论。

资源退化口径：目标或采样器归属未知、与目标同组等仅能读取主进程的场景，保留known_member_footprint_mib，complete=false且total_footprint_mib为空；主进程可读不等于整应用完整。不可回填旧样本；新驱动需重新取证。

长时运行结束后，用 `python3 scripts/analyze-memory-soak.py --stream <memory-soak.jsonl> --receipt <receipt.json>` 核对完整七拍/20次顺序、时间跨度、目标启动身份和独立runner成功回执。时间戳允许100ms采样耗时差异，末条idle_seconds仍须至少1800秒；不把主进程极值当整应用预算，不从协议完成自动推断预算通过。核验器六项失败场景回归纳入release。传输manifest、二进制/驱动哈希与来宾身份仍由独立记录核对，核验器不取代这些证据。

原生UI烟测的工作台动效清理检查必须在可见窗口执行；隐藏WKWebView的固定延时不作为动画完成证明。显式隔离烟测显示工作台，等待实际有限动画finished与下一任务后核对旧裁切宿主/离场层，三秒期限失败仍阻止通过。该流程不等同逐帧品质评价。

来宾登录用户匹配不足以证明桌面可验收：环境探针及执行前须确认IOConsoleLocked明确为false，未知或锁屏拒绝。门禁持有最长600秒的caffeinate临时租约防止自动息屏，结束释放，不修改系统锁屏偏好。回执单独标console_unlocked；旧未包含锁屏证明的回执不追改。

监控空态检查先明确返回概览，再只从非离场层的wb-content查询实际监控区域。用量页没有监控区是正确行为，不能由隐藏WebView中的旧克隆抵扣概览检查；无实际监控区仍失败。

修正采样器后的独立正式产物长测证据：`.build/vm/resource-664652c76c44484e8419345f67a9fbbf/`。原始48行与独立成功回执已取回，目标身份/原始驱动及二进制哈希核对一致；7拍满30分钟、20次创建回收、47个完整观测、0个不完整。独立wrapper每2秒核对锁屏及console UID，1880拍均通过、最大间隔2.069秒且覆盖协议结束，不据此声称连续状态已证明。原始receipt与analysis仍为budget_assessed=false：这是空VM协议及完整归因证据，真实负载预算另行验收；不回填旧样本，不抵扣其他版本原生门禁。

完整观测的资源coalition账本加总：idle80.13–81.82MiB，open119.87–139.03MiB，released82.31–102.60MiB；回收态首尾增长20.29MiB，末五次95.85/97.98/99.75/101.08/102.60，尚无稳态证据。此数不是独占物理内存，也不是实际用户负载预算。保留coalition-observations.json，后续按成员定位增长来源，不能据主进程可读或协议成功判预算通过。

原生窗口生命周期门禁使用同一vendored Tao依赖的独立十次创建/drop循环，核对窗口和内容视图zeroing weak引用失效，并由heap -s读取前后窗口/视图/委托的实际分配数量，三类均须回到基线。只检查弱引用会漏过dealloc未调用父类导致的占用残留。默认只允许真实VM；既有显式桌面发布路径须提供--allow-desktop，普通回归不运行。编译后helper签名、源文件/依赖锁与三处补丁源哈希一并传输核对；新默认回执包含window-lifetime，旧四门禁回执保留原样。此最小回归不替代正式工作台90秒回收、材质/草稿/动效和长时资源验收。

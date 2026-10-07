# 14 · 隔离原生验收

对应 DELIVERY-01、KEEP-01 与 UX-01。原生验收使用已有 Tart macOS VM；运行器不创建账户、不配置密码或登录桌面、不自动安装依赖。凭据不进入仓库、输出或传输包。

## 运行契约

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

三个门禁顺序执行：三窗口 UI 冒烟、原生 Dock 五段显隐、正式应用冷启动与单实例。Dock 驱动在主线程完成每个动作后等待测试端核对 NSRunningApplication.activationPolicy，再确认进入下一步；创建耗时不压缩后续阶段，超时或原生状态不符即失败。来宾独占目录锁保护本运行器间互斥；残留锁须人工检查，不自动删除。旧临时脚本不认识此锁，长时内存采样与其他原生自动化必须先停止，不能并行扰动同一来宾。

证据留在忽略目录 `.build/vm/acceptance-*/`：manifest、完整 gates.log 与 receipt。只有全部测试正常退出且最终完成标记存在，receipt 才通过；连接、传输、身份验证或超时失败不能复用旧回执。发布脚本每次打包后重新执行，不接受历史绿色日志抵扣。

## 验收边界与当前证据

已核实既有 VM 的真实用户名、同 build、虚拟化与公钥认证；来宾路径以现场探测为准。未登录桌面时 console UID=0，运行器正确拒绝。隔离条件/地址解析六项回归通过，Rust 回归 861 通过、11 忽略；原账户桌面登录已恢复，本轮完整原生门禁通过：证据目录 `.build/vm/acceptance-0c0b0fb375a54720a5f0b6427a921789/`，receipt 的 passed=true，三项门禁退出为0。首次 Dock 固定时钟驱动曾失败，改为完成/观测确认后通过，不以旧产物暖启动结果抵扣。

单显示器空环境 VM 不证明多屏/DPI、真实工具 AX、用户会话深链、完整业务负载或获奖品质。UI 冒烟不证明逐帧无抖动；内存专项必须实际满 30 分钟并完成 20 次打开/回收，记录单调时间、目标启动身份与有效结构化样本。相关退出条件继续按 07/09 逐项验收。

## 长时资源协议

`scripts/memory-soak.py --pid <existing-pid> --binary <formal-bundle-binary> --log <application-log>` 只在已登录的 macOS VM 内运行，与原生门禁共用独占锁。前置是正式单实例门禁留下的同一应用，工作台已打开；不挑选同名 PID、不启动替代实例、不读取历史标记假冒新动作。

初次收起并取得新回收日志后开始空闲计时，按真实单调时间采集 0/5/10/15/20/25/30 分钟七拍。随后完成 20 次创建/采样/隐藏/真实回收；每次系统派发、目标启动身份与新日志标记都复核，失败停止，不追加完成结论。每行是独立 JSON，保留主进程/可归属成员的测量状态及缺失值，原始日志正文不输出。

完成只证明协议执行，`budget_assessed=false` 明示资源预算仍需分析；无法完整归属的样本不补零、不变为预算通过。空 VM 仍不能代表带用户会话和监控负载的常驻预算。六项协议回归覆盖满 30 分钟、完整 20 次、目标身份/采样失败、旧日志拒绝及新标记核对；不将虚拟时钟测试写成实际长时验收。

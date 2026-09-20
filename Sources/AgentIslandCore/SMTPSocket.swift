import Foundation
import Network

// MARK: - SMTP 的真实 socket 实现

/// 基于 Network.framework 的 SMTP 客户端会话。
///
/// **只支持隐式 TLS（465 端口）**，这是有意的收窄而不是偷懒：
/// Network.framework 不提供「在已建立的 TCP 连接上原地升级为 TLS」，
/// 而 STARTTLS(25/587) 正是那个形态。QQ 邮箱与 163 邮箱都提供 465，
/// 因此覆盖常见场景的成本最低；其余端口在配置层就被拒绝并给出说明，
/// 不会让用户以为配好了却每次都失败。
public final class SMTPSocketConnection: SMTPSessionIO, @unchecked Sendable {
    private let host: String
    private let port: Int
    private let implicitTLS: Bool
    private let queue = DispatchQueue(label: "com.agentisland.smtp", qos: .utility)
    private var connection: NWConnection?
    /// connect() 安排的建立超时兜底（见 connect 注释：.waiting 之后可能再无状态回调）
    /// 在 finish/close 里取消，避免到点后再回调一次
    private var pendingTimeout: DispatchWorkItem?
    /// 已读到但未消费的字节（NWConnection.receive 的边界与行边界不一致，必须自己攒）
    private var pending = Data()
    private let stateLock = NSLock()

    public init(host: String, port: Int, implicitTLS: Bool) {
        self.host = host
        self.port = port
        self.implicitTLS = implicitTLS
    }

    /// 建立连接。`timeout` 兜底是必需的而不是保险丝：对端不可达时 NWConnection 停在
    /// `.waiting` 里自己重试路径，状态永远不会走到 `.ready/.failed`——
    /// 实测（连一个没人监听的地址）`connect()` 就此挂住，续体泄漏、界面上
    /// 「发送测试」永远显示「正在送出…」。
    public func connect(timeout: TimeInterval = 10) async -> Bool {
        guard implicitTLS else { return false }   // 配置层已拦，这里再兜一道
        guard let endpointPort = NWEndpoint.Port(rawValue: UInt16(clamping: port)), port > 0,
              port <= Int(UInt16.max) else { return false }   // 越界端口不该静默变成 465
        // 刻意不设 sec_protocol_options_set_verify_block：一旦设了就是**接管**信任评估，
        // 而回调里拿到的 sec_trust_t 非空并不代表证书可信（框架总会建一个），
        // `completion(true)` 等于对任意证书放行——授权码就会发给任意一台中间机器。
        // 默认路径下 Network.framework 按连接的域名做 SNI 与服务端证书校验。
        let parameters = NWParameters(tls: NWProtocolTLS.Options(), tcp: .init())
        let conn = NWConnection(host: NWEndpoint.Host(host), port: endpointPort, using: parameters)
        connection = conn
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            // 回调在 `queue` 上跑、续体只允许 resume 一次：用锁保护的箱子而不是捕获 var
            // （捕获 var 既是数据竞争，也是 Swift 6 语言模式的硬错误）
            let once = ResumeOnce()
            @Sendable func finish(_ success: Bool) {
                guard once.first() else { return }
                stateLock.lock()
                let timer = pendingTimeout
                pendingTimeout = nil
                stateLock.unlock()
                timer?.cancel()
                conn.stateUpdateHandler = nil
                if !success { conn.cancel() }   // 让停在 .waiting 的连接彻底退出
                continuation.resume(returning: success)
            }
            conn.stateUpdateHandler = { state in
                switch state {
                case .ready: finish(true)
                case .failed, .cancelled: finish(false)
                // .setup/.preparing 是 start 之后必经的过渡态，落到兜底分支会让
                // connect() 在刚建立时就报「连不上」，等于 SMTP 通道永远不通
                case .setup, .preparing, .waiting: break   // 交给下面的定时兜底收敛
                @unknown default: finish(false)
                }
            }
            // 独立定时兜底：.waiting 之后再无状态回调时，靠这一次触发收敛
            let timer = DispatchWorkItem { finish(false) }
            stateLock.lock()
            pendingTimeout = timer
            stateLock.unlock()
            queue.asyncAfter(deadline: .now() + timeout, execute: timer)
            conn.start(queue: queue)
        }
    }

    /// 写一行。也要有独立超时：对端只收不回时 NWConnection 的 send 回调可以一直不来，
    /// 而 `.contentProcessed` 只在数据真的落地或连接取消时才结算
    public func writeLine(_ text: String) async -> Bool {
        await write(text, timeout: 10)
    }

    private func write(_ text: String, timeout: TimeInterval) async -> Bool {
        guard let conn = connection else { return false }
        return await withCheckedContinuation { (continuation: CheckedContinuation<Bool, Never>) in
            let once = ResumeOnce()
            func finish(_ value: Bool) {
                guard once.first() else { return }
                timer?.cancel()
                continuation.resume(returning: value)
            }
            var timer: DispatchWorkItem?
            timer = DispatchWorkItem { finish(false) }
            queue.asyncAfter(deadline: .now() + timeout, execute: timer!)
            conn.send(content: Data((text + "\r\n").utf8), completion: .contentProcessed { error in
                finish(error == nil)
            })
        }
    }

    /// 读到一行为止。超时必须是独立定时器：`deadline` 只在 receive 回调里比较的话，
    /// 一个「TLS 通了但就是不回行」的对端（限流中的邮箱服务器就是这个形态）
    /// 永远不会触发回调，续体就永久挂住——连接与已解密的授权码一起泄漏
    public func readLine(timeout: TimeInterval) async -> String? {
        guard let conn = connection else { return nil }
        return await withCheckedContinuation { (continuation: CheckedContinuation<String?, Never>) in
            let once = ResumeOnce()
            func finish(_ value: String?) {
                guard once.first() else { return }
                timer?.cancel()
                continuation.resume(returning: value)
            }
            var timer: DispatchWorkItem?
            timer = DispatchWorkItem { finish(nil) }
            queue.asyncAfter(deadline: .now() + timeout, execute: timer!)
            func step() {
                stateLock.lock()
                if let newline = pending.firstIndex(of: 0x0A) {
                    let line = Data(pending[pending.startIndex...newline])
                    pending.removeSubrange(pending.startIndex...newline)
                    stateLock.unlock()
                    finish(String(data: line, encoding: .utf8)?
                        .trimmingCharacters(in: .init(charactersIn: "\r\n")))
                    return
                }
                stateLock.unlock()
                conn.receive(minimumIncompleteLength: 1, maximumLength: 4096) { [weak self] data, _, isComplete, error in
                    guard let self else { finish(nil); return }
                    if let data, !data.isEmpty {
                        self.stateLock.lock()
                        self.pending.append(data)
                        self.stateLock.unlock()
                        step()
                        return
                    }
                    if isComplete || error != nil { finish(nil); return }
                    step()   // 没读到东西就继续等，超时由上面的定时器负责
                }
            }
            step()
        }
    }

    public func close() {
        stateLock.lock()
        let timer = pendingTimeout
        pendingTimeout = nil
        stateLock.unlock()
        timer?.cancel()   // 连接被关掉后，兜底定时器不该再 resume 一次
        connection?.cancel()
        connection = nil
    }
}

/// 「首次调用返回 true、之后恒 false」的原子开关。
/// 用它是因为 NWConnection 的状态回调可能连来两次（.ready 后又 .cancelled），
/// 而 CheckedContinuation resume 两次是致命陷阱。
private final class ResumeOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var used = false

    /// 占到「唯一一次恢复权」时返回 true
    func first() -> Bool {
        lock.lock(); defer { lock.unlock() }
        if used { return false }
        used = true
        return true
    }
}

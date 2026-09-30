//! SMTP 会话：把一封通知按 RFC 822 组装并逐行谈成。
//!
//! 对齐 Swift `SMTPClient` / `SMTPMessage` / `RFC822Date` / `SMTPHello`。
//!
//! **只支持隐式 TLS（465）**，与 Swift 同一条有意收窄：STARTTLS（25/587）要在已建立的
//! TCP 上原地升级 TLS，各家客户端库支持不一，而 QQ / 163 都提供 465，
//! 覆盖常见场景的成本最低。其余端口在配置层就被拒绝并给出说明，不会让用户以为配好了却每次都失败。
//!
//! IO 全部走 [`Session`]：这正是它能被**离线测试**的原因（Swift 侧同一个设计）。

use crate::notifier::Outcome;
use crate::render::SmtpTarget;
use base64::Engine;
use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// 每一步的等待上限（Swift `timeout` 默认 10 秒）
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// SMTP 会话的最小接口（Swift `SMTPSessionIO`）。
/// 读一行（去掉行尾 CRLF）与写一行（自带 CRLF）就够描述整个协议，
/// 于是协议逻辑不必碰网络，可以用脚本化的假会话逐条断言。
pub trait Session {
    /// 读一行；超时或连接已断返回 `None`
    fn read_line(&mut self, timeout: Duration) -> Option<String>;
    /// 写一行（自带 CRLF）；连接已断返回 false
    fn write_line(&mut self, text: &str) -> bool;
    fn close(&mut self);
}

/// SMTP 的一行：把裸 CR/LF 折成空格，避免消息里夹一个换行就在协议里多写一条命令。
/// 先折 CRLF 再折单字符，否则 `\r\n` 会变成两个空格。
pub fn smtp_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

fn base64_text(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// RFC 822 的 `Date:` —— `EEE, dd MMM yyyy HH:mm:ss ±HHMM`，**本地时间带偏移**
/// （Swift 的 `DateFormatter` 没设 timeZone，用的是当前时区）。
pub fn rfc822_date(now_ms: i64) -> String {
    let date = crate::localclock::local_time(now_ms)
        .and_then(|tm| crate::localclock::offset_minutes(now_ms, &tm).map(|offset| (tm, offset)));
    let Some((tm, offset_minutes)) = date else {
        return "Thu, 01 Jan 1970 00:00:00 +0000".to_string();
    };
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let offset = offset_minutes.abs();
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} {}{:02}{:02}",
        WEEKDAYS[tm.tm_wday.clamp(0, 6) as usize],
        tm.tm_mday,
        MONTHS[tm.tm_mon.clamp(0, 11) as usize],
        tm.tm_year + 1900,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        sign,
        offset / 60,
        offset % 60
    )
}

/// EHLO 用的本机主机名；拿不到就用 `localhost`（Swift `SMTPHello.hostName` 同口径）
pub fn hostname() -> String {
    let mut buffer = [0u8; 256];
    let ok = unsafe {
        libc::gethostname(
            buffer.as_mut_ptr() as *mut libc::c_char,
            buffer.len() as libc::size_t,
        )
    } == 0;
    if !ok {
        return "localhost".to_string();
    }
    let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).trim().to_string();
    if name.is_empty() {
        "localhost".to_string()
    } else {
        name
    }
}

/// 要逐行写给服务器的内容，最后一行是终止符 `.`。
///
/// 两处必须照搬的细节：
/// ① 正文先按 CR/LF 归一化，再以 CRLF 逐行发；
/// ② **行首的 `.` 必须加倍**，否则一行以 `.` 开头的正文会被服务器当成邮件结束。
pub fn message_lines(subject: &str, body: &str, target: &SmtpTarget, now_ms: i64) -> Vec<String> {
    let mut lines: Vec<String> = vec![
        format!("From: {}", smtp_line(&target.from)),
        format!("To: {}", smtp_line(&target.to)),
        // 主题走 RFC 2047 的 base64：中文主题直接写进头里会被各家服务器拒或乱码
        format!("Subject: =?UTF-8?B?{}?=", base64_text(subject)),
        format!("Date: {}", rfc822_date(now_ms)),
        "MIME-Version: 1.0".to_string(),
        "Content-Type: text/plain; charset=utf-8".to_string(),
        "Content-Transfer-Encoding: 8bit".to_string(),
        String::new(),
    ];
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.split('\n') {
        lines.push(if line.starts_with('.') {
            format!(".{line}")
        } else {
            line.to_string()
        });
    }
    lines.push(".".to_string());
    lines
}

/// 跑完一次会话并**保证**关掉连接。
///
/// 失败分支若不关，每条走 SMTP 的通道每次失败都留下一条活的 TLS 连接
/// （连同已解密的授权码）。关连接放在这一层而不是调用方，
/// 是因为只有这里能被离线测试观察到「失败路径也关了」。
pub fn run(
    session: &mut dyn Session,
    target: &SmtpTarget,
    subject: &str,
    body: &str,
    now_ms: i64,
) -> Outcome {
    let outcome = session_inner(session, target, subject, body, now_ms);
    session.close();
    outcome
}

fn code_of(line: &str) -> i64 {
    line.chars().take(3).collect::<String>().parse().unwrap_or(-1)
}

/// 读一条回复并检查状态码；多行回复（`250-…`）必须读到 `250 ` 那行为止。
/// 返回 `Some(失败)` 表示这一步没通过。
fn expect(session: &mut dyn Session, codes: &[i64], label: &str) -> Option<Outcome> {
    let Some(mut line) = session.read_line(TIMEOUT) else {
        return Some(Outcome::Failed {
            reason: format!("{label}：服务器无响应（超时）"),
            permanent: false,
        });
    };
    let code = code_of(&line);
    if !codes.contains(&code) {
        // SMTP 自己区分这两种：5xx 是「明确拒绝」（535 授权码错、550 拒绝中继），
        // 4xx 是「暂时不行」（421 服务器忙、450 稍后再试）——只有后者值得 5 秒后再来一次
        return Some(Outcome::Failed {
            reason: format!("{label}：回复 {code}"),
            permanent: code >= 500,
        });
    }
    // 「状态码-」是续行。只读一行的话，多行回复会把剩下的行留在流里，
    // 后面每一次读取全部错位
    while line.len() >= 4 && line.starts_with(&format!("{code}-")) {
        match session.read_line(TIMEOUT) {
            None => {
                return Some(Outcome::Failed {
                    reason: format!("{label}：续行后无响应"),
                    permanent: false,
                })
            }
            Some(next) => {
                line = next;
                if !line.starts_with(&format!("{code}")) {
                    return Some(Outcome::Failed {
                        reason: format!("{label}：回复流被打断"),
                        permanent: false,
                    });
                }
            }
        }
    }
    None
}

fn ehlo(session: &mut dyn Session) -> Option<Outcome> {
    if !session.write_line(&format!("EHLO {}", hostname())) {
        return Some(Outcome::Failed {
            reason: "EHLO：连接已断开".into(),
            permanent: false,
        });
    }
    // 多行回复要读到「单空格续行结束」为止，否则后面的读全部错位
    while let Some(line) = session.read_line(TIMEOUT) {
        if line.len() >= 4 && line.starts_with("250-") {
            continue;
        }
        if line.starts_with("250") {
            return None;
        }
        let code = code_of(&line);
        return Some(Outcome::Failed {
            reason: format!(
                "EHLO：回复异常 {}",
                line.chars().take(3).collect::<String>()
            ),
            permanent: code >= 500,
        });
    }
    Some(Outcome::Failed {
        reason: "EHLO：服务器无响应".into(),
        permanent: false,
    })
}

fn session_inner(
    session: &mut dyn Session,
    target: &SmtpTarget,
    subject: &str,
    body: &str,
    now_ms: i64,
) -> Outcome {
    if let Some(fail) = expect(session, &[220], "问候") {
        return fail;
    }
    if let Some(fail) = ehlo(session) {
        return fail;
    }

    if !session.write_line("AUTH LOGIN") {
        return Outcome::Failed {
            reason: "AUTH：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[334], "AUTH LOGIN") {
        return fail;
    }
    if !session.write_line(&base64_text(&target.user)) {
        return Outcome::Failed {
            reason: "AUTH：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[334], "AUTH 用户名") {
        return fail;
    }
    if !session.write_line(&base64_text(&target.password)) {
        return Outcome::Failed {
            reason: "AUTH：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[235], "AUTH 结果") {
        return fail;
    }

    if !session.write_line(&format!("MAIL FROM:<{}>", smtp_line(&target.from))) {
        return Outcome::Failed {
            reason: "MAIL FROM：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[250], "MAIL FROM") {
        return fail;
    }
    if !session.write_line(&format!("RCPT TO:<{}>", smtp_line(&target.to))) {
        return Outcome::Failed {
            reason: "RCPT TO：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[250, 251], "RCPT TO") {
        return fail;
    }
    if !session.write_line("DATA") {
        return Outcome::Failed {
            reason: "DATA：连接已断开".into(),
            permanent: false,
        };
    }
    if let Some(fail) = expect(session, &[354], "DATA") {
        return fail;
    }

    for line in message_lines(subject, body, target, now_ms) {
        if !session.write_line(&line) {
            return Outcome::Failed {
                reason: "正文发送中断".into(),
                permanent: false,
            };
        }
    }
    if let Some(fail) = expect(session, &[250], "收件确认") {
        return fail;
    }
    let _ = session.write_line("QUIT");
    Outcome::Delivered
}

/// 真实会话：465 隐式 TLS（`native-tls` 包一层 `TcpStream`）。
pub struct TlsSession {
    stream: native_tls::TlsStream<TcpStream>,
    pending: VecDeque<u8>,
}

impl TlsSession {
    pub fn connect(target: &SmtpTarget) -> Result<TlsSession, String> {
        if target.port != 465 {
            // 与 Swift `SMTPSocketConnection.connect()` 同一条兜底：配置层已拦，这里再兜一道
            return Err(format!(
                "只支持 465 隐式 TLS（STARTTLS 的 25/587 不支持），收到 {}",
                target.port
            ));
        }
        let connector = native_tls::TlsConnector::new()
            .map_err(|error| format!("系统 TLS 初始化失败：{error}"))?;
        let addresses: Vec<_> = (target.host.as_str(), target.port as u16)
            .to_socket_addrs()
            .map_err(|error| format!("地址解析失败：{error}"))?
            .collect();
        let mut last_error = String::new();
        for address in &addresses {
            let tcp = match TcpStream::connect_timeout(address, TIMEOUT) {
                Ok(tcp) => tcp,
                Err(error) => {
                    last_error = format!("连不上 {}:{}（{error}）", target.host, target.port);
                    continue;
                }
            };
            let _ = tcp.set_read_timeout(Some(TIMEOUT));
            let _ = tcp.set_write_timeout(Some(TIMEOUT));
            // 主机名交给 TLS 栈校验：证书对不上就失败，不静默降级
            let tls = connector
                .connect(&target.host, tcp)
                .map_err(|error| format!("TLS 握手失败：{error}"))?;
            return Ok(TlsSession {
                stream: tls,
                pending: VecDeque::new(),
            });
        }
        Err(if last_error.is_empty() {
            "地址解析为空".to_string()
        } else {
            last_error
        })
    }
}

impl Session for TlsSession {
    fn read_line(&mut self, timeout: Duration) -> Option<String> {
        use std::io::Read;
        let _ = self.stream.get_ref().set_read_timeout(Some(timeout));
        loop {
            if let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=index).collect();
                return Some(
                    String::from_utf8_lossy(&line)
                        .trim_end_matches(['\r', '\n'])
                        .to_string(),
                );
            }
            let mut buffer = [0u8; 4096];
            match self.stream.read(&mut buffer) {
                Ok(0) | Err(_) => return None,
                Ok(read) => self.pending.extend(&buffer[..read]),
            }
        }
    }

    fn write_line(&mut self, text: &str) -> bool {
        use std::io::Write;
        let _ = self.stream.get_ref().set_write_timeout(Some(TIMEOUT));
        self.stream
            .write_all(format!("{text}\r\n").as_bytes())
            .is_ok()
            && self.stream.flush().is_ok()
    }

    fn close(&mut self) {
        let _ = self.stream.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 脚本化会话：按脚本逐条回话，并记下我们写出去的东西。
    /// `None` 表示「不回话」（用来测超时路径）。
    struct Scripted {
        script: VecDeque<Option<String>>,
        written: Vec<String>,
        closed: usize,
    }

    impl Scripted {
        fn new(script: Vec<Option<&str>>) -> Scripted {
            Scripted {
                script: script
                    .into_iter()
                    .map(|line| line.map(str::to_string))
                    .collect(),
                written: Vec::new(),
                closed: 0,
            }
        }
    }

    impl Session for Scripted {
        fn read_line(&mut self, _timeout: Duration) -> Option<String> {
            self.script.pop_front().flatten()
        }
        fn write_line(&mut self, text: &str) -> bool {
            self.written.push(text.to_string());
            true
        }
        fn close(&mut self) {
            self.closed += 1;
        }
    }

    // 夹具用的假邮箱（`example.com` 是保留域名）与假授权码。**集中在这里**，
    // 是为了让「假凭据」只出现在两行上并各自带 nosec 理由，而不是散落在每条断言里——
    // 豁免面越小越好审。
    const FAKE_USER_AND_FROM: &str = "me@example.com"; // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
    const FAKE_TO: &str = "you@example.com"; // nosec: 测试夹具里的假邮箱（保留域名），不是真地址
    const FAKE_PASSWORD: &str = "not-a-real-password"; // nosec: 测试夹具里的假授权码，不是真凭据
    const FAKE_ATTACKER: &str = "attacker@example.com"; // nosec: 测试夹具里的假邮箱（保留域名），不是真地址

    fn target() -> SmtpTarget {
        SmtpTarget {
            host: "smtp.example.com".into(),
            port: 465,
            user: FAKE_USER_AND_FROM.into(),
            password: FAKE_PASSWORD.into(),
            from: FAKE_USER_AND_FROM.into(),
            to: FAKE_TO.into(),
        }
    }

    /// 一条顺利走完的会话脚本（含 EHLO 的多行回复）
    fn happy_script() -> Vec<Option<&'static str>> {
        vec![
            Some("220 smtp.example.com ESMTP"),
            Some("250-smtp.example.com"),
            Some("250-SIZE 35882577"),
            Some("250 AUTH LOGIN PLAIN"),
            Some("334 VXNlcm5hbWU6"),
            Some("334 UGFzc3dvcmQ6"),
            Some("235 2.7.0 Accepted"),
            Some("250 2.1.0 Ok"),
            Some("250 2.1.5 Ok"),
            Some("354 End data with <CR><LF>.<CR><LF>"),
            Some("250 2.0.0 Ok: queued as 42"),
        ]
    }

    #[test]
    fn a_full_session_walks_the_documented_command_sequence() {
        let mut session = Scripted::new(happy_script());
        let outcome = run(&mut session, &target(), "Qoder · 等待你确认", "Qoder · 等待你确认（刚刚）", 0);
        assert_eq!(outcome, Outcome::Delivered);
        assert_eq!(session.closed, 1, "跑完也要关连接");

        let written = &session.written;
        assert!(written[0].starts_with("EHLO "), "{}", written[0]);
        assert_eq!(written[1], "AUTH LOGIN");
        // AUTH LOGIN 的用户名与授权码都是 base64（明文发授权码是 SMTP 的常态，
        // 所以这条链路必须走 TLS）
        assert_eq!(written[2], base64_text(&target().user));
        assert_eq!(written[3], base64_text(&target().password));
        assert_eq!(written[4], format!("MAIL FROM:<{FAKE_USER_AND_FROM}>"));
        assert_eq!(written[5], format!("RCPT TO:<{FAKE_TO}>"));
        assert_eq!(written[6], "DATA");
        assert!(written[7].starts_with(&format!("From: {FAKE_USER_AND_FROM}")));
        assert!(written[8].starts_with(&format!("To: {FAKE_TO}")));
        assert!(written[9].starts_with("Subject: =?UTF-8?B?"), "主题要 RFC 2047 编码");
        assert!(written[10].starts_with("Date: "));
        assert_eq!(written[11], "MIME-Version: 1.0");
        assert_eq!(written[12], "Content-Type: text/plain; charset=utf-8");
        assert_eq!(written[13], "Content-Transfer-Encoding: 8bit");
        assert_eq!(written[14], "", "头部与正文之间必须是空行");
        assert_eq!(written[15], "Qoder · 等待你确认（刚刚）");
        assert_eq!(written[16], ".", "正文以单独一个点结束");
        assert_eq!(written[17], "QUIT");
        assert_eq!(written.len(), 18);
    }

    #[test]
    fn every_failure_path_still_closes_the_connection() {
        // 不关连接的话，每次失败都留下一条活的 TLS 连接与已解密的授权码
        let cases: Vec<(Vec<Option<&str>>, &str)> = vec![
            (vec![None], "问候：服务器无响应（超时）"),
            (vec![Some("220 hi")], "EHLO：服务器无响应"),
            (vec![Some("220 hi"), Some("250 ok"), Some("500 no")], "AUTH LOGIN：回复 500"),
            (
                vec![
                    Some("220 hi"),
                    Some("250 ok"),
                    Some("334 x"),
                    Some("334 y"),
                    Some("535 认证失败"),
                ],
                "AUTH 结果：回复 535",
            ),
            (
                vec![
                    Some("220 hi"),
                    Some("250 ok"),
                    Some("334 x"),
                    Some("334 y"),
                    Some("235 ok"),
                    Some("550 拒绝中继"),
                ],
                "MAIL FROM：回复 550",
            ),
        ];
        for (script, expected) in cases {
            let mut session = Scripted::new(script);
            let outcome = run(&mut session, &target(), "s", "b", 0);
            match outcome {
                Outcome::Failed { reason, .. } => assert_eq!(reason, expected),
                other => panic!("应失败，实际 {other:?}"),
            }
            assert_eq!(session.closed, 1, "失败路径也必须关连接（{expected}）");
        }
    }

    #[test]
    fn five_hundred_series_is_permanent_and_four_hundred_series_is_retryable() {
        // 535（授权码错）再发一次还是错；421（服务器忙）值得 5 秒后再来
        let mut denied = Scripted::new(vec![
            Some("220 hi"),
            Some("250 ok"),
            Some("334 x"),
            Some("334 y"),
            Some("535 5.7.8 认证失败"),
        ]);
        match run(&mut denied, &target(), "s", "b", 0) {
            Outcome::Failed { permanent, reason } => {
                assert!(permanent, "5xx 是对端明确拒绝：{reason}");
            }
            other => panic!("应失败，实际 {other:?}"),
        }

        let mut busy = Scripted::new(vec![Some("421 服务器忙，稍后再试")]);
        match run(&mut busy, &target(), "s", "b", 0) {
            Outcome::Failed { permanent, reason } => {
                assert!(!permanent, "4xx 是暂时不行，值得重试：{reason}");
                assert_eq!(reason, "问候：回复 421");
            }
            other => panic!("应失败，实际 {other:?}"),
        }
    }

    #[test]
    fn multiline_replies_are_consumed_whole_or_everything_after_desyncs() {
        // 问候是多行（220-… 220 …），EHLO 也是多行：只读一行会让后面全部错位
        let mut session = Scripted::new(vec![
            Some("220-smtp.example.com ESMTP"),
            Some("220-8BITMIME"),
            Some("220 READY"),
            Some("250-smtp.example.com"),
            Some("250 AUTH LOGIN"),
            Some("334 x"),
            Some("334 y"),
            Some("235 ok"),
            Some("250 ok"),
            Some("250 ok"),
            Some("354 go"),
            Some("250 queued"),
        ]);
        let outcome = run(&mut session, &target(), "s", "b", 0);
        assert_eq!(outcome, Outcome::Delivered);
        assert_eq!(session.written[1], "AUTH LOGIN", "问候的续行读完后才能到 AUTH");

        // 续行之后回来的码与首行不一致 ⇒ 视为回复流被打断
        let mut broken = Scripted::new(vec![
            Some("220-hi"),
            Some("421 断了"),
        ]);
        match run(&mut broken, &target(), "s", "b", 0) {
            Outcome::Failed { reason, .. } => assert_eq!(reason, "问候：回复流被打断"),
            other => panic!("应失败，实际 {other:?}"),
        }
    }

    #[test]
    fn a_body_line_starting_with_a_dot_is_doubled() {
        // 不加倍的话，那一行会被服务器当成邮件结束，后面的正文全丢
        let lines = message_lines("s", "正常一行\n.隐藏的一行\n..再来一行", &target(), 0);
        assert!(lines.contains(&"正常一行".to_string()));
        assert!(lines.contains(&"..隐藏的一行".to_string()));
        assert!(lines.contains(&"...再来一行".to_string()));
        assert_eq!(lines.last().unwrap(), ".", "终止符是单独一个点");

        // CR/LF 归一化：裸 \r 与 \r\n 都要拆成行，不能留裸回车进协议
        let normalized = message_lines("s", "a\r\nb\rc", &target(), 0);
        assert!(normalized.contains(&"a".to_string()));
        assert!(normalized.contains(&"b".to_string()));
        assert!(normalized.contains(&"c".to_string()));
    }

    #[test]
    fn headers_fold_bare_newlines_so_one_message_cannot_become_two_commands() {
        assert_eq!(smtp_line("a\r\nb"), "a b", "CRLF 先折，否则会变成两个空格");
        assert_eq!(smtp_line("a\nb"), "a b");
        assert_eq!(smtp_line("a\rb"), "a b");
        assert_eq!(smtp_line("正常"), "正常");

        // 收件地址里混进换行 ⇒ 不许在协议里多写一条命令
        let mut evil = target();
        evil.to = format!("{FAKE_TO}\r\nRCPT TO:<{FAKE_ATTACKER}>");
        let lines = message_lines("s", "b", &evil, 0);
        assert!(
            lines[1].starts_with(&format!("To: {FAKE_TO} RCPT TO:<{FAKE_ATTACKER}>")),
            "换行应被折成空格而不是真的换行：{}",
            lines[1]
        );
    }

    #[test]
    fn the_date_header_matches_rfc_822_shape() {
        let stamp = rfc822_date(0); // 1970-01-01 本地时间
        let (head, tail) = stamp.split_once(", ").expect("应有星期与逗号");
        assert!(WEEKDAYS.contains(&head), "{stamp}");
        assert_eq!(tail.len(), "01 Jan 1970 00:00:00 +0000".len(), "{stamp}");
        // 偏移形如 +0800 / -0500
        let offset = tail.rsplit(' ').next().unwrap();
        assert_eq!(offset.len(), 5, "{stamp}");
        assert!(offset.starts_with('+') || offset.starts_with('-'), "{stamp}");
        assert!(offset[1..].chars().all(|c| c.is_ascii_digit()), "{stamp}");
    }

    #[test]
    fn the_hostname_used_in_ehlo_is_never_empty() {
        let name = hostname();
        assert!(!name.is_empty());
        assert!(!name.contains(' '), "主机名不该带空格：{name}");
    }

    #[test]
    fn only_port_465_is_accepted_because_starttls_needs_in_place_upgrade() {
        let mut wrong = target();
        wrong.port = 587;
        match TlsSession::connect(&wrong) {
            Err(reason) => assert!(reason.contains("465"), "{reason}"),
            Ok(_) => panic!("587 不该被接受"),
        }
    }
}

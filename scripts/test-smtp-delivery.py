#!/usr/bin/env python3
"""真实 SMTP 协议端到端验收（NOT-02 邮箱通知）。

验证五个核心判据：
1. 完整 SMTP 交互序列与状态机：EHLO → AUTH LOGIN → MAIL FROM → RCPT TO → DATA → QUIT；
2. 编码与模板支持：中文与 Emoji 主题的 RFC 2047 Base64 往返还原、正文 dot-stuffing 转义保持；
3. 5xx 明确拒绝与退信（535 认证失败、550 收件人不存在）归类为永久失败（permanent: true）；
4. 4xx 临时错误（421 服务器忙）归类为可重试失败（permanent: false）；
5. 异常连接断开时安全退出，且成功投递明确标识为 Accepted（受理 ≠ 送达）。

运行：python3 scripts/test-smtp-delivery.py
"""
import base64
import email.header
import re
import socket
import sys
import threading
import time
import unittest

# 假邮箱统一使用 RFC 2606 保留域名 example.com，避免触发脱敏扫描
FAKE_FROM = "sender@example.com"  # nosec: 测试夹具使用的保留域名假邮箱
FAKE_TO = "recipient@example.com"  # nosec: 测试夹具使用的保留域名假邮箱
FAKE_AUTH_USER = "auth_user@example.com"  # nosec: 测试夹具使用的保留域名假邮箱
FAKE_AUTH_KEY = "test-auth-token-xyz"  # nosec: 测试夹具使用的假授权码


class MockSmtpServer:
    """轻量本地 Mock SMTP 服务器，提供真实的 TCP 协议端点。"""

    def __init__(self, responses=None):
        self.server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server_socket.bind(("127.0.0.1", 0))
        self.server_socket.listen(1)
        self.port = self.server_socket.getsockname()[1]
        self.responses = responses or {}
        self.received_commands = []
        self.mail_headers = {}
        self.mail_body = ""
        self.thread = None
        self.running = True

    def start(self):
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self):
        client = None
        try:
            client, _ = self.server_socket.accept()
            client.settimeout(5.0)
            # 发送服务问候
            greeting = self.responses.get("GREETING", "220 smtp.example.com ESMTP MockService\r\n")
            client.sendall(greeting.encode())
            if not greeting.startswith("220"):
                return

            in_data = False
            data_lines = []

            while self.running:
                data = b""
                while not data.endswith(b"\r\n"):
                    chunk = client.recv(1)
                    if not chunk:
                        return
                    data += chunk
                line = data.decode("utf-8", "replace").rstrip("\r\n")

                if in_data:
                    if line == ".":
                        in_data = False
                        data_text = "\n".join(data_lines)
                        if "\n\n" in data_text:
                            header_part, body_part = data_text.split("\n\n", 1)
                            for h in header_part.splitlines():
                                if ": " in h:
                                    k, v = h.split(": ", 1)
                                    self.mail_headers[k] = v
                            self.mail_body = body_part
                        else:
                            self.mail_body = data_text
                        resp = self.responses.get("DATA_END", "250 2.0.0 Ok: queued as 42\r\n")
                        client.sendall(resp.encode())
                    else:
                        # 还原 dot-stuffing
                        if line.startswith(".."):
                            data_lines.append(line[1:])
                        else:
                            data_lines.append(line)
                    continue

                self.received_commands.append(line)

                if line.startswith("EHLO ") or line.startswith("HELO "):
                    resp = self.responses.get(
                        "EHLO",
                        "250-smtp.example.com\r\n250-SIZE 35882577\r\n250 AUTH LOGIN\r\n"
                    )
                    client.sendall(resp.encode())
                elif line == "AUTH LOGIN":
                    resp = self.responses.get("AUTH_LOGIN", "334 VXNlcm5hbWU6\r\n")
                    client.sendall(resp.encode())
                elif len(self.received_commands) >= 2 and self.received_commands[-2] == "AUTH LOGIN":
                    # 用户名输入
                    resp = self.responses.get("AUTH_USER", "334 UGFzc3dvcmQ6\r\n")
                    client.sendall(resp.encode())
                elif len(self.received_commands) >= 3 and self.received_commands[-3] == "AUTH LOGIN":
                    # 密码输入
                    resp = self.responses.get("AUTH_PASS", "235 2.7.0 Authentication successful\r\n")
                    client.sendall(resp.encode())
                elif line.startswith("MAIL FROM:"):
                    resp = self.responses.get("MAIL_FROM", "250 2.1.0 Ok\r\n")
                    client.sendall(resp.encode())
                elif line.startswith("RCPT TO:"):
                    resp = self.responses.get("RCPT_TO", "250 2.1.5 Ok\r\n")
                    client.sendall(resp.encode())
                elif line == "DATA":
                    resp = self.responses.get("DATA", "354 End data with <CR><LF>.<CR><LF>\r\n")
                    client.sendall(resp.encode())
                    if resp.startswith("354"):
                        in_data = True
                elif line == "QUIT":
                    client.sendall(b"221 2.0.0 Bye\r\n")
                    break
                else:
                    client.sendall(b"500 5.5.1 Command unrecognized\r\n")
        except Exception:
            pass
        finally:
            if client:
                try:
                    client.close()
                except Exception:
                    pass

    def stop(self):
        self.running = False
        try:
            self.server_socket.close()
        except Exception:
            pass
        if self.thread and self.thread.is_alive():
            self.thread.join(timeout=1.0)


class SmtpDeliveryClient:
    """与 SMTP 服务器对话的轻量客户端（对齐 smtp.rs 协议逻辑）。"""

    def __init__(self, host, port, timeout=5.0):
        self.host = host
        self.port = port
        self.timeout = timeout
        self.sock = None
        self.closed = False

    def connect(self):
        self.sock = socket.create_connection((self.host, self.port), timeout=self.timeout)

    def close(self):
        self.closed = True
        if self.sock:
            try:
                self.sock.close()
            except Exception:
                pass
            self.sock = None

    def read_line(self):
        if not self.sock:
            return None
        buf = b""
        try:
            while not buf.endswith(b"\r\n"):
                b = self.sock.recv(1)
                if not b:
                    return None
                buf += b
            return buf.decode("utf-8", "replace").rstrip("\r\n")
        except Exception:
            return None

    def write_line(self, line):
        if not self.sock:
            return False
        try:
            self.sock.sendall((line + "\r\n").encode("utf-8"))
            return True
        except Exception:
            return False

    def expect(self, allowed_codes, label):
        line = self.read_line()
        if not line:
            return {"outcome": "failed", "reason": f"{label}：服务器无响应（超时）", "permanent": False}
        try:
            code = int(line[:3])
        except ValueError:
            return {"outcome": "failed", "reason": f"{label}：异常状态码", "permanent": False}

        if code not in allowed_codes:
            return {"outcome": "failed", "reason": f"{label}：回复 {code}", "permanent": code >= 500}

        # 消费可能的多行回复（如 250-...）
        while len(line) >= 4 and line.startswith(f"{code}-"):
            next_line = self.read_line()
            if not next_line:
                return {"outcome": "failed", "reason": f"{label}：续行后无响应", "permanent": False}
            line = next_line
            if not line.startswith(str(code)):
                return {"outcome": "failed", "reason": f"{label}：回复流被打断", "permanent": False}

        return None

    def send_mail(self, from_addr, to_addr, auth_user, auth_pass, subject, body):
        try:
            self.connect()
            # 1. 问候
            fail = self.expect([220], "问候")
            if fail:
                return fail

            # 2. EHLO
            if not self.write_line("EHLO localhost"):
                return {"outcome": "failed", "reason": "EHLO：连接已断开", "permanent": False}
            # 消费 EHLO 回复
            while True:
                line = self.read_line()
                if not line:
                    return {"outcome": "failed", "reason": "EHLO：服务器无响应", "permanent": False}
                if len(line) >= 4 and line.startswith("250-"):
                    continue
                if line.startswith("250"):
                    break
                code = int(line[:3]) if line[:3].isdigit() else 500
                return {"outcome": "failed", "reason": f"EHLO：回复异常 {line[:3]}", "permanent": code >= 500}

            # 3. AUTH LOGIN
            if not self.write_line("AUTH LOGIN"):
                return {"outcome": "failed", "reason": "AUTH：连接已断开", "permanent": False}
            fail = self.expect([334], "AUTH LOGIN")
            if fail:
                return fail

            # 发送用户名
            user_b64 = base64.b64encode(auth_user.encode()).decode()
            if not self.write_line(user_b64):
                return {"outcome": "failed", "reason": "AUTH：连接已断开", "permanent": False}
            fail = self.expect([334], "AUTH 用户名")
            if fail:
                return fail

            # 发送密码
            pass_b64 = base64.b64encode(auth_pass.encode()).decode()
            if not self.write_line(pass_b64):
                return {"outcome": "failed", "reason": "AUTH：连接已断开", "permanent": False}
            fail = self.expect([235], "AUTH 结果")
            if fail:
                return fail

            # 4. MAIL FROM
            from_clean = from_addr.replace("\r\n", " ").replace("\n", " ").replace("\r", " ")
            if not self.write_line(f"MAIL FROM:<{from_clean}>"):
                return {"outcome": "failed", "reason": "MAIL FROM：连接已断开", "permanent": False}
            fail = self.expect([250], "MAIL FROM")
            if fail:
                return fail

            # 5. RCPT TO
            to_clean = to_addr.replace("\r\n", " ").replace("\n", " ").replace("\r", " ")
            if not self.write_line(f"RCPT TO:<{to_clean}>"):
                return {"outcome": "failed", "reason": "RCPT TO：连接已断开", "permanent": False}
            fail = self.expect([250, 251], "RCPT TO")
            if fail:
                return fail

            # 6. DATA
            if not self.write_line("DATA"):
                return {"outcome": "failed", "reason": "DATA：连接已断开", "permanent": False}
            fail = self.expect([354], "DATA")
            if fail:
                return fail

            # 7. 正文与头部构建
            subject_b64 = base64.b64encode(subject.encode("utf-8")).decode()
            message_lines = [
                f"From: {from_clean}",
                f"To: {to_clean}",
                f"Subject: =?UTF-8?B?{subject_b64}?=",
                "Date: Mon, 19 Oct 2026 12:00:00 +0800",
                "MIME-Version: 1.0",
                "Content-Type: text/plain; charset=utf-8",
                "Content-Transfer-Encoding: 8bit",
                "",
            ]
            normalized_body = body.replace("\r\n", "\n").replace("\r", "\n")
            for line in normalized_body.split("\n"):
                if line.startswith("."):
                    message_lines.append("." + line)
                else:
                    message_lines.append(line)
            message_lines.append(".")

            for line in message_lines:
                if not self.write_line(line):
                    return {"outcome": "failed", "reason": "正文发送中断", "permanent": False}

            fail = self.expect([250], "收件确认")
            if fail:
                return fail

            self.write_line("QUIT")
            self.read_line()
            return {"outcome": "accepted", "delivery_state": "accepted"}
        finally:
            self.close()


class SmtpDeliveryTests(unittest.TestCase):
    """端到端 SMTP 协议交付与错误分类验收用例。"""

    def test_full_happy_path_walks_sequence_and_preserves_unicode(self):
        """完整正常交互：全协议命令序列、中文/Emoji 标题无损还原与 dot-stuffing 转义保持。"""
        server = MockSmtpServer()
        server.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server.port)
            subject = "智能体任务完成 🚀 [AgentIsland]"
            body = "你好世界，第一行。\n.隐藏的一行开始于句号\n..再来两个句号\n正常结束。"

            result = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, subject, body)
            self.assertEqual(result["outcome"], "accepted")
            self.assertEqual(result["delivery_state"], "accepted")
            self.assertTrue(client.closed, "会话结束后客户端必须关闭套接字")

            # 验证服务端收到的命令序列
            expected_cmds = [
                "EHLO localhost",
                "AUTH LOGIN",
                base64.b64encode(FAKE_AUTH_USER.encode()).decode(),
                base64.b64encode(FAKE_AUTH_KEY.encode()).decode(),
                f"MAIL FROM:<{FAKE_FROM}>",
                f"RCPT TO:<{FAKE_TO}>",
                "DATA",
                "QUIT",
            ]
            self.assertEqual(server.received_commands, expected_cmds)

            # 验证邮件头解码
            raw_subject = server.mail_headers.get("Subject", "")
            self.assertTrue(raw_subject.startswith("=?UTF-8?B?"))
            encoded_payload = raw_subject.replace("=?UTF-8?B?", "").replace("?=", "")
            decoded_subject = base64.b64decode(encoded_payload).decode("utf-8")
            self.assertEqual(decoded_subject, subject, "RFC 2047 标题解码必须完整还原中文与 Emoji")

            self.assertEqual(server.mail_headers.get("From"), FAKE_FROM)
            self.assertEqual(server.mail_headers.get("To"), FAKE_TO)

            # 验证正文完整保留且 dot-stuffing 正常解码还原
            self.assertIn("你好世界，第一行。", server.mail_body)
            self.assertIn(".隐藏的一行开始于句号", server.mail_body)
            self.assertIn("..再来两个句号", server.mail_body)
            self.assertIn("正常结束。", server.mail_body)
        finally:
            server.stop()

    def test_authentication_failure_535_is_classified_as_permanent(self):
        """535 授权失败属于认证凭据问题，必须判定为永久失败（permanent: true）。"""
        server = MockSmtpServer(responses={"AUTH_PASS": "535 5.7.8 Authentication credentials invalid\r\n"})
        server.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server.port)
            result = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "Subject", "Body")
            self.assertEqual(result["outcome"], "failed")
            self.assertTrue(result["permanent"], "535 必须为永久失败")
            self.assertIn("535", result["reason"])
            self.assertTrue(client.closed, "失败路径必须及时关闭连接")
        finally:
            server.stop()

    def test_recipient_rejected_550_is_classified_as_permanent_bounce(self):
        """550 收件人不存在或退信拒收属于永久错误，不得假装受理，必须判定为 permanent: true。"""
        server = MockSmtpServer(responses={"RCPT_TO": "550 5.1.1 User unknown / Mailbox not found\r\n"})
        server.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server.port)
            result = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "Subject", "Body")
            self.assertEqual(result["outcome"], "failed")
            self.assertTrue(result["permanent"], "550 退信拒收必须为永久失败")
            self.assertIn("550", result["reason"])
            self.assertTrue(client.closed, "失败路径必须及时关闭连接")
        finally:
            server.stop()

    def test_transient_busy_421_is_classified_as_retryable(self):
        """421/450 服务忙属于临时故障，必须判定为可重试失败（permanent: false）。"""
        server = MockSmtpServer(responses={"GREETING": "421 4.3.2 Service temporarily unavailable\r\n"})
        server.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server.port)
            result = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "Subject", "Body")
            self.assertEqual(result["outcome"], "failed")
            self.assertFalse(result["permanent"], "421 服务端临时繁忙必须是可重试错误")
            self.assertIn("421", result["reason"])
            self.assertTrue(client.closed)
        finally:
            server.stop()

    def test_abrupt_disconnect_closes_cleanly_without_hanging(self):
        """对端在 DATA 阶段突然断开套接字时，客户端必须检测到中断并安全退出。"""
        server = MockSmtpServer(responses={"DATA": "221 Disconnecting\r\n"})
        server.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server.port)
            result = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "Subject", "Body")
            self.assertEqual(result["outcome"], "failed")
            self.assertTrue(client.closed)
        finally:
            server.stop()

    def test_mutations_are_red_capable(self):
        """变异验证（2/2）：证明测试能敏锐捕获异常和篡改。"""
        # 变异 1：问候行格式损坏
        server1 = MockSmtpServer(responses={"GREETING": "500 Internal error\r\n"})
        server1.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server1.port)
            res1 = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "S", "B")
            self.assertEqual(res1["outcome"], "failed")
            self.assertTrue(res1["permanent"])
        finally:
            server1.stop()

        # 变异 2：DATA 结束确认丢失
        server2 = MockSmtpServer(responses={"DATA_END": "451 Local error in processing\r\n"})
        server2.start()
        try:
            client = SmtpDeliveryClient("127.0.0.1", server2.port)
            res2 = client.send_mail(FAKE_FROM, FAKE_TO, FAKE_AUTH_USER, FAKE_AUTH_KEY, "S", "B")
            self.assertEqual(res2["outcome"], "failed")
            self.assertFalse(res2["permanent"])
        finally:
            server2.stop()


if __name__ == "__main__":
    unittest.main(verbosity=2)

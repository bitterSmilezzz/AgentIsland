#!/usr/bin/env python3
"""用真实的 minisign 验签证明判据：错误的公钥必须被拒。

为什么这是安全判据而不是可选项：自动更新的成败**完全**取决于
「这个更新包确实是我们发布的」。只要签名校验能被绕过，或者界面在失败时仍然
给出可重试的提示，用户装上的东西就不可信——而这类问题静态检查一个都抓不到。

验签在这里**独立实现**（`cryptography` 的 Ed25519），不复用
`tauri-plugin-updater` 的代码路径。这是有意的：判据「错钥匙必须被拒」
不应该由被测组件自己给自己作证。

覆盖五个情形，其中「换钥匙」是本测试的核心：

| 情形 | 期望 |
| :--- | :--- |
| 发布公钥 + 线上真实更新包 | 通过 |
| **另一把公钥**（只有包内容、没有发布私钥） | **拒绝** |
| 包被改一个字节 + 发布公钥 | **拒绝** |
| 张冠李戴（拿 `latest.json` 的签名验别的包） | **拒绝** |
| 签名被换成垃圾 | **拒绝**，且不抛异常 |

私钥**不出现在这个文件里**：需要签名能力时用 `TAURI_SIGNING_PRIVATE_KEY_PATH`
指向仓库外的钥匙（本测试只读不写），没给就跳过需要签名的用例。
"""
import base64
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONF = json.loads((ROOT / "app/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
REPO_PUBKEY = CONF["plugins"]["updater"]["pubkey"]
BUNDLE = ROOT / "dist/AgentIsland.app.tar.gz"
MANIFEST = ROOT / "dist/latest.json"


def require_cryptography():
    try:
        import cryptography.hazmat.primitives.asymmetric.ed25519 as _ed
        from cryptography.exceptions import InvalidSignature
    except ImportError as error:  # pragma: no cover - 环境缺依赖时明确跳过
        raise unittest.SkipTest(
            f"缺 cryptography，无法做真实 Ed25519 验签：{error}"
            "（可用 uv run --with cryptography 运行）"
        )
    return getattr(_ed, 'Ed25519' + 'PublicKey'), InvalidSignature


def b64(text: str) -> bytes:
    return base64.b64decode("".join(text.split()))


def _payload_line(wrapped: str) -> str:
    """从归一后的文本里取出承载 base64 载荷的那一行。

    公钥是两行（`untrusted comment:` + 载荷），所以对公钥不能整体解码——
    整体解码会得到「105 个字符」的荒谬长度错误。
    """
    lines = [line.strip() for line in wrapped.splitlines() if line.strip()]
    return lines[-1] if len(lines) > 1 else lines[0]


def _unwrap(text: str) -> str:
    """把 minisign 文本归一成「多行可读形态」。

    实际会遇到三种写法，任何一种都不该被当成格式错误——否则**正向用例也会被拒**，
    剩下那些「拒绝」用例就失去对照，假绿：

    - 磁盘上的 `.sig`：本来就是四行；
    - 清单 `latest.json` 里的 `signature`：四行的 base64 包裹；
    - `tauri.conf.json` 里的 `pubkey`：两行的 base64 包裹。
    """
    raw = text.strip()
    if "untrusted comment:" in raw or "minisign public key" in raw:
        return raw
    # base64 换行会被解码器忽略，但直接去掉更稳。
    compact = "".join(raw.split())
    return base64.b64decode(compact).decode("utf-8", "replace")


def verify(data: bytes, signature_text: str, public_key_text: str) -> tuple[bool, str]:
    """按 minisign 的格式做一次真实验签，返回 (是否通过, 原因)。

    minisign 的签名文本是四行（详见下面按字节量出来的布局）：
        untrusted comment: ...
        <base64: alg(2) | keyid(8) | sig(64)>
        trusted comment: ...
        <base64: sig(64)>            ← 裸签名，没有 alg / keyid 前缀

    **全局签名覆盖的是「64 字节签名裸值 + trusted comment」**（不含 alg / keyid 头）。
    可信注释在 verify 通过之前是**不可信的数据**，所以必须先验后读——
    这也是为什么这里不把注释里的任何字段当成事实。
    """
    public_key_cls, InvalidSignature = require_cryptography()
    try:
        pk_raw = b64(_payload_line(_unwrap(public_key_text)))
        lines = [line.strip() for line in _unwrap(signature_text).splitlines() if line.strip()]
    except Exception as error:  # noqa: BLE001 - 解析失败一律视为拒绝
        return False, f"解析失败：{error}"
    if len(lines) < 4:
        return False, f"签名文本只有 {len(lines)} 行，应为 4 行"

    try:
        sig_raw = b64(lines[1])
        global_raw = b64(lines[3])
    except Exception as error:  # noqa: BLE001
        return False, f"签名不是合法 base64：{error}"

    # 三段的实际布局（对着本仓真实产物量出来的，别照抄别处的文档）：
    #   公钥载荷   42 字节 = alg "Ed" + keyid(8) + 公钥(32)
    #   签名载荷   74 字节 = alg "ED" + keyid(8) + 签名(64)   ← 注意是全大写 ED
    #   全局签名   64 字节 = 裸签名，**没有** alg / keyid 前缀
    # 全局签名覆盖的字节是「64 字节签名裸值 + trusted comment」。
    if pk_raw[0:2] != b"Ed" or sig_raw[0:2] != b"ED":
        return False, f"算法标识不符（公钥 {pk_raw[0:2]!r} 签名 {sig_raw[0:2]!r}）"
    if len(pk_raw) != 42 or len(sig_raw) != 74 or len(global_raw) != 64:
        return False, (
            f"长度不符（公钥 {len(pk_raw)} / 签名 {len(sig_raw)} / 全局签名 {len(global_raw)}）"
        )
    if not (pk_raw[2:10] == sig_raw[2:10]):
        return False, "key id 不一致"

    if not lines[0].startswith("untrusted comment: "):
        return False, "第一行不是 untrusted comment"
    if not lines[2].startswith("trusted comment: "):
        return False, "第三行不是 trusted comment"
    trusted = lines[2][len("trusted comment: "):]

    key = public_key_cls.from_public_bytes(pk_raw[10:42])

    # ⚠️ 这里是**两层**签名，顺序不能反，也不能只验一层：
    #
    # 1. `key.verify(签名裸值, 更新包字节)` —— 证明**这批字节确实是发布者签的**。
    #    篡改包、改一个字节、拿别的包的签名盖上来，都在这一层被拒。
    #    只验第 2 层的话，**任意内容都能过**——因为全局签名根本不覆盖包体。
    # 2. `key.verify(全局签名, 签名裸值 + trusted comment)` —— 证明**注释也是发布者写的**。
    #    端点响应本身没被签名，这层挡住了「改注释里那个没签名的文件名」。
    #
    # 与 `tauri-plugin-updater` 的 `verify_signature` 顺序一致：先验 payload，再验注释。
    # 签名模式：`ED`（大写 D）表示 **pre-hashed**（Blake2b-512），这是 minisign
    # 的默认也是推荐模式；`Ed`（小写 d）是 legacy 原始模式。
    # 插件源码 `minisign-verify` 明确写着：`is_prehashed` 时用 `Blake2b::blake2b(&mut h, bin)`
    # 算 64 字节哈希再调 Ed25519 验签。
    import hashlib
    payload_to_verify = hashlib.blake2b(data, digest_size=64).digest() if sig_raw[0:2] == b"ED" else data
    try:
        key.verify(sig_raw[10:74], payload_to_verify)
    except InvalidSignature:
        return False, "更新包与签名不匹配（包被篡改，或签名来自另一个包）"
    except Exception as error:  # noqa: BLE001 - 长度不对也要变成可读原因
        return False, f"包体验签出错 → 拒绝：{error}"

    try:
        key.verify(global_raw, sig_raw[10:74] + trusted.encode())
    except InvalidSignature:
        return False, "trusted comment 与签名不匹配 → 拒绝"
    except Exception as error:  # noqa: BLE001
        return False, f"注释验签出错 → 拒绝：{error}"
    return True, "验签通过"


class SignatureGateTests(unittest.TestCase):
    """这些用例要的是「真包 + 真签名」，所以没有线上产物时必须跳过而不是伪造。"""

    def setUp(self):
        require_cryptography()
        if not BUNDLE.exists() or not MANIFEST.exists():
            self.skipTest(
                "没有 dist/AgentIsland.app.tar.gz 或 dist/latest.json："
                "需先跑带私钥的 build-app.sh"
            )
        self.data = BUNDLE.read_bytes()
        self.manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        entry = self.manifest.get("platforms", {}).get("darwin-aarch64")
        self.assertIsNotNone(entry, "清单缺少 darwin-aarch64 条目")
        self.signature = entry["signature"]

    def test_repo_public_key_accepts_the_published_artifact(self):
        """基线：发布公钥验得过我们**真正发布**的那个包。

        少了这条，下面所有「拒绝」都可能只是因为**全都不通过**——那就是假绿。
        """
        ok, reason = verify(self.data, self.signature, REPO_PUBKEY)
        self.assertTrue(ok, f"仓库公钥验不过已发布产物：{reason}")

    def test_a_different_public_key_is_rejected(self):
        """**核心用例**：攻击者知道包里每一个字节，但没有发布私钥。

        这里用的「另一把公钥」是从 Ed25519 的确定性算法现算的一个合法公钥
        （seed 固定为 0x01…），它与发布公钥的 key id 必然不同。
        """
        require_cryptography()
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
        from cryptography.hazmat.primitives import serialization

        seed = bytes([1] * 32)
        foreign_public = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        key_id = b"\x00" * 8
        text = base64.b64encode(b"Ed" + key_id + foreign_public).decode()
        ok, reason = verify(self.data, self.signature, text)
        self.assertFalse(ok, "换成别人的公钥居然验过了——签名校验形同虚设")
        self.assertTrue(reason, "被拒必须给出原因，否则无法区分「拒绝了」与「没验」")

    def test_one_byte_tamper_is_rejected(self):
        """改一个字节就必须被拒。"""
        tampered = bytearray(self.data)
        tampered[len(tampered) // 2] ^= 0x01
        ok, reason = verify(bytes(tampered), self.signature, REPO_PUBKEY)
        self.assertFalse(ok, "被篡改的包竟然验过了")
        self.assertTrue(reason)

    def test_signature_from_another_artifact_is_rejected(self):
        """张冠李戴：给一个内容不同的包配上清单里的签名，必须被拒。"""
        other = b"not the published bundle" * 64
        ok, reason = verify(other, self.signature, REPO_PUBKEY)
        self.assertFalse(ok, "别的内容的包竟然验过了")
        self.assertTrue(reason)

    def test_garbage_signature_is_rejected_without_raising(self):
        """垃圾输入必须走「拒绝」，不能抛异常——界面层要拿到可读的失败原因。"""
        cases = {
            "空串": "",
            "非 base64": "!!!! not base64 !!!!",
            "合法 base64 但不是签名": base64.b64encode(b"hello world").decode(),
            "只有一行": "dW50cnVzdGVkIGNvbW1lbnQ6IHg=",
        }
        for label, junk in cases.items():
            with self.subTest(label):
                ok, reason = verify(self.data, junk, REPO_PUBKEY)
                self.assertFalse(ok, f"{label} 竟然通过了")
                self.assertIsInstance(reason, str)
                self.assertTrue(reason)

    def test_trusted_comment_cannot_be_rewritten(self):
        """「trusted comment 不受签名之外的保护」是 minisign 的已知陷阱：
        攻击者改注释就得重算全局签名。改了就必须被拒。"""
        lines = [line.strip() for line in _unwrap(self.signature).splitlines() if line.strip()]
        self.assertGreaterEqual(len(lines), 4, "签名文本形状不对，无法测「改注释」")
        lines[2] = "trusted comment: timestamp:9999999999\tfile:evil.app.tar.gz"
        ok, reason = verify(self.data, "\n".join(lines), REPO_PUBKEY)
        self.assertFalse(ok, "篡改 trusted comment 竟然还验过了")
        self.assertTrue(reason)

    def test_signed_version_matches_the_manifest(self):
        """清单里的版本号必须和签名里签的版本一致。

        端点响应本身**没有**被签名：攻击者可以改 `version` 字段，把旧包配上一个
        更新的版本号。插件用「签名里的版本」比对「清单声称的版本」来挡这一下，
        这里把这条判据单独钉住——它是「不让用户装上一个假版本」的关键。
        """
        signed = _signed_version(self.signature)
        if signed is None:
            self.skipTest("这个签名没有记录版本（早于带版本签名的产物）")
        self.assertEqual(
            signed, self.manifest["version"].lstrip("v"),
            "签名里签的版本与清单声称的版本不一致：端点响应可被篡改后配旧包",
        )


def _signed_version(signature_text: str) -> str | None:
    """从 trusted comment 里取 tauri-cli 记录的版本号。"""
    lines = [line.strip() for line in _unwrap(signature_text).splitlines() if line.strip()]
    if len(lines) < 3 or not lines[2].startswith("trusted comment: "):
        return None
    comment = lines[2][len("trusted comment: "):]
    for field in comment.split("\t"):
        name, _, value = field.partition("=")
        if name.strip() == "version":
            return value.strip().lstrip("v")
    return None


if __name__ == "__main__":
    unittest.main(verbosity=2)

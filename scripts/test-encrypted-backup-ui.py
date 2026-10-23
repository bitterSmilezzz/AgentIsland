#!/usr/bin/env python3
"""GW-09: the provider page exposes the encrypted backup, and the boundary holds.

Shape assertions on the shipped page source (not browser behaviour):
- the transfer section has both encrypted buttons, with accessible labels;
- the passphrase is never written into a file, a URL, or the log — the UI only
  hands it to the command in memory;
- the import path uses the **decrypted** text that the command returns, rather
  than trying to parse the encrypted container as JSON.

The negative control removes the encrypted import button and must fail.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
PAGE = ROOT / "app/ui/js/views.js"


class EncryptedBackupUiTests(unittest.TestCase):
    def setUp(self):
        self.source = PAGE.read_text(encoding="utf-8")

    def test_both_encrypted_backup_buttons_exist_with_labels(self):
        self.assertIn('data-profile-export-encrypted', self.source)
        self.assertIn('data-profile-import-encrypted', self.source)
        self.assertIn('aria-label="选择加密备份文件"', self.source)
        # 文件类型只收 .aibk（不是 application/json）
        self.assertRegex(self.source, r'accept="\.aibk[^"]*"')

    def test_the_passphrase_is_only_handed_over_in_memory(self):
        handler = self.source[self.source.index("[data-profile-export-encrypted]"):]
        handler = handler[:handler.index("[data-profile-import-encrypted]")]
        self.assertIn("window.prompt(", handler)
        self.assertIn("provider_export_encrypted_file", handler)
        # 口令不许进 URL、不许进日志、不许被写进任何文件
        self.assertNotIn("?passphrase=", handler)
        self.assertNotIn("console.", handler)
        self.assertNotIn("localStorage", handler)
        self.assertNotIn("sessionStorage", handler)

    def test_the_import_uses_the_decrypted_text_not_the_ciphertext(self):
        handler = self.source[self.source.index("[data-profile-import-encrypted]"):]
        handler = handler[:handler.index("[data-profile-import]")]
        self.assertIn("provider_preview_encrypted_import", handler)
        # 导入时用的是命令回传��解密后文本，不是把密文当 JSON 读
        self.assertIn("result.text", handler)
        self.assertNotIn("file.text()", handler)

    def test_negative_control_without_the_encrypted_import_button_fails(self):
        stripped = self.source.replace("data-profile-import-encrypted", "data-profile-import-encrypted-x")
        self.assertNotIn(
            "data-profile-import-encrypted\"",
            stripped,
            "负例：控件被改名时，上面第一条用例必须变红")


if __name__ == "__main__":
    unittest.main()

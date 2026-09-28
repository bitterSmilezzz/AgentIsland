#!/usr/bin/env python3
"""列出 Rust 侧**全项目零引用**的 `pub fn`，并分清死代码 / 平台代码 / JS 调用。

## 这份脚本只回答一个问题，而且只这一个

> 「有哪些 `pub fn` 在 Rust 源码里一次都没被提到？」

**它不算测试覆盖率。** 自动化覆盖率在这套代码上做不对——见文末「为什么不做覆盖率」。
一个三次给错答案的统计，第四次换个说法再错一次，不如把能力收窄到能核实的那一条。

## 排除的三类「零引用」（每一类都是正常的，不该报警）

| 类别 | 怎么判 | 为什么不算死代码 |
| :--- | :--- | :--- |
| **测试代码** | 文件名在 `state_tests.rs` 等测试模块里，或函数挂在 `#[cfg(test)]` 下 | 它是被测的那一方 |
| **平台代码** | 函数挂在不满足的 `#[cfg(...)]` 上（当前构建是 macOS） | `#[cfg(windows)]` 的函数在 macOS 构建上**必然**零引用，删了会砸掉 Windows 端 |
| **JS 调用的命令** | 挂了 `#[tauri::command]` | 它由前端 `invoke('x')` 调用，Rust 侧当然没有调用方 |

第三类由 `tests` 里的 `every_invoke_has_a_backing_command` 守着——那份用例
保证「JS 调用的每个名字都有对应的 Rust 命令」，与本脚本互为反面。

## 用法

```sh
python3 scripts/untested-surface.py
python3 scripts/untested-surface.py --swift     # 附带 Swift 侧的同类扫描
```

**退出码恒为 0**：它是报告不是门禁。价值在于「改过之后能重跑对比」，
做成会红的门禁只会逼人去改数字。

## 为什么不做覆盖率

v0.0.224–227 之间，手工统计先后给出 **29 → 13 → 8** 三个数字，
而 v0.0.227 逐个核实后发现 `has_unreadable_source` **是有用例的**
（`existing_file_instead_of_session_directory_is_unreadable` 直接调它）。
本脚本第一版试图用「调用图闭包」自动算覆盖率，又栽在三处：

1. **字符字面量 `'"'`** 被当成字符串起始，吞掉后面真实代码里的括号
   ⇒ 花括号配不平 ⇒ 整个文件的测试模块识别不出来。
2. **测试模块是独立文件**（`#[cfg(test)] #[path = "engine/state_tests.rs"] mod state_tests;`），
   按文件内范围判断归属必然漏。
3. **Tauri 命令没有 Rust 调用方**——它们的前缀是 JS 里的 `invoke('x')`。

第 1 条已经修好（见 `strip_noise`），第 2、3 条要修就得引入跨文件模块解析与
JS 侧索引，那是**另一件事**，不该塞进这个脚本。**结论记在这里，
免得下一个人再造一遍。**
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUST_SRC = ROOT / "app" / "src-tauri" / "src"
SWIFT_SRC = ROOT / "Sources"

# 当前构建满足的 cfg 组合（本机 macOS）。换平台时补对应分支，
# 不要删掉不满足的——它们正是「平台代码」的判据。
SATISFIED = {"target_os = \"macos\"", "unix", "not(windows)"}

# 测试专用文件：它们整体是用例，不是被测对象
TEST_FILES = {"testutil.rs", "state_tests.rs"}


def strip_comments_only(text: str) -> str:
    """只把**注释**抹成空格，字符串内容原样保留。

    为什么要单独一个版本：Swift 的字符串插值 `\(expr)` 里是**可执行代码**
    （`"\(...) \(Foo.statusLabel(x))"` 是真实调用），把字符串内容抹掉等于
    把调用方一起抹掉 ⇒ 零引用统计全线报错。
    「零引用」这件事本身只需要去掉**文档注释**造成的假引用，不需要动字符串。

    Rust 侧则相反：它有 `'\''` / `'\"'` 这类字符字面量与 `"{x}"` 这类格式串，
    抹内容更安全（见 `strip_noise`）。**两套规则不能合并**。
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        if text[i] == '/' and i + 1 < n and text[i + 1] == '/':
            while i < n and text[i] != '\n':
                out[i] = ' '
                i += 1
        elif text[i] == '/' and i + 1 < n and text[i + 1] == '*':
            end = text.find('*/', i + 2)
            end = n if end < 0 else end + 2
            for k in range(i, end):
                if out[k] != '\n':
                    out[k] = ' '
            i = end
        else:
            i += 1
    return ''.join(out)


def strip_noise(text: str) -> str:
    """把注释、字符串与字符字面量的**内容**换成空格，保留行结构与偏移量。

    为什么必须做：注释里常常写着「调过 `CFBooleanGetValue`」这类**证据**，
    不剥掉就会把文档当成代码扫出来。
    **字符字面量必须单独处理**——`'"'` 含一个双引号，当成字符串起始会吞掉
    后面真实代码里的括号（本脚本第一版就栽在这里，症状是花括号配平失败）。
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        ch = text[i]
        if ch == '/' and i + 1 < n and text[i + 1] == '/':
            while i < n and text[i] != '\n':
                out[i] = ' '
                i += 1
        elif ch == '/' and i + 1 < n and text[i + 1] == '*':
            end = text.find('*/', i + 2)
            end = n if end < 0 else end + 2
            for k in range(i, end):
                if out[k] != '\n':
                    out[k] = ' '
            i = end
        elif ch == "'":
            nxt = i + 1
            if nxt < n and text[nxt] == '\\':
                close = text.find("'", nxt + 2)
                if close > 0 and close - (nxt + 1) <= 2:
                    for k in range(i, close + 1):
                        out[k] = ' '
                    i = close + 1
                    continue
            elif nxt + 1 < n and text[nxt + 1] == "'":
                for k in range(i, nxt + 2):
                    out[k] = ' '
                i = nxt + 2
                continue
            i += 1  # 生命周期（'static），原样放行
        elif ch == '"':
            out[i] = ' '
            i += 1
            while i < n and text[i] != '"':
                if text[i] == '\\':
                    out[i] = ' '
                    i += 1
                    if i < n:
                        out[i] = ' '
                        i += 1
                    continue
                if out[i] != '\n':
                    out[i] = ' '
                i += 1
            if i < n:
                out[i] = ' '
                i += 1
        else:
            i += 1
    return ''.join(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument('--swift', action='store_true', help='附带 Swift 侧同类扫描')
    args = ap.parse_args()

    files = sorted(RUST_SRC.rglob('*.rs'))
    raw = {p: strip_noise(p.read_text()) for p in files}
    blob = "\n".join(raw.values())

    dead: list[str] = []
    platform: list[str] = []
    js_only: list[str] = []

    for p, text in raw.items():
        if p.name in TEST_FILES:
            continue
        for m in re.finditer(r'(?m)^[ \t]*pub[ \t]+(?:async[ \t]+)?fn[ \t]+(\w+)', text):
            name = m.group(1)
            # 除声明本身外，全 Rust 源码再无出现 ⇒ 零引用
            if len(re.findall(rf'\b{re.escape(name)}\b', blob)) > 1:
                continue
            line = text[:m.start()].count('\n') + 1
            entry = f"{p.name}:{line}: {name}"
            # ① 平台代码：本机构建够不到的 cfg 分支
            head = text[max(0, m.start() - 300):m.start()]
            cfgs = re.findall(r'#\[cfg\(([^\]]*)\)\]', head)
            if any(c and c not in SATISFIED for c in cfgs[-1:]):
                platform.append(f"{entry}  (cfg: {cfgs[-1]})")
            # ② JS 调用的 Tauri 命令
            elif 'tauri::command' in head:
                js_only.append(entry)
            # ③ 测试函数挂在 #[cfg(test)] 下
            elif any('test' in c for c in cfgs):
                continue
            else:
                dead.append(entry)

    def block(title: str, items: list[str], empty: str) -> None:
        print(f"== {title}：{len(items)} 个 ==")
        for line in items:
            print(f"  {line}")
        if not items:
            print(f"  {empty}")
        print()

    block("Rust：死代码（本平台可达、零引用、不是 Tauri 命令）", dead, "（无）")
    block("Rust：平台代码（cfg 分支够不到，**不是死代码**）", platform, "（无）")
    block("Rust：由 JS invoke 调用的 Tauri 命令（Rust 侧零引用是正常的）", js_only, "（无）")

    if args.swift:
        # **Tests 也要扫**：一个只被用例引用的函数不是死代码（用例就是它的消费者）。
        # 只扫 Sources 会把 `expiredEvidence` 这类报成零引用——事实是它在 Tests 里有 3 处引用。
        st = sorted(SWIFT_SRC.rglob('*.swift'))
        tdir = ROOT / "Tests"
        tf = sorted(tdir.rglob('*.swift'))
        sraw = [strip_comments_only(f.read_text()) for f in st]
        traw = [strip_comments_only(f.read_text()) for f in tf]
        sblob, tblob = "\n".join(sraw), "\n".join(traw)
        prod, testonly = [], []
        for f, text in zip(st, sraw):
            for m in re.finditer(r'\bpublic\s+(?:static\s+)?func\s+(\w+)', text):
                name = m.group(1)
                in_prod = len(re.findall(rf'\b{re.escape(name)}\b', sblob))
                in_test = len(re.findall(rf'\b{re.escape(name)}\b', tblob))
                line = text[:m.start()].count(chr(10)) + 1
                if in_prod > 1:
                    continue                      # 有生产调用方，不是问题
                if in_test > 0:
                    testonly.append(f"{f.name}:{line}: {name}  (仅 {in_test} 处测试引用)")
                else:
                    prod.append(f"{f.name}:{line}: {name}")
        print("注意：Swift 侧同样**只报「有没有产生点」**，不做覆盖率——")
        print("      方法调用 / 协议见证 / 闭包捕获让调用图在这个精度下不可靠。")
        block("Swift：全仓零引用的 public func（**没有产生点**）", prod, "（无）")
        block("Swift：只有测试引用的 public func（不是死代码，但生产侧没人用）", testonly, "（无）")

    print("这份统计替代不了逐个核实（v0.0.224–227 手工版错了三次）。")
    print("改动之后请重跑对比，而不是把这里的数字当结论。")
    return 0


if __name__ == '__main__':
    sys.exit(main())

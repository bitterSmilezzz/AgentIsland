#!/usr/bin/env python3
"""Build a native fixture using the real island views; no real agent data."""
import glob
import pathlib
import subprocess
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix="island-layout-") as temp:
    temp = pathlib.Path(temp)
    source = (root / "Sources/AgentIsland/IslandView.swift").read_text()
    # 汇总条现为可点击分析入口；探针须包在完整调用之后，不能插进 trailing closure 中间。
    needle = """TokenSummaryBar(total: engine.grandTotal) {
                    controller.route = .tokenAnalytics
                }"""
    assert source.count(needle) == 1
    source = source.replace(needle, needle + """
                    .background(GeometryReader { proxy in
                        Color.clear.onAppear { LayoutProbe.frame = proxy.frame(in: .global) }
                            .onChange(of: proxy.frame(in: .global)) { LayoutProbe.frame = $0 }
                    })""")
    view = temp / "IslandView.swift"
    view.write_text(source)
    sources = [str(p) for p in (root / "Sources/AgentIsland").glob("*.swift")
               if p.name not in ("AgentIslandApp.swift", "IslandView.swift")]
    binary = temp / "layout-test"
    # 构建新鲜度：先确保 debug 产物与源码一致（否则链接到过期对象，验收失真）
    subprocess.run(["swift", "build"], check=True)
    # SwiftPM 6.4 的 Xcode 风格产物把 swiftmodule 与合并对象直接放在
    # .build/debug；旧版则使用 Modules/ + AgentIslandCore.build/*.swift.o。
    # 两种布局都接受，避免清理缓存或切换工具链后门禁自身失效。
    debug_dir = root / ".build/debug"
    legacy_modules = debug_dir / "Modules"
    module_dir = legacy_modules if legacy_modules.exists() else debug_dir
    core_objects = glob.glob(str(debug_dir / "AgentIslandCore.build/*.swift.o"))
    merged_object = debug_dir / "AgentIslandCore.o"
    if not core_objects and merged_object.exists():
        core_objects = [str(merged_object)]
    compile_cmd = ["swiftc", "-module-cache-path", "/private/tmp/agentisland-clang-cache",
                   "-I", str(module_dir), *sources, str(view),
                   str(root / "Tests/LayoutRegression/main.swift"),
                   *core_objects,
                   "-o", str(binary)]
    result = subprocess.run(compile_cmd, capture_output=True)
    if result.returncode != 0:
        # 吞掉 stderr 会让最需要本脚本的改动时刻只剩一个退出码（R19）
        print(result.stderr.decode(), file=__import__("sys").stderr)
        raise SystemExit(result.returncode)
    subprocess.run([str(binary)], check=True)

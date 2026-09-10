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
    needle = "TokenSummaryBar(total: engine.grandTotal)"
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
    subprocess.run(["swiftc", "-module-cache-path", "/private/tmp/agentisland-clang-cache",
                    "-I", str(root / ".build/debug/Modules"), *sources, str(view),
                    str(root / "Tests/LayoutRegression/main.swift"),
                    *glob.glob(str(root / ".build/debug/AgentIslandCore.build/*.swift.o")),
                    "-o", str(binary)], check=True, capture_output=True)
    subprocess.run([str(binary)], check=True)

#!/bin/bash
# 构建与发布共用的应用名称。
APP_NAME="AgentIsland"

# 需要 tomllib 的工具（构建收据）要求 Python >= 3.11。macOS 自带 /usr/bin/python3 是 3.9，
# 裸 `python3` 会让同一个门禁在一台机器上通过、在另一台直接崩溃——所以显式挑一个满足
# 下限的解释器，而不是把结果交给 PATH 顺序。找不到时 PYTHON_MIN 为空，由调用方明确失败。
PYTHON_MIN=""
_pick_python() {
    local floor="${1:-3.11}" candidate version
    for candidate in python3.14 python3.13 python3.12 python3.11 python3; do
        command -v "$candidate" >/dev/null 2>&1 || continue
        version="$("$candidate" -c 'import sys;print("%d.%d"%sys.version_info[:2])' 2>/dev/null)" || continue
        case "$version" in
            "$floor"|3.1[1-9]|4.*) PYTHON_MIN="$candidate"; return 0 ;;
        esac
    done
    return 1
}
_pick_python 3.11 || PYTHON_MIN=""

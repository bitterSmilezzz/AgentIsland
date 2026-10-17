#!/bin/bash
# POSIX 重型阶段互斥入口。不改调用方 trap，不依赖外部 flock 命令。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"
[[ -n "$PYTHON_MIN" ]] || { echo 'Python >= 3.11 is required' >&2; exit 2; }
exec "$PYTHON_MIN" "$SCRIPT_DIR/automation/heavy_build_lock.py" "$@"

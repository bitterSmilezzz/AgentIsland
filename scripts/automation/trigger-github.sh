#!/bin/sh
# Thin compatibility entry point; Python owns parsing and process execution.
exec python3 "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)/trigger.py" "$@"

#!/bin/sh
set -eu
# No cargo clean, shell-time parsing, bc, fixed ports, or changes to normal build artifacts.
exec python3 tests/bench.py "$@"

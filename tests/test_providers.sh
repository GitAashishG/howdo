#!/bin/sh
set -eu
# Compatibility entry point. Python runs the same full suite on every platform.
exec python3 tests/test_cli.py "${1:-./target/release/howdo}"

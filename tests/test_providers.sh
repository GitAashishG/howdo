#!/bin/sh
set -e

# Integration tests for all provider configurations against a mock server.
# Usage: ./tests/test_providers.sh <path-to-q-binary>

BINARY="${1:-./target/release/howdo}"
PORT=9999
MOCK_PID=""
CONFIG_DIR=""
PASS=0
FAIL=0

cleanup() {
    [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null || true
    [ -n "$CONFIG_DIR" ] && rm -rf "$CONFIG_DIR" 2>/dev/null || true
}
trap cleanup EXIT

# ── Start mock server ────────────────────────────────────────────────────

python3 tests/mock_server.py "$PORT" &
MOCK_PID=$!
sleep 1

if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    echo "FAIL: Mock server did not start"
    exit 1
fi

# ── Helpers ──────────────────────────────────────────────────────────────

CONFIG_DIR=$(mktemp -d)
CONFIG_FILE="$CONFIG_DIR/howdo/config.json"
mkdir -p "$CONFIG_DIR/howdo"

write_config() {
    echo "$1" > "$CONFIG_FILE"
}

run_test() {
    local name="$1"
    local expected="$2"

    # Use XDG_CONFIG_HOME to point q at our temp config
    output=$(echo "n" | XDG_CONFIG_HOME="$CONFIG_DIR" "$BINARY" test query 2>&1 || true)

    if echo "$output" | grep -q "$expected"; then
        echo "  PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $name"
        echo "    Expected output containing: $expected"
        echo "    Got: $output"
        FAIL=$((FAIL + 1))
    fi
}

# ── Tests ────────────────────────────────────────────────────────────────

echo ""
echo "Running provider tests against mock server on :$PORT"
echo ""

# 1) Local LLM — no auth, OpenAI format
write_config '{"provider":"local","base_url":"http://127.0.0.1:'"$PORT"'/v1","model":"default"}'
run_test "Local LLM" "mock-openai-ok"

# 2) OpenAI — Bearer auth, OpenAI format
write_config '{"provider":"openai","base_url":"http://127.0.0.1:'"$PORT"'/v1","model":"gpt-4o-mini","api_key":"sk-test"}'
run_test "OpenAI" "mock-openai-ok"

# 3) Azure OpenAI — api-key header, full URL, OpenAI format
write_config '{"provider":"azure_openai","base_url":"http://127.0.0.1:'"$PORT"'/openai/deployments/gpt-4/chat/completions?api-version=2024-12-01-preview","model":"","api_key":"azure-test"}'
run_test "Azure OpenAI" "mock-openai-ok"

# 4) Anthropic — x-api-key header, /v1/messages, Anthropic format
write_config '{"provider":"anthropic","base_url":"http://127.0.0.1:'"$PORT"'","model":"claude-sonnet-4-20250514","api_key":"sk-ant-test"}'
run_test "Anthropic" "mock-anthropic-ok"

# 5) Other (OpenAI-compatible) — Bearer auth, OpenAI format
write_config '{"provider":"other","base_url":"http://127.0.0.1:'"$PORT"'/v1","model":"mixtral","api_key":"other-test"}'
run_test "Other (OpenAI-compatible)" "mock-openai-ok"

# 6) --version flag
version_output=$("$BINARY" --version 2>&1)
if echo "$version_output" | grep -qE "^howdo [0-9]+\.[0-9]+\.[0-9]+"; then
    echo "  PASS: --version"
    PASS=$((PASS + 1))
else
    echo "  FAIL: --version (got: $version_output)"
    FAIL=$((FAIL + 1))
fi

# 7) --help flag
help_output=$("$BINARY" --help 2>&1)
if echo "$help_output" | grep -q "natural language"; then
    echo "  PASS: --help"
    PASS=$((PASS + 1))
else
    echo "  FAIL: --help (got: $help_output)"
    FAIL=$((FAIL + 1))
fi

# 8) No args shows usage
noargs_output=$("$BINARY" 2>&1 || true)
if echo "$noargs_output" | grep -q "Usage"; then
    echo "  PASS: no-args usage"
    PASS=$((PASS + 1))
else
    echo "  FAIL: no-args usage (got: $noargs_output)"
    FAIL=$((FAIL + 1))
fi

# ── Summary ──────────────────────────────────────────────────────────────

echo ""
echo "Results: $PASS passed, $FAIL failed"
echo ""

[ "$FAIL" -eq 0 ] || exit 1

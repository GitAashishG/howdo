#!/bin/sh
# bench.sh — Startup time, binary size, and resource benchmarks for q
# Usage: ./tests/bench.sh [path-to-binary]

set -e

BINARY="${1:-./target/release/howdo}"
RUNS=50

if [ ! -f "$BINARY" ]; then
    echo "Binary not found: $BINARY"
    echo "Run: cargo build --release"
    exit 1
fi

echo "═══════════════════════════════════════════════════"
echo "  q benchmark"
echo "═══════════════════════════════════════════════════"
echo ""

# ── Binary info ──────────────────────────────────────────────────────────

SIZE_BYTES=$(wc -c < "$BINARY" | tr -d ' ')
if [ "$SIZE_BYTES" -ge 1048576 ]; then
    SIZE_HUMAN=$(echo "scale=2; $SIZE_BYTES / 1048576" | bc)M
elif [ "$SIZE_BYTES" -ge 1024 ]; then
    SIZE_HUMAN=$(echo "scale=1; $SIZE_BYTES / 1024" | bc)K
else
    SIZE_HUMAN="${SIZE_BYTES}B"
fi

echo "  Binary:         $BINARY"
echo "  Size:           $SIZE_HUMAN ($SIZE_BYTES bytes)"
echo "  Version:        $("$BINARY" --version 2>&1 || echo 'unknown')"

# file type (arch, linking)
if command -v file >/dev/null 2>&1; then
    FILE_INFO=$(file "$BINARY" | sed "s|^.*: ||")
    echo "  Type:           $FILE_INFO"
fi

# linked libraries count
if command -v otool >/dev/null 2>&1; then
    DYLIBS=$(otool -L "$BINARY" 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')
    echo "  Dylibs:         $DYLIBS"
elif command -v ldd >/dev/null 2>&1; then
    SOLIBS=$(ldd "$BINARY" 2>/dev/null | wc -l | tr -d ' ')
    echo "  Shared libs:    $SOLIBS"
fi

echo ""

# ── Startup time (--version, no network) ─────────────────────────────────

echo "  Startup time (--version, $RUNS runs):"

TIMES=""
for i in $(seq 1 "$RUNS"); do
    # Use shell built-in time via subshell; capture only real elapsed
    T=$( { time "$BINARY" --version >/dev/null 2>&1; } 2>&1 )
    # Extract seconds from "real 0m0.003s" format
    MS=$(echo "$T" | grep real | sed 's/.*0m//' | sed 's/s//')
    TIMES="$TIMES $MS"
done

# Calculate min, max, avg, median
SORTED=$(echo "$TIMES" | tr ' ' '\n' | grep -v '^$' | sort -n)
COUNT=$(echo "$SORTED" | wc -l | tr -d ' ')
MIN=$(echo "$SORTED" | head -1)
MAX=$(echo "$SORTED" | tail -1)
AVG=$(echo "$SORTED" | awk '{s+=$1} END {printf "%.4f", s/NR}')
MEDIAN_LINE=$(( (COUNT + 1) / 2 ))
MEDIAN=$(echo "$SORTED" | sed -n "${MEDIAN_LINE}p")

# Convert to ms for readability
MIN_MS=$(echo "$MIN * 1000" | bc | sed 's/^\./0./')
MAX_MS=$(echo "$MAX * 1000" | bc | sed 's/^\./0./')
AVG_MS=$(echo "$AVG * 1000" | bc | sed 's/^\./0./')
MED_MS=$(echo "$MEDIAN * 1000" | bc | sed 's/^\./0./')

echo "    min:    ${MIN_MS} ms"
echo "    max:    ${MAX_MS} ms"
echo "    avg:    ${AVG_MS} ms"
echo "    median: ${MED_MS} ms"
echo ""

# ── Startup time (--help, no network) ────────────────────────────────────

echo "  Startup time (--help, $RUNS runs):"

TIMES=""
for i in $(seq 1 "$RUNS"); do
    T=$( { time "$BINARY" --help >/dev/null 2>&1; } 2>&1 )
    MS=$(echo "$T" | grep real | sed 's/.*0m//' | sed 's/s//')
    TIMES="$TIMES $MS"
done

SORTED=$(echo "$TIMES" | tr ' ' '\n' | grep -v '^$' | sort -n)
MIN=$(echo "$SORTED" | head -1)
MAX=$(echo "$SORTED" | tail -1)
AVG=$(echo "$SORTED" | awk '{s+=$1} END {printf "%.4f", s/NR}')
MEDIAN_LINE=$(( ($(echo "$SORTED" | wc -l | tr -d ' ') + 1) / 2 ))
MEDIAN=$(echo "$SORTED" | sed -n "${MEDIAN_LINE}p")

MIN_MS=$(echo "$MIN * 1000" | bc | sed 's/^\./0./')
MAX_MS=$(echo "$MAX * 1000" | bc | sed 's/^\./0./')
AVG_MS=$(echo "$AVG * 1000" | bc | sed 's/^\./0./')
MED_MS=$(echo "$MEDIAN * 1000" | bc | sed 's/^\./0./')

echo "    min:    ${MIN_MS} ms"
echo "    max:    ${MAX_MS} ms"
echo "    avg:    ${AVG_MS} ms"
echo "    median: ${MED_MS} ms"
echo ""

# ── Cold LLM round-trip (mock server) ───────────────────────────────────

if command -v python3 >/dev/null 2>&1; then
    PORT=19876
    python3 tests/mock_server.py "$PORT" &
    MOCK_PID=$!
    sleep 0.5

    if kill -0 "$MOCK_PID" 2>/dev/null; then
        TMPDIR_BENCH=$(mktemp -d)
        mkdir -p "$TMPDIR_BENCH/howdo"
        echo "{\"provider\":\"local\",\"base_url\":\"http://127.0.0.1:$PORT/v1\",\"model\":\"default\"}" > "$TMPDIR_BENCH/howdo/config.json"

        echo "  LLM round-trip (mock server, $RUNS runs):"

        TIMES=""
        for i in $(seq 1 "$RUNS"); do
            T=$( { time echo "n" | XDG_CONFIG_HOME="$TMPDIR_BENCH" "$BINARY" hello 2>&1 >/dev/null; } 2>&1 )
            MS=$(echo "$T" | grep real | sed 's/.*0m//' | sed 's/s//')
            TIMES="$TIMES $MS"
        done

        SORTED=$(echo "$TIMES" | tr ' ' '\n' | grep -v '^$' | sort -n)
        MIN=$(echo "$SORTED" | head -1)
        MAX=$(echo "$SORTED" | tail -1)
        AVG=$(echo "$SORTED" | awk '{s+=$1} END {printf "%.4f", s/NR}')
        MEDIAN_LINE=$(( ($(echo "$SORTED" | wc -l | tr -d ' ') + 1) / 2 ))
        MEDIAN=$(echo "$SORTED" | sed -n "${MEDIAN_LINE}p")

        MIN_MS=$(echo "$MIN * 1000" | bc | sed 's/^\./0./')
        MAX_MS=$(echo "$MAX * 1000" | bc | sed 's/^\./0./')
        AVG_MS=$(echo "$AVG * 1000" | bc | sed 's/^\./0./')
        MED_MS=$(echo "$MEDIAN * 1000" | bc | sed 's/^\./0./')

        echo "    min:    ${MIN_MS} ms"
        echo "    max:    ${MAX_MS} ms"
        echo "    avg:    ${AVG_MS} ms"
        echo "    median: ${MED_MS} ms"
        echo ""

        kill "$MOCK_PID" 2>/dev/null || true
        rm -rf "$TMPDIR_BENCH"
    else
        echo "  (mock server failed to start, skipping LLM round-trip)"
        echo ""
    fi
else
    echo "  (python3 not found, skipping LLM round-trip)"
    echo ""
fi

# ── Build time (optional, if cargo available) ────────────────────────────

if command -v cargo >/dev/null 2>&1; then
    echo "  Clean release build time:"
    cargo clean -q 2>/dev/null || true
    BUILD_T=$( { time cargo build --release -q 2>&1; } 2>&1 )
    BUILD_S=$(echo "$BUILD_T" | grep real | sed 's/.*0m//' | sed 's/s//')
    echo "    ${BUILD_S}s"
    echo ""
fi

# ── Summary ──────────────────────────────────────────────────────────────

echo "═══════════════════════════════════════════════════"
echo "  Done."
echo "═══════════════════════════════════════════════════"

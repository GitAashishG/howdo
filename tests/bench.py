"""Reproducible startup and mock round-trip benchmarks; optional isolated clean build."""

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import threading
import time
from pathlib import Path

from mock_server import create_server

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument(
    "binary", nargs="?", default="target/release/howdo" + (".exe" if os.name == "nt" else "")
)
parser.add_argument("--runs", type=int, default=50)
parser.add_argument(
    "--build", action="store_true", help="Measure a clean build in a temporary target directory"
)
args = parser.parse_args()
if args.runs < 1:
    parser.error("--runs must be positive")
binary = str(Path(args.binary).resolve())
if not Path(binary).is_file():
    parser.error("Binary not found; run cargo build --release --locked")
print(f"Binary: {binary}\nSize: {Path(binary).stat().st_size / 1048576:.2f} MiB")
print(subprocess.check_output([binary, "--version"], text=True).strip())


def measure(label, command, env=None):
    samples = []
    for _ in range(args.runs):
        start = time.perf_counter()
        subprocess.run(command, input="", capture_output=True, text=True, env=env, check=True)
        samples.append((time.perf_counter() - start) * 1000)
    print(
        f"{label} ({args.runs} runs): min {min(samples):.2f}, median {statistics.median(samples):.2f}, "
        f"mean {statistics.mean(samples):.2f}, max {max(samples):.2f} ms"
    )


measure("--version", [binary, "--version"])
measure("--help", [binary, "--help"])
server = create_server()
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix="howdo-bench-") as directory:
        config = Path(directory) / "howdo" / "config.json"
        config.parent.mkdir()
        config.write_text(
            json.dumps(
                {
                    "provider": "local",
                    "base_url": f"http://127.0.0.1:{server.server_port}/v1",
                    "model": "default",
                }
            )
        )
        env = dict(os.environ, XDG_CONFIG_HOME=directory)
        for name in ["HOWDO_PROFILE", "HOWDO_SHELL"]:
            env.pop(name, None)
        measure("Mock generation (no execution)", [binary, "--print", "hello"], env)
finally:
    server.shutdown()
    server.server_close()
    thread.join()
if args.build:
    with tempfile.TemporaryDirectory(prefix="howdo-build-bench-") as directory:
        start = time.perf_counter()
        subprocess.run(
            ["cargo", "build", "--release", "--locked"],
            env=dict(os.environ, CARGO_TARGET_DIR=directory),
            check=True,
        )
        print(f"Isolated clean release build: {time.perf_counter() - start:.2f} s")

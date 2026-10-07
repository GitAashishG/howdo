"""Single-threaded PTY driver, isolated from the threaded mock server test process."""

import json
import os
import pty
import select
import signal
import sys
import termios
import time

spec = json.load(sys.stdin)
argv = spec["argv"]
steps = [(prompt.encode(), answer.encode()) for prompt, answer in spec["steps"]]
pid, master = pty.fork()
if pid == 0:
    os.execve(argv[0], argv, os.environ)
data = b""
step = 0
position = 0
deadline = time.monotonic() + 10
status = None

try:
    while time.monotonic() < deadline:
        if select.select([master], [], [], 0.05)[0]:
            try:
                block = os.read(master, 65536)
            except OSError:
                block = b""
            data += block
        if step < len(steps) and steps[step][0] in data[position:]:
            os.write(master, steps[step][1])
            position = len(data)
            step += 1
        waited, status = os.waitpid(pid, os.WNOHANG)
        if waited:
            # Drain final output even if the process exited before the last read.
            while select.select([master], [], [], 0)[0]:
                try:
                    block = os.read(master, 65536)
                except OSError:
                    break
                if not block:
                    break
                data += block
            break
    else:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        raise RuntimeError(f"PTY timed out at step {step}: {data!r}")
    lflag = termios.tcgetattr(master)[3]
    print(
        json.dumps(
            {
                "code": os.waitstatus_to_exitcode(status),
                "output": data.decode(errors="replace"),
                "steps": step,
                "canonical": bool(lflag & termios.ICANON),
                "echo": bool(lflag & termios.ECHO),
            }
        )
    )
finally:
    os.close(master)

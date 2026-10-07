"""A configurable, credential-safe local LLM mock. No third-party dependencies."""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


class MockLLM(BaseHTTPRequestHandler):
    def do_POST(self):
        state = self.server.state
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length)) if length else {}
        state["requests"].append(
            {
                "path": self.path,
                "headers": {key.lower(): value for key, value in self.headers.items()},
                "body": body,
            }
        )
        time.sleep(state.get("delay", 0))
        gate = state.get("response_gate_file")
        if gate:
            deadline = time.monotonic() + 10
            while not Path(gate).exists():
                if time.monotonic() >= deadline:
                    self.send_error(500, "Mock response gate was not released")
                    return
                time.sleep(0.005)
        response = state.get("response")
        if response is None:
            messages = body.get("messages", [])
            explaining = messages and messages[-1].get("content", "").startswith(
                "Explain this command:"
            )
            command = state.get(
                "command",
                "echo mock-anthropic-ok" if "/messages" in self.path else "echo mock-openai-ok",
            )
            text = (
                "Prints text to standard output. Review any assumptions before executing."
                if explaining
                else command
            )
            if "/messages" in self.path:
                response = {"content": [{"type": "text", "text": text}], "stop_reason": "end_turn"}
            else:
                response = {
                    "choices": [
                        {
                            "message": {"role": "assistant", "content": text, "tool_calls": []},
                            "finish_reason": "stop",
                        }
                    ]
                }
        data = response if isinstance(response, bytes) else json.dumps(response).encode()
        self.send_response(state.get("status", 200))
        self.send_header("Content-Type", "application/json")
        for key, value in state.get("headers", {}).items():
            self.send_header(key, value)
        if state.get("chunked"):
            self.send_header("Transfer-Encoding", "chunked")
        elif not state.get("omit_length"):
            self.send_header("Content-Length", str(state.get("advertised_length", len(data))))
        self.end_headers()
        try:
            if state.get("chunked"):
                self.wfile.write(f"{len(data):x}\r\n".encode() + data + b"\r\n0\r\n\r\n")
            else:
                self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when a client cancels or rejects an oversized response.

    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, *_args):
        pass  # Never log request bodies or credentials.


def create_server(port=0):
    server = ThreadingHTTPServer(("127.0.0.1", port), MockLLM)
    server.daemon_threads = True
    server.state = {"requests": []}
    return server


if __name__ == "__main__":
    server = create_server(int(sys.argv[1]) if len(sys.argv) > 1 else 9999)
    print(f"Mock server on :{server.server_port}", flush=True)
    server.serve_forever()

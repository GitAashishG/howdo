"""Mock LLM server that mimics OpenAI, Azure, and Anthropic APIs for testing."""
from http.server import HTTPServer, BaseHTTPRequestHandler
import json
import sys


class MockLLM(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length)) if length else {}

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()

        # Anthropic format
        if "/v1/messages" in self.path:
            resp = {"content": [{"type": "text", "text": "echo mock-anthropic-ok"}]}
        # OpenAI / Azure / Local format
        else:
            resp = {
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "echo mock-openai-ok",
                        }
                    }
                ]
            }

        self.wfile.write(json.dumps(resp).encode())

    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"ok"}')

    def log_message(self, format, *args):
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 9999
    server = HTTPServer(("127.0.0.1", port), MockLLM)
    print(f"Mock server on :{port}", flush=True)
    server.serve_forever()

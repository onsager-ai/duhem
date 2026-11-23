"""Serve fixed rubric replies over the two provider HTTP shapes."""

from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
import socketserver

DUMMY_KEY = "duhem-score-example-dummy-key"


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/health":
            self.send_error(404)
            return
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ready")

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/v1/messages":
            key = self.headers.get("x-api-key")
            if (key != DUMMY_KEY
                    or self.headers.get("anthropic-version") != "2023-06-01"):
                self.send_error(401)
                return
            # Echo the dummy key in a reply to exercise evidence masking.
            text = json.dumps({"score": 8, "reason": f"Fixture reply for {key}"})
            response = {
                "model": "fixture-anthropic-reported",
                "content": [{"type": "text", "text": text}],
            }
        elif self.path == "/v1/chat/completions":
            if self.headers.get("Authorization") != f"Bearer {DUMMY_KEY}":
                self.send_error(401)
                return
            score = 11 if request["model"] == "fixture-out-of-range" else 3
            text = json.dumps({"score": score, "reason": "Fixed fixture reply"})
            response = {
                "model": "fixture-openai-reported",
                "choices": [{"message": {"content": text}}],
            }
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(response).encode("utf-8"))

    def log_message(self, *_):
        pass


class LoopbackServer(HTTPServer):
    def server_bind(self):
        # Avoid reverse DNS for the numeric loopback address.
        socketserver.TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


LoopbackServer(("127.0.0.1", int(os.environ["PORT"])), Handler).serve_forever()

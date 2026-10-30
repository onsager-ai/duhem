"""A real HTTP fixture exposing a persistent startup count, without judgments."""
from http.server import BaseHTTPRequestHandler, HTTPServer
import os
from pathlib import Path
import socketserver
import sys

state = Path(sys.argv[1])
state.write_text(str(int(state.read_text()) + 1) if state.exists() else "1")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(state.read_bytes())

    def log_message(self, *_):
        pass


class LoopbackServer(HTTPServer):
    def server_bind(self):
        # A numeric loopback fixture needs no reverse-DNS lookup.
        socketserver.TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


LoopbackServer(("127.0.0.1", int(os.environ["PORT"])), Handler).serve_forever()

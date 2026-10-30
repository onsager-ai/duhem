"""Real long-lived child fixtures for owned-process integration tests."""
import http.server
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

mode = sys.argv[1]
if mode == "server":
    state = Path(sys.argv[2])
    state.write_text(str(int(state.read_text()) + 1) if state.exists() else "1")

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/health" and self.headers.get("X-Ready") != "yes":
                self.send_response(403)
                self.end_headers()
                return
            self.send_response(200)
            self.end_headers()
            self.wfile.write(state.read_bytes())

        def log_message(self, *_):
            pass

    server = http.server.HTTPServer(("127.0.0.1", int(sys.argv[3])), Handler)
    print("READY", flush=True)
    server.serve_forever()
elif mode == "exit":
    print("exit-tail", flush=True)
    sys.exit(7)
elif mode in ("split", "secret"):
    secret = os.environ["PROCESS_SECRET"]
    sys.stdout.write(secret[:5])
    sys.stdout.flush()
    time.sleep(0.08)
    sys.stdout.write(secret[5:] + "\n")
    sys.stdout.flush()
    if mode == "split":
        print("e" * 80000, file=sys.stderr, flush=True)
        print("x" * 80000, flush=True)
    print("END", flush=True)
elif mode == "tree":
    child = subprocess.Popen([sys.executable, "-u", __file__, "refuse", sys.argv[2]])
    deadline = time.monotonic() + 5
    while not Path(sys.argv[2]).exists():
        if child.poll() is not None or time.monotonic() > deadline:
            sys.exit(3)
        time.sleep(0.01)
    print("READY", flush=True)
elif mode == "escaped-pipe":
    # Deliberately violates the supported group contract. The finalizer must
    # report incomplete drain, never successful cleanup. The test harness
    # explicitly kills this separately identified fixture afterward.
    child = subprocess.Popen(
        [sys.executable, "-u", __file__, "refuse", sys.argv[2]],
        start_new_session=True,
    )
    deadline = time.monotonic() + 5
    while not Path(sys.argv[2]).exists():
        if child.poll() is not None or time.monotonic() > deadline:
            sys.exit(3)
        time.sleep(0.01)
    print("READY", flush=True)
elif mode == "refuse":
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    Path(sys.argv[2]).write_text(str(os.getpid()))
    print("READY", flush=True)
elif mode == "idle":
    print("READY", flush=True)
elif mode != "unready":
    raise ValueError(mode)

while True:
    time.sleep(0.05)

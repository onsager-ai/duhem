"""Runnable local permission app for the named-session worked example.

Uses real HTTP, server-issued session cookies, and a SQLite permission store.
Test-only accounts: admin/admin-password and user1/user1-password.
Run: python3 app.py --port 3000 --database /tmp/named-sessions.sqlite
"""
import argparse
import http.cookies
import http.server
import secrets
import sqlite3
import urllib.parse

LOGIN = '''<h1>Login</h1><form method="post" action="/login">
<label>Username <input name="username"></label>
<label>Password <input name="password" type="password"></label>
<button>Sign in</button></form>'''
ADMIN = '''<script>localStorage.setItem('admin-only', 'yes');
sessionStorage.setItem('admin-only', 'yes');</script><h1>Admin</h1><form method="post" action="/grant">
<button>Grant access</button></form>'''


class App(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def redirect(self, path, cookie=None):
        self.send_response(303)
        self.send_header("Location", path)
        if cookie:
            self.send_header("Set-Cookie", cookie)
        self.end_headers()

    def page(self, html, status=200):
        body = ("<!doctype html><html><body>" + html + "</body></html>").encode()
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def identity(self):
        cookies = http.cookies.SimpleCookie(self.headers.get("Cookie", ""))
        token = cookies.get("session")
        if token:
            row = self.server.db.execute(
                "SELECT username FROM sessions WHERE token = ?", (token.value,)
            ).fetchone()
            return row[0] if row else None
        return None

    def do_GET(self):
        user = self.identity()
        if self.path == "/login":
            self.page(LOGIN)
        elif self.path in ("/admin", "/reports") and not user:
            self.redirect("/login")
        elif self.path == "/admin":
            self.page(ADMIN if user == "admin" else "<h1>Forbidden</h1>")
        elif self.path == "/reports":
            allowed = user == "admin" or self.server.db.execute(
                "SELECT reports FROM users WHERE username = ?", (user,)
            ).fetchone()[0]
            self.page("<h1>Reports</h1>" if allowed else "<h1>Access denied</h1>")
        elif self.path == "/storage":
            self.page('''<script>document.body.innerHTML =
              localStorage.getItem('admin-only') || sessionStorage.getItem('admin-only')
              ? '<h1>Leaked storage</h1>' : '<h1>Clean storage</h1>';</script>''')
        else:
            self.page("<h1>Not found</h1>", 404)

    def do_POST(self):
        fields = urllib.parse.parse_qs(self.rfile.read(int(self.headers.get("Content-Length", 0))).decode())
        if self.path == "/login":
            user = fields.get("username", [""])[0]
            password = fields.get("password", [""])[0]
            row = self.server.db.execute(
                "SELECT username FROM users WHERE username = ? AND password = ?", (user, password)
            ).fetchone()
            if not row:
                self.page(LOGIN, 401)
                return
            token = secrets.token_hex(24)
            self.server.db.execute("INSERT INTO sessions VALUES (?, ?)", (token, user))
            self.server.db.commit()
            self.redirect("/admin" if user == "admin" else "/reports",
                          f"session={token}; Path=/; HttpOnly; SameSite=Lax")
        elif self.path == "/grant" and self.identity() == "admin":
            self.server.db.execute("UPDATE users SET reports = 1 WHERE username = 'user1'")
            self.server.db.commit()
            self.page(ADMIN + "<h2>Access granted</h2>")
        else:
            self.page("<h1>Forbidden</h1>", 403)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=3000)
    parser.add_argument("--database", required=True)
    args = parser.parse_args()
    db = sqlite3.connect(args.database)
    db.executescript('''CREATE TABLE IF NOT EXISTS users(username TEXT PRIMARY KEY, password TEXT, reports INTEGER);
        CREATE TABLE IF NOT EXISTS sessions(token TEXT PRIMARY KEY, username TEXT);
        INSERT OR IGNORE INTO users VALUES ('admin', 'admin-password', 1), ('user1', 'user1-password', 0);''')
    server = http.server.HTTPServer(("127.0.0.1", args.port), App)
    server.db = db
    print(f"http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()

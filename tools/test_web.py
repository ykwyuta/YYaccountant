"""Start the real Web server on a demo database and verify HTTP security and API behaviour.

Usage: python tools/test_web.py target/release/yyaccountant [--browser]
With --browser, tools/test_web_browser.cjs drives Chromium through every view
(requires the playwright package on NODE_PATH) and stores screenshots in
target/web-validation/.
"""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Client:
    def __init__(self, port):
        self.port = port
        self.cookie = None
        self.csrf = ""

    def request(self, method, path, body=None, origin=True, csrf=True, host=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        headers = {"Host": host or f"127.0.0.1:{self.port}"}
        if body is not None:
            headers["Content-Type"] = "application/json"
        if origin:
            headers["Origin"] = f"http://127.0.0.1:{self.port}"
        if self.cookie:
            headers["Cookie"] = self.cookie
        if csrf and self.csrf:
            headers["X-YY-CSRF"] = self.csrf
        c.request(method, path, json.dumps(body) if body is not None else None, headers)
        r = c.getresponse()
        raw = r.read()
        cookie = r.getheader("Set-Cookie")
        if cookie:
            self.cookie = cookie.split(";")[0]
        try:
            data = json.loads(raw)
        except ValueError:
            data = raw
        return r.status, data, r

    def api(self, tool, arguments):
        return self.request("POST", "/web/api", {"tool": tool, "arguments": arguments})


def wait(port, process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(process.stderr.read())
        try:
            socket.create_connection(("127.0.0.1", port), timeout=1).close()
            return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError("server did not start")


def main():
    executable = Path(sys.argv[1]).resolve()
    browser = "--browser" in sys.argv
    out = Path("target/web-validation")
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="yyaccountant-web-") as directory:
        directory = Path(directory)
        database = directory / "demo.sqlite3"
        config = directory / "auth.toml"
        subprocess.run([str(executable), "--init-demo", str(database), "--today", time.strftime("%Y-%m-%d")],
                       check=True, capture_output=True, timeout=120)
        passwords = {}
        for user, extra in (("owner", []), ("viewer", ["--read-only"])):
            r = subprocess.run([str(executable), "--generate-password", user, "--auth-config", str(config)] + extra,
                               check=True, capture_output=True, text=True, timeout=30)
            passwords[user] = json.loads(r.stdout)["password"]
        port = free_port()
        process = subprocess.Popen([str(executable), "--web", str(database), "--listen", f"127.0.0.1:{port}",
                                    "--auth-config", str(config)], stderr=subprocess.PIPE, text=True)
        try:
            wait(port, process)
            c = Client(port)
            status, page, _ = c.request("GET", "/")
            assert status == 200 and b"YYaccountant" in page
            _, _, r = c.request("GET", "/web/app.js")
            assert "script-src 'self'" in r.getheader("Content-Security-Policy")
            assert c.request("GET", "/", host="evil.example")[0] == 403
            assert c.request("POST", "/web/api", {"tool": "book_list", "arguments": {}})[0] == 401
            assert c.request("POST", "/web/login", {"password": "yya_wrong"})[0] == 401
            assert c.request("POST", "/web/login", {"password": passwords["owner"]}, origin=False)[0] == 403
            status, session, _ = c.request("POST", "/web/login", {"password": passwords["owner"]})
            assert status == 200 and c.cookie and "HttpOnly" not in c.cookie
            c.csrf = session["csrf"]
            assert c.request("POST", "/web/api", {"tool": "book_list", "arguments": {}}, csrf=False)[0] == 403
            status, books, _ = c.api("book_list", {})
            assert status == 200 and len(books["books"]) == 2
            status, book, _ = c.api("book_get", {"book_id": 1})
            assert "entries" not in book and book["entry_count"] > 100
            food = next(a["id"] for a in book["accounts"] if a["code"] == "501")
            cash = next(a["id"] for a in book["accounts"] if a["code"] == "101")
            quick = {"operation": "quick_entry", "quick": {"kind": "expense", "date": time.strftime("%Y-%m-%d"),
                                                           "amount": "1,280", "account": food, "via": cash,
                                                           "description": "Web試験"}}
            status, result, _ = c.api("book_change", {"book_id": 1, "expected_revision": book["revision"],
                                                      "actor": "spoofed", "change": quick})
            assert status == 200 and result["revision"] == book["revision"] + 1, result
            status, conflict, _ = c.api("book_change", {"book_id": 1, "expected_revision": book["revision"],
                                                        "change": quick})
            assert status == 409, conflict
            _, audit, _ = c.api("audit_list", {"book_id": 1, "after": 0})
            while len(audit["changes"]) == 100:
                _, audit, _ = c.api("audit_list", {"book_id": 1, "after": audit["changes"][-1]["id"]})
            assert audit["changes"][-1]["actor"] == "owner"
            status, backup, response = c.request("POST", "/web/backup", {})
            assert status == 200 and backup[:16] == b"SQLite format 3\x00"
            viewer = Client(port)
            _, session, _ = viewer.request("POST", "/web/login", {"password": passwords["viewer"]})
            viewer.csrf = session["csrf"]
            assert session["read_only"]
            assert viewer.api("book_change", {"book_id": 1, "expected_revision": 1, "change": quick})[0] == 403
            assert viewer.api("statistics", {"book_id": 1, "from": "2026-01-01", "to": "2026-12-31"})[0] == 200
            assert c.request("POST", "/web/logout", {})[0] == 200
            assert c.api("book_list", {})[0] == 401
            if browser:
                env = dict(os.environ, YYA_URL=f"http://127.0.0.1:{port}/", YYA_PASSWORD=passwords["owner"],
                           YYA_OUT=str(out.resolve()))
                subprocess.run(["node", str(Path(__file__).with_name("test_web_browser.cjs"))], check=True, env=env,
                               timeout=300)
        finally:
            process.terminate()
            process.wait(timeout=10)
    print("Web validation passed")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Minimal fake Jenkins for manual/agent testing of Leeroy's connection logic.

    scripts/fake-jenkins.py [--port 8099] [--auth USER:TOKEN] [--delay SECS]
                            [--status CODE] [--tls]

Serves GET /whoAmI/api/json (under any path prefix) with an X-Jenkins header.
--auth      require HTTP basic auth, 401 otherwise; the user is echoed back
--delay     sleep before answering (to see the "connecting" state)
--status    always answer with this HTTP status
--tls       serve HTTPS with a throwaway self-signed cert (needs openssl)
"""

import argparse
import base64
import json
import subprocess
import ssl
import tempfile
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

VERSION = "2.504.1"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8099)
    parser.add_argument("--auth")
    parser.add_argument("--delay", type=float, default=0)
    parser.add_argument("--status", type=int)
    parser.add_argument("--tls", action="store_true")
    args = parser.parse_args()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            time.sleep(args.delay)
            if args.status:
                return self.reply(args.status, {"error": "forced"})
            if not self.path.split("?")[0].endswith("/whoAmI/api/json"):
                return self.reply(404, {"error": "not found"})
            user = "anonymous"
            if args.auth:
                expected = "Basic " + base64.b64encode(args.auth.encode()).decode()
                if self.headers.get("Authorization") != expected:
                    return self.reply(401, {"error": "unauthorized"})
                user = args.auth.split(":", 1)[0]
            self.reply(200, {"name": user, "authenticated": user != "anonymous"})

        def reply(self, status: int, body: dict) -> None:
            data = json.dumps(body).encode()
            self.send_response(status)
            self.send_header("X-Jenkins", VERSION)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    scheme = "http"
    if args.tls:
        scheme = "https"
        tmp = Path(tempfile.mkdtemp())
        cert, key = tmp / "cert.pem", tmp / "key.pem"
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=IP:127.0.0.1,DNS:localhost",
                "-keyout",
                str(key),
                "-out",
                str(cert),
            ],
            check=True,
            capture_output=True,
        )
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        server.socket = ctx.wrap_socket(server.socket, server_side=True)
    print(f"fake Jenkins {VERSION} on {scheme}://127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

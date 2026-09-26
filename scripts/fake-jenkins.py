#!/usr/bin/env python3
"""Minimal fake Jenkins for manual/agent testing of Leeroy.

    scripts/fake-jenkins.py [--port 8099] [--auth USER:TOKEN] [--delay SECS]
                            [--status CODE] [--tls] [--jobs N] [--churn]

Serves (under any path prefix, with an X-Jenkins header):
  GET /whoAmI/api/json   the current user
  GET /api/json          a job tree: folders, a multibranch project, every status
--auth      require HTTP basic auth, 401 otherwise; the user is echoed back
--delay     sleep before answering (to see "connecting" / "refreshing")
--status    always answer with this HTTP status
--tls       serve HTTPS with a throwaway self-signed cert (needs openssl)
--jobs N    add N generated jobs under generated/ (for long lists)
--churn     change one job's status on every job-list request (auto-refresh)
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
COLORS = ["blue", "red", "yellow", "aborted", "notbuilt", "disabled", "blue_anime"]


def job(folder: str, name: str, color: str) -> dict:
    full = f"{folder}/{name}" if folder else name
    return {
        "_class": "hudson.model.FreeStyleProject",
        "name": name,
        "fullName": full,
        "url": f"http://jenkins/job/{full}/",
        "color": color,
    }


def folder(parent: str, name: str, children: list, cls: str = "Folder") -> dict:
    full = f"{parent}/{name}" if parent else name
    return {
        "_class": f"com.cloudbees.hudson.plugins.folder.{cls}",
        "name": name,
        "fullName": full,
        "url": f"http://jenkins/job/{full}/",
        "jobs": children,
    }


def job_tree(extra: int, churn_step: int) -> dict:
    jobs = [
        folder(
            "",
            "backend",
            [
                folder(
                    "backend",
                    "api",
                    [
                        job("backend/api", "main", "blue"),
                        job("backend/api", "release-1.2", "red_anime"),
                        job("backend/api", "pr-17", "yellow"),
                    ],
                    cls="WorkflowMultiBranchProject",
                ),
                job("backend", "worker", "yellow"),
                job("backend", "nightly-db-backup", "aborted"),
            ],
        ),
        folder(
            "",
            "frontend",
            [
                folder(
                    "frontend",
                    "web",
                    [
                        job("frontend/web", "main", "blue_anime"),
                        job("frontend/web", "pr-42", "notbuilt"),
                    ],
                    cls="WorkflowMultiBranchProject",
                ),
            ],
        ),
        job("", "docs", "disabled"),
        job("", "infra-terraform-plan", "blue"),
        folder("", "empty-folder", []),
    ]
    if extra:
        jobs.append(
            folder(
                "",
                "generated",
                [
                    job("generated", f"job-{i:04}", COLORS[i % len(COLORS)])
                    for i in range(extra)
                ],
            )
        )
    if churn_step:
        target = jobs[3]  # infra-terraform-plan
        target["color"] = COLORS[churn_step % len(COLORS)]
    return {"_class": "hudson.model.Hudson", "jobs": jobs}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8099)
    parser.add_argument("--auth")
    parser.add_argument("--delay", type=float, default=0)
    parser.add_argument("--status", type=int)
    parser.add_argument("--tls", action="store_true")
    parser.add_argument("--jobs", type=int, default=0)
    parser.add_argument("--churn", action="store_true")
    args = parser.parse_args()
    job_requests = [0]

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            time.sleep(args.delay)
            if args.status:
                return self.reply(args.status, {"error": "forced"})
            path = self.path.split("?")[0]
            user = "anonymous"
            if args.auth:
                expected = "Basic " + base64.b64encode(args.auth.encode()).decode()
                if self.headers.get("Authorization") != expected:
                    return self.reply(401, {"error": "unauthorized"})
                user = args.auth.split(":", 1)[0]
            if path.endswith("/whoAmI/api/json"):
                return self.reply(
                    200, {"name": user, "authenticated": user != "anonymous"}
                )
            if path.endswith("/api/json"):
                job_requests[0] += 1
                step = job_requests[0] if args.churn else 0
                return self.reply(200, job_tree(args.jobs, step))
            self.reply(404, {"error": "not found"})

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

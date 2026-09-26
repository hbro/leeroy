#!/usr/bin/env python3
"""Minimal fake Jenkins for manual/agent testing of Leeroy.

    scripts/fake-jenkins.py [--port 8099] [--auth USER:TOKEN] [--delay SECS]
                            [--status CODE] [--tls] [--jobs N] [--churn]

Serves (under any path prefix, with an X-Jenkins header):
  GET /whoAmI/api/json   the current user
  GET /api/json          a job tree: folders, a multibranch project, every status
  GET /job/../api/json             the job: allBuilds (numbers, with gaps) + lastBuild
  GET /job/../<n>/api/json         build n (404 if it doesn't exist)
  GET /job/../lastBuild/api/json   the job's last build (404 if never built)
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
from urllib.parse import unquote

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


def find_color(items: list, full_name: str):
    for item in items:
        if item.get("fullName") == full_name and "jobs" not in item:
            return item.get("color")
        found = find_color(item.get("jobs", []), full_name)
        if found is not None:
            return found
    return None


SERVER_START_MS = int(time.time() * 1000)

RESULTS = {
    "blue": "SUCCESS",
    "red": "FAILURE",
    "yellow": "UNSTABLE",
    "aborted": "ABORTED",
    "disabled": "SUCCESS",
}


def build_numbers(full_name: str, color: str) -> list:
    """Build numbers of a job, oldest first, with gaps (deleted builds)."""
    if color == "notbuilt":
        return []
    last = 40 + len(full_name) % 17
    return [n for n in range(last - 12, last + 1) if n % 5 != 0 or n == last]


def build(full_name: str, color: str, number: int):
    """A plausible build of a job, or None if that build doesn't exist."""
    numbers = build_numbers(full_name, color)
    if number not in numbers:
        return None
    last = numbers[-1]
    running = number == last and color.endswith("_anime")
    if number == last:
        result = (
            None if running else RESULTS.get(color.removesuffix("_anime"), "SUCCESS")
        )
    else:
        result = (
            "FAILURE"
            if number % 4 == 0
            else "UNSTABLE"
            if number % 7 == 0
            else "SUCCESS"
        )
    started = (
        SERVER_START_MS - (last - number) * 3_600_000
    )  # fixed, so running builds progress
    data = {
        "_class": "org.jenkinsci.plugins.workflow.job.WorkflowRun",
        "number": number,
        "displayName": f"#{number}",
        "result": result,
        "building": running,
        "timestamp": started - (150_000 if running else 600_000),
        "duration": 0 if running else 60_000 + number * 1_000,
        "estimatedDuration": 300_000,
        "description": "Release candidate" if "release" in full_name else None,
        "actions": [
            {
                "_class": "hudson.model.CauseAction",
                "causes": [
                    {
                        "shortDescription": "Started by user Hans"
                        if number % 2
                        else "Started by timer"
                    }
                ],
            },
            {},
        ],
        "changeSets": [
            {
                "items": [
                    {
                        "commitId": f"{number:04x}abcd4567",
                        "msg": f"Change for build {number}\n\ndetails",
                        "author": {"fullName": "Alice"},
                    },
                    {
                        "commitId": "89ef0123abcd",
                        "msg": "Bump version",
                        "author": {"fullName": "Bob"},
                    },
                ]
            }
        ],
    }
    if full_name.startswith("backend/"):
        data["actions"].append(
            {
                "_class": "hudson.model.ParametersAction",
                "parameters": [
                    {"name": "ENV", "value": "staging"},
                    {"name": "DRY_RUN", "value": False},
                ],
            }
        )
    return data


def last_build(full_name: str, color: str):
    numbers = build_numbers(full_name, color)
    return build(full_name, color, numbers[-1]) if numbers else None


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
            if "/job/" in path and path.endswith("/api/json"):
                parts = path.split("/")
                names = [
                    unquote(parts[i + 1])
                    for i, p in enumerate(parts[:-1])
                    if p == "job"
                ]
                full_name = "/".join(names)
                color = find_color(job_tree(args.jobs, 0)["jobs"], full_name)
                if color is None:
                    return self.reply(404, {"error": "no such job"})
                # What follows the last /job/<name>/: "", "lastBuild" or a number.
                last_job = max(i for i, p in enumerate(parts[:-1]) if p == "job") + 1
                tail = parts[last_job + 1 : -2]
                if not tail:
                    numbers = build_numbers(full_name, color)
                    return self.reply(
                        200,
                        {
                            "allBuilds": [{"number": n} for n in reversed(numbers)],
                            "lastBuild": last_build(full_name, color),
                        },
                    )
                if tail == ["lastBuild"]:
                    data = last_build(full_name, color)
                elif tail[0].isdigit():
                    data = build(full_name, color, int(tail[0]))
                else:
                    data = None
                if data is None:
                    return self.reply(404, {"error": "no such build"})
                return self.reply(200, data)
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

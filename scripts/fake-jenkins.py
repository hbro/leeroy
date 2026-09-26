#!/usr/bin/env python3
"""Minimal fake Jenkins for manual/agent testing of Leeroy.

    scripts/fake-jenkins.py [--port 8099] [--auth USER:TOKEN] [--delay SECS]
                            [--status CODE] [--tls] [--jobs N] [--churn]

Serves (under any path prefix, with an X-Jenkins header):
  GET /whoAmI/api/json   the current user
  GET /api/json          a job tree: folders, a multibranch project, every status;
                         with tree=...builds[...]{0,N}: each job's newest N builds
  GET /job/../api/json             the job: allBuilds (numbers, with gaps) + lastBuild
  GET /job/../<n>/api/json         build n (404 if it doesn't exist)
  GET /job/../lastBuild/api/json   the job's last build (404 if never built)
  GET /job/../<n>/logText/progressiveText?start=B   console output from byte B;
      a running build's log grows by a line every 0.3s (X-More-Data: true)
--auth      require HTTP basic auth, 401 otherwise; the user is echoed back.
            Without it, any request is allowed and a basic-auth user, if sent,
            is echoed back too (to show a user name in screenshots).
Also works as an HTTP proxy for any host (absolute request URLs), so Leeroy
can be pointed at e.g. http://jenkins.example.com via proxy.url.
--delay     sleep before answering (to see "connecting" / "refreshing")
--status    always answer with this HTTP status
--tls       serve HTTPS with a throwaway self-signed cert (needs openssl)
--jobs N    add N generated jobs under generated/ (for long lists)
--churn     change one job's status on every job-list request (auto-refresh)
"""

import argparse
import re
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


def console_line(i: int) -> str:
    """Line i of a fake log: ANSI colours, \r progress bars, tabs, UTF-8."""
    kind = i % 10
    if kind == 0:
        return f"\x1b[1m[Pipeline] stage\x1b[0m {{ (Stage {i // 10}) }}"
    if kind == 3:
        return f"Downloading artifact-{i}.jar 10%\r50%\r100% ✓"
    if kind == 5:
        return f"\x1b[32m[INFO]\x1b[0m\tTests run: {i}, Failures: 0 — ümlaut ok"
    if kind == 7:
        return f"+ ./gradlew build --info -Pversion=1.{i} " + "-Dlong.option=value " * 8
    return f"[step {i:05}] doing work…"


def console_text(full_name: str, color: str, number: int):
    """(log bytes, still running) for a build, or None if it doesn't exist."""
    data = build(full_name, color, number)
    if data is None:
        return None
    running = data["building"]
    if running:
        # Grows over time: base lines + one per 0.3 s since the server started.
        count = 20 + int((time.time() * 1000 - SERVER_START_MS) / 300)
    else:
        count = 150 + number % 50
    lines = ["Started by user Hans", "Running in Durability level: MAX_SURVIVABILITY"]
    lines += [console_line(i) for i in range(count)]
    if not running:
        lines.append(f"Finished: {data['result']}")
    return ("\n".join(lines) + "\n").encode(), running


def last_build(full_name: str, color: str):
    numbers = build_numbers(full_name, color)
    return build(full_name, color, numbers[-1]) if numbers else None


def add_builds(items: list, limit: int) -> None:
    """Give every job its newest `limit` builds, like tree=...builds[..]{0,N}."""
    for item in items:
        if "jobs" in item:
            add_builds(item["jobs"], limit)
            continue
        numbers = build_numbers(item["fullName"], item["color"])
        item["builds"] = [
            {
                k: v
                for k, v in build(item["fullName"], item["color"], n).items()
                if k in ("number", "result", "building", "timestamp", "duration")
            }
            for n in reversed(numbers[-limit:])
        ]


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
            # Proxy requests carry an absolute URL: keep only its path.
            self.path = re.sub(r"^https?://[^/]+", "", self.path) or "/"
            path = self.path.split("?")[0]
            user = "anonymous"
            auth = self.headers.get("Authorization", "")
            if args.auth:
                expected = "Basic " + base64.b64encode(args.auth.encode()).decode()
                if auth != expected:
                    return self.reply(401, {"error": "unauthorized"})
                user = args.auth.split(":", 1)[0]
            elif auth.startswith("Basic "):
                try:
                    user = base64.b64decode(auth[6:]).decode().split(":", 1)[0]
                except ValueError:
                    pass
            if path.endswith("/whoAmI/api/json"):
                return self.reply(
                    200, {"name": user, "authenticated": user != "anonymous"}
                )
            if "/job/" in path:
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
                tail = parts[last_job + 1 :]
                if tail[-2:] == ["api", "json"]:
                    tail = tail[:-2]
                if not tail:
                    numbers = build_numbers(full_name, color)
                    return self.reply(
                        200,
                        {
                            "allBuilds": [{"number": n} for n in reversed(numbers)],
                            "lastBuild": last_build(full_name, color),
                        },
                    )
                if (
                    len(tail) == 3
                    and tail[0].isdigit()
                    and tail[1:]
                    == [
                        "logText",
                        "progressiveText",
                    ]
                ):
                    log = console_text(full_name, color, int(tail[0]))
                    if log is None:
                        return self.reply(404, {"error": "no such build"})
                    text, running = log
                    query = self.path.split("?", 1)[1] if "?" in self.path else ""
                    params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
                    start = min(int(params.get("start", "0")), len(text))
                    return self.reply_bytes(
                        text[start:],
                        {"X-Text-Size": str(len(text))}
                        | ({"X-More-Data": "true"} if running else {}),
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
                tree = job_tree(args.jobs, step)
                query = unquote(self.path.split("?", 1)[1]) if "?" in self.path else ""
                limit = re.search(r"builds\[[^\]]*\]\{0,(\d+)\}", query)
                if limit:  # build history: each job's newest N builds
                    add_builds(tree["jobs"], int(limit.group(1)))
                return self.reply(200, tree)
            self.reply(404, {"error": "not found"})

        def reply_bytes(self, body: bytes, headers: dict) -> None:
            self.send_response(200)
            self.send_header("X-Jenkins", VERSION)
            self.send_header("Content-Type", "text/plain;charset=UTF-8")
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

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

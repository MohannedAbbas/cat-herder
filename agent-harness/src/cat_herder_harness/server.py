"""Harness server: a long-running process that serves `/healthz`.

This is the skeleton of the real harness (PLAN.md §7): it currently just serves a
health endpoint so its sandbox pod becomes Ready. The Deep Agents loop, gRPC control
surface, and event streaming are the next steps.
"""

from __future__ import annotations

import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RUN_ID = os.environ.get("RUN_ID", "unknown")
PORT = int(os.environ.get("PORT", "8080"))


class HealthHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/healthz":
            self.send_response(200)
            body = b"ok\n"
        else:
            self.send_response(404)
            body = b"not found\n"
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        pass  # silence per-request logging


def main() -> None:
    server = ThreadingHTTPServer(("0.0.0.0", PORT), HealthHandler)
    print(f"harness listening on :{PORT} (run_id={RUN_ID})", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

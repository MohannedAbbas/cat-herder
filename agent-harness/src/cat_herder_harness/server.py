"""Harness server: drives a Deep Agents (LangGraph) agent inside the sandbox pod.

Serves `/healthz` so the pod becomes Ready, then runs a Deep Agents agent on the
`TASK_SPEC` using `MODEL` (provider-qualified, e.g. `anthropic:claude-sonnet-4-6`).

The LLM API key is read from `LLM_API_KEY` (injected by the control plane from a
Kubernetes Secret); it is mapped to the provider's own env var that LangChain reads.
"""

from __future__ import annotations

import json
import os
import subprocess
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

RUN_ID = os.environ.get("RUN_ID", "unknown")
PORT = int(os.environ.get("PORT", "8080"))
# Control-plane URL used to report run events (injected by the control plane).
CONTROL_URL = os.environ.get("CAT_HERDER_CONTROL_URL", "").rstrip("/")
# The repository to clone and work on (injected from the run's `repo` field).
REPO_URL = os.environ.get("REPO_URL", "").strip()


def report_event(event_type: str, status: str | None = None) -> None:
    """Report a run event back to the control plane (best-effort, non-fatal)."""
    if not CONTROL_URL:
        return
    payload: dict = {"type": event_type}
    if status:
        payload["status"] = status
    req = urllib.request.Request(
        f"{CONTROL_URL}/v1/runs/{RUN_ID}/events",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=10):
            pass
    except Exception as exc:  # noqa: BLE001 — reporting is best-effort
        print(f"failed to report {event_type} event: {exc}", flush=True)


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
        pass


def set_provider_key(model: str) -> None:
    """Map the generic `LLM_API_KEY` to the provider env var LangChain expects."""
    key = os.environ.get("LLM_API_KEY", "").strip()
    if not key:
        return
    provider = model.split(":", 1)[0].lower()
    if provider == "openai":
        os.environ.setdefault("OPENAI_API_KEY", key)
    elif provider == "anthropic":
        os.environ.setdefault("ANTHROPIC_API_KEY", key)
    elif provider == "google" or provider == "google_genai":
        os.environ.setdefault("GOOGLE_API_KEY", key)
    elif provider == "deepseek":
        os.environ.setdefault("DEEPSEEK_API_KEY", key)
    # add more providers as needed


def clone_repo(repo_url: str) -> bool:
    """Clone the target repo into /repo. Returns True on success."""
    try:
        subprocess.run(
            ["git", "clone", "--depth", "1", repo_url, "/repo"],
            check=True,
            capture_output=True,
            text=True,
        )
        print(f"cloned {repo_url} into /repo", flush=True)
        return True
    except subprocess.CalledProcessError as exc:
        print(f"git clone failed: {exc.stderr.strip()}", flush=True)
        return False


def run_agent(task_spec: str, model: str) -> str:
    """Run a Deep Agents agent on the task and return its final message."""
    from deepagents import create_deep_agent
    from deepagents.backends import CompositeBackend, FilesystemBackend, StateBackend

    # Route /repo/* to the real filesystem (the cloned repo), while agent-internal
    # state (memories, offloaded tool results) stays in the ephemeral StateBackend.
    # This lets the agent read AND modify the repo. Without a FilesystemBackend,
    # Deep Agents' tools only see an empty virtual FS.
    backend = CompositeBackend(
        default=StateBackend(),
        routes={"/repo/": FilesystemBackend(root_dir="/repo")},
    )
    agent = create_deep_agent(
        model=model,
        system_prompt="You are a coding agent.",
        backend=backend,
    )
    result = agent.invoke({"messages": [{"role": "user", "content": task_spec}]})
    messages = result.get("messages", []) if isinstance(result, dict) else []
    if messages:
        last = messages[-1]
        return str(getattr(last, "content", last))
    return str(result)


def main() -> None:
    server = ThreadingHTTPServer(("0.0.0.0", PORT), HealthHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"harness listening on :{PORT} (run_id={RUN_ID})", flush=True)

    task_spec = os.environ.get("TASK_SPEC", "").strip()
    model = os.environ.get("MODEL", "").strip()

    if not task_spec or not model:
        print("no TASK_SPEC/MODEL configured; idling (health only)", flush=True)
        threading.Event().wait()
        return

    set_provider_key(model)

    # Clone the target repo (if any) so the agent can read and modify it.
    if REPO_URL:
        clone_repo(REPO_URL)

    report_event("started")
    try:
        result = run_agent(task_spec, model)
        print(f"agent completed: {result}", flush=True)
        report_event("finished", status="succeeded")
    except Exception as exc:  # noqa: BLE001 — keep the pod alive and log
        print(f"agent failed: {exc}", flush=True)
        report_event("finished", status="failed")

    # Keep the health server alive after the agent finishes.
    threading.Event().wait()


if __name__ == "__main__":
    main()

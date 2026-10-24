"""Harness server: drives a Deep Agents (LangGraph) agent inside the sandbox pod.

Serves `/healthz` so the pod becomes Ready, then runs a Deep Agents agent on the
`TASK_SPEC` using `MODEL` (provider-qualified, e.g. `anthropic:claude-sonnet-4-6`).

The LLM API key is read from `LLM_API_KEY` (injected by the control plane from a
Kubernetes Secret); it is mapped to the provider's own env var that LangChain reads.
"""

from __future__ import annotations

import base64
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
# Git token for private-repo clones (optional). Used only via a git askpass
# helper so the credential never appears in argv or logs.
GIT_TOKEN = os.environ.get("GIT_TOKEN", "").strip()

# Env vars the agent's shell must NOT inherit — keeps tokens out of the reach of
# the agent's `env`/shell commands.
SECRET_ENV_KEYS = {
    "GIT_TOKEN",
    "LLM_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GOOGLE_API_KEY",
}


def report_event(
    event_type: str,
    status: str | None = None,
    report: str | None = None,
    changes: list | None = None,
) -> None:
    """Report a run event back to the control plane (best-effort, non-fatal)."""
    if not CONTROL_URL:
        return
    payload: dict = {"type": event_type}
    if status:
        payload["status"] = status
    if report:
        payload["report"] = report
    if changes is not None:
        payload["changes"] = changes
    req = urllib.request.Request(
        f"{CONTROL_URL}/v1/runs/{RUN_ID}/events",
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30):
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


# Only allow explicit, safe git URL schemes. This blocks git's `ext::` transport
# (arbitrary command execution), `file://`, `git://`, and any leading-`-` option
# injection.
ALLOWED_REPO_SCHEMES = {"https", "ssh"}


def is_safe_repo_url(url: str) -> bool:
    if not url or url.startswith("-"):
        return False
    if "::" in url or url.startswith("file:"):
        return False
    scheme = url.split(":", 1)[0].lower()
    return scheme in ALLOWED_REPO_SCHEMES


def write_git_askpass() -> str:
    """Write a git askpass script that supplies `GIT_TOKEN`, returning its path.

    git invokes the askpass script once per credential prompt, passing the prompt
    text as the first argument. We answer the username prompt with a placeholder
    and the password prompt with the token, keeping the token out of the clone
    argv (and therefore out of `ps` and process logs).
    """
    path = "/tmp/cat-herder-git-askpass.sh"
    script = (
        "#!/bin/sh\n"
        "case \"$1\" in\n"
        "  *sername*) echo \"git\" ;;\n"
        "  *) echo \"$GIT_TOKEN\" ;;\n"
        "esac\n"
    )
    with open(path, "w", encoding="utf-8") as f:
        f.write(script)
    os.chmod(path, 0o700)
    return path


def clone_repo(repo_url: str) -> bool:
    """Clone the target repo into /repo. Returns True on success."""
    if not is_safe_repo_url(repo_url):
        print(f"refusing to clone unsafe repo URL: {repo_url}", flush=True)
        return False

    env = os.environ.copy()
    if GIT_TOKEN:
        env["GIT_ASKPASS"] = write_git_askpass()
        env["GIT_TERMINAL_PROMPT"] = "0"
        env["GIT_TOKEN"] = GIT_TOKEN

    try:
        subprocess.run(
            ["git", "clone", "--depth", "1", "--", repo_url, "/repo"],
            check=True,
            capture_output=True,
            text=True,
            env=env,
        )
        print(f"cloned {repo_url} into /repo", flush=True)
        return True
    except subprocess.CalledProcessError as exc:
        print(f"git clone failed: {exc.stderr.strip()}", flush=True)
        return False


def run_agent(task_spec: str, model: str) -> str:
    """Run a Deep Agents agent on the task and return its final message."""
    from deepagents import create_deep_agent
    from deepagents.backends import LocalShellBackend

    # A single LocalShellBackend gives the agent a real filesystem rooted at /repo
    # PLUS a shell (`execute`) that runs in /repo, so it can build/test. The shell
    # inherits the harness environment minus the secrets, so tokens aren't exposed
    # via `env`. (The `execute` tool binds to the *default* backend, hence no
    # StateBackend default here.)
    shell_env = {k: v for k, v in os.environ.items() if k not in SECRET_ENV_KEYS}
    backend = LocalShellBackend(root_dir="/repo", env=shell_env)

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


def _file_change(path: str) -> dict:
    """Build a change entry for an added/modified file (content read from /repo)."""
    full = os.path.join("/repo", path)
    try:
        with open(full, "rb") as f:
            content = f.read()
    except OSError:
        content = b""
    return {
        "path": path,
        "status": "modified",
        "content_base64": base64.b64encode(content).decode("ascii"),
    }


def collect_changes() -> list[dict]:
    """Collect the agent's repo changes as a list of change entries."""
    if not os.path.isdir("/repo/.git"):
        return []
    try:
        subprocess.run(
            ["git", "-C", "/repo", "add", "-A"],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError:
        return []

    proc = subprocess.run(
        ["git", "-C", "/repo", "diff", "--cached", "--name-status"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return []

    changes: list[dict] = []
    for line in proc.stdout.splitlines():
        if not line:
            continue
        parts = line.split("\t")
        status = parts[0]
        if status == "D":
            changes.append({"path": parts[1], "status": "deleted"})
        elif status.startswith("R"):
            # rename: R<score>\told\tnew
            changes.append({"path": parts[1], "status": "deleted"})
            changes.append(_file_change(parts[2]))
        else:  # A or M
            changes.append(_file_change(parts[1]))
    return changes


def main() -> None:
    server = ThreadingHTTPServer(("0.0.0.0", PORT), HealthHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"harness listening on :{PORT} (run_id={RUN_ID})", flush=True)

    task_spec = os.environ.get("TASK_SPEC", "").strip()
    model = os.environ.get("MODEL", "").strip()

    if not task_spec:
        print("no TASK_SPEC configured; idling (health only)", flush=True)
        threading.Event().wait()
        return

    if not model:
        # A run with no agent/model selected — fail cleanly instead of hanging.
        print("no MODEL configured for this run; reporting failure", flush=True)
        report_event("finished", status="failed")
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
        changes = collect_changes()
        report_event("finished", status="succeeded", report=result, changes=changes)
    except Exception as exc:  # noqa: BLE001 — keep the pod alive and log
        print(f"agent failed: {exc}", flush=True)
        report_event("finished", status="failed")

    # Keep the health server alive after the agent finishes.
    threading.Event().wait()


if __name__ == "__main__":
    main()

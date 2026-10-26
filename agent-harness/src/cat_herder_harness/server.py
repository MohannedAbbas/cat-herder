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
import queue
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import grpc

import control_pb2
import control_pb2_grpc

RUN_ID = os.environ.get("RUN_ID", "unknown")
PORT = int(os.environ.get("PORT", "8080"))
# Control-plane URL used to report run events (injected by the control plane).
CONTROL_URL = os.environ.get("CAT_HERDER_CONTROL_URL", "").rstrip("/")
# Control-plane gRPC endpoint (host:port) for the control channel (PLAN.md §6/§9).
# When set, events + heartbeats stream over gRPC instead of the HTTP POST.
GRPC_URL = os.environ.get("CAT_HERDER_GRPC_URL", "").strip()
# The repository to clone and work on (injected from the run's `repo` field).
REPO_URL = os.environ.get("REPO_URL", "").strip()
# Working directory the repo is cloned into and the agent is rooted at. Overridable
# via REPO_DIR so the harness isn't tied to a fixed absolute path.
REPO_DIR = os.environ.get("REPO_DIR", "/repo").strip() or "/repo"
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

# --- stuck-run watchdog -----------------------------------------------------
# The agent runs in this (main) thread while the health server runs in a daemon
# thread. We track "last activity" and expose it on `/livez`: if the agent makes no
# progress for STUCK_TIMEOUT_SECS, `/livez` returns 503 so the pod's liveness probe
# (kubelet) kills it. The harness also self-reports a `stuck` event once, so the
# control plane can mark the run terminal and delete the sandbox promptly.
STUCK_TIMEOUT_SECS = float(os.environ.get("STUCK_TIMEOUT_SECS", "900"))

_state_lock = threading.Lock()
_last_activity = time.monotonic()
_agent_running = False
_agent_finished = False
_stuck_reported = False

# --- token usage metering ---------------------------------------------------
# Accumulated (input, output) token counts from the agent's LLM calls, summed from
# each streamed state snapshot's messages. Reported with the `finished` event.
_usage_input = 0
_usage_output = 0


def record_usage(state: dict) -> None:
    """Sum token usage across a state snapshot's messages (the snapshot is the full
    message history up to that point, so this is a running total, not a delta)."""
    global _usage_input, _usage_output
    in_tok = out_tok = 0
    for msg in state.get("messages", []):
        usage = getattr(msg, "usage_metadata", None)
        if not usage:
            meta = getattr(msg, "response_metadata", {}) or {}
            usage = meta.get("token_usage") or meta.get("usage")
        if usage:
            in_tok += int(usage.get("input_tokens") or usage.get("prompt_tokens") or 0)
            out_tok += int(usage.get("output_tokens") or usage.get("completion_tokens") or 0)
    _usage_input, _usage_output = in_tok, out_tok


def mark_activity() -> None:
    """Record agent progress (called from the agent's stream as it advances)."""
    global _last_activity
    with _state_lock:
        _last_activity = time.monotonic()


def set_agent_running(running: bool) -> None:
    global _agent_running
    with _state_lock:
        _agent_running = running


def set_agent_finished(finished: bool) -> None:
    global _agent_finished
    with _state_lock:
        _agent_finished = finished


def is_stuck() -> bool:
    """True when the agent is running, not finished, and has been silent too long."""
    with _state_lock:
        if not _agent_running or _agent_finished:
            return False
        return (time.monotonic() - _last_activity) > STUCK_TIMEOUT_SECS


def report_stuck_once() -> None:
    """Best-effort, one-shot `stuck` event to the control plane (off-thread)."""
    global _stuck_reported
    with _state_lock:
        if _stuck_reported:
            return
        _stuck_reported = True
    threading.Thread(target=lambda: report_event("stuck"), daemon=True).start()


# --- gRPC control channel ---------------------------------------------------
# Replaces the HTTP events POST when CAT_HERDER_GRPC_URL is set: the harness streams
# events + a periodic heartbeat over one bidirectional gRPC stream per run (§6/§9).
HEARTBEAT_INTERVAL_SECS = 30.0

_control_channel: "ControlChannel | None" = None


class ControlChannel:
    """Streams run events + checkpoints + heartbeats to the control plane over gRPC,
    and receives the resume-seed checkpoint back (§6/§9, docs/design/checkpointing.md)."""

    def __init__(self, grpc_url: str, run_id: int) -> None:
        self._queue: "queue.Queue[control_pb2.HarnessMessage]" = queue.Queue()
        self._run_id = run_id
        self._channel = grpc.insecure_channel(grpc_url)
        self._stub = control_pb2_grpc.HarnessControlStub(self._channel)
        # The resume seed (latest checkpoint) the control plane sends after Register.
        self._seed_event = threading.Event()
        self._seed: "tuple[str, str, str, str, bytes] | None" = None
        self._thread = threading.Thread(
            target=self._run, daemon=True, name="control-channel"
        )
        self._thread.start()

    def send_event(
        self,
        event_type: str,
        status: str | None = None,
        report: str | None = None,
        changes: list | None = None,
        usage: dict | None = None,
    ) -> None:
        msg = control_pb2.HarnessMessage()
        ev = msg.event
        ev.event_type = event_type
        ev.status = status or ""
        ev.report = report or ""
        for c in changes or []:
            fc = ev.changes.add()
            fc.path = c["path"]
            fc.status = c["status"]
            fc.content_base64 = c.get("content_base64") or ""
        if usage:
            ev.usage.input_tokens = int(usage.get("input_tokens", 0))
            ev.usage.output_tokens = int(usage.get("output_tokens", 0))
        self._queue.put(msg)

    def send_checkpoint(
        self,
        thread_id: str,
        checkpoint_ns: str,
        checkpoint_id: str,
        serde_type: str,
        payload: bytes,
    ) -> None:
        msg = control_pb2.HarnessMessage()
        cp = msg.checkpoint
        cp.thread_id = thread_id
        cp.checkpoint_ns = checkpoint_ns
        cp.checkpoint_id = checkpoint_id
        cp.serde_type = serde_type
        cp.payload = payload
        self._queue.put(msg)

    def wait_for_seed(self, timeout: float = 30.0) -> "tuple[str, str, str, str, bytes] | None":
        """Block until the control plane's resume seed arrives (or timeout).

        Returns `(thread_id, checkpoint_ns, checkpoint_id, serde_type, payload)` when a
        checkpoint exists to resume from, else `None` (fresh run).
        """
        self._seed_event.wait(timeout)
        return self._seed

    def _generator(self):
        # The first message identifies the run.
        yield control_pb2.HarnessMessage(register=control_pb2.Register(run_id=self._run_id))

        last_heartbeat = time.monotonic()
        while True:
            try:
                msg = self._queue.get(timeout=1.0)
                yield msg
            except queue.Empty:
                pass

            if time.monotonic() - last_heartbeat >= HEARTBEAT_INTERVAL_SECS:
                last_heartbeat = time.monotonic()
                yield control_pb2.HarnessMessage(heartbeat=control_pb2.Heartbeat())

    def _run(self) -> None:
        try:
            responses = self._stub.Connect(self._generator())
            got_seed = False
            for resp in responses:
                if not got_seed:
                    got_seed = True
                    if resp.WhichOneof("msg") == "checkpoint" and resp.checkpoint.checkpoint_id:
                        cp = resp.checkpoint
                        self._seed = (
                            cp.thread_id,
                            cp.checkpoint_ns,
                            cp.checkpoint_id,
                            cp.serde_type,
                            cp.payload,
                        )
                    self._seed_event.set()
                # The Signal direction (pause/resume/cancel) is unused until HITL (§10).
        except grpc.RpcError as exc:  # noqa: BLE001 — the run still completes
            print(f"gRPC control channel error: {exc}", flush=True)
        finally:
            # Ensure callers waiting on the seed don't block forever if the channel
            # errors before the seed arrives.
            self._seed_event.set()


def report_event(
    event_type: str,
    status: str | None = None,
    report: str | None = None,
    changes: list | None = None,
    usage: dict | None = None,
) -> None:
    """Report a run event back to the control plane (best-effort, non-fatal)."""
    if _control_channel is not None:
        try:
            _control_channel.send_event(event_type, status, report, changes, usage)
        except Exception as exc:  # noqa: BLE001 — reporting is best-effort
            print(f"failed to enqueue {event_type} event: {exc}", flush=True)
        return

    # HTTP fallback (no gRPC endpoint configured, e.g. local dev).
    if not CONTROL_URL:
        return
    payload: dict = {"type": event_type}
    if status:
        payload["status"] = status
    if report:
        payload["report"] = report
    if changes is not None:
        payload["changes"] = changes
    if usage is not None:
        payload["usage"] = usage
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
            # Readiness: the process is up and serving.
            self._respond(200, b"ok\n")
        elif self.path == "/livez":
            # Liveness: the agent is making progress (or is idle/finished).
            if is_stuck():
                report_stuck_once()
                self._respond(503, b"stuck\n")
            else:
                self._respond(200, b"ok\n")
        else:
            self._respond(404, b"not found\n")

    def _respond(self, code: int, body: bytes) -> None:
        self.send_response(code)
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

    The script lives in a fresh per-process temp directory (mode 0700) rather than a
    fixed absolute path, so concurrent harness instances can't collide.
    """
    askpass_dir = tempfile.mkdtemp(prefix=f"cat-herder-git-{RUN_ID}-")
    path = os.path.join(askpass_dir, "cat-herder-git-askpass.sh")
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
    """Clone the target repo into REPO_DIR. Returns True on success."""
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
            ["git", "clone", "--depth", "1", "--", repo_url, REPO_DIR],
            check=True,
            capture_output=True,
            text=True,
            env=env,
        )
        print(f"cloned {repo_url} into {REPO_DIR}", flush=True)
        return True
    except subprocess.CalledProcessError as exc:
        print(f"git clone failed: {exc.stderr.strip()}", flush=True)
        return False


def run_agent(
    task_spec: str,
    model: str,
    checkpointer=None,
    resume: bool = False,
) -> str:
    """Run a Deep Agents agent on the task and return its final message.

    When `checkpointer` is set, the agent's LangGraph state is checkpointed on
    `thread_id = RUN_ID` (flushed to the control plane). `resume=True` continues from
    the last checkpoint (no new input) instead of starting a fresh run.
    """
    from deepagents import create_deep_agent
    from deepagents.backends import LocalShellBackend

    # A single LocalShellBackend gives the agent a real filesystem rooted at REPO_DIR
    # PLUS a shell (`execute`) that runs in REPO_DIR, so it can build/test. The shell
    # inherits the harness environment minus the secrets, so tokens aren't exposed
    # via `env`. (The `execute` tool binds to the *default* backend, hence no
    # StateBackend default here.)
    #
    # `virtual_mode=False` makes the filesystem tools (read_file/write_file/ls/glob)
    # resolve absolute paths against the REAL REPO_DIR, matching the shell's working
    # directory. With the default virtual_mode=True, an absolute path like
    # `<REPO_DIR>/foo` is re-anchored under root_dir and lands at `<REPO_DIR>/<REPO_DIR>/foo`,
    # silently splitting the agent's edits between two locations.
    shell_env = {k: v for k, v in os.environ.items() if k not in SECRET_ENV_KEYS}
    backend = LocalShellBackend(root_dir=REPO_DIR, virtual_mode=False, env=shell_env)

    agent = create_deep_agent(
        model=model,
        system_prompt="You are a coding agent.",
        backend=backend,
        checkpointer=checkpointer,
    )
    config = {"configurable": {"thread_id": RUN_ID}}
    # Stream so we can heartbeat progress; the stuck watchdog's `/livez` reads the
    # last-activity timestamp we bump on each yielded chunk.
    mark_activity()
    result: dict = {}
    # Resume from the last checkpoint (no new input) when a seed was restored;
    # otherwise start a fresh run with the task spec.
    initial = None if resume else {"messages": [{"role": "user", "content": task_spec}]}
    for chunk in agent.stream(initial, config=config, stream_mode="values"):
        mark_activity()
        record_usage(chunk)
        result = chunk
    messages = result.get("messages", []) if isinstance(result, dict) else []
    if messages:
        last = messages[-1]
        return str(getattr(last, "content", last))
    return str(result)


def _file_change(path: str) -> dict:
    """Build a change entry for an added/modified file (content read from REPO_DIR)."""
    full = os.path.join(REPO_DIR, path)
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
    if not os.path.isdir(os.path.join(REPO_DIR, ".git")):
        return []
    try:
        subprocess.run(
            ["git", "-C", REPO_DIR, "add", "-A"],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError:
        return []

    proc = subprocess.run(
        ["git", "-C", REPO_DIR, "diff", "--cached", "--name-status"],
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
    global _control_channel

    server = ThreadingHTTPServer(("0.0.0.0", PORT), HealthHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"harness listening on :{PORT} (run_id={RUN_ID})", flush=True)

    # Start the gRPC control channel (if configured) so events + heartbeats stream
    # back to the control plane over one bidirectional stream (§6/§9).
    if GRPC_URL:
        run_id_int = int(RUN_ID) if RUN_ID.isdigit() else 0
        try:
            _control_channel = ControlChannel(GRPC_URL, run_id_int)
            print(
                f"gRPC control channel connected to {GRPC_URL} (run_id={RUN_ID})",
                flush=True,
            )
        except Exception as exc:  # noqa: BLE001 — fall back to HTTP
            print(f"failed to start gRPC control channel: {exc}", flush=True)

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

    # Checkpointing (§6, docs/design/checkpointing.md): the agent's LangGraph state is
    # checkpointed locally and flushed to the control plane over the gRPC channel. On a
    # re-created pod the control plane returns the latest checkpoint (the resume seed)
    # and we resume from it. Import lazily so the health server is up before langgraph.
    from checkpoint_saver import RemoteCheckpointSaver

    saver = None
    resume = False
    if _control_channel is not None:
        seed = _control_channel.wait_for_seed()
        saver = RemoteCheckpointSaver(_control_channel.send_checkpoint)
        if seed is not None:
            thread_id, checkpoint_ns, checkpoint_id, serde_type, payload = seed
            saver.seed(thread_id, checkpoint_ns, checkpoint_id, serde_type, payload)
            resume = True
            print(f"resuming run {RUN_ID} from checkpoint {checkpoint_id}", flush=True)
    else:
        # No control channel (local dev / HTTP fallback): checkpoint locally only.
        saver = RemoteCheckpointSaver(lambda *_: None)

    report_event("started")
    set_agent_running(True)
    try:
        result = run_agent(task_spec, model, checkpointer=saver, resume=resume)
        print(f"agent completed: {result}", flush=True)
        changes = collect_changes()
        usage = {"input_tokens": _usage_input, "output_tokens": _usage_output}
        report_event("finished", status="succeeded", report=result, changes=changes, usage=usage)
    except Exception as exc:  # noqa: BLE001 — keep the pod alive and log
        print(f"agent failed: {exc}", flush=True)
        usage = {"input_tokens": _usage_input, "output_tokens": _usage_output}
        report_event("finished", status="failed", usage=usage)
    finally:
        set_agent_running(False)
        set_agent_finished(True)

    # Keep the health server alive after the agent finishes (the control plane
    # deletes the sandbox once the run is terminal; see the run lifecycle).
    threading.Event().wait()


if __name__ == "__main__":
    main()

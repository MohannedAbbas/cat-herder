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
import re
import subprocess
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import grpc

from . import control_pb2
from . import control_pb2_grpc
from . import credential_pb2
from . import credential_pb2_grpc

RUN_ID = os.environ.get("RUN_ID", "unknown")
PORT = int(os.environ.get("PORT", "8080"))
# Control-plane URL used to report run events (injected by the control plane).
CONTROL_URL = os.environ.get("CAT_HERDER_CONTROL_URL", "").rstrip("/")
# Control-plane gRPC endpoint (host:port) for the control channel (PLAN.md §6/§9).
# When set, events + heartbeats stream over gRPC instead of the HTTP POST.
GRPC_URL = os.environ.get("CAT_HERDER_GRPC_URL", "").strip()
# JIT credential broker gRPC endpoint (host:port) for runtime on-demand credentials
# (PLAN.md §11.3). When set, the harness exposes a `credential` tool that mints a
# scoped short-lived credential and returns an opaque `cred://<id>` handle; the
# materialized secret is cached in-process and delivered out-of-band (never to the model).
BROKER_URL = os.environ.get("CAT_HERDER_BROKER_URL", "").strip()
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

# --- secret redaction (PLAN.md §16) ----------------------------------------
# Defense-in-depth: scrub credentials/tokens from anything we report back to the
# control plane, so a leaked secret never reaches the event log.
_SECRET_PREFIXES = (
    "ghp_", "gho_", "ghu_", "ghs_", "ghr_",
    "sk-", "AKIA", "ASIA", "xoxb-", "xoxp-", "glpat-",
)
_SECRET_RE = re.compile(
    r"(?:" + "|".join(re.escape(p) for p in _SECRET_PREFIXES) + r")[A-Za-z0-9_-]{16,}"
    r"|Bearer[ \t]+[A-Za-z0-9._-]{8,}"
)


def redact(text: str) -> str:
    """Scrub credential material from a string (PLAN.md §16)."""
    if not text:
        return text
    return _SECRET_RE.sub("[REDACTED]", text)

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
        # HITL signals (resume/cancel) from the control plane, consumed by the agent loop.
        self._signals: "queue.Queue[tuple[str, str] | None]" = queue.Queue()
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

    def send_interrupt(self, resume_token: str, payload: str) -> None:
        msg = control_pb2.HarnessMessage()
        msg.interrupt.resume_token = resume_token
        msg.interrupt.payload = payload
        self._queue.put(msg)

    def wait_for_seed(self, timeout: float = 30.0) -> "tuple[str, str, str, str, bytes] | None":
        """Block until the control plane's resume seed arrives (or timeout).

        Returns `(thread_id, checkpoint_ns, checkpoint_id, serde_type, payload)` when a
        checkpoint exists to resume from, else `None` (fresh run).
        """
        self._seed_event.wait(timeout)
        return self._seed

    def wait_for_signal(self) -> "tuple[str, str] | None":
        """Block until the control plane signals resume/cancel (PLAN.md §10).

        Returns `(signal, response)`, or `None` if the channel closed first.
        """
        return self._signals.get()

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
        # Reconnect for the life of the pod. The API pod can be mid-restart when the
        # harness first connects (e.g. a Tilt/Helm image roll), and a single transient
        # UNAVAILABLE must not permanently drop event/heartbeat reporting (§6/§9).
        first_attempt = True
        backoff = 1.0
        while True:
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
                    elif resp.WhichOneof("msg") == "signal":
                        # HITL: hand the resume/cancel signal to the waiting agent loop (§10).
                        self._signals.put((resp.signal.signal, resp.signal.response))
                backoff = 1.0
            except grpc.RpcError as exc:  # noqa: BLE001 — retry rather than give up
                print(
                    f"gRPC control channel error (retrying in {backoff:.0f}s): {exc}",
                    flush=True,
                )
            finally:
                if first_attempt:
                    # Never block the agent's startup waiting for the channel: treat the
                    # first (possibly failed) attempt as the seed decision, and keep
                    # retrying in the background so events/heartbeats eventually flow.
                    first_attempt = False
                    self._seed_event.set()
            time.sleep(backoff)
            backoff = min(backoff * 2, 30.0)


def report_event(
    event_type: str,
    status: str | None = None,
    report: str | None = None,
    changes: list | None = None,
    usage: dict | None = None,
) -> None:
    """Report a run event back to the control plane (best-effort, non-fatal)."""
    if report:
        report = redact(report)
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


# --- runtime credential broker (PLAN.md §11.3) -----------------------------
# The `credential` tool mints a scoped, short-lived credential via the broker and
# returns an opaque `cred://<id>` handle. The materialized secret is cached in-process
# (keyed by grant id) and delivered out-of-band — through the pod-local `/credential/<id>`
# endpoint and a git askpass helper — never into the agent's context, checkpoints, or
# the event log.

_credential_secrets: dict[str, str] = {}
_credential_lock = threading.Lock()


class BrokerClient:
    """Mints credentials via the broker gRPC and caches the secret out-of-band."""

    def __init__(self, broker_url: str, run_id: int) -> None:
        self._run_id = run_id
        self._channel = grpc.insecure_channel(broker_url)
        self._stub = credential_pb2_grpc.CredentialBrokerStub(self._channel)

    def mint(self, target: str, action: str, scope: str) -> str:
        """Mint a scoped credential, cache its secret, and return the opaque handle."""
        req = credential_pb2.MintRequest(
            run_id=self._run_id, target=target, action=action, scope=scope
        )
        resp = self._stub.Mint(req)
        with _credential_lock:
            _credential_secrets[resp.grant.id] = resp.secret
        return f"cred://{resp.grant.id}"

    def revoke(self, grant_id: str) -> None:
        req = credential_pb2.RevokeRequest(run_id=self._run_id, grant_id=grant_id)
        self._stub.Revoke(req)


def credential_secret(handle: str) -> str | None:
    """Materialize a cached secret by `cred://<id>` handle (out-of-band)."""
    with _credential_lock:
        return _credential_secrets.get(handle)


def make_credential_tool(broker: BrokerClient):
    """Build the `credential` LangChain tool: mints a scoped credential and returns an
    opaque `cred://<id>` handle (the secret is delivered out-of-band, never here)."""
    from langchain_core.tools import tool

    @tool
    def credential(target: str, action: str, scope: str) -> str:
        """Mint a short-lived, scoped credential for a git target.

        `target` is a git host identity (e.g. "github.com/org/repo"), `action` the
        operation ("clone", "push", ...), and `scope` "read" or "write". Returns an
        opaque `cred://<id>` handle; the secret is materialized out-of-band by the git
        credential helper, never returned here.
        """
        try:
            return broker.mint(target, action, scope)
        except Exception as exc:  # noqa: BLE001 — surface the error to the agent
            return f"credential mint failed: {exc}"

    return credential


def write_cred_askpass(handle: str) -> str:
    """Write a git askpass helper that materializes the cached secret for `handle`
    (a `cred://<id>` handle) out-of-band via the pod-local `/credential/<id>` endpoint.
    Returns the helper's path. The secret never appears in the agent's context; git
    receives it directly on its credential prompt.
    """
    grant_id = handle.removeprefix("cred://")
    askpass_dir = tempfile.mkdtemp(prefix=f"cat-herder-cred-{RUN_ID}-")
    path = os.path.join(askpass_dir, "cat-herder-cred-askpass.py")
    script = (
        "#!/usr/bin/env python3\n"
        "import urllib.request\n"
        f'print(urllib.request.urlopen("http://127.0.0.1:{PORT}/credential/{grant_id}",'
        ' timeout=10).read().decode())\n'
    )
    with open(path, "w", encoding="utf-8") as f:
        f.write(script)
    os.chmod(path, 0o700)
    return path


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
        elif self.path.startswith("/credential/"):
            # Pod-local sidecar endpoint that materializes a cached credential secret
            # out-of-band for the git askpass helper (PLAN.md §11.3). Bound to the
            # pod's own loopback, so the secret is never returned to the model.
            grant_id = self.path[len("/credential/"):]
            secret = credential_secret(f"cred://{grant_id}")
            if secret is None:
                self._respond(404, b"not found\n")
            else:
                self._respond(200, secret.encode("utf-8"))
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


def set_provider_base_url(model: str) -> None:
    """Route the model's API through the LiteLLM metering proxy (PLAN.md §12).

    The proxy is the authoritative budget hard stop — all model traffic must go
    through it. LiteLLM exposes an OpenAI-compatible `/v1` route; per-provider base-url
    env vars are pointed at it so the provider SDK calls the proxy, not the raw
    provider endpoint (which the sandbox egress policy forbids anyway).
    """
    proxy = os.environ.get("CAT_HERDER_LLM_PROXY_URL", "").strip().rstrip("/")
    if not proxy:
        return
    provider = model.split(":", 1)[0].lower()
    base = f"{proxy}/v1"
    if provider == "openai":
        os.environ["OPENAI_BASE_URL"] = base
    elif provider == "deepseek":
        # DeepSeek is OpenAI-compatible; set both so whichever env var the client
        # reads routes through the proxy.
        os.environ["OPENAI_BASE_URL"] = base
        os.environ.setdefault("DEEPSEEK_BASE_URL", base)
    elif provider == "anthropic":
        os.environ["ANTHROPIC_BASE_URL"] = base
    elif provider == "google" or provider == "google_genai":
        os.environ.setdefault("GOOGLE_BASE_URL", base)


def parse_interrupt_on() -> dict:
    """Tools that pause for human approval (PLAN.md §10). Read from
    `CAT_HERDER_INTERRUPT_ON` (a JSON object mapping tool name -> true). Empty (off)
    by default; the actual tool selection is a policy concern (§11.8).
    """
    raw = os.environ.get("CAT_HERDER_INTERRUPT_ON", "").strip()
    if not raw:
        return {}
    try:
        value = json.loads(raw)
        return value if isinstance(value, dict) else {}
    except json.JSONDecodeError:
        print(f"invalid CAT_HERDER_INTERRUPT_ON: {raw}", flush=True)
        return {}


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

    # Already materialized — either by the control plane's init container or a
    # resumed workspace on the durable RWX PVC (§6/§13). Skip re-cloning so a resumed
    # agent sees its own edits instead of a fresh checkout.
    if os.path.isdir(os.path.join(REPO_DIR, ".git")):
        print(f"workspace already materialized at {REPO_DIR}; skipping clone", flush=True)
        return True

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
    interrupt_on: dict | None = None,
    on_interrupt=None,
    tools=None,
) -> str:
    """Run a Deep Agents agent on the task and return its final message.

    When `checkpointer` is set, the agent's LangGraph state is checkpointed on
    `thread_id = RUN_ID` (flushed to the control plane). `resume=True` continues from
    the last checkpoint (no new input) instead of starting a fresh run.

    `interrupt_on` (and `on_interrupt`) wire human-in-the-loop (§10): when the agent
    pauses on an interrupt, `on_interrupt(interrupt, config)` is called; it returns the
    LangGraph resume decision dict (or `None` to cancel).

    `tools` are extra tools appended to the agent's built-in shell/filesystem tools
    (e.g. the runtime `credential` tool, §11.3).
    """
    from deepagents import create_deep_agent
    from deepagents.backends import LocalShellBackend
    from langgraph.types import Command

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
        interrupt_on=interrupt_on,
        tools=tools,
    )
    config = {"configurable": {"thread_id": RUN_ID}}
    # Stream so we can heartbeat progress; the stuck watchdog's `/livez` reads the
    # last-activity timestamp we bump on each yielded chunk.
    mark_activity()
    result: dict = {}
    # Resume from the last checkpoint (no new input) when a seed was restored;
    # otherwise start a fresh run with the task spec.
    initial = None if resume else {"messages": [{"role": "user", "content": task_spec}]}
    while True:
        for chunk in agent.stream(initial, config=config, stream_mode="values"):
            mark_activity()
            record_usage(chunk)
            result = chunk
        # A stream that paused on an interrupt ends with `__interrupt__` in the state.
        interrupts = result.get("__interrupt__", ()) if isinstance(result, dict) else ()
        if not interrupts:
            break
        decision = on_interrupt(interrupts[0], config) if on_interrupt is not None else None
        if decision is None:
            # Cancelled by the human — stop without a final report (the run is already
            # terminal in the control plane).
            return ""
        initial = Command(resume=decision)

    messages = result.get("messages", []) if isinstance(result, dict) else []
    if messages:
        last = messages[-1]
        return str(getattr(last, "content", last))
    return str(result)


def parse_workflow_steps() -> list[dict]:
    """Read the resolved workflow steps from `WORKFLOW_STEPS` (JSON
    `[{"agent_type": ..., "model": ...}]`), injected by the control plane (§7)."""
    raw = os.environ.get("WORKFLOW_STEPS", "").strip()
    if not raw:
        return []
    try:
        value = json.loads(raw)
        return value if isinstance(value, list) else []
    except json.JSONDecodeError:
        print(f"invalid WORKFLOW_STEPS: {raw}", flush=True)
        return []


def run_workflow(
    steps: list[dict],
    checkpointer=None,
    resume: bool = False,
    tools=None,
) -> str:
    """Compile the workflow's ordered steps into a linear LangGraph `StateGraph` (§7).

    One node per step; each node runs a Deep Agents agent bound to that step's model
    (role prompt) over the task + the accumulated prior output, and the accumulated
    messages feed the next node. The graph checkpoints on `thread_id = RUN_ID`, so a
    workflow pauses/resumes like a single-agent run (§6).
    """
    from typing_extensions import TypedDict

    from deepagents import create_deep_agent
    from deepagents.backends import LocalShellBackend
    from langgraph.graph import END, START, StateGraph

    shell_env = {k: v for k, v in os.environ.items() if k not in SECRET_ENV_KEYS}
    backend = LocalShellBackend(root_dir=REPO_DIR, virtual_mode=False, env=shell_env)

    class WorkflowState(TypedDict):
        messages: list

    def step_node(step: dict):
        def node(state: WorkflowState) -> dict:
            model = (step.get("model") or "").strip()
            role = (step.get("agent_type") or "agent").strip()
            prior = state.get("messages", [])
            if not model:
                # An unknown agent type resolves to no model; skip the step rather
                # than guess a model.
                return {"messages": prior}

            set_provider_key(model)
            set_provider_base_url(model)
            agent = create_deep_agent(
                model=model,
                system_prompt=(
                    f"You are the '{role}' step of a workflow. Complete this step and "
                    "report a concise result for the next step."
                ),
                backend=backend,
                tools=tools,
            )
            task = os.environ.get("TASK_SPEC", "")
            if prior:
                context = "\n\n".join(
                    f"[{m.get('role', 'step')}] {m.get('content', '')}"
                    for m in prior
                    if isinstance(m, dict) and m.get("content")
                )
                user = f"{task}\n\nPrior steps:\n{context}" if task else context
            else:
                user = task
            out = agent.invoke({"messages": [{"role": "user", "content": user}]})
            msgs = out.get("messages", [])
            last = str(getattr(msgs[-1], "content", msgs[-1])) if msgs else ""
            return {"messages": prior + [{"role": role, "content": last}]}

        return node

    graph = StateGraph(WorkflowState)
    for i, step in enumerate(steps):
        graph.add_node(f"step_{i}", step_node(step))
    graph.add_edge(START, "step_0")
    for i in range(len(steps) - 1):
        graph.add_edge(f"step_{i}", f"step_{i + 1}")
    graph.add_edge(f"step_{len(steps) - 1}", END)
    app = graph.compile(checkpointer=checkpointer)
    config = {"configurable": {"thread_id": RUN_ID}}

    mark_activity()
    result: dict = {}
    initial = None if resume else {"messages": []}
    for chunk in app.stream(initial, config=config, stream_mode="values"):
        mark_activity()
        result = chunk
    messages = result.get("messages", []) if isinstance(result, dict) else []
    if messages:
        last = messages[-1]
        return str(last.get("content", last) if isinstance(last, dict) else last)
    return ""


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

    # Runtime credential broker (PLAN.md §11.3): expose the `credential` tool when a
    # broker endpoint is configured. The tool returns opaque handles; the secret is
    # materialized out-of-band by the git askpass helper.
    credential_tool = None
    if BROKER_URL:
        run_id_int = int(RUN_ID) if RUN_ID.isdigit() else 0
        try:
            broker_client = BrokerClient(BROKER_URL, run_id_int)
            credential_tool = make_credential_tool(broker_client)
            print(
                f"credential broker connected to {BROKER_URL} (run_id={RUN_ID})",
                flush=True,
            )
        except Exception as exc:  # noqa: BLE001 — the run still completes without it
            print(f"failed to start credential broker client: {exc}", flush=True)

    task_spec = os.environ.get("TASK_SPEC", "").strip()
    model = os.environ.get("MODEL", "").strip()
    # A `workflow` run has per-step models (injected via WORKFLOW_STEPS), not a single
    # MODEL (§7).
    workflow_steps = parse_workflow_steps()

    if not task_spec:
        print("no TASK_SPEC configured; idling (health only)", flush=True)
        threading.Event().wait()
        return

    if not model and not workflow_steps:
        # A run with no agent/model (and no workflow steps) — fail cleanly instead of
        # hanging.
        print("no MODEL or WORKFLOW_STEPS configured; reporting failure", flush=True)
        report_event("finished", status="failed")
        threading.Event().wait()
        return

    if not workflow_steps:
        set_provider_key(model)
        # Route model traffic through the LiteLLM metering proxy (PLAN.md §12) so every
        # call is metered against the run's budget before reaching the provider.
        set_provider_base_url(model)

    # Clone the target repo (if any) so the agent can read and modify it.
    if REPO_URL:
        clone_repo(REPO_URL)

    # Checkpointing (§6, docs/design/checkpointing.md): the agent's LangGraph state is
    # checkpointed locally and flushed to the control plane over the gRPC channel. On a
    # re-created pod the control plane returns the latest checkpoint (the resume seed)
    # and we resume from it. Import lazily so the health server is up before langgraph.
    from .checkpoint_saver import RemoteCheckpointSaver

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

    # Human-in-the-loop (§10): tools listed in CAT_HERDER_INTERRUPT_ON pause for
    # approval. `on_interrupt` reports the interrupt and blocks for the control plane's
    # resume/cancel signal, then returns the LangGraph resume decision.
    interrupt_on = parse_interrupt_on()

    def on_interrupt(interrupt, config):
        tup = saver.get_tuple(config)
        resume_token = tup.config["configurable"]["checkpoint_id"] if tup else ""
        payload = json.dumps(interrupt.value, default=str)
        payload = redact(payload)
        print(f"interrupt: {payload} (resume_token={resume_token})", flush=True)
        if _control_channel is None:
            return None  # no control channel -> nothing to approve against
        _control_channel.send_interrupt(resume_token, payload)
        signal = _control_channel.wait_for_signal()
        if signal is None or signal[0] != "resume":
            return None  # cancelled
        return {interrupt.id: {"type": "approve"}}

    report_event("started")
    set_agent_running(True)
    try:
        if workflow_steps:
            result = run_workflow(
                workflow_steps,
                checkpointer=saver,
                resume=resume,
                tools=[credential_tool] if credential_tool is not None else None,
            )
        else:
            result = run_agent(
                task_spec,
                model,
                checkpointer=saver,
                resume=resume,
                interrupt_on=interrupt_on,
                on_interrupt=on_interrupt,
                tools=[credential_tool] if credential_tool is not None else None,
            )
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

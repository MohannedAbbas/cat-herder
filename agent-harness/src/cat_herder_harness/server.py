"""Harness server: drives a Deep Agents (LangGraph) agent inside the sandbox pod.

Serves `/healthz` so the pod becomes Ready, then runs a Deep Agents agent on the
`TASK_SPEC` using `MODEL` (provider-qualified, e.g. `anthropic:claude-sonnet-4-6`).

The LLM API key is read from `LLM_API_KEY` (injected by the control plane from a
Kubernetes Secret); it is mapped to the provider's own env var that LangChain reads.
"""

from __future__ import annotations

import os
import threading
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
    # add more providers as needed


def run_agent(task_spec: str, model: str) -> str:
    """Run a Deep Agents agent on the task and return its final message."""
    from deepagents import create_deep_agent

    agent = create_deep_agent(model=model, system_prompt="You are a coding agent.")
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
    try:
        result = run_agent(task_spec, model)
        print(f"agent completed: {result}", flush=True)
    except Exception as exc:  # noqa: BLE001 — keep the pod alive and log
        print(f"agent failed: {exc}", flush=True)

    # Keep the health server alive after the agent finishes.
    threading.Event().wait()


if __name__ == "__main__":
    main()

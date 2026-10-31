"""Unit tests for the live run-log path (NEXT_STEPS #18).

Covers the harness side: `ControlChannel.send_log`, `emit_log` (redaction +
fallback), `emit_chunk_messages` (LLM/tool extraction), and the stdout/stderr tee.

Run with:  PYTHONPATH=agent-harness/src python -m pytest agent-harness/tests
"""

from __future__ import annotations

import io

import pytest

from cat_herder_harness import server


class _FakeChannel:
    def __init__(self) -> None:
        self.logs: list[tuple[str, str]] = []

    def send_log(self, stream: str, line: str) -> None:
        self.logs.append((stream, line))


@pytest.fixture(autouse=True)
def _reset_state(monkeypatch):
    monkeypatch.setattr(server, "_control_channel", None)
    monkeypatch.setattr(server, "_last_logged_key", None)
    monkeypatch.setattr(server, "_fallback_stream", None)
    yield
    monkeypatch.setattr(server, "_control_channel", None)
    monkeypatch.setattr(server, "_last_logged_key", None)
    monkeypatch.setattr(server, "_fallback_stream", None)


def test_send_log_enqueues_harness_message():
    """ControlChannel.send_log enqueues a HarnessMessage carrying `msg.log`."""
    channel = server.ControlChannel.__new__(server.ControlChannel)
    import queue

    channel._queue = queue.Queue()
    channel.send_log("llm", "hello")

    msg = channel._queue.get_nowait()
    assert msg.WhichOneof("msg") == "log"
    assert msg.log.stream == "llm"
    assert msg.log.line == "hello"


def test_emit_log_redacts_and_sends_over_channel(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    token = "ghp_" + "a" * 24
    server.emit_log("tool", f"curl -H 'Authorization: {token}'")

    assert len(fake.logs) == 1
    stream, line = fake.logs[0]
    assert stream == "tool"
    assert token not in line
    assert "[REDACTED]" in line


def test_emit_log_falls_back_to_stdout(monkeypatch):
    # No control channel -> written to the original stdout (no tee recursion).
    buf = io.StringIO()
    monkeypatch.setattr(server, "_fallback_stream", buf)
    server.emit_log("stdout", "cloned repo")
    assert "cloned repo" in buf.getvalue()


def test_tee_does_not_recurse_without_channel(monkeypatch):
    # A teed lifecycle print with no control channel must not loop forever and still
    # reaches the pod's real stdout exactly once (via write-through).
    original = io.StringIO()
    tee = server._TeeStream(original, "stdout")
    tee.write("agent completed: ok\n")
    assert original.getvalue() == "agent completed: ok\n"


def test_emit_chunk_messages_extracts_ai_message(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    class _AIMessage:
        type = "ai"
        content = "I will run the tests."

    server.emit_chunk_messages({"messages": [_AIMessage()]})
    assert fake.logs == [("llm", "I will run the tests.")]


def test_emit_chunk_messages_extracts_tool_message(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    class _ToolMessage:
        type = "tool"
        name = "execute"
        content = "ok\n2 passed"

    server.emit_chunk_messages({"messages": [_ToolMessage()]})
    assert len(fake.logs) == 1
    stream, line = fake.logs[0]
    assert stream == "tool"
    assert "execute" in line
    assert "2 passed" in line


def test_emit_chunk_messages_handles_list_content(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    class _AIMessage:
        type = "ai"
        content = [{"type": "text", "text": "part one"}, {"type": "text", "text": "part two"}]

    server.emit_chunk_messages({"messages": [_AIMessage()]})
    assert fake.logs == [("llm", "part one\npart two")]


def test_emit_chunk_messages_dict_step_output(monkeypatch):
    # Workflow graph state carries plain dicts; a step output is logged as llm.
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)
    server.emit_chunk_messages({"messages": [{"role": "coder", "content": "done coding"}]})
    assert fake.logs == [("llm", "done coding")]


def test_emit_chunk_messages_dedupes_repeated_tail(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    class _AIMessage:
        type = "ai"
        content = "same line"

    msg = _AIMessage()
    server.emit_chunk_messages({"messages": [msg]})
    server.emit_chunk_messages({"messages": [msg]})
    assert fake.logs == [("llm", "same line")]


def test_emit_chunk_messages_ignores_human_messages(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    class _HumanMessage:
        type = "human"
        content = "do the task"

    server.emit_chunk_messages({"messages": [_HumanMessage()]})
    assert fake.logs == []


def test_tee_stream_emits_complete_lines_and_write_through(monkeypatch):
    fake = _FakeChannel()
    monkeypatch.setattr(server, "_control_channel", fake)

    buffer = io.StringIO()
    tee = server._TeeStream(buffer, "stdout")
    tee.write("cloned repo\n")
    tee.write("partial ")
    tee.flush()  # flushes the trailing partial line
    tee.write("done\n")

    assert "cloned repo\npartial done\n" in buffer.getvalue()
    assert ("stdout", "cloned repo") in fake.logs
    assert ("stdout", "partial ") in fake.logs
    assert ("stdout", "done") in fake.logs

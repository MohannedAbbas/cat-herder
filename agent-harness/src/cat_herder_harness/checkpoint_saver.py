"""A LangGraph checkpointer that keeps state in-memory and flushes it to the control
plane over the gRPC control channel (PLAN.md §6, docs/design/checkpointing.md).

The pod has no database access: the control plane is the sole writer to Postgres. This
saver is the authoritative *local* buffer for the running pod — LangGraph reads/writes
it synchronously — and it pushes each checkpoint (the full `CheckpointTuple`, serialized
with LangGraph's `JsonPlusSerializer`) to the control plane asynchronously. On a
re-created pod, the harness seeds this saver from the checkpoint the control plane
returns, so LangGraph resumes from the last checkpoint.
"""

from __future__ import annotations

import threading
from typing import Any, Callable, Iterator, Optional, Sequence

from langgraph.checkpoint.base import (
    BaseCheckpointSaver,
    Checkpoint,
    CheckpointMetadata,
    CheckpointTuple,
    ChannelVersions,
    get_checkpoint_id,
    get_checkpoint_metadata,
    writes_sort_key,
)
from langchain_core.runnables import RunnableConfig

# The control plane stores the checkpoint opaquely; this callback hands it the
# (thread_id, checkpoint_ns, checkpoint_id, serde_type, payload) of each serialized
# snapshot. `serde_type` + `payload` are the output of `JsonPlusSerializer.dumps_typed`.
FlushFn = Callable[[str, str, str, str, bytes], None]


class RemoteCheckpointSaver(BaseCheckpointSaver):
    """In-memory checkpointer that flushes serialized checkpoints to the control plane."""

    def __init__(self, on_checkpoint: FlushFn) -> None:
        super().__init__()
        self._on_checkpoint = on_checkpoint
        self._lock = threading.RLock()
        # thread_id -> checkpoint_ns -> checkpoint_id -> record
        self._checkpoints: dict[str, dict[str, dict[str, dict[str, Any]]]] = {}
        # thread_id -> checkpoint_ns -> [checkpoint_id ...] in insertion order
        self._order: dict[tuple[str, str], list[str]] = {}
        # (thread_id, checkpoint_ns, checkpoint_id) -> [(task_id, idx, channel, value, task_path)]
        self._writes: dict[tuple[str, str, str], list[tuple[str, int, str, Any, str]]] = {}

    # --- snapshot (de)serialization -----------------------------------------

    def _snapshot(
        self, thread_id: str, checkpoint_ns: str, checkpoint_id: str
    ) -> Optional[tuple[str, bytes]]:
        """Serialize the checkpoint + pending writes into one opaque blob, or None."""
        rec = self._checkpoints.get(thread_id, {}).get(checkpoint_ns, {}).get(checkpoint_id)
        if rec is None:
            return None
        snapshot = {
            "checkpoint": rec["checkpoint"],
            "metadata": rec["metadata"],
            "parent_checkpoint_id": rec["parent_checkpoint_id"],
            "pending_writes": [
                [task_id, idx, channel, value, task_path]
                for (task_id, idx, channel, value, task_path) in self._writes.get(
                    (thread_id, checkpoint_ns, checkpoint_id), []
                )
            ],
        }
        return self.serde.dumps_typed(snapshot)

    def seed(self, thread_id: str, checkpoint_ns: str, checkpoint_id: str, serde_type: str, payload: bytes) -> None:
        """Load a resume snapshot returned by the control plane into the local store."""
        snapshot = self.serde.loads_typed((serde_type, payload))
        with self._lock:
            rec = {
                "checkpoint": snapshot["checkpoint"],
                "metadata": snapshot["metadata"],
                "parent_checkpoint_id": snapshot.get("parent_checkpoint_id"),
            }
            self._checkpoints.setdefault(thread_id, {}).setdefault(checkpoint_ns, {})[
                checkpoint_id
            ] = rec
            self._order.setdefault((thread_id, checkpoint_ns), []).append(checkpoint_id)
            writes = [
                tuple(entry) for entry in snapshot.get("pending_writes", [])
            ]
            self._writes[(thread_id, checkpoint_ns, checkpoint_id)] = writes

    # --- checkpoint saver interface -----------------------------------------

    def _emit(self, thread_id: str, checkpoint_ns: str, checkpoint_id: str) -> None:
        """Serialize and flush the checkpoint to the control plane (best-effort)."""
        serialized = self._snapshot(thread_id, checkpoint_ns, checkpoint_id)
        if serialized is None:
            return
        serde_type, payload = serialized
        try:
            self._on_checkpoint(thread_id, checkpoint_ns, checkpoint_id, serde_type, payload)
        except Exception as exc:  # noqa: BLE001 — flush is best-effort
            print(f"checkpoint flush failed: {exc}", flush=True)

    def put(
        self,
        config: RunnableConfig,
        checkpoint: Checkpoint,
        metadata: CheckpointMetadata,
        new_versions: ChannelVersions,
    ) -> RunnableConfig:
        thread_id: str = config["configurable"]["thread_id"]
        checkpoint_ns: str = config["configurable"].get("checkpoint_ns", "")
        checkpoint_id: str = checkpoint["id"]
        parent_checkpoint_id = config["configurable"].get("checkpoint_id")

        with self._lock:
            rec = {
                "checkpoint": checkpoint.copy(),
                "metadata": get_checkpoint_metadata(config, metadata),
                "parent_checkpoint_id": parent_checkpoint_id,
            }
            self._checkpoints.setdefault(thread_id, {}).setdefault(checkpoint_ns, {})[
                checkpoint_id
            ] = rec
            order = self._order.setdefault((thread_id, checkpoint_ns), [])
            if checkpoint_id not in order:
                order.append(checkpoint_id)

        self._emit(thread_id, checkpoint_ns, checkpoint_id)

        return {
            "configurable": {
                "thread_id": thread_id,
                "checkpoint_ns": checkpoint_ns,
                "checkpoint_id": checkpoint_id,
            }
        }

    def put_writes(
        self,
        config: RunnableConfig,
        writes: Sequence[tuple[str, Any]],
        task_id: str,
        task_path: str = "",
    ) -> None:
        thread_id: str = config["configurable"]["thread_id"]
        checkpoint_ns: str = config["configurable"].get("checkpoint_ns", "")
        checkpoint_id: str = get_checkpoint_id(config) or ""
        key = (thread_id, checkpoint_ns, checkpoint_id)
        with self._lock:
            stored = self._writes.setdefault(key, [])
            start = len(stored)
            for idx, (channel, value) in enumerate(writes):
                stored.append((task_id, start + idx, channel, value, task_path))

        # Pending writes are part of the resume snapshot; re-emit the checkpoint so the
        # control plane's latest copy includes them.
        self._emit(thread_id, checkpoint_ns, checkpoint_id)

    def _ordered_writes(
        self, thread_id: str, checkpoint_ns: str, checkpoint_id: str
    ) -> list[tuple[str, str, Any]]:
        stored = self._writes.get((thread_id, checkpoint_ns, checkpoint_id), [])
        return [
            (task_id, channel, value)
            for (task_id, _idx, channel, value, task_path) in sorted(
                stored, key=lambda w: writes_sort_key(w[4], w[0], w[1])
            )
        ]

    def get_tuple(self, config: RunnableConfig) -> Optional[CheckpointTuple]:
        thread_id: str = config["configurable"]["thread_id"]
        checkpoint_ns: str = config["configurable"].get("checkpoint_ns", "")
        checkpoints = self._checkpoints.get(thread_id, {}).get(checkpoint_ns, {})
        if not checkpoints:
            return None

        checkpoint_id = get_checkpoint_id(config)
        if checkpoint_id is None:
            checkpoint_id = self._order.get((thread_id, checkpoint_ns), [])[-1]
        rec = checkpoints.get(checkpoint_id)
        if rec is None:
            return None

        parent_config = None
        if rec["parent_checkpoint_id"]:
            parent_config = {
                "configurable": {
                    "thread_id": thread_id,
                    "checkpoint_ns": checkpoint_ns,
                    "checkpoint_id": rec["parent_checkpoint_id"],
                }
            }
        return CheckpointTuple(
            config={
                "configurable": {
                    "thread_id": thread_id,
                    "checkpoint_ns": checkpoint_ns,
                    "checkpoint_id": checkpoint_id,
                }
            },
            checkpoint=rec["checkpoint"],
            metadata=rec["metadata"],
            pending_writes=self._ordered_writes(thread_id, checkpoint_ns, checkpoint_id),
            parent_config=parent_config,
        )

    def list(
        self,
        config: Optional[RunnableConfig],
        *,
        filter: Optional[dict[str, Any]] = None,
        before: Optional[RunnableConfig] = None,
        limit: Optional[int] = None,
    ) -> Iterator[CheckpointTuple]:
        if config is None:
            return
        thread_id: str = config["configurable"]["thread_id"]
        checkpoint_ns: str = config["configurable"].get("checkpoint_ns", "")
        checkpoints = self._checkpoints.get(thread_id, {}).get(checkpoint_ns, {})
        order = self._order.get((thread_id, checkpoint_ns), [])
        ids = list(reversed(order))
        if before is not None:
            before_id = get_checkpoint_id(before)
            if before_id in checkpoints:
                pos = order.index(before_id)
                ids = list(reversed(order[:pos]))
        if limit is not None:
            ids = ids[:limit]
        for checkpoint_id in ids:
            rec = checkpoints.get(checkpoint_id)
            if rec is None:
                continue
            yield CheckpointTuple(
                config={
                    "configurable": {
                        "thread_id": thread_id,
                        "checkpoint_ns": checkpoint_ns,
                        "checkpoint_id": checkpoint_id,
                    }
                },
                checkpoint=rec["checkpoint"],
                metadata=rec["metadata"],
                pending_writes=self._ordered_writes(thread_id, checkpoint_ns, checkpoint_id),
                parent_config=(
                    {
                        "configurable": {
                            "thread_id": thread_id,
                            "checkpoint_ns": checkpoint_ns,
                            "checkpoint_id": rec["parent_checkpoint_id"],
                        }
                    }
                    if rec["parent_checkpoint_id"]
                    else None
                ),
            )

    def delete_thread(self, thread_id: str) -> None:
        with self._lock:
            self._checkpoints.pop(thread_id, None)
            for ns in list(self._order.keys()):
                if ns[0] == thread_id:
                    self._order.pop(ns, None)
            for key in list(self._writes.keys()):
                if key[0] == thread_id:
                    self._writes.pop(key, None)

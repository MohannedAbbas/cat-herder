"""Harness server: drives Deep Agents, exposes the gRPC control surface.

Contract (start/pause/resume/cancel, event + checkpoint streaming, heartbeat)
is defined in `proto/control.proto`. This is a skeleton; the real
implementation embeds OSS LangGraph and deepagents.
"""

from __future__ import annotations


async def main() -> None:
    raise NotImplementedError("harness control surface not yet implemented")


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())

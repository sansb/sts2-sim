"""A standard-library client for `sts-sim diff-serve` (#2999, under #2827).

`differential.py` owned this client, but it also drove the Python oracle (it
imported `project_state`, which imported `combat_sim`; both are deleted), so every tool that only
wanted to talk to the Rust engine inherited the simulator import. #2999
deleted the differential with the rest of the Python/Rust parity tooling and
moved the client here, verbatim apart from its error class, for the tools that
replay lines on the Rust engine alone (`export_research_review.py`,
`verify_search_suite.py`).
"""

from __future__ import annotations

import json
import pathlib
import subprocess
from typing import Any

#: The line protocol `src/diff_serve.rs` announces in its greeting.
PROTOCOL = "diff-serve-v1"


class DiffServeRefusal(NotImplementedError):
    """The engine process did something a driver must not paper over."""


class DiffServe:
    """The Rust engine, spoken to over `diff-serve-v1`."""

    def __init__(self, binary: pathlib.Path) -> None:
        self.binary = binary
        self.process = subprocess.Popen(
            [str(binary), "diff-serve"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            text=True, bufsize=1)
        greeting = self._readline()
        if greeting.get("protocol") != PROTOCOL:
            raise DiffServeRefusal(
                f"diff-serve announced {greeting!r}, not {PROTOCOL!r}; a "
                "driver that does not recognise the protocol must not proceed")
        self.requests = 0

    def _readline(self) -> dict[str, Any]:
        assert self.process.stdout is not None
        line = self.process.stdout.readline()
        if not line:
            raise DiffServeRefusal(
                "diff-serve closed its output; the engine died mid-run")
        return json.loads(line)

    def request(self, payload: dict[str, Any]) -> dict[str, Any]:
        assert self.process.stdin is not None
        self.requests += 1
        self.process.stdin.write(json.dumps(payload) + "\n")
        self.process.stdin.flush()
        return self._readline()

    def load(self, document: dict[str, Any]) -> dict[str, Any]:
        return self.request({"cmd": "load", "entry": document})

    def legal(self) -> dict[str, Any]:
        return self.request({"cmd": "legal"})

    def apply(self, action: dict[str, Any]) -> dict[str, Any]:
        return self.request({"cmd": "apply", "action": action})

    def project(self) -> dict[str, Any]:
        return self.request({"cmd": "project"})

    def close(self) -> None:
        if self.process.poll() is None:
            try:
                self.request({"cmd": "quit"})
            except Exception:  # noqa: BLE001 — closing must not mask a result
                self.process.kill()
        self.process.wait(timeout=30)

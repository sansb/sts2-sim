"""Run-history RNG counter prediction through the Rust port (#2827 item C1).

The legacy review path (a fight with neither floor saves nor a capture)
predicts the run-lifetime RNG counters entering a fight from the ``.run``
history. That prediction used to be ``replay_fight.predict`` plus the
``solve_fight.*_counter_entering`` accounting; it is now ``sts-sim
run-counters``. This module is only the adapter: it projects the parsed run
into an ``sts-sim-run-history-v1`` document and reads the answer back.

It imports neither ``combat_sim`` nor ``solve_fight`` (a test pins that), so it
survives the Python simulator's deletion (#2827 item F).
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import tempfile
from typing import Any

from relay_parser import RunSummary, parse_run

HISTORY_SCHEMA = "sts-sim-run-history-v1"
COUNTERS_SCHEMA = "sts-sim-run-counters-v1"

# The deck-row fields the accounting reads (`solve_fight` read `id`,
# `upgrade_level`, `enchantment` and `upgrade_ambiguous`).
_DECK_KEYS = ("id", "upgrade_level", "enchantment")


class RunCountersRefusal(NotImplementedError):
    """``sts-sim run-counters`` refused, by name."""

    def __init__(self, code: str, detail: str):
        super().__init__(detail)
        self.code = code
        self.detail = detail


def _deck_row(row: dict) -> dict:
    projected = {key: row.get(key) for key in _DECK_KEYS}
    projected["upgrade_ambiguous"] = bool(row.get("upgrade_ambiguous"))
    return projected


def history_document(raw_run: dict, run: RunSummary) -> dict:
    """The ``sts-sim-run-history-v1`` document for a parsed run."""
    return {
        "schema": HISTORY_SCHEMA,
        "run": raw_run,
        "fights": [
            {
                "node_index": fight.node_index,
                "encounter_id": fight.encounter_id,
                "monster_ids": list(fight.monster_ids),
                "turns_taken": fight.turns_taken,
                "relics_entering": list(fight.relics_entering),
                "potions_used": list(fight.potions_used),
                "deck_entering": [_deck_row(row) for row in fight.deck_entering],
            }
            for fight in run.fights
        ],
    }


def load_history(run_path: str | pathlib.Path) -> dict:
    """Read and parse a ``.run`` file into its history document."""
    path = pathlib.Path(run_path)
    raw = json.loads(path.read_text())
    return history_document(raw, parse_run(str(path)))


def predict(binary: str | pathlib.Path, history: dict, fight_index: int,
            build: str) -> dict[str, Any]:
    """Run ``sts-sim run-counters`` for one fight; refusals raise by name."""
    with tempfile.NamedTemporaryFile("w", suffix=".json") as handle:
        json.dump(history, handle)
        handle.flush()
        completed = subprocess.run(
            [str(binary), "run-counters", "--build", build,
             "--history", handle.name, "--fight", str(fight_index)],
            capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        detail = completed.stderr.strip()
        code = "run_counters_failed"
        prefix = "refusal: run-counters: "
        if detail.startswith(prefix):
            code = detail[len(prefix):].split(":", 1)[0].strip() or code
        raise RunCountersRefusal(code, detail)
    document = json.loads(completed.stdout)
    if document.get("schema") != COUNTERS_SCHEMA:
        raise RunCountersRefusal(
            "run_counters_schema",
            f"unexpected run-counters schema {document.get('schema')!r}")
    return document


def counter_values(document: dict) -> dict[str, int | None]:
    """Stream name -> predicted value (``None`` where not known)."""
    return {name: row["value"] for name, row in document["counters"].items()}

#!/usr/bin/env python3
"""Replay retained suite witnesses in fresh Rust processes; keep failures separate.

Rust engine only. Until #2999 the replay came from `search_experiment.py`'s
`verify`, which also carried an optional advisory Python lockstep and imported
the simulator to offer it. That tool's `prepare` mode was a Python-rooted
generator and retired with the simulator (#2827), so the Rust-only half of
`verify` lives here now, unchanged: this tool never asked for the Python
lockstep. On the committed search reports it produces the same summary as
before (measured in the #2999 walk).
"""
import argparse
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from canonical_document import differential_digest  # noqa: E402
from diff_serve_client import DiffServe  # noqa: E402
from search_suite import RUST, entry_path  # noqa: E402


def verify(document, actions, binary):
    """Replay `actions` from `document` on the Rust engine, step by step."""
    rust = DiffServe(binary)
    trace = []
    try:
        response = rust.load(document)
        if "ok" not in response:
            raise RuntimeError(response)
        if response["ok"]["digest"] != differential_digest(document):
            raise RuntimeError("entry digest mismatch")
        current = document
        for i, action in enumerate(actions):
            if os.environ.get("SEARCH_REPLAY_PROGRESS"):
                print(f"Replay step {i}: {action}", file=sys.stderr, flush=True)
            cards = {c["uid"]: c for pile in current["piles"].values() for c in pile}
            card = cards.get(action.get("uid"))
            label = (card["id"] + ("+" if card.get("upgrade") else "")) if card else action["kind"]
            if action["kind"] == "select":
                answer = action["answer"]
                if answer["kind"] == "card_uid":
                    selected = cards[answer["uid"]]
                    label = "Select " + selected["id"] + ("+" if selected.get("upgrade") else "")
                else:
                    label = "Select option " + str(answer["index"])
            response = rust.apply(action)
            if "ok" not in response:
                raise RuntimeError((i, action, response))
            projected = rust.project()
            if "ok" not in projected:
                raise RuntimeError(projected)
            after = projected["ok"]["state"]
            trace.append({"step": i, "turn": current["player"].get("turn", 1),
                          "action": action, "label": label,
                          "hp_before": current["player"].get("hp", 0), "hp_after": after["player"].get("hp", 0),
                          "enemy_hp_after": sum(max(0, m.get("hp", 0)) for m in after.get("monsters", [])),
                          "digest": response["ok"]["digest"]})
            current = after
        return {"rust_replayed": True, "python_lockstep": False,
                "trace": trace, "final_digest": differential_digest(current), "final_state": current}
    finally:
        rust.close()


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("reports", type=Path, nargs="+")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--binary", type=Path, default=RUST / "target/release/sts-sim")
    a = p.parse_args()
    summary = []
    for path in a.reports:
        report = json.loads(path.read_text())
        fights = {fight["id"]: fight for fight in report["manifest"]["fights"]}
        checked = wins = unsupported = 0
        for row in report["runs"]:
            best = row.get("best")
            if best is None:
                unsupported += 1
                continue
            entry = json.loads(entry_path(fights[row["fight_id"]]).read_text())
            assert differential_digest(entry) == row["entry_digest"]
            replay = verify(entry, best["actions"], a.binary)
            assert replay["final_digest"] == best["final_digest"]
            assert replay["final_state"]["player"].get("hp", 0) == best["combat_hp"]
            checked += 1
            wins += int(best["won"])
        summary.append({"report": path.name, "witnesses_replayed": checked,
                        "wins": wins, "refused_or_failed": unsupported})
    a.out.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()

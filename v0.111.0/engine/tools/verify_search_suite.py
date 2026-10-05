#!/usr/bin/env python3
"""Replay retained suite witnesses in fresh Rust processes; keep failures separate.

A search report (`search_suite.py`'s output) keeps, for every search, the best
action line it found, the digest of the entry it started from and the digest of
the state the line ended in. This tool loads each entry into a fresh
`sts-sim diff-serve` process, applies the line one action at a time and holds
the result to those digests. It answers one question: does the engine in front
of you still reach the reported state by the reported line?

Every run gets its own verdict. A run that does not verify is listed with the
reason (see `FAILURE_KINDS`) and the tool goes on to the next one, then exits
1. Nothing is asserted away and nothing stops at the first failure.

With no report named, the tool checks the committed reports under
`eval/search`. Most of those are **historical**: they record a measurement
made on the engine of the day they were written, were never regenerated, and
no longer replay on a later engine. `eval/search/historical-reports.json`
names each one with the reason. A historical report is not replayed: the tool
checks that the file is still byte-identical to what was frozen, prints the
recorded reason and moves on. `--include-historical` replays it anyway, which
shows how far the engine has moved since.

Rust engine only. Until #2999 the replay came from `search_experiment.py`'s
`verify`, which also carried an optional advisory Python lockstep and imported
the simulator to offer it. That tool's `prepare` mode was a Python-rooted
generator and retired with the simulator (#2827), so the Rust-only half of
`verify` lives here now.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from canonical_document import differential_digest  # noqa: E402
from diff_serve_client import DiffServe  # noqa: E402
from search_suite import RUST, entry_path  # noqa: E402

SEARCH = RUST.parent / "eval" / "search"
REGISTRY = SEARCH / "historical-reports.json"
REGISTRY_SCHEMA = "search-historical-reports-v1"

#: Why a run did not verify, in the order the checks run.
FAILURE_KINDS = {
    "entry_missing": "the fight's entry document is not in the tree, or is "
                     "not the file the manifest froze",
    "entry_changed": "the entry document in the tree is not the one the "
                     "search started from (its digest differs)",
    "entry_refused": "the engine refuses to load the entry document",
    "entry_does_not_round_trip": "the engine loads the entry but projects a "
                                 "different root",
    "line_does_not_replay": "an action of the line is illegal or refused",
    "final_hp_differs": "the line replays but ends at a different HP",
    "final_digest_differs": "the line replays to the reported HP but the "
                            "final document differs",
}


class WitnessFailure(RuntimeError):
    """One run did not verify. `kind` is a key of `FAILURE_KINDS`."""

    def __init__(self, kind, detail):
        super().__init__(f"{kind}: {detail}")
        self.kind = kind
        self.detail = detail


def root_is_entry_plus_bookkeeping_record(rust, document):
    """An entry written before `player.session_bookkeeping` existed (#3660).

    Rust loads it without the record and projects the root with it. That one
    added key is the only difference accepted; an entry that carries a record
    must round-trip as it stands. The same rule as
    `rust_replay.RustReplay._root_is_the_entry_plus_its_bookkeeping_record`.
    """
    if "session_bookkeeping" in document.get("player", {}):
        return False
    projected = rust.project()
    if "ok" not in projected:
        return False
    root = projected["ok"]["state"]
    if root.get("player", {}).pop("session_bookkeeping", None) is None:
        return False
    return differential_digest(root) == differential_digest(document)


def verify(document, actions, binary):
    """Replay `actions` from `document` on the Rust engine, step by step."""
    rust = DiffServe(binary)
    trace = []
    try:
        response = rust.load(document)
        if "ok" not in response:
            raise WitnessFailure("entry_refused", json.dumps(response, sort_keys=True))
        if (response["ok"]["digest"] != differential_digest(document)
                and not root_is_entry_plus_bookkeeping_record(rust, document)):
            raise WitnessFailure("entry_does_not_round_trip", "entry digest mismatch")
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
                raise WitnessFailure(
                    "line_does_not_replay",
                    f"step {i} of {len(actions)} ({label}) {json.dumps(action, sort_keys=True)}: "
                    + json.dumps(response, sort_keys=True))
            projected = rust.project()
            if "ok" not in projected:
                raise WitnessFailure(
                    "line_does_not_replay",
                    f"step {i} of {len(actions)}: " + json.dumps(projected, sort_keys=True))
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


def check_run(row, fight, binary):
    """Hold one run with a witness to its report. Raises `WitnessFailure`."""
    best = row["best"]
    try:
        entry = json.loads(entry_path(fight).read_text())
    except (OSError, ValueError) as error:
        raise WitnessFailure("entry_missing", str(error)) from error
    if differential_digest(entry) != row["entry_digest"]:
        raise WitnessFailure(
            "entry_changed",
            f"the tree's entry digests to {differential_digest(entry)}, the report "
            f"started from {row['entry_digest']}")
    replay = verify(entry, best["actions"], binary)
    hp = replay["final_state"]["player"].get("hp", 0)
    if hp != best["combat_hp"]:
        raise WitnessFailure("final_hp_differs", f"ends at {hp} HP, the report says {best['combat_hp']}")
    if replay["final_digest"] != best["final_digest"]:
        raise WitnessFailure(
            "final_digest_differs",
            f"ends at {replay['final_digest']}, the report says {best['final_digest']}")


def check_report(path, binary):
    """One summary row for a report: every witness replayed, failures listed."""
    report = json.loads(path.read_text())
    fights = {fight["id"]: fight for fight in report["manifest"]["fights"]}
    checked = wins = unsupported = 0
    failures = []
    for index, row in enumerate(report["runs"]):
        best = row.get("best")
        if best is None:
            unsupported += 1
            continue
        try:
            check_run(row, fights[row["fight_id"]], binary)
        except WitnessFailure as failure:
            failures.append({"run": index, "fight_id": row["fight_id"], "mode": row.get("mode"),
                             "seed": row.get("seed"), "kind": failure.kind, "detail": failure.detail})
            continue
        checked += 1
        wins += int(best["won"])
    return {"report": path.name, "witnesses_replayed": checked, "wins": wins,
            "refused_or_failed": unsupported, "failed": len(failures), "failures": failures}


def load_registry(path):
    """`{report file name: record}` for the historical reports beside `path`."""
    if not path.exists():
        return {}
    registry = json.loads(path.read_text())
    if registry.get("schema") != REGISTRY_SCHEMA:
        raise SystemExit(f"{path}: schema is {registry.get('schema')!r}, not {REGISTRY_SCHEMA!r}")
    return registry["reports"]


def check_historical(path, record):
    """A historical report is not replayed; it must still be the frozen file."""
    row = {"report": path.name, "historical": True, "reason": record["reason"],
           "witnesses": record["witnesses"], "failed": 0, "failures": []}
    found = hashlib.sha256(path.read_bytes()).hexdigest() if path.exists() else None
    if found != record["sha256"]:
        row["failed"] = 1
        row["failures"] = [{
            "kind": "historical_report_changed",
            "detail": (f"sha256 is {found}, frozen as {record['sha256']}" if found else "the file is missing")
                      + "; a historical report is a record and is not edited or regenerated"}]
    return row


def is_report(path):
    """A committed JSON under `eval/search` that `search_suite.py` wrote."""
    try:
        document = json.loads(path.read_text())
    except ValueError:
        return False
    return isinstance(document, dict) and "runs" in document and "manifest" in document


def committed_reports(registry_path, registry):
    """Every registered historical report, then every other committed report."""
    directory = registry_path.parent
    paths = [directory / name for name in registry]
    paths += [path for path in sorted(directory.glob("*.json"))
              if path.name not in registry and path != registry_path and is_report(path)]
    return paths


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("reports", type=Path, nargs="*",
                   help="reports to check (default: every committed report under eval/search)")
    p.add_argument("--out", type=Path, help="also write the summary here")
    p.add_argument("--binary", type=Path, default=RUST / "target/release/sts-sim")
    p.add_argument("--registry", type=Path, default=REGISTRY,
                   help="the historical-report registry (default: eval/search/historical-reports.json)")
    p.add_argument("--include-historical", action="store_true",
                   help="replay historical reports too, instead of skipping them by their recorded reason")
    a = p.parse_args(argv)
    registry = load_registry(a.registry)
    directory = a.registry.resolve().parent
    reports = a.reports or committed_reports(a.registry, registry)
    if not a.binary.exists():
        raise SystemExit(f"{a.binary}: no engine binary; build it with "
                         "`cargo build --locked --release --bin sts-sim`, or pass --binary")
    summary = []
    for path in reports:
        record = registry.get(path.name) if path.resolve().parent == directory else None
        if record is not None and not a.include_historical:
            row = check_historical(path, record)
            if not row["failed"]:
                print(f"{path.name}: historical, not replayed ({record['reason']})", file=sys.stderr)
        else:
            row = check_report(path, a.binary)
            if record is not None:
                row["historical"] = True
        for failure in row["failures"]:
            where = "" if "run" not in failure else (
                f" run {failure['run']} ({failure['fight_id']} {failure['mode']} seed={failure['seed']})")
            print(f"{path.name}{where}: {failure['kind']}: {failure['detail']}", file=sys.stderr)
        summary.append(row)
    text = json.dumps(summary, indent=2) + "\n"
    if a.out:
        a.out.write_text(text)
    print(json.dumps([{key: value for key, value in row.items() if key not in ("failures", "reason")}
                      for row in summary]))
    failed = sum(row["failed"] for row in summary)
    if failed:
        print(f"{failed} failure(s); see the lines above"
              + (f" or {a.out}" if a.out else ""), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

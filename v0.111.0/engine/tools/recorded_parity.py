#!/usr/bin/env python3
"""Parity of the Rust recorded-line resolver with the eval fixtures (#3578, B1).

Every fixture under ``eval/fights/<id>/`` holds the fight's root
(``entry.canonical.json``) and the player's own line as the census resolved
it with the Python resolver (``human_line.json``: the wire actions, the
engine's digest after each, and the terminal). ``provenance.json`` names the
capture by sha256.

For each fixture whose capture is found under the given directories, run
``sts-sim recorded-line ENTRY CAPTURE`` and require the same actions, the
same step digests and the same terminal. A fixture whose capture is not on
this machine is counted and skipped: the captures are not in the repository.

Usage:
    recorded_parity.py [--binary PATH] CAPTURE_DIR [CAPTURE_DIR ...]
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve()
CRATE = HERE.parents[1]
FIGHTS = CRATE.parent / "eval" / "fights"


def captures_by_sha(directories) -> dict:
    found = {}
    for directory in directories:
        for path in pathlib.Path(directory).expanduser().rglob("*.mcr"):
            found.setdefault(hashlib.sha256(path.read_bytes()).hexdigest(), path)
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("capture_dirs", nargs="+")
    parser.add_argument("--binary", default=str(CRATE / "target" / "release" / "sts-sim"))
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    captures = captures_by_sha(args.capture_dirs)

    counts = collections.Counter()
    failures = []
    for fight in sorted(FIGHTS.iterdir()):
        line_path = fight / "human_line.json"
        if not line_path.exists():
            counts["fixture has no human line"] += 1
            continue
        provenance = json.loads((fight / "provenance.json").read_text())
        capture = captures.get(provenance.get("capture_sha256"))
        if capture is None:
            counts["capture not on this machine"] += 1
            continue
        expected = json.loads(line_path.read_text())
        out = subprocess.run(
            [args.binary, "recorded-line", str(fight / "entry.canonical.json"), str(capture)],
            capture_output=True, check=False)
        if out.returncode != 0:
            failures.append(f"{fight.name}: sts-sim exited {out.returncode}: {out.stderr.decode()[:160]}")
            continue
        answer = json.loads(out.stdout)
        if "refusal" in answer:
            counts[f"refused: {answer['refusal']['check']}"] += 1
            failures.append(f"{fight.name}: {answer['refusal']}")
            continue
        got = answer["ok"]
        wrong = [key for key in ("actions", "step_digests", "terminal") if got[key] != expected[key]]
        if wrong:
            counts["differs"] += 1
            failures.append(f"{fight.name}: differs in {', '.join(wrong)}")
            continue
        counts["identical"] += 1

    for label, count in sorted(counts.items()):
        print(f"{label}: {count}")
    for failure in failures[: (len(failures) if args.verbose else 25)]:
        print("FAIL:", str(failure)[:300])
    if failures:
        print(f"{len(failures)} failure(s)")
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())

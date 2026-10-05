#!/usr/bin/env python3
"""Parity of the Rust `.mcr` decoder with the Python one (#3578, slice A).

For every replay file given, decode it with ``python/mcr_parser.py`` and with
``sts-sim mcr-decode`` and require the same document. The Rust decoder carries
one build's tables, so:

* a file the Python decoder reads as the tables' build must decode to the
  SAME document in Rust (compared as parsed JSON: 64-bit integers exactly,
  floats as the doubles both sides print);
* a file from any other build must be refused by Rust as
  ``unsupported_replay_build``;
* a file the Python decoder itself rejects must be refused by Rust too.

``--write-pins`` rewrites ``fixtures/mcr_decode_pins_v1.json``: the sha256 of
each committed v0.111.0 fixture's decoded document as the Rust decoder prints
it, taken only after the two decoders agreed on that file. The cargo test
``tests/mcr_decode.rs`` checks those digests without Python.

Usage:
    mcr_parity.py [--binary PATH] [--write-pins] [PATH ...]

A PATH is a ``.mcr`` file or a directory searched recursively. With no PATH,
the committed fixtures under ``python/testdata`` are checked.
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
PYTHON = CRATE.parent / "python"
PINS = CRATE / "fixtures" / "mcr_decode_pins_v1.json"
TABLES = json.loads((CRATE / "data" / "mcr_tables.v0.111.0.json").read_text())

sys.path.insert(0, str(PYTHON))
import mcr_parser  # noqa: E402


def canonical(document) -> str:
    """One spelling of a decoded document: sorted keys, no spaces."""
    return json.dumps(document, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def rust_decode(binary: pathlib.Path, path: pathlib.Path):
    out = subprocess.run([str(binary), "mcr-decode", str(path)], capture_output=True, check=False)
    if out.returncode != 0:
        raise RuntimeError(f"{path}: sts-sim exited {out.returncode}: {out.stderr.decode()[:200]}")
    return json.loads(out.stdout), out.stdout.rstrip(b"\n")


def python_decode(path: pathlib.Path):
    """(document, None) or (None, why)."""
    try:
        return mcr_parser.decode(path), None
    except Exception as error:  # the decoder raises several types
        return None, f"{type(error).__name__}: {error}"


def files(paths):
    for raw in paths:
        path = pathlib.Path(raw).expanduser()
        if path.is_dir():
            yield from sorted(path.rglob("*.mcr"))
        else:
            yield path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("paths", nargs="*")
    parser.add_argument("--binary", default=str(CRATE / "target" / "release" / "sts-sim"))
    parser.add_argument("--write-pins", action="store_true")
    args = parser.parse_args()
    binary = pathlib.Path(args.binary)
    targets = list(files(args.paths or [PYTHON / "testdata"]))
    if not targets:
        print("no .mcr files found")
        return 1

    counts = collections.Counter()
    failures = []
    pins = {}
    for path in targets:
        expected, why = python_decode(path)
        actual, printed = rust_decode(binary, path)
        refused = actual.get("refusal") if set(actual) == {"refusal"} else None
        if expected is None:
            counts["python rejects"] += 1
            if refused is None:
                failures.append(f"{path}: Python rejects ({why}) but Rust decoded it")
            continue
        if expected["version"] != TABLES["game_version"] or expected["model_id_hash"] != TABLES["model_id_hash"]:
            counts[f"other build ({expected['version']})"] += 1
            if not refused or refused["code"] != "unsupported_replay_build":
                failures.append(f"{path}: {expected['version']} should be unsupported_replay_build, got {str(actual)[:120]}")
            continue
        counts["same build"] += 1
        if refused is not None:
            failures.append(f"{path}: Rust refused a file Python decodes: {refused}")
            continue
        # Round-trip Python's document through JSON so both sides are compared
        # as what a consumer of the JSON would read.
        if json.loads(canonical(expected)) != actual:
            failures.append(f"{path}: decoded documents differ")
            continue
        counts["identical"] += 1
        if path.parent == PYTHON / "testdata":
            # The pin is over what the Rust decoder PRINTS (serde_json: sorted
            # keys, no spaces), so the cargo test needs no second formatter.
            pins[path.name] = hashlib.sha256(printed).hexdigest()

    for label, count in sorted(counts.items()):
        print(f"{label}: {count}")
    for failure in failures[:20]:
        print("FAIL:", failure)
    if failures:
        print(f"{len(failures)} failure(s)")
        return 1
    if args.write_pins:
        PINS.write_text(json.dumps({
            "schema": "mcr-decode-pins-v1",
            "note": "sha256 of each fixture's decoded document as sts-sim mcr-decode prints it (sorted keys, "
                    "no spaces). Written by tools/mcr_parity.py --write-pins after the Rust and Python decoders agreed.",
            "game_version": TABLES["game_version"],
            "fixtures": dict(sorted(pins.items())),
        }, indent=1) + "\n")
        print(f"wrote {len(pins)} pins to {PINS.relative_to(CRATE)}")
    print(f"ok: {len(targets)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())

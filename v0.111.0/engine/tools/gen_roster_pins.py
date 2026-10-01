#!/usr/bin/env python3
"""Relabel the frozen E4b roster pins from the eval manifest (stdlib only).

Issues: #2532 (F2, elite) and #2531 (F1, weak) under #2529/#2528 made the
pins; #2999 (under #2827) froze them.

What the pins are
-----------------
`fixtures/<pool>_rosters_v1.json` holds one case per fight whose encounter
dispatches to one of the pool's registered keys: the fight's identity, the
Niche and Encounter stream state it created monsters from, and the roster the
Python simulator's `combat_sim.make_monsters` built. `encounters::oracle`
replays every case through the Rust builders. The roster half is a **frozen
Python-oracle measurement**. Until #2999 this tool re-derived it from the local
capture corpus through `make_monsters`. #2827 item F deletes that simulator,
so the roster half can no longer be regenerated. The generator mode went with
it, and the committed cases are now frozen data. The last generator is in git
history before #2999.

What this tool still does
-------------------------
One pin input lives in git rather than in the corpus: each case's `fixture`
label, the eval-manifest id of the same `(seed, node)`. The manifest is
re-seeded far more often than the corpus was ever re-derived (six commits,
2026-09-14 to 2026-09-17, and #2919 is another).
`encounters::oracle::every_fixture_label_is_the_eval_manifests` re-derives the
labels from the committed `eval/manifest.json`, so a manifest change fails the
`rust port` lane until the pins are relabelled. `--relabel` is that
relabelling, and it needs no simulator:

* every non-synthetic case's `fixture` becomes the manifest's id at its
  `(seed, node_index)`, or null where the manifest has no fixture there;
* `fixture_fights` and each `per_encounter[...]["fixtures"]` are recounted;
* `unpinned_fixtures` lists the manifest fixtures that dispatch to the pool
  but are **not** a pinned case, sorted by id. This key is written only when
  the list is non-empty, so on today's manifest the committed bytes are
  unchanged. These are fixtures a re-seed added after the freeze. No Python
  roster exists for them. The `.mcr` certification census
  (`eval_suite.py census`) covers them against the game's own checksums
  instead. The crate test requires the list to be exact in both directions,
  so a re-seed still cannot silently change a pool's membership.

Everything else is left alone, byte for byte: the cases, their rosters and
streams, `corpus_fights`, `explicit_fixture_fights` (how many cases were
added from a fixture's own provenance at the last Python generation) and the
synthetic roots. A synthetic root whose `(seed, node)` becomes a manifest
fixture refuses by name, as the generator did.

Membership uses the crate test's rule: the manifest entry's `encounter`, matched
by substring against the pin file's own `registered_keys`, exactly one owner
or none (`oracle::pooled_key`).

Usage::

    python3 sim/v0.111.0/engine/tools/gen_roster_pins.py --relabel
    python3 sim/v0.111.0/engine/tools/gen_roster_pins.py --relabel --check

A PR that changes `eval/manifest.json` runs the first and commits the result.
`--check` exits non-zero naming each stale pool.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
BUILD_DIR = HERE.parents[2]
MANIFEST = BUILD_DIR / "eval" / "manifest.json"
TOOL = "sim/v0.111.0/engine/tools/gen_roster_pins.py"

#: Every committed pin file, which is `encounters::oracle::PINS`. The
#: controls check this against the crate's list and against the fixture
#: directory, so a pool cannot be added on one side only.
POOLS = ("boss", "elite", "event", "normal_a", "normal_b", "normal_c",
         "scrolls_of_biting", "weak")


def fixture_path(pool: str, rust_dir: pathlib.Path = RUST_DIR) -> pathlib.Path:
    return rust_dir / "fixtures" / f"{pool}_rosters_v1.json"


def dispatch(wire_id: str, keys: list[str]) -> str | None:
    """Which registered key a wire id dispatches to (`oracle::pooled_key`).

    Substring, not equality: `ENCOUNTER.DECIMILLIPEDE_ELITE` is built by the
    builder registered under `DECIMILLIPEDE`. Ambiguity raises rather than
    picking.
    """
    owners = [key for key in keys if key in wire_id]
    if not owners:
        return None
    if len(owners) > 1:
        raise SystemExit(f"{TOOL}: {wire_id} is claimed by {owners}")
    return owners[0]


def fixture_ids(manifest: dict) -> dict[tuple[str, int], str]:
    """`(seed, node) -> fixture id`, refusing two fixtures at one fight."""
    labels: dict[tuple[str, int], str] = {}
    for fight in manifest.get("fights", []):
        at = (fight["seed"], fight["node"])
        if at in labels:
            raise SystemExit(f"{TOOL}: eval manifest has two fixtures at "
                             f"{at}: {labels[at]} and {fight['id']}")
        labels[at] = fight["id"]
    return labels


def relabel(pins: dict, manifest: dict, pool: str) -> dict:
    """The pin document with its labels re-derived from `manifest`."""
    labels = fixture_ids(manifest)
    keys = pins["registered_keys"]
    out = json.loads(json.dumps(pins))          # a deep copy
    recorded = set()
    fixtures_per_encounter: dict[str, int] = {}
    for case in out["cases"]:
        at = (case["seed"], case["node_index"])
        if case.get("synthetic"):
            if at in labels:
                raise SystemExit(
                    f"{TOOL}: {pool}: synthetic root {at} is eval fixture "
                    f"{labels[at]}")
            continue
        if case["node_index"] is None:
            raise SystemExit(f"{TOOL}: {pool}/{case['encounter']} "
                             f"({case['seed']}): a pinned case has no node")
        case["fixture"] = labels.get(at)
        recorded.add(at)
        if case["fixture"] is not None:
            fixtures_per_encounter[case["encounter"]] = (
                fixtures_per_encounter.get(case["encounter"], 0) + 1)
    out["fixture_fights"] = sum(fixtures_per_encounter.values())
    for wire, counts in out["per_encounter"].items():
        counts["fixtures"] = fixtures_per_encounter.get(wire, 0)
    unpinned = sorted(
        fight["id"] for fight in manifest.get("fights", [])
        if fight.get("encounter")
        and dispatch(fight["encounter"], keys) is not None
        and (fight["seed"], fight["node"]) not in recorded)
    out.pop("unpinned_fixtures", None)
    if unpinned:
        out["unpinned_fixtures"] = unpinned
    return out


def render(pins: dict) -> str:
    """The generator's byte layout, unchanged since the pins were made."""
    return json.dumps(pins, indent=1, sort_keys=True) + "\n"


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(
        description=__doc__.splitlines()[0])
    ap.add_argument("--relabel", action="store_true", required=True,
                    help="re-derive every pool's fixture labels from the "
                         "committed eval manifest")
    ap.add_argument("--check", action="store_true",
                    help="compare with the committed pins instead of "
                         "writing them")
    ap.add_argument("--manifest", type=pathlib.Path, default=MANIFEST,
                    help=argparse.SUPPRESS)
    ap.add_argument("--rust-dir", type=pathlib.Path, default=RUST_DIR,
                    help=argparse.SUPPRESS)
    args = ap.parse_args(argv)
    manifest = json.loads(args.manifest.read_text())
    stale = []
    for pool in POOLS:
        path = fixture_path(pool, args.rust_dir)
        text = path.read_text()
        new = render(relabel(json.loads(text), manifest, pool))
        if new == text:
            print(f"{path.name}: fresh")
            continue
        if args.check:
            stale.append(pool)
            print(f"{path.name}: STALE", file=sys.stderr)
            continue
        path.write_text(new)
        doc = json.loads(new)
        print(f"relabelled {path.name}: {doc['fixture_fights']} labelled "
              f"cases, {len(doc.get('unpinned_fixtures', []))} unpinned "
              "manifest fixtures")
    if stale:
        print(f"{TOOL}: stale pools {stale}; run `--relabel` without "
              "`--check` and commit the result", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

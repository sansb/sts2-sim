#!/usr/bin/env python3
"""Score a draw-blind play policy over the certified eval fights.

Floor-level Coach results rest on a play policy; this measures the policy
itself across the standing fixture set (the build's eval/ tree, #1283) so a
policy change is judged on ~175 real fights, never on the one floor that
prompted it. Every sample is a fresh shuffle and fresh game RNG of the
captured root with the captured opening hand (`COACH_RESAMPLE=1`); the
reshuffle depends only on the sample index, so two policies are compared on
the same shuffles and the comparison is paired per fight.

    policy_eval.py run --binary coach_lookahead --env COACH_HORIZON=2 \\
        --samples 20 --out two.json
    policy_eval.py compare baseline.json one.json two.json

Potions are never used by these policies; fights the human won with potions
are therefore a harder baseline than the table's own no-potion setup.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import subprocess
import tempfile
import time

RUST = Path(__file__).resolve().parents[1]
EVAL = RUST.parent / "eval"
BIN = RUST / "target/release/examples"


def fixtures(category):
    manifest = json.loads((EVAL / "manifest.json").read_text())
    return [f for f in manifest["fights"] if category in f["categories"]]


def kind(encounter):
    encounter = encounter or ""
    return ("boss" if encounter.endswith("_BOSS")
            else "elite" if encounter.endswith("_ELITE") else "hallway")


def add_card(entry, card):
    """Put one more physical card in the draw pile (the reshuffle places it)."""
    player = entry["player"]
    entry["piles"]["draw"].append({"id": card, "upgrade": 0, "uid": player["next_card_uid"]})
    player["next_card_uid"] += 1


def run_one(fixture, binary, samples, env, extra_card=None):
    root_dir = EVAL / "fights" / fixture["id"]
    entry = json.loads((root_dir / "entry.canonical.json").read_text())
    if extra_card:
        add_card(entry, extra_card)
    human = json.loads((root_dir / "human_line.json").read_text())["terminal"]
    row = {"id": fixture["id"], "encounter": fixture["encounter"],
           "kind": kind(fixture["encounter"]), "character": fixture["character"],
           "categories": fixture["categories"],
           "human": {"hp_lost": entry["player"]["hp"] - max(human["hp"], 0),
                     "won": bool(human.get("won"))}}
    with tempfile.TemporaryDirectory(prefix="policy-eval-") as tmp:
        path = Path(tmp) / "roots.json"
        path.write_text(json.dumps([entry] * samples))
        started = time.time()
        result = subprocess.run(
            ["taskpolicy", "-c", "background", str(BIN / binary), str(path), "0"],
            capture_output=True, text=True, env={**os.environ, **env,
                                                 "COACH_RESAMPLE": "1"})
    row["seconds"] = round(time.time() - started, 2)
    if result.returncode:
        row["error"] = result.stderr.strip()[:300]
        return row
    trials = json.loads(result.stdout)["trials"]
    row.update(n=len(trials), cutoffs=sum(t["cutoff"] for t in trials),
               won=[bool(t["won"]) for t in trials],
               hp_lost=[t["hp_lost"] for t in trials],
               turns=[t["turn"] for t in trials])
    return row


def run(args):
    env = dict(item.split("=", 1) for item in args.env)
    env.setdefault("COACH_THREADS", str(args.threads))
    chosen = fixtures(args.category)
    if args.limit:
        chosen = chosen[:args.limit]
    started = time.time()
    with ThreadPoolExecutor(args.jobs) as pool:
        rows = []
        for row in pool.map(lambda f: run_one(f, args.binary, args.samples, env,
                                              args.add_card), chosen):
            rows.append(row)
            state = row.get("error", "")[:80] or (
                f"win {sum(row['won'])}/{row['n']} hp_lost "
                f"{sum(row['hp_lost']) / row['n']:.1f}")
            print(f"{len(rows):3d}/{len(chosen)} {row['id']} {row['kind']:7s} "
                  f"{row['seconds']:6.1f}s {state}", flush=True)
    out = {"schema": "policy-eval-v1", "binary": args.binary, "env": env,
           "add_card": args.add_card,
           "samples": args.samples, "category": args.category,
           "wall_seconds": round(time.time() - started, 1), "fights": rows}
    Path(args.out).write_text(json.dumps(out, indent=1) + "\n")
    print(args.out)


def mean(xs):
    return sum(xs) / len(xs) if xs else float("nan")


def ci95(xs):
    if len(xs) < 2:
        return float("nan")
    m = mean(xs)
    return 1.96 * math.sqrt(sum((x - m) ** 2 for x in xs) / (len(xs) - 1) / len(xs))


def label(result):
    env = {k: v for k, v in result["env"].items() if k != "COACH_THREADS"}
    if result.get("add_card"):
        env["+card"] = result["add_card"]
    return result["binary"].removeprefix("coach_") + (
        "(" + ",".join(f"{k.removeprefix('COACH_').lower()}={v}"
                       for k, v in sorted(env.items())) + ")" if env else "")


def compare(args):
    results = [json.loads(Path(p).read_text()) for p in args.results]
    names = [label(r) for r in results]
    by_id = [{f["id"]: f for f in r["fights"] if "error" not in f} for r in results]
    common = sorted(set.intersection(*(set(b) for b in by_id)))
    errors = {n: sum(1 for f in r["fights"] if "error" in f) for n, r in zip(names, results)}
    print(f"fights measured by every policy: {common.__len__()}  "
          f"(refused per policy: {errors})")
    print(f"wall time: " + ", ".join(f"{n} {r['wall_seconds']:.0f}s"
                                      for n, r in zip(names, results)))
    groups = [("all", lambda f: True), ("hallway", lambda f: f["kind"] == "hallway"),
              ("elite", lambda f: f["kind"] == "elite"), ("boss", lambda f: f["kind"] == "boss"),
              ("human A9/A10", lambda f: "human" in f["categories"])]
    for title, keep in groups:
        ids = [i for i in common if keep(by_id[0][i])]
        if not ids:
            continue
        print(f"\n{title} ({len(ids)} fights)")
        for name, fights in zip(names, by_id):
            win = mean([mean(fights[i]["won"]) for i in ids])
            hp = mean([mean(fights[i]["hp_lost"]) for i in ids])
            print(f"  {name:44s} win {win * 100:5.1f}%   hp_lost {hp:5.1f}")
        if title.startswith("human"):
            hwin = mean([by_id[0][i]["human"]["won"] for i in ids])
            hhp = mean([by_id[0][i]["human"]["hp_lost"] for i in ids])
            print(f"  {'human (with potions)':44s} win {hwin * 100:5.1f}%   hp_lost {hhp:5.1f}")
        # Paired per fight: each later policy against the first.
        for name, fights in list(zip(names, by_id))[1:]:
            dw = [mean(fights[i]["won"]) - mean(by_id[0][i]["won"]) for i in ids]
            dh = [mean(fights[i]["hp_lost"]) - mean(by_id[0][i]["hp_lost"]) for i in ids]
            print(f"    {name} vs {names[0]}: win {mean(dw) * 100:+5.1f} ±{ci95(dw) * 100:.1f}pt, "
                  f"hp_lost {mean(dh):+5.1f} ±{ci95(dh):.1f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run")
    r.add_argument("--binary", required=True, choices=["coach_policy", "coach_lookahead"])
    r.add_argument("--env", nargs="*", default=[], help="KEY=VALUE for the policy binary")
    r.add_argument("--samples", type=int, default=20)
    r.add_argument("--category", default="certified")
    r.add_argument("--limit", type=int, default=0)
    r.add_argument("--add-card", default=None,
                   help="one more copy of this card id in every deck (e.g. PYRE)")
    r.add_argument("--jobs", type=int, default=4, help="fixtures in flight at once")
    r.add_argument("--threads", type=int, default=4, help="COACH_THREADS per fixture")
    r.add_argument("--out", required=True)
    c = sub.add_parser("compare")
    c.add_argument("results", nargs="+")
    args = parser.parse_args()
    run(args) if args.command == "run" else compare(args)


if __name__ == "__main__":
    main()

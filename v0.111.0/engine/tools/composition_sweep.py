#!/usr/bin/env python3
"""Run, aggregate and compare the composition sweep (#3877).

The sweep builds synthetic fight roots (a starter deck plus one or two things
under test), takes each through the crate's public path and plays seeded random
legal lines, writing one JSON line per root. The driver and the line shape are
`examples/composition_sweep/` (`main.rs`, `sweep.rs`). It is a measurement
tool: run it after a batch of merges and compare against the last output. It
is not a gate, and "ok" means "nothing was met", never "certified".

    composition_sweep.py run --mode single --out target/sweep/single
    composition_sweep.py run --mode pairs --match FETCH --out target/sweep/fetch
    composition_sweep.py run --mode pairs --out target/sweep/pairs --low-priority
    composition_sweep.py run --mode chains --character IRONCLAD --rollouts 200 \
        --park-round-trips --out target/sweep/chains
    composition_sweep.py aggregate target/sweep/single target/sweep/pairs
    composition_sweep.py compare OLD NEW        (also: --compare OLD NEW)

`run` writes one file per character and shard into `--out`. The full pairs
mode is about three million roots; it drops the `ok` lines by default (each
file ends with `summary` lines that carry the counts) and wants
`--low-priority` on a shared machine. A slice keeps them, which is what lets
`compare` restrict the other side to the roots the slice ran.

`aggregate` and `compare` take files or directories of `*.jsonl`, several
separated by commas. A class is a stage plus its refusal with instance
numbers removed. For a pair or an enchanted card, a finding that one member
already raises alone is subtracted when the single-dimension rows are among
the inputs, so the pair tables show what the composition adds.
"""
import argparse
import collections
import glob
import json
import os
from pathlib import Path
import re
import subprocess
import sys

CRATE = Path(__file__).resolve().parents[1]
EXAMPLE = "composition_sweep"
CHARACTERS = ("IRONCLAD", "SILENT", "DEFECT", "NECROBINDER", "REGENT")
# The member kinds of each composed dimension: card, relic, potion, encounter.
# An enchanted card's only member with a single-dimension row is the card.
MEMBERS = {
    "pair_rr": "rr", "pair_rr_rev": "rr", "pair_cc": "cc", "pair_cr": "cr",
    "pair_pc": "pc", "pair_pr": "pr", "pair_pp": "pp", "pair_ec": "ec",
    "pair_er": "er", "pair_ep": "ep", "ench": "-c",
}
SINGLES = {"card": "c", "relic": "r", "potion": "p", "encounter": "e"}
# Opening classes whose text after the class is the engine's own refusal name.
NAMED_OPENING = ("engine_not_implemented", "boundary_unrepresentable")


def normalize(stage, detail):
    """A refusal with the per-instance parts removed."""
    detail = re.sub(r"card#\d+", "card#", detail)
    detail = re.sub(r"Card\(\d+\)", "Card()", detail)
    detail = re.sub(r"\buid \d+", "uid #", detail)
    detail = re.sub(r'mid-combat card identity "[A-Z0-9_]+"\+\d+',
                    "mid-combat card identity ...", detail)
    if stage in ("opening", "entry"):
        name, _, rest = detail.partition(":")
        if name in NAMED_OPENING:
            return f"{name}: {rest.strip()[:140]}"
        return name
    if stage == "panic":
        # `panicked at src/file.rs:LINE:COL:\nmessage`: keep file and message.
        detail = re.sub(r":\d+:\d+:", ":", detail).replace("\n", " ")
    return detail[:160]


def findings(row):
    """Every (stage, class) one root met, the first sweep's rows included."""
    out = []
    for part in [row] + row.get("more", []):
        stage, detail = part["stage"], part.get("detail", "")
        if stage == "ok":
            continue
        if stage == "line" and detail.startswith("legal: "):
            stage, detail = "stuck", detail[len("legal: "):]
        if stage == "admission":
            out.extend(("admission", normalize(stage, piece))
                       for piece in detail.split(" ;; "))
        else:
            out.append((stage, normalize(stage, detail)))
    return out


def paths_of(spec):
    out = []
    for part in spec.split(","):
        path = Path(part)
        if path.is_dir():
            out.extend(sorted(path.glob("*.jsonl")))
        elif any(ch in part for ch in "*?["):
            out.extend(Path(match) for match in sorted(glob.glob(part)))
        else:
            out.append(path)
    if not out:
        raise SystemExit(f"no sweep output at {spec!r}")
    return out


def rows_of(paths):
    for path in paths:
        with open(path) as handle:
            for line in handle:
                try:
                    yield json.loads(line)
                except json.JSONDecodeError:
                    continue  # a line cut off by an interrupted run


def members_of(row):
    kinds = MEMBERS.get(row["dim"])
    if kinds is None:
        return []
    first, second = row["item"].split("@" if row["dim"] == "ench" else " & ")
    return [(kind, name) for kind, name in zip(kinds, (first, second)) if kind != "-"]


class Sweep:
    """One side's output: classes, coverage, and the roots a slice ran."""

    def __init__(self, spec, examples=4, keep=None):
        paths = paths_of(spec)
        self.examples = examples
        self.alone = collections.defaultdict(set)   # (ch, kind, name) -> findings
        self.coverage = collections.Counter()       # (ch, dim) -> roots
        self.stages = collections.Counter()         # (dim, stage) -> roots
        self.classes = {}
        self.keys = collections.defaultdict(set)    # (ch, dim) -> items, when known
        counted = collections.Counter()
        summarized = collections.Counter()
        for row in rows_of(paths):
            if row.get("summary"):
                summarized[(row["ch"], row["dim"])] += row["roots"]
                for stage, count in row["stages"].items():
                    if stage == "ok":
                        self.stages[(row["dim"], "ok")] += count
                continue
            cell = (row["ch"], row["dim"])
            counted[cell] += 1
            kind = SINGLES.get(row["dim"])
            item = row["item"]
            if kind == "c":
                kind, item = ("c", item[:-2]) if item.endswith("+0") else (None, item)
            if kind:
                self.alone[(row["ch"], kind, item)].update(findings(row))
        for cell, count in counted.items():
            self.coverage[cell] = max(count, summarized.get(cell, 0))
        for cell, count in summarized.items():
            self.coverage.setdefault(cell, count)
        # A cell with a line for every root it ran (its `ok` lines were kept,
        # or it had none) names its roots.
        self.complete = {cell for cell, count in counted.items()
                         if summarized.get(cell, count) == count}
        self._second_pass(paths, keep, set(summarized))

    def _second_pass(self, paths, keep, summarized):
        for row in rows_of(paths):
            if row.get("summary"):
                continue
            cell = (row["ch"], row["dim"])
            if cell in self.complete:
                self.keys[cell].add(row["item"])
            if keep is not None and not keep(cell, row["item"]):
                continue
            if row["stage"] == "ok":
                if cell not in summarized:
                    self.stages[(row["dim"], "ok")] += 1
                continue
            self.stages[(row["dim"], row["stage"])] += 1
            known = set()
            for kind, name in members_of(row):
                known |= self.alone.get((row["ch"], kind, name), set())
            for finding in findings(row):
                if finding in known:
                    continue
                entry = self.classes.setdefault(finding, {
                    "roots": 0, "dims": collections.Counter(), "chars": set(), "examples": [],
                    "action": None,
                })
                entry["roots"] += 1
                entry["dims"][row["dim"]] += 1
                entry["chars"].add(row["ch"][0])
                label = row["item"] if row["dim"] in MEMBERS else f"{row['item']} ({row['ch'].title()})"
                if len(entry["examples"]) < self.examples and label not in entry["examples"]:
                    entry["examples"].append(label)
                if entry["action"] is None and row.get("action"):
                    entry["action"] = row["action"]

    def table(self):
        order = ["panic", "crash", "timeout", "stuck", "roundtrip", "line",
                 "admission", "boundary", "opening", "entry"]
        rank = {stage: index for index, stage in enumerate(order)}
        rows = sorted(self.classes.items(),
                      key=lambda item: (rank.get(item[0][0], 99), -item[1]["roots"], item[0][1]))
        lines = ["| stage | class | roots | dimensions | characters | examples |",
                 "|---|---|---|---|---|---|"]
        for (stage, name), entry in rows:
            dims = ", ".join(f"{dim} {count}" for dim, count in sorted(entry["dims"].items()))
            examples = "; ".join(entry["examples"])
            if entry["action"]:
                examples += f" (at `{entry['action']}`)"
            lines.append(f"| {stage} | {name} | {entry['roots']} | {dims} | "
                         f"{''.join(sorted(entry['chars']))} | {examples} |")
        return "\n".join(lines)

    def totals(self):
        dims = sorted({dim for dim, _ in self.stages})
        stages = sorted({stage for _, stage in self.stages})
        lines = ["| dimension | roots | " + " | ".join(stages) + " |",
                 "|---|---|" + "---|" * len(stages)]
        for dim in dims:
            roots = sum(count for (cell_dim, _), count in self.stages.items() if cell_dim == dim)
            cells = " | ".join(str(self.stages.get((dim, stage), 0)) for stage in stages)
            lines.append(f"| {dim} | {roots} | {cells} |")
        return "\n".join(lines)


def aggregate(args):
    sweep = Sweep(args.paths, examples=args.examples)
    print(sweep.totals())
    print()
    print(sweep.table())
    if args.json:
        payload = [{"stage": stage, "class": name, "roots": entry["roots"],
                    "dims": dict(entry["dims"]), "characters": "".join(sorted(entry["chars"])),
                    "examples": entry["examples"], "action": entry["action"]}
                   for (stage, name), entry in sorted(sweep.classes.items())]
        Path(args.json).write_text(json.dumps(payload, indent=1) + "\n")


def compare(old_spec, new_spec, examples=4):
    """Classes that appeared or disappeared between two sweeps.

    Only the (character, dimension) cells both sides ran are compared. Where
    one side ran fewer roots of a cell and kept its `ok` lines, the other side
    is cut down to those roots, so a slice compares cleanly against a full
    sweep. A cell that differs in size with no way to tell which roots ran is
    compared whole and named, because a class can then vanish by not being run.
    """
    old, new = Sweep(old_spec, examples), Sweep(new_spec, examples)
    cells = sorted(set(old.coverage) & set(new.coverage))
    restrict, unknown = {}, []
    for cell in cells:
        if old.coverage[cell] == new.coverage[cell]:
            continue
        small = old if old.coverage[cell] < new.coverage[cell] else new
        if cell in small.complete:
            restrict[cell] = small.keys[cell]
        else:
            unknown.append(cell)
    shared = set(cells)

    def keep(cell, item):
        return cell in shared and (cell not in restrict or item in restrict[cell])

    old, new = Sweep(old_spec, examples, keep), Sweep(new_spec, examples, keep)
    roots = sum(min(old.coverage[cell], new.coverage[cell]) for cell in cells)
    print(f"compared {len(cells)} (character, dimension) cells, about {roots} roots; "
          f"{len(restrict)} cut down to the smaller side's roots")
    for cell in unknown:
        print(f"  coverage differs and the smaller side kept no ok lines: {cell[0]} {cell[1]} "
              f"(old {old.coverage[cell]}, new {new.coverage[cell]})")
    appeared = sorted(set(new.classes) - set(old.classes))
    disappeared = sorted(set(old.classes) - set(new.classes))
    for title, names, side in (("appeared", appeared, new), ("disappeared", disappeared, old)):
        print(f"\n## {title}: {len(names)}")
        for stage, name in names:
            entry = side.classes[(stage, name)]
            print(f"- {stage}: {name} ({entry['roots']} roots; {'; '.join(entry['examples'])})")
    print("\n## changed counts")
    for key in sorted(set(old.classes) & set(new.classes)):
        before, after = old.classes[key]["roots"], new.classes[key]["roots"]
        if before != after:
            print(f"- {key[0]}: {key[1]}: {before} -> {after}")
    return appeared, disappeared


def binary(args):
    if args.binary:
        return Path(args.binary)
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "--locked", "--example", EXAMPLE],
                       cwd=CRATE, check=True)
    return CRATE / "target/release/examples" / EXAMPLE


def run(args):
    program = binary(args)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    sliced = bool(args.match or args.only or args.limit is not None)
    omit_ok = args.omit_ok if args.omit_ok is not None else (args.mode == "pairs" and not sliced)
    characters = CHARACTERS if args.character == "all" else (args.character.upper(),)
    base = [str(program), "--mode", args.mode, "--rollouts", str(args.rollouts),
            "--depth", str(args.depth), "--round-trips", str(args.round_trips),
            "--root-timeout", str(args.root_timeout)]
    if args.seed is not None:
        base += ["--seed", str(args.seed)]
    if args.park_round_trips:
        base.append("--park-round-trips")
    for text in args.match:
        base += ["--match", text]
    if args.only:
        base += ["--only", args.only]
    if args.limit is not None:
        base += ["--limit", str(args.limit)]
    if omit_ok:
        base.append("--omit-ok")
    if args.low_priority and sys.platform == "darwin":
        base = ["taskpolicy", "-c", "background"] + base
    jobs = [(character, shard) for character in characters for shard in range(args.shards)]
    running, failed = [], []
    while jobs or running:
        while jobs and len(running) < args.jobs:
            character, shard = jobs.pop(0)
            target = out / f"{args.mode}_{character}_{shard}of{args.shards}.jsonl"
            command = base + ["--character", character, "--shard", f"{shard}/{args.shards}",
                              "--out", str(target)]
            running.append((subprocess.Popen(command, stderr=subprocess.DEVNULL), target))
        process, target = running.pop(0)
        if process.wait() != 0:
            failed.append(target)
    if failed:
        raise SystemExit(f"sweep workers failed: {', '.join(map(str, failed))}")
    sweep = Sweep(str(out))
    print(sweep.totals())
    print()
    print(sweep.table())


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--compare":
        compare(sys.argv[2], sys.argv[3])
        return
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    r = commands.add_parser("run", help="build the driver and sweep one mode")
    r.add_argument("--mode", required=True, choices=("single", "pairs", "enchant", "chains"))
    r.add_argument("--out", required=True, help="directory for the per-shard files")
    r.add_argument("--character", default="all")
    r.add_argument("--rollouts", type=int, default=3,
                   help="random lines per root (the first sweep: 12 single, 3 pairs)")
    r.add_argument("--depth", type=int, default=400, help="actions per line")
    r.add_argument("--seed", type=int)
    r.add_argument("--round-trips", type=int, default=1,
                   help="lines per root that reload one sampled state")
    r.add_argument("--park-round-trips", action="store_true",
                   help="also reload every state that has a choice pending")
    r.add_argument("--match", action="append", default=[],
                   help="keep roots whose item contains this text (repeatable)")
    r.add_argument("--only", help="comma-separated dimensions")
    r.add_argument("--limit", type=int)
    r.add_argument("--shards", type=int, default=1, help="files per character")
    r.add_argument("--jobs", type=int, default=max(1, min(5, (os.cpu_count() or 2) // 2)))
    r.add_argument("--root-timeout", type=int, default=120)
    r.add_argument("--omit-ok", dest="omit_ok", action="store_true", default=None)
    r.add_argument("--keep-ok", dest="omit_ok", action="store_false")
    r.add_argument("--low-priority", action="store_true")
    r.add_argument("--binary", help="a built driver, instead of building one")
    r.add_argument("--no-build", action="store_true")
    r.set_defaults(func=run)
    a = commands.add_parser("aggregate", help="distinct classes with counts and examples")
    a.add_argument("paths", help="files or directories, comma-separated")
    a.add_argument("--examples", type=int, default=4)
    a.add_argument("--json", help="also write the classes here")
    a.set_defaults(func=aggregate)
    c = commands.add_parser("compare", help="classes that appeared or disappeared")
    c.add_argument("old")
    c.add_argument("new")
    c.add_argument("--examples", type=int, default=4)
    c.set_defaults(func=lambda args: compare(args.old, args.new, args.examples))
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Encounter coverage census: the E4b frontier, stated as a measurement.

#2528 acceptance (4) and #2529's "coverage census" deliverable. One row per
`content_tables::ENCOUNTER_MATCH_ORDER` id (88 today), carrying:

* **demand** — how many corpus fights and how many checked-in eval fixtures
  use the encounter. Demand is the wave's ordering key, not its membership
  key: root `CLAUDE.md`'s "literal parity is the target — do NOT usage-weight
  the remaining port work" means a zero-corpus encounter still gets a family,
  it just goes last within it.
* **provenance** — which `content/encounters/*.py` module and which builder
  function define it in the Python oracle, with the source line span a porter
  reads.
* **shape** — `tabular` (a straight-line roster the DLL content manifest can
  generate, PORT_PLAN §5) or `procedural` (phases, conditional rosters, raw
  stream draws at creation — hand-ported with IL citations), plus the exact
  syntactic signals that decided it, so the classification is auditable
  rather than asserted.
* **rust** — whether the crate builds a roster for the id yet. Derived, not
  declared: the tool looks for the E4b registry symbol in `src/` and reports
  every id absent while it does not exist. `--check` re-derives this column
  (and every other non-demand one) against the committed file, which is what
  keeps a landed family from leaving an `absent` row behind it.

Read-only. It opens the save/capture corpus, the eval manifest, the frozen
provenance data and the generated Rust tables, and writes nothing except the
two artifacts it is asked for (`--json` / `--markdown`).

Since #2827 item D the **provenance** and **shape** columns come from
`data/encounter_provenance.v0.111.0.json`, the Python-derived half frozen as
data, so neither `--check` nor a regeneration imports the Python simulator.
Item F deleted the simulator and the content package, and with them the
`--write-provenance` mode that derived that file: it is frozen data now,
sha256-pinned by `frozen_oracle_data.py`. The shape-signal walk below
(`shape_signals`) stays; the census's controls exercise it on synthetic
modules.

Standard library only; run directly:

    python3 versions/v0.111.0/rust/tools/encounter_coverage_census.py \
        --json versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.json \
        --markdown versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.md

`--no-corpus` skips the ~30s capture scan and leaves every `corpus_fights`
null; a census without corpus demand says so in its own header rather than
reporting zeros that look like measurements.
"""

from __future__ import annotations

import argparse
import ast
import collections
import json
import pathlib
import re
import sys
from typing import Any, Dict, List, Optional, Set, Tuple

HERE = pathlib.Path(__file__).resolve()
TOOLS_DIR = HERE.parent
RUST_DIR = TOOLS_DIR.parent
VERSION_DIR = RUST_DIR.parent
ROOT_DIR = VERSION_DIR.parents[1]
MANIFEST = VERSION_DIR / "eval" / "manifest.json"
#: The Python-derived provenance/shape columns, frozen as data (#2827 item D).
PROVENANCE = RUST_DIR / "data" / "encounter_provenance.v0.111.0.json"
PROVENANCE_SCHEMA = "sts-encounter-provenance-v1"
PROVENANCE_COLUMNS = ("registered_key", "module", "builder", "source",
                      "shape", "shape_signals")
CONTENT_TABLES = RUST_DIR / "src" / "content_tables.rs"
DEFAULT_CAPTURES = pathlib.Path.home() / "sts2-captures"

SCHEMA = "sts-encounter-coverage-census-v1"
BUILD = "v0.111.0"

# The E4b registry symbol. It does not exist yet — that absence IS the census
# result today, and the tool must report it by looking rather than by holding
# a hand-maintained "all absent" list that would rot the day E4b lands.
RUST_REGISTRY_SYMBOL = "ENCOUNTER_ROSTER_BUILDERS"

# `ENCOUNTER_MATCH_ORDER` entries are Rust enum variants; the Python registry
# keys are SCREAMING_SNAKE id fragments. One shared normal form joins them.
_CAMEL_BOUNDARY = re.compile(r"(?<=[a-z0-9])(?=[A-Z])")

# Names a builder may call without that making it procedural: the roster
# vocabulary itself, plus stdlib shapes that carry no game decision.
_NEUTRAL_CALLS = frozenset({
    "Monster", "range", "len", "sorted", "tuple", "list", "set", "dict",
    "enumerate", "min", "max", "int", "abs", "reversed", "zip", "sum",
    "any", "all",
})

# `_EncounterCtx` methods that are pure roster plumbing. `unique_hp` and
# `fixed` are the tabular HP-roll primitives (one Niche draw per monster);
# `done` stamps creation-order uids. Anything else on `ctx` is a signal.
_NEUTRAL_CTX_METHODS = frozenset({"unique_hp", "fixed", "done"})


def _normalize(name: str) -> str:
    """One case-insensitive key shared by the Rust variant and the Python id.

    `EncounterId::CorpseSlugsWeak`, `CORPSE_SLUGS_WEAK` and
    `ENCOUNTER.CORPSE_SLUGS_WEAK` all normalize to `corpseslugsweak`.
    """
    name = name.rsplit(".", 1)[-1]
    return _CAMEL_BOUNDARY.sub("_", name).replace("_", "").lower()


class DispatchError(RuntimeError):
    """An observed encounter id no single registered key claims."""


def dispatch(observed: str, keys: List[str]) -> str:
    """Which registered key owns an observed wire id.

    `combat_sim.make_monsters` dispatches by **substring**, not equality —
    `ENCOUNTER.DECIMILLIPEDE_ELITE` is built by the builder registered under
    `DECIMILLIPEDE`. Demand counting has to use the same rule or the two
    encounters whose key is a proper prefix of their wire id would read as
    zero-demand while their fights were silently dropped.

    The docstring in `make_monsters` asserts the keys are disjoint. This
    function *checks* that rather than relying on it: an id claimed by two
    keys, or by none, raises instead of picking one.
    """
    target = _normalize(observed)
    owners = [key for key in keys if _normalize(key) in target]
    if len(owners) == 1:
        return owners[0]
    raise DispatchError(
        f"{observed}: {len(owners)} registered keys claim it ({owners!r})")


# ---------------------------------------------------------------------------
# The Rust side: the id axis, and whether anything builds a roster for it
# ---------------------------------------------------------------------------

def rust_encounter_ids(path: pathlib.Path = CONTENT_TABLES) -> List[str]:
    """`ENCOUNTER_MATCH_ORDER`, in its declared order.

    Parsed out of the generated table rather than re-derived from Python: the
    census is about the crate's id axis, and reading Python for both halves
    would make the row set agree with itself by construction.
    """
    text = path.read_text()
    match = re.search(
        r"pub static ENCOUNTER_MATCH_ORDER: \[EncounterId; (\d+)\] = \[(.*?)\];",
        text, re.S)
    if match is None:
        raise SystemExit(f"{path}: no ENCOUNTER_MATCH_ORDER found")
    declared = int(match.group(1))
    ids = re.findall(r"EncounterId::(\w+)", match.group(2))
    if len(ids) != declared:
        raise SystemExit(
            f"{path}: ENCOUNTER_MATCH_ORDER declares {declared} ids, "
            f"body lists {len(ids)}")
    return ids


def rust_built_ids(src_dir: pathlib.Path) -> Tuple[Set[str], Optional[str]]:
    """Which `EncounterId`s the crate can actually build a roster for.

    Returns `(variants, registry_path)`. While `RUST_REGISTRY_SYMBOL` does not
    exist anywhere under `src/`, the set is empty and the path is `None` —
    which is the honest statement of the frontier before E4b starts, and turns
    into a real measurement the moment E4b lands the registry.
    """
    for path in sorted(src_dir.rglob("*.rs")):
        text = path.read_text()
        match = re.search(
            RUST_REGISTRY_SYMBOL + r"[^=]*=\s*(?:&)?\[(.*?)\];", text, re.S)
        if match is None:
            continue
        variants = set(re.findall(r"EncounterId::(\w+)", match.group(1)))
        try:
            name = str(path.relative_to(ROOT_DIR))
        except ValueError:  # a synthetic tree under test
            name = str(path)
        return variants, name
    return set(), None


# ---------------------------------------------------------------------------
# The Python side: which module defines an encounter, and what shape it is
# ---------------------------------------------------------------------------

class _ModuleIndex:
    """Module-level function defs of one `content/encounters/*.py` shard."""

    def __init__(self, name: str, path: pathlib.Path) -> None:
        self.name = name
        self.path = path
        self.tree = ast.parse(path.read_text(), filename=str(path))
        self.functions: Dict[str, ast.FunctionDef] = {
            node.name: node for node in self.tree.body
            if isinstance(node, ast.FunctionDef)
        }


def _resolve(modules: Dict[str, _ModuleIndex], module: _ModuleIndex,
             name: str) -> Optional[_ModuleIndex]:
    """Where `name` is defined: this shard first, then the package.

    Pool shards import each other's roster helpers — `normal._corpse_slugs_normal`
    delegates to `weak.build_corpse_slugs` — so a walk that stopped at the
    shard boundary would label the caller procedural for the wrong reason
    ("calls something I cannot see") instead of the right one (that helper
    draws from the Encounter stream). A name defined in exactly one shard
    resolves there; an ambiguous or absent one stays an opaque helper.
    """
    if name in module.functions:
        return module
    owners = [m for m in modules.values() if name in m.functions]
    return owners[0] if len(owners) == 1 else None


def shape_signals(module: _ModuleIndex, function: str,
                  seen: Optional[Set[str]] = None,
                  modules: Optional[Dict[str, _ModuleIndex]] = None) -> Set[str]:
    """Every syntactic reason this builder is not a flat table.

    The walk follows module-local helper calls transitively, because the
    roster shape of e.g. `_corpse_slugs_weak` lives entirely in the shared
    `build_corpse_slugs` it delegates to. An empty set means `tabular`.

    The signals, and why each one is one:

    * `encounter_rng` — the builder draws from the per-fight Encounter stream
      at creation (`EncounterModel::GenerateMonstersWithSlots`, RVA 0x7f88c,
      seeds it from `Seed + TotalFloor + hash(Id.Entry)`). Consumption order
      is behaviour, not data.
    * `raw_niche_draw` — a Niche draw outside `ctx.unique_hp` / `ctx.fixed`.
    * `floor_dependent` — the roster reads `total_floor`.
    * `branching` / `loop_while` — a conditional roster.
    * `ctx:<method>` — an `_EncounterCtx` surface beyond the three tabular
      primitives.
    * `helper:<name>` — a call out to `combat_sim` or another module, which a
      generated table cannot express.
    """
    seen = set() if seen is None else seen
    modules = {module.name: module} if modules is None else modules
    key = f"{module.name}.{function}"
    if key in seen:
        return set()
    seen.add(key)
    node = module.functions.get(function)
    if node is None:
        return {f"helper:{function}"}

    out: Set[str] = set()
    for child in ast.walk(node):
        if isinstance(child, (ast.If, ast.IfExp)):
            out.add("branching")
        elif isinstance(child, ast.While):
            out.add("loop_while")
        elif isinstance(child, ast.Attribute) and child.attr == "total_floor":
            out.add("floor_dependent")
        elif isinstance(child, ast.Call):
            func = child.func
            if isinstance(func, ast.Attribute):
                if func.attr == "encounter_rng":
                    out.add("encounter_rng")
                elif (func.attr == "next_int"
                      and isinstance(func.value, ast.Attribute)
                      and func.value.attr == "niche"):
                    out.add("raw_niche_draw")
                elif (isinstance(func.value, ast.Name)
                      and func.value.id == "ctx"
                      and func.attr not in _NEUTRAL_CTX_METHODS):
                    out.add(f"ctx:{func.attr}")
            elif isinstance(func, ast.Name):
                if func.id in _NEUTRAL_CALLS:
                    continue
                owner = _resolve(modules, module, func.id)
                if owner is not None:
                    out |= shape_signals(owner, func.id, seen, modules)
                else:
                    out.add(f"helper:{func.id}")
    return out


# ---------------------------------------------------------------------------
# Demand: corpus fights and checked-in fixtures
# ---------------------------------------------------------------------------

def fixture_counts(manifest: pathlib.Path = MANIFEST) -> Dict[str, int]:
    """Fixtures per observed encounter wire id, from the eval manifest."""
    counts: Dict[str, int] = collections.Counter()
    data = json.loads(manifest.read_text())
    for fight in data.get("fights", []):
        encounter = fight.get("encounter")
        if encounter:
            counts[encounter] += 1
    return dict(counts)


def corpus_counts(captures: pathlib.Path) -> Tuple[Dict[str, int],
                                                   Dict[str, Any]]:
    """Fights per encounter over the local capture corpus.

    Reuses `eval_suite.discover_fights` / `derive_encounter` rather than
    re-deriving the pairing rule: the census's denominator must be the same
    513 fights `eval_suite census` measures, or the demand order is about a
    different corpus than the acceptance bar.
    """
    sys.path.insert(0, str(TOOLS_DIR))
    import eval_suite  # noqa: PLC0415 - optional, corpus-only dependency

    state = eval_suite.discover_fights(captures)
    counts: Dict[str, int] = collections.Counter()
    unpaired = no_encounter = 0
    for key, _ in state["fights"].items():
        spath = state["first_save"].get(key)
        if spath is None:
            unpaired += 1
            continue
        save = json.loads(spath.read_text())
        encounter, _kind = eval_suite.derive_encounter(
            save, key, spath, state["run_saves"])
        if not encounter:
            no_encounter += 1
            continue
        counts[encounter] += 1
    totals = {
        "fights": len(state["fights"]),
        "with_encounter": sum(counts.values()),
        "no_encounter": no_encounter,
        "unpaired": unpaired,
        "saves": state["saves"],
    }
    return dict(counts), totals


# ---------------------------------------------------------------------------
# Rows
# ---------------------------------------------------------------------------

def frozen_provenance(path: pathlib.Path = PROVENANCE
                      ) -> Dict[str, Dict[str, Any]]:
    """The committed provenance rows; refuses a malformed file by name."""
    data = json.loads(path.read_text())
    if data.get("schema") != PROVENANCE_SCHEMA:
        raise SystemExit(f"{path}: schema {data.get('schema')!r} is not "
                         f"{PROVENANCE_SCHEMA!r}")
    rows = data["rows"]
    for key, row in rows.items():
        if sorted(row) != sorted(PROVENANCE_COLUMNS):
            raise SystemExit(f"{path}: row {key} carries {sorted(row)}, "
                             f"expected {sorted(PROVENANCE_COLUMNS)}")
    return rows


def build_rows(*, captures: Optional[pathlib.Path],
               provenance: Optional[Dict[str, Dict[str, Any]]] = None
               ) -> Dict[str, Any]:
    ids = rust_encounter_ids()
    built, registry = rust_built_ids(RUST_DIR / "src")
    fixtures = fixture_counts()
    corpus: Dict[str, int] = {}
    corpus_totals: Optional[Dict[str, Any]] = None
    if captures is not None:
        corpus, corpus_totals = corpus_counts(captures)
    if provenance is None:
        provenance = frozen_provenance()

    # Demand is observed as wire ids (`ENCOUNTER.DECIMILLIPEDE_ELITE`) and
    # charged to the registered key that `make_monsters` would dispatch to
    # (`DECIMILLIPEDE`), via the same substring rule.
    corpus_by_key: Dict[str, int] = collections.Counter()
    fixtures_by_key: Dict[str, int] = collections.Counter()
    observed_by_key: Dict[str, Set[str]] = collections.defaultdict(set)
    for observed, count in corpus.items():
        key = _normalize(dispatch(observed, ids))
        corpus_by_key[key] += count
        observed_by_key[key].add(observed)
    for observed, count in fixtures.items():
        key = _normalize(dispatch(observed, ids))
        fixtures_by_key[key] += count
        observed_by_key[key].add(observed)

    rows: List[Dict[str, Any]] = []
    for variant in ids:
        key = _normalize(variant)
        entry = provenance.get(key)
        row: Dict[str, Any] = {
            "registered_key": None,
            "rust_variant": variant,
            "observed_wire_ids": sorted(observed_by_key.get(key, ())),
            "corpus_fights": corpus_by_key.get(key, 0) if captures else None,
            "fixtures": fixtures_by_key.get(key, 0),
            "module": None,
            "builder": None,
            "source": None,
            "shape": None,
            "shape_signals": [],
            "rust": "built" if variant in built else "absent",
        }
        if entry is None:
            # An id the crate names and the Python registry does not build.
            # Never silently dropped: it is a row with an explicit reason.
            row["shape"] = "unregistered"
            row["shape_signals"] = ["no_python_builder"]
            rows.append(row)
            continue
        row.update({column: entry[column] for column in PROVENANCE_COLUMNS})
        rows.append(row)

    census: Dict[str, Any] = {
        "schema": SCHEMA,
        "build": BUILD,
        "generated_by": "versions/v0.111.0/rust/tools/encounter_coverage_census.py",
        "encounter_ids": len(ids),
        "rust_registry": registry,
        "corpus_measured": captures is not None,
        "corpus_totals": corpus_totals,
        "summary": _summary(rows),
        "rows": rows,
    }
    return census


def _summary(rows: List[Dict[str, Any]]) -> Dict[str, Any]:
    shapes = collections.Counter(row["shape"] for row in rows)
    status = collections.Counter(row["rust"] for row in rows)
    pools = collections.Counter(
        (row["module"] or "<unregistered>") for row in rows)
    demand = [row for row in rows if (row["corpus_fights"] or 0) > 0]
    return {
        "shapes": dict(sorted(shapes.items())),
        "rust": dict(sorted(status.items())),
        "pools": dict(sorted(pools.items())),
        "with_corpus_demand": len(demand),
        "zero_corpus_demand": len(rows) - len(demand),
        "corpus_fights_covered": sum(row["corpus_fights"] or 0 for row in rows),
        "fixtures_covered": sum(row["fixtures"] for row in rows),
    }


def markdown(census: Dict[str, Any]) -> str:
    rows = sorted(
        census["rows"],
        key=lambda r: (-(r["corpus_fights"] or 0), -r["fixtures"],
                       r["rust_variant"]))
    out: List[str] = []
    out.append("# Encounter coverage census (E4b frontier)")
    out.append("")
    out.append(
        f"Generated by `{census['generated_by']}` for build "
        f"`{census['build']}`. One row per "
        f"`content_tables::ENCOUNTER_MATCH_ORDER` id "
        f"({census['encounter_ids']} today), ordered by corpus demand.")
    out.append("")
    if not census["corpus_measured"]:
        out.append(
            "**Corpus demand not measured** (`--no-corpus`): every "
            "`corpus` cell is blank rather than zero.")
        out.append("")
    else:
        totals = census["corpus_totals"] or {}
        out.append(
            f"Corpus: {totals.get('fights')} fights over "
            f"{totals.get('saves')} saves — {totals.get('with_encounter')} "
            f"carry an encounter, {totals.get('no_encounter')} resolve to no "
            f"encounter, {totals.get('unpaired')} have no entry save.")
        out.append("")
    registry = census["rust_registry"]
    out.append(
        f"Rust roster registry: `{registry}`." if registry else
        "Rust roster registry: **absent** — no `"
        f"{RUST_REGISTRY_SYMBOL}` exists under `versions/v0.111.0/rust/src/`, "
        "so every id below is `absent` by derivation, not by assertion.")
    out.append("")
    summary = census["summary"]
    out.append(
        f"Shapes: {summary['shapes']} · Rust: {summary['rust']} · "
        f"with corpus demand: {summary['with_corpus_demand']} · "
        f"zero corpus demand: {summary['zero_corpus_demand']}")
    out.append("")
    out.append(
        "`registered key` is what `make_monsters` matches (a **substring** of "
        "the wire id); a `+` marks a key whose observed wire ids differ from "
        "it, listed in the JSON's `observed_wire_ids`.")
    out.append("")
    out.append(
        "| registered key | corpus | fixtures | module | builder | shape | "
        "signals | rust |")
    out.append("|---|---:|---:|---|---|---|---|---|")
    for row in rows:
        corpus = ("" if row["corpus_fights"] is None
                  else str(row["corpus_fights"]))
        module = (row["module"] or "").replace("content/encounters/", "")
        signals = ", ".join(row["shape_signals"]) or "—"
        key = row["registered_key"] or row["rust_variant"]
        inexact = any(observed != key for observed in row["observed_wire_ids"])
        out.append(
            f"| `{key}`{' +' if inexact else ''} | {corpus} | "
            f"{row['fixtures']} | {module} | `{row['builder'] or '—'}` "
            f"{row['source'] or ''} | {row['shape']} | {signals} | "
            f"{row['rust']} |")
    out.append("")
    return "\n".join(out)


#: Columns a `--check` run re-derives. Everything else in a row is corpus
#: demand, which needs the local captures and cannot be checked in CI.
#:
#: `fixtures` is here because it is *manifest* demand, not corpus demand:
#: `fixture_counts` reads `eval/manifest.json` and `dispatch` charges each
#: wire id to the checked-in id axis, neither of which touches the captures
#: (`build_rows(captures=None)` computes it identically). It was left out with
#: the corpus columns until 2026-09-21, which is how the 2026-09-17 manifest
#: re-seed (`bd7717a5`) moved five rows' fixture counts under a census that
#: kept passing `--check`. `observed_wire_ids` stays excluded — it merges
#: manifest and corpus observations, so it is only half re-derivable.
CHECKED_COLUMNS = ("registered_key", "rust_variant", "module", "builder",
                   "source", "shape", "shape_signals", "rust", "fixtures")


def check(committed_path: pathlib.Path) -> List[str]:
    """Findings where the committed census disagrees with today's sources.

    Deliberately **not** a `git diff --exit-code` freshness check: corpus
    demand is measured against `~/sts2-captures`, which exists on exactly one
    host, so re-deriving the whole document in CI would compare a measured
    artifact against an unmeasurable one and go red for the wrong reason.

    What *is* checkable is everything the census derives from checked-in
    sources: the id axis, which Python module and builder defines each id, its
    line span, the tabular/procedural verdict with its signals, and — the
    column that actually rots — whether the crate builds it. A family wave PR
    that lands a roster without regenerating this file leaves an `absent` row
    behind a built encounter, which nothing else in the port gate can see
    (#2531).
    """
    if not committed_path.exists():
        return [f"{committed_path} is missing"]
    committed = json.loads(committed_path.read_text())
    fresh = build_rows(captures=None)
    findings: List[str] = []
    if committed.get("encounter_ids") != fresh["encounter_ids"]:
        findings.append(
            f"id axis: committed {committed.get('encounter_ids')} ids, "
            f"sources have {fresh['encounter_ids']}")
    if committed.get("rust_registry") != fresh["rust_registry"]:
        findings.append(
            f"rust registry: committed {committed.get('rust_registry')!r}, "
            f"sources have {fresh['rust_registry']!r}")
    by_variant = {row["rust_variant"]: row for row in committed.get("rows", ())}
    for row in fresh["rows"]:
        recorded = by_variant.get(row["rust_variant"])
        if recorded is None:
            findings.append(f"{row['rust_variant']}: no committed row")
            continue
        for column in CHECKED_COLUMNS:
            if recorded.get(column) != row[column]:
                findings.append(
                    f"{row['rust_variant']}.{column}: committed "
                    f"{recorded.get(column)!r}, sources have {row[column]!r}")
    for variant in sorted(set(by_variant) - {r["rust_variant"]
                                             for r in fresh["rows"]}):
        findings.append(f"{variant}: committed row has no id behind it")
    return findings


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--captures", type=pathlib.Path,
                        default=DEFAULT_CAPTURES,
                        help="capture corpus root (default: ~/sts2-captures)")
    parser.add_argument("--no-corpus", action="store_true",
                        help="skip the corpus scan; leave demand unmeasured")
    parser.add_argument("--check", action="store_true",
                        help="verify the committed census still agrees with "
                             "the sources it derives, corpus demand aside; "
                             "write nothing")
    parser.add_argument("--json", type=pathlib.Path,
                        help="write the census JSON here")
    parser.add_argument("--markdown", type=pathlib.Path,
                        help="write the census table here")
    args = parser.parse_args(argv)

    if args.check:
        findings = check(RUST_DIR / "ENCOUNTER_COVERAGE_CENSUS.json")
        for finding in findings:
            print(f"STALE: {finding}", file=sys.stderr)
        if findings:
            print("re-run: python3 versions/v0.111.0/rust/tools/"
                  "encounter_coverage_census.py --json "
                  "versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.json "
                  "--markdown "
                  "versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.md",
                  file=sys.stderr)
            return 1
        print("encounter coverage census is current "
              "(corpus demand not re-measured)")
        return 0

    captures: Optional[pathlib.Path] = None
    if not args.no_corpus:
        if not args.captures.is_dir():
            print(f"no capture corpus at {args.captures}; "
                  "re-run with --no-corpus to census without demand",
                  file=sys.stderr)
            return 2
        captures = args.captures

    census = build_rows(captures=captures)
    if args.json:
        args.json.write_text(json.dumps(census, indent=1, sort_keys=False)
                             + "\n")
    if args.markdown:
        args.markdown.write_text(markdown(census))
    if not args.json and not args.markdown:
        print(markdown(census))
    else:
        print(json.dumps(census["summary"], indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

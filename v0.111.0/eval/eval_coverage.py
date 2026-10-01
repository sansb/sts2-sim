#!/usr/bin/env python3
"""How much of fight-space the eval suite covers, as JSON and two SVGs.

Every number is measured on **certified** lines only: a fixture whose recorded
human line replays through Rust with every native checkpoint agreeing. Each
axis has one rule for "exercised", so a card sitting in a deck never counts:

* encounters: a certified line exists for it, per tier;
* boss/elite x character: the one cross product worth tracking, because the
  same boss plays differently per class. Every boss and elite row and every
  character column is drawn, empty or not;
* cards: played at least once in a certified line, per owning pool
  (`data/game_values.json` `colors`, read from the game's `AllCardPools`);
* relics: held entering a certified fight, per pool;
* potions: used in a certified line.

An empty grid cell is coloured by why it is empty, from the census the suite
was seeded from: *captured* (an A9/A10 capture exists but does not certify
yet, which is engine work) or *not captured* (nobody has played it at A9/A10
yet). That half needs the local capture corpus, so it is stored in
`coverage.json` under `corpus` and carried forward by any run that has no
census to hand (`eval_suite.py add`, `--check`).

Outputs, all beside this file:

* `coverage.json`: the measurement;
* `COVERAGE.svg`: the grid and the per-axis bars;
* `coverage-badge.svg`: a one-line summary for the README.

The reference lists (the "universe") are a snapshot, `coverage_universe.json`,
because their sources live outside the solver trigger set. Refresh it on a
version bump or when the encounter registry changes (`--refresh-universe`).

    python3 sim/v0.111.0/eval/eval_coverage.py                  # rewrite outputs
    python3 sim/v0.111.0/eval/eval_coverage.py --census C.json  # and the corpus half
    python3 sim/v0.111.0/eval/eval_coverage.py --check          # fail if stale
    python3 sim/v0.111.0/eval/eval_coverage.py --refresh-universe
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from typing import Any, Dict, List, Optional

EVAL_DIR = pathlib.Path(__file__).resolve().parent
VERSION_DIR = EVAL_DIR.parent
ROOT_DIR = VERSION_DIR.parent  # `sim/`, or the sts2-sim repo
MANIFEST = EVAL_DIR / "manifest.json"
UNIVERSE = EVAL_DIR / "coverage_universe.json"
OUT_JSON = EVAL_DIR / "coverage.json"
OUT_SVG = EVAL_DIR / "COVERAGE.svg"
OUT_BADGE = EVAL_DIR / "coverage-badge.svg"
TOOL = "sim/v0.111.0/eval/eval_coverage.py"
SCHEMA = "sts-eval-coverage-v1"

CHARACTERS = ("IRONCLAD", "SILENT", "DEFECT", "NECROBINDER", "REGENT")
SHORT = {"IRONCLAD": "IC", "SILENT": "SI", "DEFECT": "DE",
         "NECROBINDER": "NB", "REGENT": "RG"}
TIERS = ("weak", "normal", "elite", "boss", "event")
CARD_POOLS = tuple(c.lower() for c in CHARACTERS) + ("colorless",)
RELIC_POOLS = tuple(c.lower() for c in CHARACTERS) + ("shared", "event")
#: The ascension floor of the suite (`eval_suite.SEED_MIN_ASCENSION`, #2915).
MIN_ASCENSION = 9


# ---------------------------------------------------------------------------
# Universe snapshot
# ---------------------------------------------------------------------------


def build_universe() -> Dict[str, Any]:
    """Read the reference lists from their sources into one snapshot."""
    data = ROOT_DIR / "data"
    cards = json.loads((data / "cards.json").read_text())
    values = json.loads((data / "game_values.json").read_text())["builds"]
    build = values["v0.111.0"]
    while "alias" in build:
        build = values[build["alias"]]
    colors = build["colors"]
    relics = json.loads((data / "relics.json").read_text())
    loc = json.loads((data / "loc_en.json").read_text())
    census = json.loads(
        (VERSION_DIR / "engine" / "ENCOUNTER_COVERAGE_CENSUS.json").read_text())

    universe_cards = {}
    for card_id, meta in sorted(cards.items()):
        pool = colors.get(card_id)
        if pool not in CARD_POOLS or meta["rarity"] == "token" \
                or meta["type"] not in ("attack", "skill", "power"):
            continue
        universe_cards[card_id.removeprefix("CARD.")] = {
            "pool": pool, "rarity": meta["rarity"]}
    universe_relics = {
        relic_id: {"pool": meta["pool"]}
        for relic_id, meta in sorted(relics.items())
        if meta["pool"] in RELIC_POOLS}
    potions = sorted(key.removesuffix(".title") for key in loc["potions"]
                     if key.endswith(".title"))
    encounter_titles = loc.get("encounters", {})
    encounters = []
    for row in sorted(census["rows"], key=lambda r: r["registered_key"]):
        key = row["registered_key"]
        bare = key.removeprefix("ENCOUNTER.")
        tier = pathlib.Path(row["module"]).stem
        wire = sorted(row.get("observed_wire_ids") or [key])
        title = (encounter_titles.get(f"{bare}.title")
                 or next((encounter_titles.get(f"{w.removeprefix('ENCOUNTER.')}.title")
                          for w in wire if encounter_titles.get(
                              f"{w.removeprefix('ENCOUNTER.')}.title")), None)
                 or bare.replace("_", " ").title())
        encounters.append({"key": key, "tier": tier, "wire_ids": wire,
                           "title": title})
    return {
        "schema": "sts-eval-coverage-universe-v1",
        "build": "v0.111.0",
        "generated_by": TOOL + " --refresh-universe",
        "sources": ["data/cards.json", "data/game_values.json (colors)",
                    "data/relics.json", "data/loc_en.json (potions, encounter titles)",
                    "sim/v0.111.0/engine/ENCOUNTER_COVERAGE_CENSUS.json"],
        "characters": list(CHARACTERS),
        "cards": universe_cards,
        "relics": universe_relics,
        "potions": potions,
        "encounters": encounters,
    }


# ---------------------------------------------------------------------------
# Measurement
# ---------------------------------------------------------------------------


def _ratio(covered: int, total: int) -> Dict[str, int]:
    return {"covered": covered, "total": total}


def corpus_cells(rows: List[Dict[str, Any]],
                 universe: Dict[str, Any]) -> Dict[str, Any]:
    """Per (encounter, character) A9/A10 capture and certification counts."""
    wire_to_key = {wire: enc["key"] for enc in universe["encounters"]
                   for wire in enc["wire_ids"]}
    cells: Dict[str, Dict[str, Dict[str, int]]] = {}
    for row in rows:
        if (row.get("ascension") or 0) < MIN_ASCENSION:
            continue
        key = wire_to_key.get(str(row.get("encounter")))
        if key is None or row.get("character") not in CHARACTERS:
            continue
        cell = cells.setdefault(key, {}).setdefault(
            row["character"], {"captured": 0, "certified": 0})
        cell["captured"] += 1
        if row.get("lockstep") == "lockstep_ok":
            cell["certified"] += 1
    return {"measured_fights": len(rows),
            "cells": {k: dict(sorted(v.items())) for k, v in sorted(cells.items())}}


def measure(manifest: Dict[str, Any], universe: Dict[str, Any],
            corpus: Optional[Dict[str, Any]]) -> Dict[str, Any]:
    fights_dir = EVAL_DIR / "fights"
    wire_to_key = {wire: enc["key"] for enc in universe["encounters"]
                   for wire in enc["wire_ids"]}
    certified = [f for f in manifest["fights"] if "certified" in f["categories"]]
    played, held, used = set(), set(), set()
    cells: Dict[str, Dict[str, int]] = {}
    for fight in certified:
        entry = json.loads((fights_dir / fight["id"] / "entry.canonical.json").read_text())
        line = json.loads((fights_dir / fight["id"] / "human_line.json").read_text())
        by_uid = {card["uid"]: card["id"]
                  for pile in entry["piles"].values() for card in pile}
        belt = entry["player"].get("potions") or []
        for action in line["actions"]:
            if action.get("kind") == "play" and action.get("uid") in by_uid:
                played.add(by_uid[action["uid"]])
            elif action.get("kind") == "potion":
                slot = action.get("slot")
                if isinstance(slot, int) and 0 <= slot < len(belt) and belt[slot]:
                    used.add(belt[slot])
        held.update(entry["player"].get("relics_entering") or ())
        key = wire_to_key.get(fight["encounter"])
        if key is not None:
            per = cells.setdefault(key, {})
            per[fight["character"]] = per.get(fight["character"], 0) + 1

    tiers = {}
    for tier in TIERS:
        keys = [e["key"] for e in universe["encounters"] if e["tier"] == tier]
        tiers[tier] = _ratio(sum(1 for k in keys if k in cells), len(keys))

    cards = universe["cards"]
    card_pools = {pool: _ratio(
        sum(1 for c, m in cards.items() if m["pool"] == pool and c in played),
        sum(1 for m in cards.values() if m["pool"] == pool)) for pool in CARD_POOLS}
    rarities = sorted({m["rarity"] for m in cards.values()})
    card_rarity = {r: _ratio(
        sum(1 for c, m in cards.items() if m["rarity"] == r and c in played),
        sum(1 for m in cards.values() if m["rarity"] == r)) for r in rarities}
    relics = universe["relics"]
    relic_pools = {pool: _ratio(
        sum(1 for r, m in relics.items() if m["pool"] == pool and r in held),
        sum(1 for m in relics.values() if m["pool"] == pool)) for pool in RELIC_POOLS}
    potions = universe["potions"]

    corpus_map = (corpus or {}).get("cells", {})
    grid = {}
    for tier in ("boss", "elite"):
        rows = []
        for enc in universe["encounters"]:
            if enc["tier"] != tier:
                continue
            row = {"key": enc["key"], "title": enc["title"], "cells": {}}
            for character in CHARACTERS:
                count = cells.get(enc["key"], {}).get(character, 0)
                captured = corpus_map.get(enc["key"], {}).get(
                    character, {}).get("captured", 0)
                state = ("certified" if count else
                         "captured" if captured else "not_captured")
                row["cells"][character] = {"certified_fixtures": count,
                                           "captured": captured, "state": state}
            rows.append(row)
        grid[tier] = rows

    def cross(tier):
        return _ratio(sum(1 for r in grid[tier] for c in r["cells"].values()
                          if c["state"] == "certified"),
                      len(grid[tier]) * len(CHARACTERS))

    return {
        "schema": SCHEMA,
        "build": manifest["build"],
        "generated_by": TOOL,
        "universe_sha256": hashlib.sha256(UNIVERSE.read_bytes()).hexdigest(),
        "fixtures": len(manifest["fights"]),
        "certified": len(certified),
        "encounters": {"by_tier": tiers, "all": _ratio(
            len([k for k in cells if k in {e["key"] for e in universe["encounters"]}]),
            len(universe["encounters"]))},
        "boss_x_character": cross("boss"),
        "elite_x_character": cross("elite"),
        "cards_played": {"by_pool": card_pools, "by_rarity": card_rarity,
                         "all": _ratio(sum(1 for c in cards if c in played), len(cards))},
        "relics_held": {"by_pool": relic_pools,
                        "all": _ratio(sum(1 for r in relics if r in held), len(relics))},
        "potions_used": _ratio(sum(1 for p in potions if p in used), len(potions)),
        "grid": grid,
        "corpus": corpus or {"measured_fights": 0, "cells": {}},
    }


# ---------------------------------------------------------------------------
# SVG rendering
# ---------------------------------------------------------------------------

STYLE = """<style>
.bg{fill:#ffffff}.t{font:12px -apple-system,Segoe UI,Helvetica,Arial,sans-serif;fill:#24292f}
.m{fill:#57606a}.h{font-weight:600}.n{font-size:11px;fill:#0a3622}
.cert{fill:#2da44e}.capt{fill:#d4a72c}.none{fill:#eaeef2}.track{fill:#eaeef2}.bar{fill:#2da44e}
@media (prefers-color-scheme:dark){.bg{fill:#0d1117}.t{fill:#e6edf3}.m{fill:#8d96a0}
.none,.track{fill:#21262d}.cert,.bar{fill:#2ea043}.capt{fill:#bb8009}.n{fill:#ffffff}}
</style>"""


def _esc(text: str) -> str:
    return (text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))


def render_svg(cov: Dict[str, Any]) -> str:
    cell, gap, name_w, row_h = 30, 4, 170, 22
    panel_w = name_w + len(CHARACTERS) * (cell + gap)
    # Two panels, plus room for the right column's bar counts.
    width = panel_w * 2 + 40 + 60
    out: List[str] = []
    y = 24
    out.append(f'<text class="t h" x="0" y="{y}">Eval coverage, {cov["build"]}: '
               f'{cov["fixtures"]} fixtures, {cov["certified"]} certified (A9/A10)</text>')
    y += 34
    top = y
    heights = []
    for index, tier in enumerate(("boss", "elite")):
        x0 = index * (panel_w + 40)
        ratio = cov[f"{tier}_x_character"]
        label = {"boss": "Bosses", "elite": "Elites"}[tier]
        out.append(f'<text class="t h" x="{x0}" y="{top}">{label} x character '
                   f'<tspan class="m">{ratio["covered"]}/{ratio["total"]}</tspan></text>')
        for col, character in enumerate(CHARACTERS):
            cx = x0 + name_w + col * (cell + gap) + cell / 2
            out.append(f'<text class="t m" x="{cx:.0f}" y="{top + 20}" '
                       f'text-anchor="middle">{SHORT[character]}</text>')
        ry = top + 28
        for row in cov["grid"][tier]:
            out.append(f'<text class="t" x="{x0}" y="{ry + 15}">{_esc(row["title"])}</text>')
            for col, character in enumerate(CHARACTERS):
                state = row["cells"][character]
                klass = {"certified": "cert", "captured": "capt",
                         "not_captured": "none"}[state["state"]]
                cx = x0 + name_w + col * (cell + gap)
                out.append(f'<rect class="{klass}" x="{cx}" y="{ry}" width="{cell}" '
                           f'height="{row_h - 3}" rx="3"/>')
                if state["certified_fixtures"]:
                    out.append(f'<text class="t n" x="{cx + cell / 2:.0f}" y="{ry + 14}" '
                               f'text-anchor="middle">{state["certified_fixtures"]}</text>')
            ry += row_h
        heights.append(ry)
    y = max(heights) + 12
    legend = (("cert", "certified line (count)"),
              ("capt", "captured at A9/A10, not certified yet"),
              ("none", "not captured at A9/A10 yet"))
    lx = 0
    for klass, label in legend:
        out.append(f'<rect class="{klass}" x="{lx}" y="{y}" width="14" height="14" rx="3"/>')
        out.append(f'<text class="t m" x="{lx + 20}" y="{y + 11}">{label}</text>')
        lx += 36 + len(label) * 6.4
    y += 40

    bar_x, bar_w = 180, 200
    sections = [
        ("Cards played, by pool", [(p.title(), cov["cards_played"]["by_pool"][p])
                                   for p in CARD_POOLS]),
        ("Relics held, by pool", [(p.title(), cov["relics_held"]["by_pool"][p])
                                  for p in RELIC_POOLS]),
        ("Encounters, by tier", [(t.title(), cov["encounters"]["by_tier"][t])
                                 for t in TIERS]),
        ("Potions used", [("All potions", cov["potions_used"])]),
    ]
    col_y = [y, y]
    for index, (title, rows) in enumerate(sections):
        column = index % 2
        x0 = column * (panel_w + 40)
        sy = col_y[column]
        out.append(f'<text class="t h" x="{x0}" y="{sy}">{title}</text>')
        sy += 10
        for label, ratio in rows:
            frac = ratio["covered"] / ratio["total"] if ratio["total"] else 0
            out.append(f'<text class="t" x="{x0}" y="{sy + 13}">{label}</text>')
            out.append(f'<rect class="track" x="{x0 + bar_x - 60}" y="{sy + 3}" '
                       f'width="{bar_w}" height="12" rx="3"/>')
            if frac:
                out.append(f'<rect class="bar" x="{x0 + bar_x - 60}" y="{sy + 3}" '
                           f'width="{bar_w * frac:.1f}" height="12" rx="3"/>')
            out.append(f'<text class="t m" x="{x0 + bar_x - 60 + bar_w + 8}" y="{sy + 13}">'
                       f'{ratio["covered"]}/{ratio["total"]}</text>')
            sy += 20
        col_y[column] = sy + 22
    height = max(col_y) + 4
    note = (f'Grid colours from a census of {cov["corpus"]["measured_fights"]} captured '
            f'fights. Cards count only when played; relics when held; potions when used.')
    out.append(f'<text class="t m" x="0" y="{height}">{_esc(note)}</text>')
    height += 12
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
            f'viewBox="0 0 {width} {height}" role="img" aria-label="Eval suite coverage">'
            f'<title>Eval suite coverage</title>{STYLE}'
            f'<rect class="bg" width="{width}" height="{height}"/>'
            f'<g transform="translate(0,0)">' + "".join(out) + "</g></svg>\n")


def render_badge(cov: Dict[str, Any]) -> str:
    left = "eval"
    right = (f'{cov["certified"]} certified · encounters '
             f'{cov["encounters"]["all"]["covered"]}/{cov["encounters"]["all"]["total"]} · '
             f'boss×class {cov["boss_x_character"]["covered"]}/'
             f'{cov["boss_x_character"]["total"]} · cards '
             f'{round(100 * cov["cards_played"]["all"]["covered"] / cov["cards_played"]["all"]["total"])}%')
    lw = 10 + len(left) * 7
    rw = 10 + len(right) * 6.3
    width = lw + rw
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="20" '
            f'role="img" aria-label="{left}: {_esc(right)}"><title>{left}: {_esc(right)}</title>'
            f'<rect width="{lw}" height="20" rx="3" fill="#555"/>'
            f'<rect x="{lw}" width="{rw:.0f}" height="20" rx="3" fill="#2da44e"/>'
            f'<rect x="{lw}" width="4" height="20" fill="#2da44e"/>'
            f'<g fill="#fff" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">'
            f'<text x="{lw / 2:.0f}" y="14" text-anchor="middle">{left}</text>'
            f'<text x="{lw + rw / 2:.0f}" y="14" text-anchor="middle">{_esc(right)}</text>'
            f'</g></svg>\n')


# ---------------------------------------------------------------------------
# Entry points
# ---------------------------------------------------------------------------


def render_all(corpus_rows: Optional[List[Dict[str, Any]]] = None) -> Dict[str, str]:
    """Every output's bytes. The corpus half comes from `corpus_rows` when
    given, else from the committed `coverage.json`."""
    manifest = json.loads(MANIFEST.read_text())
    universe = json.loads(UNIVERSE.read_text())
    if corpus_rows is not None:
        corpus = corpus_cells(corpus_rows, universe)
    elif OUT_JSON.is_file():
        corpus = json.loads(OUT_JSON.read_text()).get("corpus")
    else:
        corpus = None
    cov = measure(manifest, universe, corpus)
    return {OUT_JSON.name: json.dumps(cov, indent=1, sort_keys=True) + "\n",
            OUT_SVG.name: render_svg(cov),
            OUT_BADGE.name: render_badge(cov)}


def write(corpus_rows: Optional[List[Dict[str, Any]]] = None) -> Dict[str, str]:
    outputs = render_all(corpus_rows)
    for name, text in outputs.items():
        (EVAL_DIR / name).write_text(text, encoding="utf-8")
    return outputs


def stale() -> List[str]:
    """Outputs whose committed bytes differ from a fresh render."""
    return [name for name, text in render_all().items()
            if not (EVAL_DIR / name).is_file()
            or (EVAL_DIR / name).read_text(encoding="utf-8") != text]


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--census", type=pathlib.Path,
                        help="a census JSON (`eval_suite.py census --census-json`) "
                             "to re-measure the captured / not-captured split from")
    parser.add_argument("--check", action="store_true",
                        help="exit non-zero if any output is stale")
    parser.add_argument("--refresh-universe", action="store_true",
                        help="re-snapshot the reference lists from their sources")
    args = parser.parse_args(argv)
    if args.refresh_universe:
        UNIVERSE.write_text(json.dumps(build_universe(), indent=1, sort_keys=True) + "\n",
                            encoding="utf-8")
    if args.check:
        names = stale()
        for name in names:
            print(f"STALE: {name}")
        if names:
            print(f"re-run: python3 {TOOL}")
        return 1 if names else 0
    rows = None
    if args.census:
        rows = json.loads(args.census.read_text())["rows"]
    cov = json.loads(write(rows)[OUT_JSON.name])
    print(f"wrote coverage for {cov['certified']} certified fixtures: encounters "
          f"{cov['encounters']['all']['covered']}/{cov['encounters']['all']['total']}, "
          f"boss x character {cov['boss_x_character']['covered']}/"
          f"{cov['boss_x_character']['total']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Rust port coverage badge — the README banner for the v0.111.0 port.

Emits three synchronized artifacts, mirroring the Python simulator's
`sim/v0.111.0/python/tools/build_coverage_matrix.py`:

- sim/v0.111.0/engine/coverage.json — the machine-readable ledger
- sim/v0.111.0/engine/COVERAGE.md   — the per-family human breakdown
- sim/v0.111.0/engine/coverage.svg  — the compact README banner

Derivation (nothing here is transcribed)
----------------------------------------
The counts are read out of the same registries the engine itself derives
its capability manifest from — `sts_sim::engine::admission::capability_manifest`
flat-maps `steps::FAMILIES` / `moves::FAMILIES`, publishes
`engine::admission::IMPLEMENTED_RELICS`, and reads the generated enum counts.
This tool reads those registries out of the checked-in source, stdlib-only, so
it needs no toolchain and cannot report a number the crate does not hold:

- `src/ids.rs`        — `impl StepKind { pub const COUNT }`, ditto `MoveKind`
                        (the denominators);
- `src/steps/mod.rs`  — `FAMILY_OF` (which family owns each kind) and
                        `FAMILIES` (the per-family registries), ditto
                        `src/moves/mod.rs`;
- `src/steps/<family>.rs` — that family's `pub const IMPLEMENTED`, the
                        wave PR's own edit surface (PORT_PLAN.md D4/D6);
- `src/content_tables.rs` — the generated `CENSUS_INERT_RELICS` and effective
                        `TEMPLATE_RELIC_STEPS` identity partitions;
- `src/engine/admission.rs` — `IMPLEMENTED_RELICS`, the exact modeled-body
                        registry published by the capability manifest.

The parse is cross-checked against the array-length annotations Rust itself
enforces (`[(StepKind, &str); 333]`, `[(&str, &[StepKind]); 30]`), against
`StepKind::COUNT`, and against the family ownership table — a broken parse
raises instead of quietly reporting a wrong number.

A step or move listed in `IMPLEMENTED` has a claimed ported body. A relic in
`IMPLEMENTED_RELICS` has a complete modeled body. The source-derived family-
triage gate rejects a listed step/move body that directly names its own
`*KindNotModeled` refusal; these are structural claims, not execution evidence.

It is NOT guaranteed to be exercised. The differential's `must_cover`
(DIFFERENTIAL.md) does reject a claimed kind no trajectory reached, but since
the 2026-08-23 decision record it runs only in `rust-port.yml`'s
`Differential smoke` step, which is `continue-on-error` and therefore cannot
fail a build. So the "exercised" half is reported, not enforced, and this
step/move number should be read as ported-and-claimed. #1481 tracks the gap.
The relic denominator includes only combat-active identities. Census-inert
inventory support remains a separate axis; compiled template programs count
as implemented mechanics only when `IMPLEMENTED_RELICS` admits their identity.

Freshness (`--check`) and why it is not byte-identity
-----------------------------------------------------
PORT_PLAN.md §4 is explicit that a file every wave PR must edit "would put a
merge conflict in every wave PR" — that is why smoke configs are a glob'd
directory rather than one shared list. A committed aggregate badge is exactly
such a file, so requiring every wave PR to regenerate it would reintroduce
the conflict the layout was designed to eliminate (and a CONFLICTING PR gets
no workflow runs at all).

`--check` therefore guards the two directions that matter and tolerates only
the harmless one:

1. **Structure is exact.** Denominators, family lists and per-family owned
   counts must match the source. These move only when the generated dispatch
   moves — a codegen PR, never a parallel wave PR — so exactness is free.
2. **Never overstates.** The committed implemented set must be a subset of
   what the crate actually implements. The badge can lag reality; it can
   never claim a kind that has no body.
3. **Lag is bounded.** A committed count more than `LAG_BUDGET_FRACTION` of
   the total behind reality is red, so the banner cannot quietly rot for a
   whole wave. Fixing it is one command and a one-actor, three-file PR.

`--check --exact` demands byte-identity; that is what the regenerating PR
runs to confirm the tree it just wrote is idempotent.

Usage
-----
    python3 sim/v0.111.0/engine/tools/build_coverage_badge.py
    python3 sim/v0.111.0/engine/tools/build_coverage_badge.py --check
    python3 sim/v0.111.0/engine/tools/build_coverage_badge.py --check --exact
"""

from __future__ import annotations

import argparse
import html
import json
import math
import pathlib
import re
import sys

RUST = pathlib.Path(__file__).resolve().parent.parent
SRC = RUST / "src"

# How far the committed banner may lag the crate before `--check` goes red,
# as a fraction of that category's total kind count. Chosen so a wave PR
# never has to touch these artifacts (see the module docstring) while a
# refresh stays due every few merged waves.
LAG_BUDGET_FRACTION = 0.05

CATEGORIES = (
    # (key, enum name, dispatch subdirectory, badge label)
    ("steps", "StepKind", "steps", "steps"),
    ("moves", "MoveKind", "moves", "moves"),
)

RELIC_KEY = "relics"
RELIC_LABEL = "active relics"


class DeriveError(RuntimeError):
    """The source did not parse the way this tool requires."""


def _read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8")


def _array_body(text: str, header: str, where: str) -> str:
    """The `[...]` body of a `pub static NAME: [...] = [ ... ];` item."""
    start = text.find(header)
    if start < 0:
        raise DeriveError(f"{where}: no `{header}`")
    open_bracket = text.index("[", start + len(header))
    end = text.find("\n];", open_bracket)
    if end < 0:
        raise DeriveError(f"{where}: unterminated `{header}`")
    return text[open_bracket + 1:end]


def enum_total(enum: str) -> int:
    """`impl <enum> { pub const COUNT: usize = N; }` from the generated ids."""
    text = _read(SRC / "ids.rs")
    at = text.find(f"\nimpl {enum} {{")
    if at < 0:
        raise DeriveError(f"ids.rs: no `impl {enum}`")
    match = re.search(r"pub const COUNT: usize = (\d+);", text[at:])
    if match is None:
        raise DeriveError(f"ids.rs: no COUNT in `impl {enum}`")
    return int(match.group(1))


def enum_variants(enum: str) -> set[str]:
    """The complete generated discriminant set for one id enum."""
    text = _read(SRC / "ids.rs")
    start = text.find(f"pub enum {enum} {{")
    impl_at = text.find(f"\nimpl {enum} {{", start)
    if start < 0 or impl_at < 0:
        raise DeriveError(f"ids.rs: no complete `{enum}` declaration")
    variants = re.findall(
        r"^\s{4}([A-Za-z0-9_]+)\s*=\s*\d+,\s*$",
        text[start:impl_at],
        re.MULTILINE,
    )
    total = enum_total(enum)
    if len(variants) != total or len(set(variants)) != total:
        raise DeriveError(
            f"ids.rs: parsed {len(variants)} unique `{enum}` variants, "
            f"COUNT is {total}")
    return set(variants)


def _relic_array(
    path: pathlib.Path,
    opening_pattern: str,
    row_pattern: str,
    name: str,
) -> tuple[set[str], int | None]:
    """Parse one generated relic-id array and its optional declared length."""
    text = _read(path)
    opening = re.search(opening_pattern, text)
    if opening is None:
        raise DeriveError(f"{path.name}: no `{name}`")
    end = text.find("];", opening.end())
    if end < 0:
        raise DeriveError(f"{path.name}: unterminated `{name}`")
    rows = re.findall(row_pattern, text[opening.end():end], re.MULTILINE)
    unique = set(rows)
    if len(rows) != len(unique):
        raise DeriveError(f"{path.name}: duplicate relic in `{name}`")
    declared = int(opening.group(1)) if opening.lastindex else None
    if declared is not None and len(rows) != declared:
        raise DeriveError(
            f"{path.name}: parsed {len(rows)} `{name}` rows, "
            f"Rust declares {declared}")
    return unique, declared


def relic_coverage() -> dict:
    """Exact modeled bodies among combat-active relic identities."""
    known = enum_variants("RelicId")
    inert, inert_declared = _relic_array(
        SRC / "content_tables.rs",
        r"pub static CENSUS_INERT_RELICS:\s*\[RelicId;\s*(\d+)\]\s*=\s*\[",
        r"RelicId::([A-Za-z0-9_]+)",
        "CENSUS_INERT_RELICS",
    )
    template_only, template_declared = _relic_array(
        SRC / "content_tables.rs",
        r"pub static TEMPLATE_RELIC_STEPS:\s*"
        r"\[\(RelicId,\s*&\[RelicHook\]\);\s*(\d+)\]\s*=\s*\[",
        r"^\s*\(RelicId::([A-Za-z0-9_]+),\s*&\[",
        "TEMPLATE_RELIC_STEPS",
    )
    exact, _declared = _relic_array(
        SRC / "engine/admission.rs",
        r"pub const IMPLEMENTED_RELICS:\s*&\[RelicId\]\s*=\s*&\[",
        r"RelicId::([A-Za-z0-9_]+)",
        "IMPLEMENTED_RELICS",
    )
    for label, rows in (
        ("census-inert", inert),
        ("template-only", template_only),
        ("implemented", exact),
    ):
        unknown = sorted(rows - known)
        if unknown:
            raise DeriveError(
                f"{label} relic registry names unknown RelicId values: "
                f"{unknown}")
    if inert & template_only:
        raise DeriveError(
            "census-inert and effective template-only relic registries overlap")
    active = known - inert
    if not template_only <= active:
        raise DeriveError("effective template-only relic is not combat-active")
    if not exact <= active:
        raise DeriveError("implemented relic registry contains a census-inert id")
    hand_authored = active - template_only
    return {
        "axis": "combat-active exact bodies",
        "enum": "RelicId",
        "known_total": len(known),
        "inert_total": inert_declared,
        "template_only_total": template_declared,
        "hand_authored_total": len(hand_authored),
        "total": len(active),
        "implemented": len(exact),
        "template_implemented": len(exact & template_only),
        "exact_ids": sorted(exact),
    }


def owners(enum: str, subdir: str, total: int) -> dict[str, str]:
    """kind -> owning family, from the generated `FAMILY_OF` table."""
    where = f"{subdir}/mod.rs"
    text = _read(SRC / subdir / "mod.rs")
    header = f"pub static FAMILY_OF: [({enum}, &str);"
    declared = re.search(re.escape(header) + r"\s*(\d+)\]", text)
    if declared is None:
        raise DeriveError(f"{where}: no `FAMILY_OF` length annotation")
    body = _array_body(text, header, where)
    pairs = re.findall(rf"\(\s*{enum}::(\w+)\s*,\s*\"(\w+)\"\s*\)", body)
    table = dict(pairs)
    if len(pairs) != len(table):
        raise DeriveError(f"{where}: duplicate kind in `FAMILY_OF`")
    if len(table) != int(declared.group(1)):
        raise DeriveError(
            f"{where}: parsed {len(table)} `FAMILY_OF` rows, "
            f"Rust declares {declared.group(1)}")
    if len(table) != total:
        raise DeriveError(
            f"{where}: `FAMILY_OF` covers {len(table)} kinds, "
            f"`{enum}::COUNT` is {total}")
    return table


def families(enum: str, subdir: str) -> list[str]:
    """The family modules, in the order `FAMILIES` concatenates them."""
    where = f"{subdir}/mod.rs"
    text = _read(SRC / subdir / "mod.rs")
    header = f"pub static FAMILIES: [(&str, &[{enum}]);"
    declared = re.search(re.escape(header) + r"\s*(\d+)\]", text)
    if declared is None:
        raise DeriveError(f"{where}: no `FAMILIES` length annotation")
    body = _array_body(text, header, where)
    names = re.findall(r"\(\s*\"(\w+)\"\s*,\s*(\w+)::IMPLEMENTED\s*\)", body)
    for name, module in names:
        if name != module:
            raise DeriveError(
                f"{where}: family {name!r} registered from module {module!r}")
    if len(names) != int(declared.group(1)):
        raise DeriveError(
            f"{where}: parsed {len(names)} families, "
            f"Rust declares {declared.group(1)}")
    return [name for name, _module in names]


def implemented(enum: str, subdir: str, family: str) -> list[str]:
    """That family's own `IMPLEMENTED` registry — the wave PR's edit surface."""
    where = f"{subdir}/{family}.rs"
    text = _read(SRC / subdir / f"{family}.rs")
    # rustfmt may break the initializer onto its own line, so match loosely.
    opening = re.search(
        rf"pub const IMPLEMENTED:\s*&\[{enum}\]\s*=\s*&\[", text)
    if opening is None:
        raise DeriveError(f"{where}: no `IMPLEMENTED`")
    end = text.find("];", opening.end())
    if end < 0:
        raise DeriveError(f"{where}: unterminated `IMPLEMENTED`")
    kinds = re.findall(rf"{enum}::(\w+)", text[opening.end():end])
    if len(set(kinds)) != len(kinds):
        raise DeriveError(f"{where}: duplicate kind in `IMPLEMENTED`")
    return kinds


def derive() -> dict:
    """The live ledger: what this checkout of the crate implements."""
    ledger: dict[str, dict] = {"schema": "rust-port-coverage-v1",
                               "categories": {}}
    for key, enum, subdir, _label in CATEGORIES:
        total = enum_total(enum)
        owner_of = owners(enum, subdir, total)
        owned: dict[str, int] = {}
        for kind, family in owner_of.items():
            owned[family] = owned.get(family, 0) + 1
        rows = {}
        seen: set[str] = set()
        for family in families(enum, subdir):
            kinds = implemented(enum, subdir, family)
            for kind in kinds:
                if kind not in owner_of:
                    raise DeriveError(
                        f"{subdir}/{family}.rs: `IMPLEMENTED` names {kind}, "
                        "which `FAMILY_OF` does not know")
                if owner_of[kind] != family:
                    raise DeriveError(
                        f"{subdir}/{family}.rs: claims {kind}, owned by "
                        f"{owner_of[kind]!r}")
                if kind in seen:
                    raise DeriveError(f"{subdir}: {kind} claimed twice")
                seen.add(kind)
            rows[family] = {"implemented": sorted(kinds),
                            "owned": owned.get(family, 0)}
        ledger["categories"][key] = {
            "enum": enum,
            "total": total,
            "implemented": len(seen),
            "families": rows,
        }
    ledger["categories"][RELIC_KEY] = relic_coverage()
    return ledger


# ---------------------------------------------------------------- rendering

def _coverage_color(covered: int, total: int) -> str:
    """The solver banner's thresholds, so the two badges read alike."""
    pct = (covered / total) if total else 1.0
    if pct >= .75:
        return "#4c1"
    if pct >= .5:
        return "#97ca00"
    if pct >= .25:
        return "#dfb317"
    return "#fe7d37"


def render_svg(ledger: dict) -> str:
    """Render one self-contained, branch-relative coverage badge strip."""
    segments = []
    for key, _enum, _subdir, label in CATEGORIES:
        category = ledger["categories"][key]
        value = f"{category['implemented']}/{category['total']}"
        label_width = max(54, len(label) * 7 + 16)
        value_width = max(52, len(value) * 7 + 16)
        segments.append((label, value, label_width, value_width,
                         _coverage_color(category["implemented"],
                                         category["total"])))
    relics = ledger["categories"][RELIC_KEY]
    relic_value = f"{relics['implemented']}/{relics['total']}"
    segments.append((
        RELIC_LABEL,
        relic_value,
        max(54, len(RELIC_LABEL) * 7 + 16),
        max(52, len(relic_value) * 7 + 16),
        _coverage_color(relics["implemented"], relics["total"]),
    ))

    width = sum(label_width + value_width
                for _label, _value, label_width, value_width, _color
                in segments)
    title = "Rust port coverage: " + ", ".join(
        f"{label} {value}" for label, value, *_rest in segments)
    parts = [
        '<svg xmlns="http://www.w3.org/2000/svg" '
        f'width="{width}" height="28" role="img" '
        f'aria-label="{html.escape(title)}">',
        f"  <title>{html.escape(title)}</title>",
        "  <defs>",
        f'    <clipPath id="round"><rect width="{width}" height="28" rx="4"/></clipPath>',
        "  </defs>",
        '  <g clip-path="url(#round)">',
    ]
    x = 0
    texts = []
    for label, value, label_width, value_width, color in segments:
        parts.append(f'    <rect x="{x}" width="{label_width}" height="28" fill="#555"/>')
        texts.append((x + label_width / 2, label))
        x += label_width
        parts.append(f'    <rect x="{x}" width="{value_width}" height="28" fill="{color}"/>')
        texts.append((x + value_width / 2, value))
        x += value_width
    parts.extend([
        "  </g>",
        '  <g fill="#fff" text-anchor="middle" '
        'font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">',
    ])
    for center, text in texts:
        parts.append(f'    <text x="{center:g}" y="18">{html.escape(text)}</text>')
    parts.extend(["  </g>", "</svg>", ""])
    return "\n".join(parts)


def _pct(part: int, whole: int) -> str:
    return f"{(100.0 * part / whole) if whole else 100.0:.1f}%"


def render_markdown(ledger: dict) -> str:
    lines = [
        "# Rust port coverage",
        "",
        "<!-- Generated by tools/build_coverage_badge.py; do not edit. -->",
        "",
        "How far the v0.111.0 Rust port (`sts-sim`, PORT_PLAN.md) has gotten",
        "through the dispatch surface it generated complete and fills body by",
        "body. Every number below is read out of source registries used by",
        "`sts_sim::engine::admission::capability_manifest`: per-family",
        "`IMPLEMENTED` constants, generated ownership tables, and",
        "`IMPLEMENTED_RELICS`. The active relic denominator is derived from",
        "the generated inert census, so this",
        "page cannot disagree with the crate. Regenerate with:",
        "",
        "```sh",
        "python3 sim/v0.111.0/engine/tools/build_coverage_badge.py",
        "```",
        "",
        "For steps and moves, **Implemented** means the kind has a claimed",
        "ported body. For active relics it means a complete modeled body in",
        "`IMPLEMENTED_RELICS`. The source-derived family-triage gate rejects",
        "a listed body that directly names its own `*KindNotModeled` refusal;",
        "this is structural, not execution evidence. It does *not* mean the",
        "kind was exercised — the",
        "differential's `must_cover` check (DIFFERENTIAL.md) does reject a",
        "claimed kind no trajectory reached, but it runs only in the",
        "`Differential smoke` step, which is `continue-on-error` and cannot",
        "fail a build (2026-08-23 decision record). Read step/move counts as",
        "ported-and-claimed; #1481 tracks the gap. **Owned** is how many",
        "kinds the generated dispatch assigns to that family — the family's own",
        "denominator. Families are the port's unit of work (PORT_PLAN.md D4):",
        "one wave PR fills stubs in exactly one family file.",
        "",
        "This page lags rather than blocks: a wave PR does not regenerate it",
        "(that would put a shared file, and so a merge conflict, in every wave",
        "PR — PORT_PLAN.md §4). CI instead asserts the structure exactly, that",
        "the committed counts never overstate the crate, and that they never",
        "fall more than "
        f"{LAG_BUDGET_FRACTION:.0%} of the total behind it.",
        "",
        "## Overall",
        "",
        "| Surface | Implemented | Total | Families |",
        "|---|---:|---:|---:|",
    ]
    for key, _enum, _subdir, label in CATEGORIES:
        category = ledger["categories"][key]
        lines.append(
            f"| {label.capitalize()} | {category['implemented']} "
            f"({_pct(category['implemented'], category['total'])}) "
            f"| {category['total']} | {len(category['families'])} |")
    relics = ledger["categories"][RELIC_KEY]
    lines.append(
        f"| Active relics | {relics['implemented']} "
        f"({_pct(relics['implemented'], relics['total'])}) "
        f"| {relics['total']} | — |")
    lines.append(
        f"| Inert relics | {relics['inert_total']} (100.0%) "
        f"| {relics['inert_total']} | — |")
    lines.append(
        f"| Template relics | {relics['template_implemented']} "
        f"({_pct(relics['template_implemented'], relics['template_only_total'])}) "
        f"| {relics['template_only_total']} | — |")
    for key, _enum, subdir, label in CATEGORIES:
        category = ledger["categories"][key]
        lines.extend([
            "",
            f"## {label.capitalize()} by family (`src/{subdir}/`)",
            "",
            "| Family | Implemented | Owned |",
            "|---|---:|---:|",
        ])
        for family, row in sorted(category["families"].items()):
            count = len(row["implemented"])
            lines.append(
                f"| `{family}` | {count} ({_pct(count, row['owned'])}) "
                f"| {row['owned']} |")
    lines.extend([
        "",
        "## Relics by axis",
        "",
        "| Axis | Implemented | Total | Admitted | Details |",
        "|---|---:|---:|---:|---|",
        f"| Active exact bodies | {relics['implemented']} ({_pct(relics['implemented'], relics['total'])}) | {relics['total']} | {relics['implemented']} | Exact identities are published in `coverage.json` |",
        f"| Census-inert inventory | {relics['inert_total']} (100.0%) | {relics['inert_total']} | {relics['inert_total']} | Inventory-exact round trip, admitted |",
        f"| Template programs | {relics['template_implemented']} ({_pct(relics['template_implemented'], relics['template_only_total'])}) | {relics['template_only_total']} | {relics['template_implemented']} | Compiled {relics['template_only_total']} / modeled {relics['template_implemented']} / admitted {relics['template_implemented']} |",
        "",
        f"The complete relic inventory comprises {relics['known_total']} unique identities:",
        f"- **{relics['implemented'] + relics['inert_total']} admitted** ({relics['implemented']} active + {relics['inert_total']} census-inert)",
        f"- **{relics['total'] - relics['implemented']} typed-refused** unported active relics",
        f"- 34 raw template programs ({relics['template_only_total']} effective template-only + 16 overlapping hand authority)",
    ])
    lines.extend([
        "",
        "## Where the frontier is tracked",
        "",
        "- [`PORT_PLAN.md`](PORT_PLAN.md) — the architecture and wave process.",
        "- [`DIFFERENTIAL.md`](DIFFERENTIAL.md) — the gate a wave PR runs under.",
        "- [Issue #1282](https://github.com/sansb/StsHistoryViewer/issues/1282)",
        "  is the port's umbrella.",
        "",
    ])
    return "\n".join(lines)


# ------------------------------------------------------------------- checks

def _budget(total: int) -> int:
    return math.ceil(total * LAG_BUDGET_FRACTION)


def check(live: dict, exact: bool) -> list[str]:
    """Complaints about the committed artifacts, empty when they are fine."""
    problems: list[str] = []
    try:
        pinned = json.loads(_read(RUST / "coverage.json"))
    except FileNotFoundError:
        return ["coverage.json is missing; run the generator"]
    except json.JSONDecodeError as exc:
        return [f"coverage.json does not parse: {exc}"]

    pinned_categories = pinned.get("categories")
    if not isinstance(pinned_categories, dict):
        return ["coverage.json has no categories object"]
    missing = [
        key for key in (*[row[0] for row in CATEGORIES], RELIC_KEY)
        if key not in pinned_categories
    ]
    if missing:
        return [f"coverage.json has no {key} category" for key in missing]
    if not isinstance(pinned_categories[RELIC_KEY], dict):
        return ["coverage.json relics category is not an object"]

    # The three artifacts are one unit: md/svg must be renderings of the json.
    for name, rendered in (("COVERAGE.md", render_markdown(pinned)),
                           ("coverage.svg", render_svg(pinned))):
        try:
            if _read(RUST / name) != rendered:
                problems.append(
                    f"{name} is not the rendering of coverage.json")
        except FileNotFoundError:
            problems.append(f"{name} is missing; run the generator")
    if problems:
        return problems

    if exact:
        if pinned != live:
            problems.append(
                "coverage.json is not a fresh derivation "
                "(--exact demands byte-identity)")
        return problems

    if pinned.get("schema") != live["schema"]:
        return [f"coverage.json schema is {pinned.get('schema')!r}, "
                f"expected {live['schema']!r}"]

    for key, _enum, subdir, label in CATEGORIES:
        live_cat = live["categories"][key]
        pinned_cat = pinned.get("categories", {}).get(key)
        if pinned_cat is None:
            problems.append(f"coverage.json has no {key} category")
            continue
        # 1. structure is exact
        if pinned_cat.get("total") != live_cat["total"]:
            problems.append(
                f"{label}: committed total {pinned_cat.get('total')} != "
                f"{live_cat['enum']}::COUNT {live_cat['total']}")
        if sorted(pinned_cat.get("families", {})) != sorted(live_cat["families"]):
            problems.append(
                f"{label}: committed family list != src/{subdir}/mod.rs "
                "FAMILIES")
        else:
            for family, live_row in live_cat["families"].items():
                pinned_row = pinned_cat["families"][family]
                if pinned_row.get("owned") != live_row["owned"]:
                    problems.append(
                        f"{label}/{family}: committed owned "
                        f"{pinned_row.get('owned')} != FAMILY_OF's "
                        f"{live_row['owned']}")
                # 2. never overstates
                extra = sorted(set(pinned_row.get("implemented", []))
                               - set(live_row["implemented"]))
                if extra:
                    problems.append(
                        f"{label}/{family}: committed banner claims kinds the "
                        f"crate does not implement: {extra}")
        # 3. lag is bounded
        lag = live_cat["implemented"] - pinned_cat.get("implemented", 0)
        budget = _budget(live_cat["total"])
        if lag > budget:
            problems.append(
                f"{label}: banner is {lag} kinds behind the crate "
                f"({pinned_cat.get('implemented')} committed, "
                f"{live_cat['implemented']} implemented); budget is {budget}")
    live_relics = live["categories"][RELIC_KEY]
    pinned_relics = pinned_categories[RELIC_KEY]
    structural_fields = (
        "axis",
        "enum",
        "known_total",
        "inert_total",
        "template_only_total",
        "hand_authored_total",
        "total",
    )
    for field in structural_fields:
        if pinned_relics.get(field) != live_relics[field]:
            problems.append(
                f"{RELIC_LABEL}: committed {field} "
                f"{pinned_relics.get(field)!r} != source-derived "
                f"{live_relics[field]!r}")
    pinned_exact_rows = pinned_relics.get("exact_ids")
    if (not isinstance(pinned_exact_rows, list)
            or not all(isinstance(row, str) for row in pinned_exact_rows)):
        problems.append(
            f"{RELIC_LABEL}: committed exact_ids is not a string list")
        pinned_exact_rows = []
    pinned_exact = set(pinned_exact_rows)
    live_exact = set(live_relics["exact_ids"])
    if len(pinned_exact) != len(pinned_exact_rows):
        problems.append(f"{RELIC_LABEL}: committed exact_ids contains duplicates")
    extra = sorted(pinned_exact - live_exact)
    if extra:
        problems.append(
            f"{RELIC_LABEL}: committed badge claims relics the capability "
            f"manifest does not implement: {extra}")
    if pinned_relics.get("implemented") != len(pinned_exact):
        problems.append(
            f"{RELIC_LABEL}: committed implemented count does not match "
            "exact_ids")
    lag = live_relics["implemented"] - pinned_relics.get("implemented", 0)
    budget = _budget(live_relics["total"])
    if lag > budget:
        problems.append(
            f"{RELIC_LABEL}: banner is {lag} identities behind the crate "
            f"({pinned_relics.get('implemented')} committed, "
            f"{live_relics['implemented']} implemented); budget is {budget}")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check", action="store_true",
        help="verify the committed artifacts instead of rewriting them")
    parser.add_argument(
        "--exact", action="store_true",
        help="with --check, demand a byte-identical fresh derivation")
    args = parser.parse_args(argv)

    try:
        live = derive()
    except DeriveError as exc:
        print(f"cannot derive coverage: {exc}", file=sys.stderr)
        return 2

    if args.check:
        problems = check(live, args.exact)
        for problem in problems:
            print(f"coverage badge: {problem}", file=sys.stderr)
        if problems:
            print("regenerate with "
                  "`python3 sim/v0.111.0/engine/tools/"
                  "build_coverage_badge.py`", file=sys.stderr)
            return 1
        print("coverage badge is fresh enough")
        return 0

    (RUST / "coverage.json").write_text(
        json.dumps(live, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    (RUST / "COVERAGE.md").write_text(render_markdown(live), encoding="utf-8")
    (RUST / "coverage.svg").write_text(render_svg(live), encoding="utf-8")
    print("wrote sim/v0.111.0/engine/"
          "{coverage.json,COVERAGE.md,coverage.svg}")
    for key, _enum, _subdir, label in CATEGORIES:
        category = live["categories"][key]
        print(f"  {label:6s} {category['implemented']:4d}/{category['total']:<4d}"
              f" across {len(category['families'])} families")
    relics = live["categories"][RELIC_KEY]
    print(
        f"  {RELIC_LABEL:13s} {relics['implemented']:4d}/"
        f"{relics['total']:<4d}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

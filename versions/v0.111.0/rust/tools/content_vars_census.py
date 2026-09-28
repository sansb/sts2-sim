#!/usr/bin/env python3
"""The per-build canonical-vars manifest the crate's amounts are checked against.

Issue #3027. The audit that issue ran found three defect classes in the
crate's hand-carried content numbers, each of which slipped past review:

* **Bone Flute** (#2830): `BlockVar(2m, ValueProp.Unpowered)` — the `props`
  argument (4) was read as the amount, and a bare `4` lived in
  `engine/relics.rs`.
* **Fetch** (#2519): a correct *value* in the wrong *role* — the draw count
  was a literal 3, which happens to equal Fetch's `OstyDamage` 3, while the
  card's `CardsVar` is 1.
* **Misery+** (#3036): the upgrade's keyword was Exhaust in the tables; the
  game's `OnUpgrade` adds Retain.

This tool records, per build, what the game itself says those numbers are, so
the crate can be checked against them mechanically
(`tests/canonical_vars_manifest.rs`, `tests/amount_literal_scan.rs`).

What it records
---------------
For every card, relic and potion in `ModelDb`:

* each `DynamicVar` in the model's `DynamicVars` set — name, exact
  `System.Decimal` `BaseValue` as its invariant-culture text (never
  `IntValue`, which truncates Bowler Hat's 1.25), the var's runtime class, and
  its `ValueProp` flags when the class carries `Props`;
* for cards, one snapshot per upgrade level (`MutableClone`, then
  `UpgradeInternal` level by level — the game's own upgrade path, so the
  snapshots are the `get_CanonicalVars` constructor values with every
  `OnUpgrade` delta applied): the vars, the energy cost
  (`CardEnergyCost._base`), `HasEnergyCostX`, `BaseStarCost` (-1 when the card
  has none), `HasStarCostX`, and `CardModel.Keywords` by enum name;
* the IL site of each model's `get_CanonicalVars`, `OnUpgrade` and
  `get_CanonicalKeywords` — declaring class and RVA, resolved along the base
  chain — so an allowlist row can cite the method whose bytes it rests on.

How it reads them
-----------------
The values come from the game's own code, not a static parse: the headless
harness (`versions/v0.111.0/solver/harness/`, #275) boots `sts2.dll` in a bare
CoreCLR and instantiates every model through `ModelDb`, exactly as
`tools/extract_values.py` does for the site. That makes the upgrade snapshots
the result of running `OnUpgrade`, which a static reader would have to
re-implement (`UpgradeValueBy`, `AddKeyword`, `EnergyCost.UpgradeBy`, star-cost
upgrades) and could get wrong in the same way the Bone Flute read did. The
RVAs come from a static `dnfile` read of the same assembly
(`build_mcr_tables.Dll`, the reader `dll_content.py` uses).

The harness is pinned to the archived certified assembly (`STS2_GAME_DIR` is
set to `solver/dll-archive/v0.111.0/data_sts2_macos_arm64`), and the sha256 the
harness actually loaded is checked before anything is written.

Hermetic check (the #2515 precedent)
------------------------------------
Reading the assembly is local-only; CI has no DLL, no .NET runtime and no
`dnfile`. So `--check`:

* always verifies the committed manifest: schema, the `self_sha256` over its
  payload, byte-stable re-serialization, and structural sanity (every card has
  at least one level, every level carries every column, values parse as
  decimals);
* when the harness and the archived certified DLL are both present,
  re-extracts and requires the committed bytes to match;
* otherwise prints an explicit SKIP for the re-extraction. `--require-dll`
  makes that a failure instead (the version-bump runbook's mode).

Usage::

    python3 versions/v0.111.0/rust/tools/content_vars_census.py --check
    python3 versions/v0.111.0/rust/tools/content_vars_census.py --write

`--write` needs the harness venv (`pythonnet`) and `python3.12` (`dnfile`) on
this host; it runs each phase under the interpreter that has its dependency.
"""

from __future__ import annotations

import argparse
import decimal
import hashlib
import json
import os
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
BUILD_DIR = HERE.parents[2]
REPO_ROOT = HERE.parents[4]

CERTIFIED_BUILD = "v0.111.0"
CERTIFIED_DLL_SHA256 = (
    "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4")
MANIFEST_SCHEMA = "sts-sim-canonical-vars-manifest/v1"
MANIFEST_PATH = RUST_DIR / "data" / f"canonical_vars.{CERTIFIED_BUILD}.json"
GENERATED_BY = (f"versions/{CERTIFIED_BUILD}/rust/tools/"
                "content_vars_census.py --write")

#: Columns every card level carries.
LEVEL_COLUMNS = ("cost", "keywords", "star_cost", "star_x", "vars", "x_cost")
#: Columns every var carries (`props` only when the var class has `Props`).
VAR_COLUMNS = ("type", "value")
#: The IL methods whose declaring site is recorded per model.
IL_METHODS = ("get_CanonicalVars", "OnUpgrade", "get_CanonicalKeywords")
FAMILIES = ("cards", "potions", "relics")


def _main_checkout():
    """The main working tree, seen from a worktree (or this tree itself)."""
    git_path = REPO_ROOT / ".git"
    if git_path.is_dir():
        return REPO_ROOT
    try:
        pointer = git_path.read_text().split("gitdir:", 1)[1].strip()
    except (OSError, IndexError):
        return REPO_ROOT
    for parent in pathlib.Path(pointer).resolve().parents:
        if parent.name == ".git":
            return parent.parent
    return REPO_ROOT


def archived_game_dir():
    """The archived certified build's data dir, or `None` on this host."""
    for root in (REPO_ROOT, _main_checkout()):
        path = (root / "solver" / "dll-archive" / CERTIFIED_BUILD
                / "data_sts2_macos_arm64")
        if (path / "sts2.dll").exists():
            return path
    return None


def harness_dir():
    """A harness directory with its .NET runtime and venv, or `None`."""
    for root in (REPO_ROOT, _main_checkout()):
        path = root / "versions" / CERTIFIED_BUILD / "solver" / "harness"
        if (path / "dotnet").is_dir() and (path / "venv" / "bin"
                                           / "python3").exists():
            return path
    return None


def dnfile_python():
    """An interpreter with `dnfile`, or `None` (python3.12 on this Mac)."""
    for candidate in (os.environ.get("STS_DNFILE_PYTHON"),
                      "/usr/local/bin/python3.12", sys.executable):
        if not candidate or not pathlib.Path(candidate).exists():
            continue
        probe = subprocess.run([candidate, "-c", "import dnfile, dncil"],
                               capture_output=True)
        if probe.returncode == 0:
            return candidate
    return None


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


# ---------------------------------------------------------------------------
# Phase 1: live values through the headless harness (pythonnet)
# ---------------------------------------------------------------------------

def _harness_dump():
    """Every model's vars and per-level card columns, read from the engine."""
    harness = pathlib.Path(os.environ["STS_HARNESS_DIR"])
    sys.path.insert(0, str(harness))
    import experiment  # noqa: E402  (pythonnet; harness venv only)
    import host  # noqa: E402
    from host import ALL, asm  # noqa: E402
    from System.Globalization import CultureInfo  # noqa: E402

    invariant = CultureInfo.InvariantCulture
    loaded = host.GAME / "sts2.dll"
    experiment._boot_and_init()

    abstract = asm.GetType("MegaCrit.Sts2.Core.Models.AbstractModel")
    card_model = asm.GetType("MegaCrit.Sts2.Core.Models.CardModel")
    model_db = asm.GetType("MegaCrit.Sts2.Core.Models.ModelDb")

    def meth(t, name):
        m = t.GetMethod(name, ALL)
        assert m is not None, f"{t.Name}.{name}"
        return m

    m_id = meth(abstract, "get_Id")
    m_clone = meth(abstract, "MutableClone")
    m_up = meth(card_model, "UpgradeInternal")
    m_ec = meth(card_model, "get_EnergyCost")
    m_ecx = meth(card_model, "get_HasEnergyCostX")
    m_maxup = meth(card_model, "get_MaxUpgradeLevel")
    m_star = meth(card_model, "get_BaseStarCost")
    m_starx = meth(card_model, "get_HasStarCostX")
    m_kw = meth(card_model, "get_Keywords")

    def db_all(prop):
        return list(model_db.GetProperty(prop, ALL).GetValue(None))

    def dyn_vars(model):
        prop, t = None, model.GetType()
        while t is not None and prop is None:
            prop = t.GetProperty("DynamicVars", ALL)
            t = t.BaseType
        dv = prop.GetValue(model)
        if dv is None:
            return {}
        dvt = dv.GetType()
        item = dvt.GetMethod("get_Item", ALL)
        out = {}
        for key in [str(k) for k in dvt.GetMethod("get_Keys", ALL).Invoke(
                dv, None)]:
            var = item.Invoke(dv, [key])
            vt = var.GetType()
            base = vt.GetProperty("BaseValue").GetValue(var)
            row = {"type": str(vt.Name),
                   "value": str(base.ToString(invariant))}
            props = vt.GetProperty("Props", ALL)
            if props is not None:
                row["props"] = int(props.GetValue(var))
            out[key] = row
        return out

    def card_level(card):
        ec = m_ec.Invoke(card, None)
        star = m_star.Invoke(card, None)
        keywords = m_kw.Invoke(card, None)
        return {
            "cost": int(ec.GetType().GetField("_base", ALL).GetValue(ec)),
            "keywords": sorted(str(k.ToString()) for k in keywords)
            if keywords is not None else [],
            "star_cost": int(star) if star is not None else -1,
            "star_x": bool(m_starx.Invoke(card, None)),
            "vars": dyn_vars(card),
            "x_cost": bool(m_ecx.Invoke(card, None)),
        }

    out = {"loaded_dll": str(loaded), "cards": {}, "relics": {},
           "potions": {}}
    for canonical in db_all("AllCards"):
        clone = m_clone.Invoke(canonical, None)
        levels = [card_level(clone)]
        for _ in range(int(m_maxup.Invoke(clone, None))):
            m_up.Invoke(clone, None)
            levels.append(card_level(clone))
        out["cards"][str(m_id.Invoke(canonical, None).ToString())] = {
            "class": str(canonical.GetType().FullName),
            "levels": levels,
        }
    for family, prop in (("relics", "AllRelics"), ("potions", "AllPotions")):
        for model in db_all(prop):
            out[family][str(m_id.Invoke(model, None).ToString())] = {
                "class": str(model.GetType().FullName),
                "vars": dyn_vars(model),
            }
    # The harness logs to stdout, so the dump goes to its own file.
    pathlib.Path(os.environ["STS_DUMP_OUT"]).write_text(
        json.dumps(out, sort_keys=True))


# ---------------------------------------------------------------------------
# Phase 2: IL sites through dnfile
# ---------------------------------------------------------------------------

def _il_sites(classes_path):
    """{full class name: {method: [declaring class, "0xRVA"]}} via dnfile."""
    sys.path.insert(0, str(BUILD_DIR / "solver" / "tools"))
    import build_mcr_tables as mcr  # noqa: E402

    dll = mcr.Dll(str(archived_game_dir() / "sts2.dll"))
    rid_by_name = {}
    for rid in range(1, len(dll.typedefs) + 1):
        rid_by_name.setdefault(dll.full_name(rid), rid)

    def methods(rid):
        return {str(m.row.Name): m.row.Rva
                for m in dll.typedefs[rid - 1].MethodList}

    wanted = json.loads(pathlib.Path(classes_path).read_text())
    out = {}
    for full in wanted:
        rid = rid_by_name.get(full)
        if rid is None:
            raise SystemExit(f"{full}: no TypeDef in the assembly")
        sites = {}
        for method in IL_METHODS:
            cursor = rid
            while cursor is not None:
                own = methods(cursor)
                if method in own and own[method]:
                    sites[method] = [dll.type_name(cursor)[0],
                                     f"0x{own[method]:x}"]
                    break
                cursor = dll.base_typedef_rid(cursor)
        out[full] = sites
    json.dump(out, sys.stdout, sort_keys=True)


# ---------------------------------------------------------------------------
# The manifest
# ---------------------------------------------------------------------------

def self_hash(manifest):
    payload = {k: v for k, v in manifest.items() if k != "self_sha256"}
    return hashlib.sha256(json.dumps(
        payload, sort_keys=True, separators=(",", ":"),
        ensure_ascii=False).encode("utf-8")).hexdigest()


def manifest_text(manifest):
    return json.dumps(manifest, sort_keys=True, indent=1,
                      ensure_ascii=False) + "\n"


def extract():
    """Run both phases and return the manifest dict. Needs the full host."""
    game = archived_game_dir()
    harness = harness_dir()
    dnpy = dnfile_python()
    missing = [name for name, have in (("archived v0.111.0 DLL", game),
                                        ("harness dotnet+venv", harness),
                                        ("python with dnfile", dnpy))
               if have is None]
    if missing:
        raise FileNotFoundError(", ".join(missing))
    sha = sha256_of(game / "sts2.dll")
    if sha != CERTIFIED_DLL_SHA256:
        raise SystemExit(f"{game}/sts2.dll has sha256 {sha}, not the "
                         f"certified {CERTIFIED_BUILD} assembly")
    dump_path = pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / (
        f"content_vars_dump.{os.getpid()}.json")
    env = dict(os.environ, STS2_GAME_DIR=str(game),
               STS_HARNESS_DIR=str(harness), STS_DUMP_OUT=str(dump_path))
    try:
        run = subprocess.run(
            [str(harness / "venv" / "bin" / "python3"), str(HERE),
             "--_harness-dump"], env=env, capture_output=True, text=True,
            cwd=str(harness))
        if run.returncode != 0:
            raise SystemExit(f"harness dump failed:\n"
                             f"{run.stdout[-2000:]}{run.stderr[-4000:]}")
        live = json.loads(dump_path.read_text())
    finally:
        dump_path.unlink(missing_ok=True)
    if pathlib.Path(live["loaded_dll"]).resolve() != (
            game / "sts2.dll").resolve():
        raise SystemExit(f"harness loaded {live['loaded_dll']}, not {game}")

    classes = sorted({row["class"] for fam in FAMILIES
                      for row in live[fam].values()})
    classes_path = pathlib.Path(os.environ.get("TMPDIR", "/tmp")) / (
        f"content_vars_classes.{os.getpid()}.json")
    classes_path.write_text(json.dumps(classes))
    try:
        run = subprocess.run([dnpy, str(HERE), "--_il-sites",
                              str(classes_path)],
                             capture_output=True, text=True)
    finally:
        classes_path.unlink(missing_ok=True)
    if run.returncode != 0:
        raise SystemExit(f"IL site read failed:\n{run.stderr[-4000:]}")
    sites = json.loads(run.stdout)

    body = {"schema": MANIFEST_SCHEMA, "build": CERTIFIED_BUILD,
            "provenance": {"dll_sha256": sha, "generated_by": GENERATED_BY,
                           "values": "headless harness: ModelDb instances, "
                                     "MutableClone + UpgradeInternal per level",
                           "il_sites": "dnfile TypeDef/MethodDef RVAs"}}
    for fam in FAMILIES:
        body[fam] = {}
        for model_id, row in sorted(live[fam].items()):
            entry = dict(row)
            entry["il"] = sites[row["class"]]
            body[fam][model_id] = entry
    body["self_sha256"] = self_hash(body)
    return body


def verify(manifest, text):
    """Hermetic structural checks. Returns a list of problems."""
    problems = []
    if manifest.get("schema") != MANIFEST_SCHEMA:
        problems.append(f"schema {manifest.get('schema')!r}")
    if manifest.get("build") != CERTIFIED_BUILD:
        problems.append(f"build {manifest.get('build')!r}")
    if manifest.get("provenance", {}).get("dll_sha256") != \
            CERTIFIED_DLL_SHA256:
        problems.append("provenance.dll_sha256 is not the certified DLL")
    if manifest.get("self_sha256") != self_hash(manifest):
        problems.append("self_sha256 does not match the payload")
    if manifest_text(manifest) != text:
        problems.append("committed bytes are not the canonical serialization")

    def check_vars(where, vars_):
        for name, var in vars_.items():
            for col in VAR_COLUMNS:
                if col not in var:
                    problems.append(f"{where} var {name} lacks {col}")
            try:
                decimal.Decimal(var.get("value"))
            except (decimal.InvalidOperation, TypeError):
                problems.append(f"{where} var {name} value {var.get('value')!r}")

    for fam in FAMILIES:
        rows = manifest.get(fam)
        if not rows:
            problems.append(f"{fam} is empty")
            continue
        for model_id, row in rows.items():
            if "il" not in row or "class" not in row:
                problems.append(f"{model_id} lacks class/il")
            if fam == "cards":
                if not row.get("levels"):
                    problems.append(f"{model_id} has no levels")
                for level, snap in enumerate(row.get("levels", [])):
                    for col in LEVEL_COLUMNS:
                        if col not in snap:
                            problems.append(f"{model_id}@{level} lacks {col}")
                    check_vars(f"{model_id}@{level}", snap.get("vars", {}))
            else:
                check_vars(model_id, row.get("vars", {}))
    return problems


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--_harness-dump", action="store_true",
                      help=argparse.SUPPRESS)
    mode.add_argument("--_il-sites", metavar="CLASSES_JSON",
                      help=argparse.SUPPRESS)
    ap.add_argument("--require-dll", action="store_true",
                    help="with --check: fail instead of skipping the "
                         "re-extraction when the DLL or harness is absent")
    args = ap.parse_args(argv)

    if args._harness_dump:
        _harness_dump()
        return 0
    if args._il_sites:
        _il_sites(args._il_sites)
        return 0
    if args.write:
        manifest = extract()
        MANIFEST_PATH.write_text(manifest_text(manifest))
        print(f"wrote {MANIFEST_PATH.relative_to(REPO_ROOT)}: "
              + ", ".join(f"{len(manifest[f])} {f}" for f in FAMILIES))
        return 0

    text = MANIFEST_PATH.read_text()
    manifest = json.loads(text)
    problems = verify(manifest, text)
    if problems:
        for problem in problems:
            print(f"FAIL {problem}")
        return 1
    print(f"ok  {MANIFEST_PATH.name}: hermetic checks pass ("
          + ", ".join(f"{len(manifest[f])} {f}" for f in FAMILIES) + ")")
    try:
        fresh = extract()
    except FileNotFoundError as missing:
        if args.require_dll:
            print(f"FAIL re-extraction needs: {missing}")
            return 1
        print(f"SKIP re-extraction (not on this host: {missing})")
        return 0
    if manifest_text(fresh) != text:
        print(f"FAIL {MANIFEST_PATH.name} differs from a fresh extraction; "
              "run --write and review the diff")
        return 1
    print("ok  re-extraction from the archived DLL matches byte for byte")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Acceptance test for the DLL-sourced content codegen (#2496, P1 of #1282).

The claim under test is narrow and load-bearing: on v0.111.0,
`generate_content.py --source dll` — which reads content identity, per-card
rarity/type/target/cost/keywords/tags and the unlock-epoch universe out of
`sts2.dll` — produces `src/ids.rs` and `src/content_tables.rs` **byte for
byte identical** to the committed files the Python-sourced generator produces.

That equality is what makes the DLL path trustworthy on the *next* build,
where there is nothing to compare it against. Until it holds, a DLL-sourced
generator is an untested rewrite of the one artifact the whole crate stands on.

Since #2515 the same claim carries one hop further out. `--source dll` does
not hand its facts to the generator directly: it writes them all to the
committed `data/dll_content.v0.111.0.json` and generates from *that*, and
`--source manifest` replays the file with no assembly and no `dnfile` — which
is what the `rust port` workflow's codegen freshness step runs, because CI has
neither. So the proof chain is:

    DLL  ==>  manifest  ==>  ids.rs / content_tables.rs
                             ^^^^^^^^^^^^^^^^^^^^^^^^^^ committed bytes
    python  =================>

with the DLL-bearing links skipped (or, under `--require-dll`, failed) where
the archive is absent, and the manifest link always live.

Four kinds of check live here:

1. **Boundary controls** (no DLL needed, run everywhere). The declared source
   boundary is well-formed, and every fact `dll_content` claims to source from
   the assembly is actually routed through a `DllContentSource` override
   rather than quietly inherited from the Python path. This is the check that
   stops the annex from becoming a lie.
2. **Reproduction** (needs the assembly). Regenerate into a temp directory
   with `--source dll` and diff. The working tree is never written to.
3. **The manifest chain** (link 1 needs the assembly; links 2-3 run
   everywhere). A DLL-written manifest equals the committed one; the committed
   one regenerates the committed tables byte for byte under an interpreter
   with no `dnfile`; and a manifest that was tampered with is refused rather
   than trusted.
4. **Ledger liveness** (needs the assembly). `STALE_REPO_DATA` records the
   three facts where today's committed tables disagree with the assembly. Each
   entry is proven *necessary*: dropping it must break the reproduction. A
   ledger entry that no longer changes anything is a fixed defect whose record
   should be deleted, and this is what tells us.

Local-only by construction: `solver/dll-archive/` is gitignored and no CI
checkout has an `sts2.dll`, so every DLL-bearing test **skips** rather than
fails when the assembly or `dnfile` is absent. Pass `--require-dll` to turn
those skips into failures when you want hard evidence (what a reviewer and the
game-update runbook should use).

Run it under an interpreter that has `dnfile`/`dncil` — on this Mac
`python3.12`, not the rotating `python3` (3.14, no dnfile)::

    python3.12 versions/v0.111.0/rust/tools/test_dll_content_source.py
    python3.12 versions/v0.111.0/rust/tools/test_dll_content_source.py \\
        --require-dll
"""

from __future__ import annotations

import contextlib
import importlib.util
import inspect
import json
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve()
TOOLS_DIR = HERE.parent
RUST_DIR = HERE.parents[1]
BUILD_DIR = HERE.parents[2]
SOLVER_DIR = BUILD_DIR / "solver"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import dll_content  # noqa: E402  (path set up above)

#: Set by `--require-dll`: fail instead of skipping when the assembly or its
#: reader is missing.
REQUIRE_DLL = False

_GENERATED = ("ids.rs", "content_tables.rs")


#: Memo for `_facts()`. Parsing the assembly costs seconds and the result is
#: read-only, so each test paying for its own copy only made the suite slower,
#: never more independent.
_FACTS_CACHE = []


def _facts():
    """The assembly's facts, or a skip/failure when it is unreadable."""
    if _FACTS_CACHE:
        return _FACTS_CACHE[0]
    try:
        facts = dll_content.DllFacts(require_certified=True)
    except dll_content.DllUnavailable as exc:
        if REQUIRE_DLL:
            raise AssertionError(f"--require-dll but: {exc}") from exc
        raise unittest.SkipTest(str(exc)) from exc
    _FACTS_CACHE.append(facts)
    return facts


@contextlib.contextmanager
def _no_archive():
    """Simulate a checkout with no archived assembly (every CI runner)."""
    original = dll_content.archived_certified_dll
    dll_content.archived_certified_dll = lambda: None
    try:
        yield
    finally:
        dll_content.archived_certified_dll = original


@contextlib.contextmanager
def _no_manifest():
    """Simulate a crate with no committed manifest (the pre-#2515 tree)."""
    original = dll_content.default_manifest_path
    with tempfile.TemporaryDirectory() as tmp:
        absent = pathlib.Path(tmp) / "dll_content.absent.json"
        dll_content.default_manifest_path = lambda *a, **k: absent
        try:
            yield
        finally:
            dll_content.default_manifest_path = original


def _generate(out_dir, *, source, extra=()):
    """Run the generator into `out_dir`; return its completed process."""
    return subprocess.run(
        [sys.executable, str(TOOLS_DIR / "generate_content.py"),
         "--source", source, "--out-dir", str(out_dir), *extra],
        capture_output=True, text=True, check=False)


# ---------------------------------------------------------------------------
# 1. Boundary controls — no assembly required
# ---------------------------------------------------------------------------

class RustModelingAdditionsLeavePythonFrozen(unittest.TestCase):
    def test_calculated_gamble_upgrade_is_rust_owned(self):
        """The overlay lands on the generator's registry and never in the
        frozen snapshot it was replayed from (#2827 item D: the snapshot
        stands in for the frozen `combat_sim` this used to import)."""
        import registry_snapshot
        generator = _load_generator()
        registry = generator.load_registry()
        recorded = registry_snapshot.load_snapshot().CARDS
        self.assertNotIn(("CALCULATED_GAMBLE", 1), recorded)
        base = registry.CARDS[("CALCULATED_GAMBLE", 0)]
        upgraded = registry.CARDS[("CALCULATED_GAMBLE", 1)]
        self.assertEqual(upgraded.steps, base.steps)
        self.assertEqual(upgraded.cost, 0)
        self.assertTrue(upgraded.exhausts)
        self.assertTrue(upgraded.retain)
        self.assertFalse(base.retain)
        del registry.CARDS[("CALCULATED_GAMBLE", 0)]
        self.assertIn(("CALCULATED_GAMBLE", 0),
                      registry_snapshot.load_snapshot().CARDS)


class EveryRosterTierIsTheManifests(unittest.TestCase):
    """#2539: every Python roster builder's HP and tiered spawn-time amounts
    equal the committed manifest's `GetValueIfAscension` tier at A0, A1, A7,
    A8, A9 and A10.

    Native selection is `atOrAbove` iff `AscensionManager::HasLevel(gate)`
    (RVA 0x11fa83, IL_0001-IL_000d: `!(_level < gate)`, i.e. `>=`). The
    lower tiers in `combat_sim.py`/`content/encounters` were typed from
    source comments. This is what checks them against the assembly: a
    disagreement fails here, and the DLL wins. The check lives here, not in
    the solver suite, because it reads the manifest, which is outside the
    fast gate's trigger set. The `rust port` workflow runs on
    `versions/v0.111.0/solver/**` too, so an oracle edit still reaches it.

    #2827 item D retired the two checks that ran the Python engine itself
    (`make_monsters` over every registered roster, and the Two-Tailed Rat
    counter): they certified the frozen Python registry, which item F
    deletes, and the Rust rosters are pinned against those same frozen
    rosters by `fixtures/` + `encounters::oracle`. What remains reads the
    frozen registry snapshot, so it needs no Python simulator.
    """

    #: Kinds whose manifest row the DLL reader refuses (the value is computed,
    #: not a literal). Their tiers are IL-read and pinned by value in
    #: `versions/v0.111.0/solver/test_issue2539_ascension_tiers.py`.
    #: `TEST_SUBJECT` left this set in #2535 (two-hop getter delegation);
    #: `AXEBOT` and `SEWER_CLAM` left it in #2534, when the reader learned the
    #: straight-line getter and the inline-local amount spelling. They are
    #: now checked below like every other row.
    REFUSED_ROWS = {"TOUGH_EGG"}
    #: The Python field each tiered spawn-time power lands in.
    POWER_FIELDS = {"RavenousPower": "ravenous", "VitalSparkPower": "vital",
                    "PlatingPower": "mplating", "CurlUpPower": "curl_up",
                    "SkittishPower": "skittish", "SlipperyPower": "slippery",
                    # TEST_SUBJECT's row, read since #2535 followed its
                    # two-hop get_MaxInitialHp delegation.
                    "AdaptablePower": "adaptable", "EnragePower": "enrage",
                    "StockPower": "stock"}

    @staticmethod
    def _native(tier, ascension):
        if tier["gate"] is None or ascension >= tier["gate"]:
            return tier["at_or_above"]
        return tier["below"]

    def test_engine_side_amounts_are_the_manifests(self):
        """Two tiered amounts that are not roster fields: Terror Eel's
        ShriekPower threshold and Globe Head's GalvanicPower, as the frozen
        registry recorded them (`generate_content.RECORDED_FOR_TESTS`)."""
        sim = _load_registry()
        manifest = json.loads(
            (RUST_DIR / "data" / "dll_content.v0.111.0.json").read_text()
        )["facts"]["monster_models"]
        shriek = dict(manifest["TERROR_EEL"]["initial_powers"])["ShriekPower"]
        galvanic = dict(manifest["GLOBE_HEAD"]["initial_powers"])["GalvanicPower"]
        for tier, constant in ((shriek, sim.SHRIEK_THRESHOLD),
                               (galvanic, sim.GALVANIC_DAMAGE)):
            self.assertEqual(constant, sim.AscensionTier(
                tier["gate"], tier["at_or_above"], tier["below"]))

class EveryMoveConstantIsTheManifests(unittest.TestCase):
    """#2828: every tiered move-table argument is the manifest's
    `GetValueIfAscension` triple, and the frozen registry resolves it to the
    native tier at every ascension.

    Codegen (`generate_content.MoveConstantJoin`) already refuses to write the
    tables unless each `move_constant_sites` row matches and every tiered
    manifest row of a modeled kind is accounted for; this re-derives that on
    the manifest path, pins the one engine-read constant, and proves the
    join can fail. It lives here for the same reason as
    `EveryRosterTierIsTheManifests`: the manifest is outside the fast gate's
    trigger set.
    """

    @staticmethod
    def _native(tier, ascension):
        if ascension >= tier["gate"]:
            return tier["at_or_above"]
        return tier["below"]

    @staticmethod
    def _resolved(arg, cs, ascension):
        """One move argument at `ascension`: `combat_sim.at_ascension`'s rule
        (`at_or_above` iff `ascension >= gate`, native `HasLevel` 0x11fa83
        IL_0001-IL_000d), restated because the registry is now replayed from
        a snapshot that carries data, not functions (#2827 item D)."""
        if type(arg) is cs.AscensionTier:
            return arg.at_or_above if ascension >= arg.gate else arg.below
        return arg

    @staticmethod
    def _join(cs):
        generate_content = _load_generator()
        source = dll_content.ManifestContentSource(
            dll_content.ManifestFacts(require_certified=True))
        axes = generate_content.build_axes(
            cs, generate_content.move_tables(cs), source)
        return generate_content, generate_content.MoveConstantJoin(
            cs, axes, source)

    @staticmethod
    def _rows(generate_content, cs):
        """Every move row by the generator's own site spelling."""
        rows = {}
        for name, shape, value in generate_content.move_tables(cs):
            if shape == "C":
                for owner, entry in value.items():
                    rows[f"{name}.{owner}"] = entry
                continue
            tables = dict(value) if shape == "D" else {None: value}
            for kind, table in tables.items():
                sub = generate_content.classify_move_table(table)
                entries = (table if sub == "A" else
                           [(move, *entry) for move, entry in table.items()])
                prefix = f"{name}.{kind}" if kind else name
                for entry in entries:
                    rows[f"{prefix}.{entry[0]}"] = entry
        return rows

    def test_every_joined_site_resolves_to_the_native_tier(self):
        import move_constant_sites as sites
        cs = _load_registry()
        generate_content, join = self._join(cs)
        rows = self._rows(generate_content, cs)
        for site, name in sites.MOVE_CONSTANT_SITES.items():
            base, index = site[:-1].rsplit("[", 1)
            row, tier = rows[base], join.rows[name]
            for ascension in (0, 1, 7, 8, 9, 10):
                with self.subTest(site=site, ascension=ascension):
                    self.assertEqual(
                        self._resolved(row[2][int(index)], cs, ascension),
                        self._native(tier, ascension))

    def test_the_engine_read_constant_is_the_manifests(self):
        cs = _load_registry()
        tier = dll_content.ManifestFacts(
            require_certified=True).monster_move_constants()[
                "SLUMBERING_BEETLE"]["RolloutDamage"]
        self.assertEqual(cs.SLUMBERING_BEETLE_ROLLOUT, cs.AscensionTier(
            tier["gate"], tier["at_or_above"], tier["below"]))

    def test_a_registry_tier_the_assembly_disagrees_with_fails_codegen(self):
        """Mutation control: a forged Soul Fysh DE_GAS, an untiered 18, and
        a tier at a site the join does not name all stop codegen."""
        cs = _load_registry()
        _generate_content, join = self._join(cs)
        site = "moves:LOOPS.SOUL_FYSH.DE_GAS[0]"
        self.assertEqual(
            join.render(cs.AscensionTier(9, 18, 16), site),
            "Arg::Tier(move_constants::SOUL_FYSH_DE_GAS_DAMAGE)")
        for forged in (cs.AscensionTier(9, 18, 17), 18):
            with self.subTest(forged=forged), self.assertRaises(SystemExit):
                join.render(forged, site)
        with self.assertRaises(SystemExit):
            join.render(cs.AscensionTier(9, 3, 2), "moves:LOOPS.NIBBIT.BUTT[1]")


class TheDeclaredBoundaryIsWellFormed(unittest.TestCase):
    """The ledger is a contract, so it has to be readable and complete."""

    def test_the_ledger_prints_without_a_dll_or_dnfile(self) -> None:
        """`--ledger` is the documentation half and must never need the DLL.

        CI's interpreter has no dnfile, and a reviewer reading the boundary
        should not need a game install to do it.
        """
        buffer = []

        class _Sink:
            def write(self, text):
                buffer.append(text)

        dll_content.print_ledger(_Sink())
        printed = "".join(buffer)
        for fact in dll_content.DLL_SOURCED_FACTS:
            self.assertIn(fact, printed)
        for fact in dll_content.MODELING_ANNEX:
            self.assertIn(fact, printed)

    def test_no_fact_is_both_dll_sourced_and_annexed(self) -> None:
        overlap = set(dll_content.DLL_SOURCED_FACTS) & set(
            dll_content.MODELING_ANNEX)
        self.assertEqual(overlap, set())

    def test_every_entry_says_where_the_fact_comes_from(self) -> None:
        for fact, entry in dll_content.DLL_SOURCED_FACTS.items():
            self.assertEqual(len(entry), 2, fact)
            self.assertTrue(all(part.strip() for part in entry), fact)
        for fact, entry in dll_content.MODELING_ANNEX.items():
            self.assertEqual(len(entry), 2, fact)
            self.assertTrue(all(part.strip() for part in entry), fact)

    def test_stale_entries_name_a_value_an_origin_and_a_reason(self) -> None:
        for key, entry in dll_content.STALE_REPO_DATA.items():
            self.assertEqual(len(key), 2, key)
            dll_value, committed, origin, why = entry
            self.assertNotEqual(dll_value, committed, key)
            self.assertTrue(origin.strip(), key)
            self.assertGreater(len(why), 40, key)


class EveryClaimedDllFactIsActuallyRouted(unittest.TestCase):
    """The annex must not be able to become a lie by omission.

    `DllContentSource` inherits from `PythonSource`, which is what lets the
    modeling annex work at all — but it is also the failure mode: a fact could
    be *declared* DLL-sourced while its lookup quietly fell through to the
    registry, and on v0.111.0 nothing would go red, because the two agree.
    So every DLL-sourced fact is pinned to a method the DLL source actually
    overrides.
    """

    #: Declared fact -> the `PythonSource` method `DllContentSource` must
    #: override for that fact to follow the assembly.
    ROUTES = {
        "CardId": "card_ids",
        "RelicId": "relic_ids",
        "PotionId": "potion_ids",
        "EnchantmentId": "enchantment_ids",
        "CardRarity vocabulary": "card_rarity_vocabulary",
        "card.rarity": "card_rarity",
        "card.card_type": "card_type",
        "card.target_type": "card_target_type",
        "card.cost@0": "card_cost",
        "card.keywords@0": "card_keyword",
        "card.tags": "card_tags",
        "epochs": "epoch_universe",
        "monster.initial_hp": "monster_initial_hp",
        "monster.initial_powers": "monster_initial_powers",
        "monster.ctor_ints": "monster_ctor_ints",
        "monster.move_constants": "monster_move_constants",
        "encounter.rng_draws": "encounter_rng_draws",
        "card.multiplayer_constraint": "card_multiplayer_constraint",
        "card.can_be_generated_in_combat": "card_can_be_generated_in_combat",
        "potion.rarity": "potion_rarity",
        "potion.can_be_generated_in_combat":
            "potion_can_be_generated_in_combat",
        "character card pools": "character_card_pools",
        "colorless card pool": "colorless_card_pool",
        "character potion pools": "character_potion_pools",
        "shared potion pool": "shared_potion_pool",
        "shared card pools": "shared_card_pools",
        "character pool order": "character_pool_order",
        "character unlock epochs": "character_unlock_epochs",
        "mad science variants": "mad_science_variants",
    }

    def test_every_declared_fact_has_a_route(self) -> None:
        self.assertEqual(set(self.ROUTES),
                         set(dll_content.DLL_SOURCED_FACTS))

    def test_every_route_is_overridden_by_the_dll_source(self) -> None:
        for fact, method in self.ROUTES.items():
            with self.subTest(fact=fact):
                self.assertIn(method, dll_content.DllContentSource.__dict__,
                              f"{fact} is declared DLL-sourced but "
                              f"DllContentSource does not override "
                              f"{method}(); it would silently inherit the "
                              "Python value")

    def test_the_manifest_source_routes_every_fact_the_same_way(self) -> None:
        """The manifest path must not be able to drift from the DLL path.

        `ManifestContentSource` is a subclass rather than a sibling precisely
        so every routed lookup resolves to the *same function object*. If one
        were overridden — or the class were reparented onto `PythonSource` —
        that fact would silently come from the frozen registry in CI while the
        ledger still called it DLL-sourced, which is the #2515 failure mode
        one level down from the #2496 one above.
        """
        self.assertTrue(issubclass(dll_content.ManifestContentSource,
                                   dll_content.DllContentSource))
        for fact, method in self.ROUTES.items():
            with self.subTest(fact=fact):
                self.assertIs(
                    getattr(dll_content.ManifestContentSource, method),
                    getattr(dll_content.DllContentSource, method),
                    f"{fact}: the manifest source resolves {method}() to a "
                    "different implementation than the DLL source")

    def test_the_manifest_carries_every_fact_the_dll_source_reads(self) -> None:
        """A key missing from `MANIFEST_FACT_KEYS` is a silent registry read.

        `ManifestFacts` must be able to answer every question
        `DllContentSource` asks of a facts object; the census below is derived
        from the DLL facts class rather than hand-listed, so a new DLL-sourced
        lookup cannot be added without either carrying it in the manifest or
        failing here.
        """
        asked = set()
        for method in self.ROUTES.values():
            body = inspect.getsource(
                getattr(dll_content.DllContentSource, method))
            asked.update(re.findall(r"self\.facts\.([a-z_]+)\(", body))
        # `card()` is per-card; the manifest carries the whole `cards` map.
        for name in sorted(asked):
            with self.subTest(fact_method=name):
                self.assertTrue(
                    hasattr(dll_content.ManifestFacts, name),
                    f"DllContentSource asks facts.{name}() but "
                    "ManifestFacts cannot answer it")

    def test_the_generator_routes_card_columns_through_the_source(self) -> None:
        """`render_card_row` must not read a routed column off the row again.

        A direct `card.<column>` read would bypass `--source dll` for that
        column while the ledger still claimed it.
        """
        generate_content = _load_generator()
        body = inspect.getsource(generate_content.render_card_row)
        for column in dll_content.KEYWORD_COLUMNS:
            with self.subTest(column=column):
                self.assertNotIn(f"card.{column}", body)
        for column in ("card.cost", "card.tags", "card.strike_tag",
                       "card.target_type", "_CARD_RARITY_BY_ID",
                       "_CARD_TYPE_BY_ID"):
            with self.subTest(column=column):
                self.assertNotIn(column, body)


class TheDefaultSourceResolves(unittest.TestCase):
    """D1-D4 landed on 2026-09-16, so the default is `auto` (#1282 D3).

    The ladder is `dll` -> `manifest` -> `python`. Landing on `manifest` is
    not a downgrade and carries no note — those facts came out of the assembly
    and were committed. Landing on `python` IS a downgrade, because under D3
    the registry is frozen at v0.111.0, and it must never be silent: the note
    is the whole point.
    """

    def test_an_unqualified_run_uses_the_resolved_source(self) -> None:
        """End-to-end: no `--source` at all, and the run says what it used.

        Asserted against `resolve_source` rather than against a literal, so
        this passes on a host with the archive (`dll`) and on a CI checkout
        without it (`python`) while still failing if the CLI stopped
        defaulting to `auto`.
        """
        generate_content = _load_generator()
        expected, _src, _note = generate_content.resolve_source("auto")
        with tempfile.TemporaryDirectory() as tmp:
            done = subprocess.run(
                [sys.executable, str(TOOLS_DIR / "generate_content.py"),
                 "--out-dir", tmp],
                capture_output=True, text=True, check=False)
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn(f"content source: {expected}", done.stderr)
            _assert_identical(self, pathlib.Path(tmp))

    def test_auto_resolves_to_the_assembly_when_it_is_readable(self) -> None:
        _facts()  # skip with a reason when the archive or dnfile is absent
        if dll_content.archived_certified_dll() is None:
            # `auto` reads only the archive; a Steam install alone is not it.
            self.skipTest("no archived certified sts2.dll on this host")
        generate_content = _load_generator()
        kind, src, note = generate_content.resolve_source("auto")
        self.assertEqual(kind, "dll")
        self.assertEqual(note, "")
        self.assertIsInstance(src, dll_content.DllContentSource)

    def test_auto_takes_the_manifest_when_the_archive_is_absent(self) -> None:
        """The CI case, simulated so it runs on a host WITH the archive.

        This is the resolution that makes codegen hermetic: no assembly, no
        `dnfile`, and still not the frozen registry. It is silent on purpose —
        the manifest is the assembly's committed testimony, not a fallback.
        """
        generate_content = _load_generator()
        with _no_archive():
            kind, src, note = generate_content.resolve_source("auto")
        self.assertEqual(kind, "manifest")
        self.assertEqual(note, "")
        self.assertIsInstance(src, dll_content.ManifestContentSource)

    def test_auto_falls_back_loudly_only_when_both_are_absent(self) -> None:
        """Simulated, so it runs on a host that has the archive AND the file.

        The note must name both missing sources: on the day this fires, the
        reader needs to know the tables now describe a frozen v0.111.0
        registry rather than any assembly.
        """
        generate_content = _load_generator()
        with _no_archive(), _no_manifest():
            kind, src, note = generate_content.resolve_source("auto")
        self.assertEqual(kind, "python")
        self.assertIsInstance(src, dll_content.PythonSource)
        self.assertNotIsInstance(src, dll_content.DllContentSource)
        self.assertIn(dll_content.CERTIFIED_BUILD, note)
        self.assertIn("manifest", note)

    def test_an_explicit_source_never_resolves(self) -> None:
        """`--source dll` must fail loudly, not fall back to the registry."""
        generate_content = _load_generator()
        kind, src, note = generate_content.resolve_source("python")
        self.assertEqual((kind, note), ("python", ""))
        self.assertIsInstance(src, dll_content.PythonSource)
        original = dll_content.archived_certified_dll
        dll_content.archived_certified_dll = lambda: None
        try:
            with self.assertRaises(dll_content.DllUnavailable):
                generate_content.resolve_source("dll", "/nonexistent.dll")
        finally:
            dll_content.archived_certified_dll = original

    def test_make_source_still_builds_each_concrete_source(self) -> None:
        self.assertIsInstance(dll_content.make_source("python"),
                              dll_content.PythonSource)
        self.assertNotIsInstance(dll_content.make_source("python"),
                                 dll_content.DllContentSource)
        self.assertIsInstance(dll_content.make_source("manifest"),
                              dll_content.ManifestContentSource)

    def test_an_unknown_source_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            dll_content.make_source("guess")
        with self.assertRaises(ValueError):
            # `auto` is resolved by the generator, never by the factory.
            dll_content.make_source("auto")

    def test_dll_path_without_dll_source_is_refused(self) -> None:
        generate_content = _load_generator()
        with self.assertRaises(SystemExit):
            with contextlib.redirect_stderr(_Discard()):
                generate_content.main(["--dll", "/nonexistent", "--check"])

    def test_the_python_path_still_reproduces_the_committed_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            done = _generate(tmp, source="python")
            self.assertEqual(done.returncode, 0, done.stderr)
            _assert_identical(self, pathlib.Path(tmp))


# ---------------------------------------------------------------------------
# 2. Reproduction — the acceptance test
# ---------------------------------------------------------------------------

class TheDllSourceReproducesTheCommittedTables(unittest.TestCase):

    def test_the_assembly_is_the_certified_build(self) -> None:
        facts = _facts()
        self.assertEqual(facts.sha256, dll_content.CERTIFIED_DLL_SHA256)

    def test_the_inventory_matches_the_games_own_model_id_hash(self) -> None:
        """The game writes this value into every `.mcr` header.

        Recomputing it from the assembly is an external witness that the
        ModelId inventory every id axis is derived from was reconstructed
        exactly — not merely self-consistently.
        """
        facts = _facts()
        committed = json.loads(
            (SOLVER_DIR / "mcr_tables.json").read_text())["model_id_hash"]
        self.assertEqual(facts.model_id_hash(), committed)

    def test_regenerating_from_the_dll_is_byte_identical(self) -> None:
        _facts()  # skip early, with the reason, if the assembly is absent
        with tempfile.TemporaryDirectory() as tmp:
            done = _generate(tmp, source="dll")
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn("content source: dll", done.stderr)
            _assert_identical(self, pathlib.Path(tmp))

    def test_it_never_writes_into_the_working_tree(self) -> None:
        """`--out-dir` is the only reason this test can run on a clean tree.

        The manifest is covered too: `--source dll` writes one, and it must
        land beside the redirected output rather than in `data/`, or a
        regenerate-and-diff check would be diffing a file it had just
        rewritten.
        """
        _facts()
        manifest = dll_content.default_manifest_path()
        before = {name: (RUST_DIR / "src" / name).read_bytes()
                  for name in _GENERATED}
        before[manifest.name] = manifest.read_bytes()
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(_generate(tmp, source="dll").returncode, 0)
        for name in _GENERATED:
            self.assertEqual((RUST_DIR / "src" / name).read_bytes(),
                             before[name], name)
        self.assertEqual(manifest.read_bytes(), before[manifest.name],
                         manifest.name)


class TheTwoSourcesAgreeFactByFact(unittest.TestCase):
    """Byte equality is the verdict; this says *where* they agree.

    A single byte diff on a 1.2 MB file is a bad diagnostic. These assertions
    fail on the specific axis or column that moved, which is what a reader
    on update day needs.
    """

    def setUp(self) -> None:
        self.facts = _facts()
        self.cs = _load_registry()
        self.python = dll_content.PythonSource()
        self.dll = dll_content.DllContentSource(self.facts)

    def test_the_identity_axes_agree(self) -> None:
        for axis in ("card_ids", "relic_ids", "potion_ids",
                     "enchantment_ids"):
            with self.subTest(axis=axis):
                self.assertEqual(getattr(self.dll, axis)(self.cs),
                                 getattr(self.python, axis)(self.cs))

    def test_the_epoch_universe_agrees(self) -> None:
        self.assertEqual(self.dll.epoch_universe(self.cs, SOLVER_DIR)[0],
                         self.python.epoch_universe(self.cs, SOLVER_DIR)[0])

    def test_the_card_rarity_vocabulary_agrees(self) -> None:
        self.assertEqual(self.dll.card_rarity_vocabulary(self.cs),
                         self.python.card_rarity_vocabulary(self.cs))

    def test_every_card_column_agrees(self) -> None:
        for card_id in self.python.card_ids(self.cs):
            row = self.cs.CARDS.get((card_id, 0))
            if row is None:
                continue
            with self.subTest(card=card_id):
                self.assertEqual(self.dll.card_rarity(self.cs, card_id),
                                 self.python.card_rarity(self.cs, card_id))
                self.assertEqual(self.dll.card_type(self.cs, card_id),
                                 self.python.card_type(self.cs, card_id))
                self.assertEqual(
                    self.dll.card_cost(self.cs, card_id, 0, row),
                    self.python.card_cost(self.cs, card_id, 0, row))
                self.assertEqual(
                    self.dll.card_target_type(self.cs, card_id, row),
                    self.python.card_target_type(self.cs, card_id, row))
                self.assertEqual(
                    self.dll.card_tags(self.cs, card_id, row),
                    self.python.card_tags(self.cs, card_id, row))
                for column in dll_content.KEYWORD_COLUMNS:
                    self.assertEqual(
                        self.dll.card_keyword(self.cs, card_id, 0, row,
                                              column),
                        self.python.card_keyword(self.cs, card_id, 0, row,
                                                 column), column)

    def test_the_generation_pools_agree(self) -> None:
        """#2542: the assembly's own pools reproduce the harness censuses.

        `card_pool_census.json` was recorded by booting the game through the
        headless harness and asking each live `CardPoolModel` for its ordered
        cards at each unlock epoch. This asserts that a *static IL read* of
        `<Character>CardPool::GenerateAllCards` and `FilterThroughEpochs`
        produces the same rows, row for row and in the same order — which is
        what lets the generated tables be sourced from the assembly rather
        than from a probe that needs a booted CLR.
        """
        self.assertEqual(self.dll.character_pool_order(self.cs),
                         self.python.character_pool_order(self.cs))
        self.assertEqual(self.dll.character_unlock_epochs(self.cs),
                         self.python.character_unlock_epochs(self.cs))
        dll_pools = self.dll.character_card_pools(self.cs)
        python_pools = self.python.character_card_pools(self.cs)
        self.assertEqual(sorted(dll_pools), sorted(python_pools))
        for character in sorted(dll_pools):
            with self.subTest(character=character):
                self.assertEqual(dll_pools[character],
                                 python_pools[character])
        self.assertEqual(self.dll.character_potion_pools(self.cs),
                         self.python.character_potion_pools(self.cs))
        self.assertEqual(self.dll.shared_potion_pool(self.cs),
                         self.python.shared_potion_pool(self.cs))
        # #2734: the python path replays the committed manifest for these, so
        # this is the assembly re-read agreeing with the committed facts.
        self.assertEqual(self.dll.shared_card_pools(self.cs),
                         self.python.shared_card_pools(self.cs))

    def test_the_two_generation_columns_agree(self) -> None:
        """`MultiplayerConstraint` and `CanBeGeneratedInCombat`, per card.

        These are the two profile-independent predicates every combat
        generation path applies, so a drift here silently widens or narrows
        every pool at once.
        """
        pooled = {card_id
                  for rows in self.python.character_card_pools(self.cs).values()
                  for card_id, _epoch in rows}
        self.assertGreater(len(pooled), 400)
        for card_id in sorted(pooled):
            with self.subTest(card=card_id):
                self.assertEqual(
                    self.dll.card_multiplayer_constraint(self.cs, card_id),
                    self.python.card_multiplayer_constraint(self.cs, card_id))
                self.assertEqual(
                    self.dll.card_can_be_generated_in_combat(
                        self.cs, card_id),
                    self.python.card_can_be_generated_in_combat(
                        self.cs, card_id))
        for potion_id in sorted(self.python.character_potion_pools(
                self.cs)["IRONCLAD"]["potions"]):
            with self.subTest(potion=potion_id):
                self.assertEqual(
                    self.dll.potion_rarity(self.cs, potion_id),
                    self.python.potion_rarity(self.cs, potion_id))
                self.assertEqual(
                    self.dll.potion_can_be_generated_in_combat(
                        self.cs, potion_id),
                    self.python.potion_can_be_generated_in_combat(
                        self.cs, potion_id))

    def test_the_hand_written_card_type_enum_matches_the_assembly(self) -> None:
        """`CardType` is the one content enum nothing regenerates.

        It is spelled out in `generate_content.TABLE_TYPES` with explicit
        native values, so a build that renumbers or extends it would be
        silently wrong. The annex says `--report` checks this; so does this.
        """
        emitted = _load_generator().TABLE_TYPES
        for value, name in self.facts.card_type_members():
            with self.subTest(member=name):
                self.assertIn(f"    {name} = {value},", emitted)

    def test_the_native_unplayable_census_agrees(self) -> None:
        self.assertEqual(self.dll.native_unplayable_keys(self.cs),
                         self.python.native_unplayable_keys(self.cs))

    def test_every_modeled_annex_axis_is_backed_by_the_assembly(self) -> None:
        """One-directional on purpose.

        `EncounterId` and `MonsterKind` are modeled vocabularies, so the
        assembly having content the sim does not model is expected (unported
        content). The reverse — a modeled id with no content behind it — is a
        defect, and this is the only place it would surface.
        """
        generate_content = _load_generator()
        spellings = self.facts.encounter_spellings()
        unbacked = [name for name in sorted(self.cs.SUPPORTED_ENCOUNTERS)
                    if name not in spellings]
        self.assertEqual(unbacked, [])

        monsters = set(self.facts.monster_inventory())
        # `DECIMILLIPEDE_SEGMENT` is the one declared aggregate: the game
        # spells it as three classes (_FRONT/_MIDDLE/_BACK) and the sim as
        # one kind. Documented in dll_content.MODELING_ANNEX.
        aggregates = {"DECIMILLIPEDE_SEGMENT"}
        unbacked = [kind for kind in generate_content.monster_kinds(self.cs)
                    if kind not in monsters and kind not in aggregates]
        self.assertEqual(unbacked, [])


# ---------------------------------------------------------------------------
# 3. The manifest chain (#2515)
# ---------------------------------------------------------------------------

class TheManifestReproducesTheCommittedTables(unittest.TestCase):
    """The hermetic link, and the only one CI can actually run.

    Nothing here touches an assembly or imports `dnfile`, on purpose: this is
    the check the `rust port` workflow's *Codegen freshness* step performs on
    a runner that has neither.
    """

    def test_the_committed_manifest_is_present_and_well_formed(self) -> None:
        """Its absence is not a skip. CI generates FROM this file."""
        path = dll_content.default_manifest_path()
        self.assertTrue(path.exists(), f"{path} is not committed")
        facts = dll_content.ManifestFacts(path, require_certified=True)
        self.assertEqual(facts.build, dll_content.CERTIFIED_BUILD)
        self.assertEqual(facts.sha256, dll_content.CERTIFIED_DLL_SHA256)

    def test_it_records_the_games_own_model_id_hash(self) -> None:
        """The same external witness the DLL path gets, carried forward.

        `modelIdHash` is the value the game writes into every `.mcr` header,
        so this pins that the inventory in the manifest is the one the build
        actually ships — not merely a self-consistent file.
        """
        facts = dll_content.ManifestFacts(require_certified=True)
        committed = json.loads(
            (SOLVER_DIR / "mcr_tables.json").read_text())["model_id_hash"]
        self.assertEqual(facts.model_id_hash(), committed)

    def test_generating_from_the_manifest_is_byte_identical(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            done = _generate(tmp, source="manifest")
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn("content source: manifest", done.stderr)
            _assert_identical(self, pathlib.Path(tmp))

    def test_the_manifest_is_byte_stable_when_reserialized(self) -> None:
        """Freshness is a `git diff`, so the bytes have to be a pure function.

        Sorted keys, sorted set-derived lists, no wall clock and no host path:
        re-serializing what is on disk must reproduce it exactly, or the
        workflow's diff would flap for reasons unrelated to content.
        """
        path = dll_content.default_manifest_path()
        committed = path.read_text()
        self.assertEqual(
            dll_content.manifest_text(json.loads(committed)), committed)

    def test_a_hand_edited_manifest_is_refused_not_trusted(self) -> None:
        """The mutation control: prove the self-hash can fail.

        Without it, `self_sha256` would be decoration. A manifest is content
        codegen's source of truth in CI, so a tampered one must raise rather
        than emit a fabricated table (I5).
        """
        payload = json.loads(
            dll_content.default_manifest_path().read_text())
        payload["facts"]["card_ids"] = (
            payload["facts"]["card_ids"][:-1] + ["FABRICATED_CARD"])
        with self.assertRaises(dll_content.DllRefusal) as caught:
            dll_content.ManifestFacts(payload=payload)
        self.assertIn("self_sha256", str(caught.exception))

    def test_a_manifest_missing_a_fact_is_refused(self) -> None:
        """An omitted key must not silently become a registry read."""
        for key in dll_content.MANIFEST_FACT_KEYS:
            with self.subTest(dropped=key):
                payload = json.loads(
                    dll_content.default_manifest_path().read_text())
                payload["facts"].pop(key)
                payload["self_sha256"] = dll_content.manifest_self_hash(
                    payload)
                with self.assertRaises(dll_content.DllRefusal):
                    dll_content.ManifestFacts(payload=payload)

    def test_a_missing_manifest_is_unavailable_not_a_refusal(self) -> None:
        """`auto` walks past an absent manifest; it never walks past a bad one.

        The two failure kinds are deliberately different exception types, and
        `ManifestUnavailable` subclasses `DllUnavailable` so the ladder in
        `resolve_source` treats it exactly like a missing archive.
        """
        with tempfile.TemporaryDirectory() as tmp:
            absent = pathlib.Path(tmp) / "dll_content.v0.111.0.json"
            with self.assertRaises(dll_content.ManifestUnavailable):
                dll_content.ManifestFacts(absent)
        self.assertTrue(issubclass(dll_content.ManifestUnavailable,
                                   dll_content.DllUnavailable))

    def test_the_manifest_agrees_with_the_registry_fact_by_fact(self) -> None:
        """On v0.111.0 the two must still agree, column by column.

        Byte equality above is the verdict; this says *where*, so an update
        that moves one card's rarity reports that card rather than a diff on a
        1.2 MB file. It runs without an assembly, unlike its DLL-side twin.
        """
        cs = _load_registry()
        python = dll_content.PythonSource()
        manifest = dll_content.ManifestContentSource(
            dll_content.ManifestFacts(require_certified=True))
        for axis in ("card_ids", "relic_ids", "potion_ids",
                     "enchantment_ids", "card_rarity_vocabulary",
                     "native_unplayable_keys"):
            with self.subTest(axis=axis):
                self.assertEqual(getattr(manifest, axis)(cs),
                                 getattr(python, axis)(cs))
        self.assertEqual(manifest.epoch_universe(cs, SOLVER_DIR)[0],
                         python.epoch_universe(cs, SOLVER_DIR)[0])
        for card_id in python.card_ids(cs):
            row = cs.CARDS.get((card_id, 0))
            if row is None:
                continue
            with self.subTest(card=card_id):
                self.assertEqual(manifest.card_rarity(cs, card_id),
                                 python.card_rarity(cs, card_id))
                self.assertEqual(manifest.card_type(cs, card_id),
                                 python.card_type(cs, card_id))
                self.assertEqual(manifest.card_cost(cs, card_id, 0, row),
                                 python.card_cost(cs, card_id, 0, row))
                self.assertEqual(
                    manifest.card_target_type(cs, card_id, row),
                    python.card_target_type(cs, card_id, row))
                self.assertEqual(manifest.card_tags(cs, card_id, row),
                                 python.card_tags(cs, card_id, row))
                for column in dll_content.KEYWORD_COLUMNS:
                    self.assertEqual(
                        manifest.card_keyword(cs, card_id, 0, row, column),
                        python.card_keyword(cs, card_id, 0, row, column),
                        column)


class TheAssemblyWritesTheCommittedManifest(unittest.TestCase):
    """The DLL end of the chain. Needs the archive; `--require-dll` fails."""

    def test_the_dll_regenerates_the_committed_manifest_byte_for_byte(self):
        """This is what makes the committed file evidence rather than input.

        `--source dll --out-dir` writes the manifest beside the generated
        files, so the comparison never touches the working tree.

        The provenance assertion belongs with it rather than in a test of its
        own: that `--source dll` generates THROUGH the manifest is not an
        implementation detail but the reason the manifest is known complete —
        a fact it failed to carry would break the DLL path here, not only CI's
        hermetic one. (One subprocess, because each `--source dll` run reparses
        the assembly.)
        """
        _facts()
        with tempfile.TemporaryDirectory() as tmp:
            done = _generate(tmp, source="dll")
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertIn("content source: dll", done.stderr)
            self.assertIn(dll_content.default_manifest_path().name,
                          done.stderr)
            written = (pathlib.Path(tmp)
                       / dll_content.default_manifest_path().name)
            self.assertTrue(written.exists(), done.stderr)
            self.assertEqual(
                written.read_bytes(),
                dll_content.default_manifest_path().read_bytes(),
                "the assembly no longer writes the committed manifest; "
                "re-run generate_content.py --source dll")

    def test_the_report_compares_the_manifest_against_the_assembly(self) -> None:
        facts = _facts()
        committed, notes = dll_content.compare_manifest(facts)
        self.assertIsNotNone(committed)
        self.assertEqual(notes, [])

    def test_the_comparison_can_fail(self) -> None:
        """Mutation control for `compare_manifest`: a silent no-op is useless."""
        facts = _facts()
        payload = json.loads(
            dll_content.default_manifest_path().read_text())
        # Whichever card sorts first — a literal id would be one more thing to
        # re-point on a build that renames or drops it.
        victim = payload["facts"]["card_ids"][0]
        payload["facts"]["cards"][victim]["rarity"] = "mythic"
        payload["self_sha256"] = dll_content.manifest_self_hash(payload)
        with tempfile.TemporaryDirectory() as tmp:
            tampered = (pathlib.Path(tmp)
                        / dll_content.default_manifest_path().name)
            tampered.write_text(dll_content.manifest_text(payload))
            _committed, notes = dll_content.compare_manifest(facts, tampered)
        self.assertTrue(any("cards" in note for note in notes), notes)
        self.assertTrue(any(victim in note for note in notes), notes)

    def test_provenance_comes_from_the_archive_index_not_a_guess(self) -> None:
        """An unarchived assembly must be refused, not attributed.

        The manifest's build and commit are the archive index's, so a manifest
        can never claim a build nobody archived — which is the record that
        makes it auditable at all (I11; v0.108.0 is gone because nobody
        archived it).
        """
        facts = _facts()
        record = dll_content.archive_record(facts.sha256)
        self.assertEqual(record["build"], dll_content.CERTIFIED_BUILD)
        with self.assertRaises(dll_content.DllRefusal):
            dll_content.archive_record("0" * 64)


# ---------------------------------------------------------------------------
# 4. Ledger liveness
# ---------------------------------------------------------------------------

class TheStaleDataLedgerIsLiveAndMinimal(unittest.TestCase):
    """Each recorded divergence must still be doing work.

    `STALE_REPO_DATA` exists so `--source dll` reproduces committed bytes
    while the disagreement stays visible. That is only honest if every entry
    is load-bearing: an entry whose removal changes nothing describes a defect
    that has already been fixed, and leaving it would make the DLL path
    silently override a now-correct value.
    """

    def test_the_recorded_divergences_are_exactly_what_the_dll_says(self) -> None:
        facts = _facts()
        cs = _load_registry()
        python = dll_content.PythonSource()
        observed = {}
        for card_id in python.card_ids(cs):
            card = facts.card(card_id)
            if card["rarity"] != python.card_rarity(cs, card_id):
                observed[(card_id, "rarity")] = (
                    card["rarity"], python.card_rarity(cs, card_id))
            row = cs.CARDS.get((card_id, 0))
            if row is not None and sorted(card["tags"]) != sorted(row.tags):
                observed[(card_id, "tags")] = (
                    tuple(sorted(card["tags"])), tuple(sorted(row.tags)))
        self.assertEqual(
            set(observed), set(dll_content.STALE_REPO_DATA),
            "the assembly and the committed tables disagree somewhere the "
            "ledger does not record (or the ledger records a divergence that "
            "no longer exists)")
        for key, (dll_value, committed) in observed.items():
            declared = dll_content.STALE_REPO_DATA[key]
            self.assertEqual((dll_value, committed), declared[:2], key)

    def test_dropping_any_entry_breaks_the_reproduction(self) -> None:
        """The mutation control: prove the ledger can fail.

        Without this, a ledger of no-ops would pass every other test here.
        """
        _facts()
        original = dict(dll_content.STALE_REPO_DATA)
        for key in original:
            with self.subTest(dropped=key):
                dll_content.STALE_REPO_DATA.pop(key)
                try:
                    source = dll_content.DllContentSource(_facts())
                    cs = _load_registry()
                    python = dll_content.PythonSource()
                    card_id, fact = key
                    row = cs.CARDS.get((card_id, 0))
                    if fact == "rarity":
                        self.assertNotEqual(
                            source.card_rarity(cs, card_id),
                            python.card_rarity(cs, card_id))
                    elif fact == "tags":
                        self.assertNotEqual(
                            source.card_tags(cs, card_id, row),
                            python.card_tags(cs, card_id, row))
                    else:  # pragma: no cover - guard for a new fact kind
                        self.fail(f"no mutation control for fact {fact!r}")
                finally:
                    dll_content.STALE_REPO_DATA.update(original)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

class _Discard:
    def write(self, _text):
        return None

    def flush(self):
        return None


def _assert_identical(case, out_dir):
    for name in _GENERATED:
        generated = (out_dir / name).read_bytes()
        committed = (RUST_DIR / "src" / name).read_bytes()
        if generated == committed:
            continue
        case.fail(f"{name}: regenerated output differs from the committed "
                  f"file ({len(generated)} vs {len(committed)} bytes). "
                  f"Compare with: diff {out_dir / name} "
                  f"{RUST_DIR / 'src' / name}")


def _load_generator():
    if "generate_content" in sys.modules:
        return sys.modules["generate_content"]
    spec = importlib.util.spec_from_file_location(
        "generate_content", TOOLS_DIR / "generate_content.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def _load_registry():
    return _load_generator().load_registry()


if __name__ == "__main__":
    if "--require-dll" in sys.argv:
        sys.argv.remove("--require-dll")
        REQUIRE_DLL = True
    unittest.main()

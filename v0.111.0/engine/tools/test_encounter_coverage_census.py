#!/usr/bin/env python3
"""Controls for the encounter coverage census (#2528 acceptance 4, #2529).

The census is a *frontier statement*: E4b workers pick their family from it and
review checks "built / refused by name" against it. So the failure modes that
matter are the silent ones — a row that disappears, a demand count charged to
the wrong key, or a "tabular" verdict on a builder that actually draws from a
stream at creation. Each control below is aimed at one of those.

Two are worth naming, because both are live hazards in this data and neither is
visible from reading the census output:

* **Substring dispatch.** `make_monsters` matches the registered key as a
  SUBSTRING of the wire id, and ten of the twelve elite encounters are
  registered without their `_ELITE` suffix. An equality join silently reports
  zero demand for all ten while dropping 100+ corpus fights, and the totals
  still look plausible. `charges_demand_through_the_substring_rule` is that
  case.
* **Cross-shard helpers.** `normal._corpse_slugs_normal` delegates to
  `weak.build_corpse_slugs`, which draws from the Encounter stream. A signal
  walk that stopped at the shard boundary would call the caller procedural for
  the wrong reason, and — worse — the same walk over a shard-local wrapper of a
  flat helper would call a tabular builder procedural. Resolution across the
  package is pinned in both directions.

Standard library only; run directly under either interpreter:

    python3   sim/v0.111.0/engine/tools/test_encounter_coverage_census.py
    python3.12 sim/v0.111.0/engine/tools/test_encounter_coverage_census.py
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve()
TOOLS_DIR = HERE.parent
RUST_DIR = TOOLS_DIR.parent


def _load():
    spec = importlib.util.spec_from_file_location(
        "encounter_coverage_census",
        TOOLS_DIR / "encounter_coverage_census.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CENSUS = _load()
COMMITTED = RUST_DIR / "ENCOUNTER_COVERAGE_CENSUS.json"


def _module(source: str, name: str = "pool"):
    directory = pathlib.Path(tempfile.mkdtemp())
    path = directory / f"{name}.py"
    path.write_text(source)
    return CENSUS._ModuleIndex(name, path)


class Normalization(unittest.TestCase):
    """The Rust variant, the registry key and the wire id share one form."""

    def test_the_three_spellings_agree(self) -> None:
        for spelling in ("CorpseSlugsWeak", "CORPSE_SLUGS_WEAK",
                         "ENCOUNTER.CORPSE_SLUGS_WEAK"):
            self.assertEqual(CENSUS._normalize(spelling), "corpseslugsweak")

    def test_digits_do_not_split_a_word(self) -> None:
        # `BattlewornDummyEventV1Encounter` must not gain a boundary inside
        # `V1`, or it stops joining `BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER`.
        self.assertEqual(
            CENSUS._normalize("BattlewornDummyEventV1Encounter"),
            CENSUS._normalize("BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER"))


class Dispatch(unittest.TestCase):
    """Demand is charged the way `make_monsters` dispatches."""

    KEYS = ["Decimillipede", "ScrollsOfBitingWeak", "ScrollsOfBitingNormal"]

    def test_charges_demand_through_the_substring_rule(self) -> None:
        self.assertEqual(
            CENSUS.dispatch("ENCOUNTER.DECIMILLIPEDE_ELITE", self.KEYS),
            "Decimillipede")

    def test_sibling_keys_stay_distinct(self) -> None:
        self.assertEqual(
            CENSUS.dispatch("ENCOUNTER.SCROLLS_OF_BITING_WEAK", self.KEYS),
            "ScrollsOfBitingWeak")
        self.assertEqual(
            CENSUS.dispatch("ENCOUNTER.SCROLLS_OF_BITING_NORMAL", self.KEYS),
            "ScrollsOfBitingNormal")

    def test_an_unclaimed_id_raises_rather_than_vanishing(self) -> None:
        with self.assertRaises(CENSUS.DispatchError):
            CENSUS.dispatch("ENCOUNTER.NOT_A_THING", self.KEYS)

    def test_an_ambiguous_id_raises_rather_than_picking_one(self) -> None:
        with self.assertRaises(CENSUS.DispatchError):
            CENSUS.dispatch("ENCOUNTER.SLIMES_WEAK", ["Slimes", "SlimesWeak"])


class RustAxis(unittest.TestCase):
    """The id axis comes from the generated table, and is checked."""

    def test_reads_the_real_table(self) -> None:
        ids = CENSUS.rust_encounter_ids()
        self.assertEqual(len(ids), len(set(ids)))
        self.assertIn("CorpseSlugsWeak", ids)

    def test_a_length_that_disagrees_with_the_body_is_rejected(self) -> None:
        directory = pathlib.Path(tempfile.mkdtemp())
        path = directory / "content_tables.rs"
        path.write_text(
            "pub static ENCOUNTER_MATCH_ORDER: [EncounterId; 3] = [\n"
            "    EncounterId::A, EncounterId::B,\n];\n")
        with self.assertRaises(SystemExit):
            CENSUS.rust_encounter_ids(path)

    def test_absent_registry_is_derived_not_declared(self) -> None:
        directory = pathlib.Path(tempfile.mkdtemp())
        (directory / "lib.rs").write_text("pub fn nothing() {}\n")
        built, registry = CENSUS.rust_built_ids(directory)
        self.assertEqual(built, set())
        self.assertIsNone(registry)

    def test_a_present_registry_is_read(self) -> None:
        directory = pathlib.Path(tempfile.mkdtemp())
        (directory / "rosters.rs").write_text(
            "pub static ENCOUNTER_ROSTER_BUILDERS: [Builder; 2] = [\n"
            "    (EncounterId::ToadpolesWeak, toadpoles),\n"
            "    (EncounterId::SeapunkWeak, seapunk),\n];\n")
        built, registry = CENSUS.rust_built_ids(directory)
        self.assertEqual(built, {"ToadpolesWeak", "SeapunkWeak"})
        self.assertIsNotNone(registry)


FLAT = '''
def _flat(ctx):
    return ctx.done([Monster(FOO, 10, max_hp=10, slot=0)])
'''

ROLLED = '''
def _rolled(ctx):
    taken = set()
    monsters = []
    for slot in range(2):
        hp = ctx.unique_hp(4, 7, taken)
        taken.add(hp)
        monsters.append(Monster(FOO, hp, max_hp=hp, slot=slot))
    return ctx.done(monsters)
'''

STREAMED = '''
def _streamed(ctx):
    offset = ctx.encounter_rng("starter offset").next_int(0, 3)
    return ctx.done([Monster(FOO, 10, max_hp=10, loop_pos=offset)])
'''

WRAPPER = '''
def _wrapper(ctx):
    return shared(ctx, 2)
'''

RECURSIVE = '''
def _a(ctx):
    return _b(ctx)


def _b(ctx):
    return _a(ctx)
'''


class Shape(unittest.TestCase):
    """`tabular` must mean "a generated table can express this"."""

    def test_a_flat_roster_is_tabular(self) -> None:
        module = _module(FLAT)
        self.assertEqual(CENSUS.shape_signals(module, "_flat"), set())

    def test_hp_rolls_alone_stay_tabular(self) -> None:
        # One Niche draw per monster is the shared primitive every roster
        # pays, generated or not. A `range(2)` roster is still a table.
        module = _module(ROLLED)
        self.assertEqual(CENSUS.shape_signals(module, "_rolled"), set())

    def test_a_creation_time_stream_draw_is_procedural(self) -> None:
        module = _module(STREAMED)
        self.assertEqual(
            CENSUS.shape_signals(module, "_streamed"), {"encounter_rng"})

    def test_a_cross_shard_helper_is_resolved_not_guessed(self) -> None:
        caller = _module(WRAPPER, "normal")
        helper = _module(STREAMED.replace("_streamed", "shared")
                         .replace("(ctx)", "(ctx, count)"), "weak")
        modules = {"normal": caller, "weak": helper}
        self.assertEqual(
            CENSUS.shape_signals(caller, "_wrapper", modules=modules),
            {"encounter_rng"})

    def test_an_unresolvable_helper_is_named_not_ignored(self) -> None:
        caller = _module(WRAPPER, "normal")
        self.assertEqual(
            CENSUS.shape_signals(caller, "_wrapper", modules={"normal": caller}),
            {"helper:shared"})

    def test_mutual_recursion_terminates(self) -> None:
        module = _module(RECURSIVE)
        self.assertEqual(CENSUS.shape_signals(module, "_a"), set())


class CommittedCensus(unittest.TestCase):
    """The checked-in artifact is internally consistent and complete.

    Deliberately NOT a freshness check against the corpus: the census's demand
    half needs `~/sts2-captures`, and a control that silently passes wherever
    the corpus is absent is worse than no control. What is pinned here is
    everything that holds without it.
    """

    def setUp(self) -> None:
        if not COMMITTED.exists():  # pragma: no cover - first-run guard
            self.skipTest(f"{COMMITTED} not generated yet")
        self.census = json.loads(COMMITTED.read_text())

    def test_rows_are_the_rust_id_axis_in_order(self) -> None:
        self.assertEqual(
            [row["rust_variant"] for row in self.census["rows"]],
            CENSUS.rust_encounter_ids())

    def test_every_id_has_a_python_builder(self) -> None:
        unregistered = [row["rust_variant"] for row in self.census["rows"]
                        if row["shape"] == "unregistered"]
        self.assertEqual(unregistered, [])

    def test_summary_matches_the_rows(self) -> None:
        summary = self.census["summary"]
        rows = self.census["rows"]
        self.assertEqual(sum(summary["shapes"].values()), len(rows))
        self.assertEqual(sum(summary["rust"].values()), len(rows))
        self.assertEqual(
            summary["fixtures_covered"],
            sum(row["fixtures"] for row in rows))

    def test_demand_totals_agree_with_the_corpus_totals(self) -> None:
        if not self.census["corpus_measured"]:  # pragma: no cover
            self.skipTest("census generated with --no-corpus")
        totals = self.census["corpus_totals"]
        self.assertEqual(
            self.census["summary"]["corpus_fights_covered"],
            totals["with_encounter"])
        self.assertEqual(
            totals["fights"],
            totals["with_encounter"] + totals["no_encounter"]
            + totals["unpaired"])

    def test_the_frontier_is_stated_as_a_status_per_id(self) -> None:
        for row in self.census["rows"]:
            self.assertIn(row["rust"], {"built", "absent"})
            self.assertIn(row["shape"], {"tabular", "procedural"})


class TheFreshnessCheck(unittest.TestCase):
    """`--check` must be able to fail, not merely to pass (#2531).

    The column it exists for is `rust`: a family wave PR that lands a roster
    without regenerating the census leaves an `absent` row behind a built
    encounter, and nothing else in the port gate would notice.
    """

    CENSUS_JSON = RUST_DIR / "ENCOUNTER_COVERAGE_CENSUS.json"

    def test_the_committed_census_is_current(self) -> None:
        self.assertEqual(CENSUS.check(self.CENSUS_JSON), [])

    def test_a_stale_built_column_is_reported(self) -> None:
        committed = json.loads(self.CENSUS_JSON.read_text())
        built = next(row for row in committed["rows"]
                     if row["rust"] == "built")
        built["rust"] = "absent"
        path = pathlib.Path(tempfile.mkdtemp()) / "census.json"
        path.write_text(json.dumps(committed))
        findings = CENSUS.check(path)
        self.assertTrue(
            any(finding.startswith(f"{built['rust_variant']}.rust")
                for finding in findings), findings)

    def test_a_missing_file_is_a_finding_not_a_pass(self) -> None:
        path = pathlib.Path(tempfile.mkdtemp()) / "absent.json"
        self.assertTrue(CENSUS.check(path))

    def test_a_dropped_row_is_reported(self) -> None:
        committed = json.loads(self.CENSUS_JSON.read_text())
        dropped = committed["rows"].pop(0)
        path = pathlib.Path(tempfile.mkdtemp()) / "census.json"
        path.write_text(json.dumps(committed))
        self.assertIn(f"{dropped['rust_variant']}: no committed row",
                      CENSUS.check(path))

    def test_a_drifted_provenance_column_is_reported(self) -> None:
        """#2827 item D: the provenance/shape columns now come from the frozen
        `data/encounter_provenance.v0.111.0.json`, not a live Python parse.
        A census row that disagrees with that data must still be a finding."""
        committed = json.loads(self.CENSUS_JSON.read_text())
        row = next(row for row in committed["rows"]
                   if row["shape"] == "procedural")
        row["shape"] = "tabular"
        path = pathlib.Path(tempfile.mkdtemp()) / "census.json"
        path.write_text(json.dumps(committed))
        self.assertIn(
            f"{row['rust_variant']}.shape: committed 'tabular', sources have "
            "'procedural'", CENSUS.check(path))

    def test_the_frozen_provenance_is_what_the_census_reads(self) -> None:
        provenance = CENSUS.frozen_provenance()
        committed = json.loads(self.CENSUS_JSON.read_text())
        for row in committed["rows"]:
            if row["shape"] == "unregistered":
                continue
            key = CENSUS._normalize(row["registered_key"].split(".", 1)[1])
            self.assertEqual(
                {column: row[column] for column in CENSUS.PROVENANCE_COLUMNS},
                provenance[key], row["rust_variant"])

    def test_a_malformed_provenance_file_is_refused(self) -> None:
        data = json.loads(CENSUS.PROVENANCE.read_text())
        first = next(iter(data["rows"]))
        del data["rows"][first]["shape"]
        path = pathlib.Path(tempfile.mkdtemp()) / "provenance.json"
        path.write_text(json.dumps(data))
        with self.assertRaises(SystemExit):
            CENSUS.frozen_provenance(path)
        data["schema"] = "something-else"
        path.write_text(json.dumps(data))
        with self.assertRaises(SystemExit):
            CENSUS.frozen_provenance(path)

    def test_corpus_demand_is_not_checked(self) -> None:
        """Demand needs the captures, so `--check` must ignore it."""
        committed = json.loads(self.CENSUS_JSON.read_text())
        for row in committed["rows"]:
            row["corpus_fights"] = 99999
        path = pathlib.Path(tempfile.mkdtemp()) / "census.json"
        path.write_text(json.dumps(committed))
        self.assertEqual(CENSUS.check(path), [])


if __name__ == "__main__":
    unittest.main()

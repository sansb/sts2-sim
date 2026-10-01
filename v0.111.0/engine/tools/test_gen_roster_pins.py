#!/usr/bin/env python3
"""Controls for `gen_roster_pins.py --relabel`, the stdlib half of the pins.

The E4b roster pins are frozen Python-oracle data since #2999 (#2827 item F
deletes `make_monsters`). What still moves is the eval manifest, and
`encounters::oracle::every_fixture_label_is_the_eval_manifests` re-derives
every case's fixture label from it in both directions. These controls pin
the relabel that keeps that crate test satisfiable after a re-seed (#2919)
with no simulator:

* on the committed manifest the relabel reproduces every committed pin file
  byte for byte, so the committed labels are exactly the manifest's;
* a renamed, dropped or added fixture moves the labels, the counts and
  `unpinned_fixtures` the way the crate test reads them, and nothing else.
  The expectations are relative to the committed `unpinned_fixtures`, which
  #2919's re-seed made non-empty;
* it refuses rather than guesses: a synthetic root that became a fixture, an
  ambiguous dispatch, two fixtures at one fight;
* `--check` fails on a stale pool, and the tool imports no simulator.

Before #2999 this file also exercised the Python generator's corpus and
explicit-fixture sources (#2787, #2536); those went with the generator.
Standard library, no corpus, no cargo. Run directly:

    python3 <crate>/tools/test_gen_roster_pins.py
"""

from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve()
TOOLS_DIR = HERE.parent
RUST_DIR = TOOLS_DIR.parent


def _load(name: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS_DIR / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


PINS = _load("gen_roster_pins")
MANIFEST = json.loads(PINS.MANIFEST.read_text())


def _committed(pool: str) -> dict:
    return json.loads(PINS.fixture_path(pool).read_text())


class ThePoolsAreTheCratesPools(unittest.TestCase):
    def test_pools_match_the_oracle_and_the_fixture_directory(self) -> None:
        oracle = (RUST_DIR / "src" / "encounters" / "oracle.rs").read_text()
        crate = sorted(re.findall(
            r'include_str!\("\.\./\.\./fixtures/(\w+)_rosters_v1\.json"\)',
            oracle))
        on_disk = sorted(p.name.removesuffix("_rosters_v1.json")
                         for p in (RUST_DIR / "fixtures").glob(
                             "*_rosters_v1.json"))
        self.assertEqual(list(PINS.POOLS), crate)
        self.assertEqual(list(PINS.POOLS), on_disk)


class RelabelReproducesTheCommittedPins(unittest.TestCase):
    def test_every_pool_is_fresh_on_the_committed_manifest(self) -> None:
        for pool in PINS.POOLS:
            with self.subTest(pool=pool):
                text = PINS.fixture_path(pool).read_text()
                self.assertEqual(
                    PINS.render(PINS.relabel(json.loads(text), MANIFEST, pool)),
                    text)

    def test_check_mode_reports_fresh(self) -> None:
        result = subprocess.run(
            [sys.executable, str(TOOLS_DIR / "gen_roster_pins.py"),
             "--relabel", "--check"],
            capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)


class RelabelFollowsTheManifest(unittest.TestCase):
    POOL = "weak"

    def setUp(self) -> None:
        self.pins = _committed(self.POOL)
        self.case = next(c for c in self.pins["cases"]
                         if c.get("fixture") and not c.get("synthetic"))
        self.manifest = copy.deepcopy(MANIFEST)

    def fight(self) -> dict:
        return next(f for f in self.manifest["fights"]
                    if f["id"] == self.case["fixture"])

    def relabel(self) -> dict:
        return PINS.relabel(self.pins, self.manifest, self.POOL)

    def committed_unpinned(self, pins: dict = None) -> list:
        """Fixtures a re-seed added after the freeze (#2919), as committed."""
        return list((pins or self.pins).get("unpinned_fixtures", []))

    def _same_but(self, out: dict, *keys: str) -> None:
        """Nothing but the named top-level keys and the labels moved."""
        strip = lambda doc: {k: v for k, v in doc.items()
                             if k not in keys and k != "cases"}
        self.assertEqual(strip(out), strip(self.pins))
        for before, after in zip(self.pins["cases"], out["cases"]):
            self.assertEqual(dict(before, fixture=None),
                             dict(after, fixture=None))

    def test_a_renamed_fixture_relabels_its_case(self) -> None:
        self.fight()["id"] = "ffff00000000aaaa"
        out = self.relabel()
        got = next(c for c in out["cases"]
                   if (c["seed"], c["node_index"])
                   == (self.case["seed"], self.case["node_index"]))
        self.assertEqual(got["fixture"], "ffff00000000aaaa")
        self.assertEqual(out["fixture_fights"], self.pins["fixture_fights"])
        self.assertEqual(out.get("unpinned_fixtures", []),
                         self.committed_unpinned())
        self._same_but(out)

    def test_a_dropped_fixture_keeps_its_case_unlabelled(self) -> None:
        self.manifest["fights"].remove(self.fight())
        out = self.relabel()
        self.assertEqual(len(out["cases"]), len(self.pins["cases"]))
        got = next(c for c in out["cases"]
                   if (c["seed"], c["node_index"])
                   == (self.case["seed"], self.case["node_index"]))
        self.assertIsNone(got["fixture"])
        self.assertEqual(out["fixture_fights"],
                         self.pins["fixture_fights"] - 1)
        wire = self.case["encounter"]
        self.assertEqual(out["per_encounter"][wire]["fixtures"],
                         self.pins["per_encounter"][wire]["fixtures"] - 1)
        self.assertEqual(out["per_encounter"][wire]["corpus"],
                         self.pins["per_encounter"][wire]["corpus"])
        self._same_but(out, "fixture_fights", "per_encounter")

    def test_an_added_pooled_fixture_is_listed_unpinned(self) -> None:
        self.manifest["fights"] += [
            {"id": "ffff00000000bbbb", "seed": "NEWSEED", "node": 9,
             "encounter": self.case["encounter"]},
            {"id": "ffff00000000aaaa", "seed": "NEWSEED", "node": 3,
             "encounter": self.case["encounter"]},
            # Out of this pool, and a fight with no encounter: not listed.
            {"id": "ffff00000000cccc", "seed": "NEWSEED", "node": 32,
             "encounter": "ENCOUNTER.THE_INSATIABLE_BOSS"},
            {"id": "ffff00000000dddd", "seed": "NEWSEED", "node": 40,
             "encounter": None},
        ]
        out = self.relabel()
        self.assertEqual(out["unpinned_fixtures"], sorted(
            self.committed_unpinned()
            + ["ffff00000000aaaa", "ffff00000000bbbb"]))
        self.assertEqual(out["fixture_fights"], self.pins["fixture_fights"])
        self._same_but(out, "unpinned_fixtures")
        # And the rendered bytes carry it, sorted with the other keys.
        self.assertIn('"unpinned_fixtures": [', PINS.render(out))

    def test_the_unpinned_list_follows_the_manifest_back_out(self) -> None:
        stale = dict(self.pins, unpinned_fixtures=sorted(
            self.committed_unpinned() + ["ffff00000000bbbb"]))
        out = PINS.relabel(stale, self.manifest, self.POOL)
        self.assertEqual(out.get("unpinned_fixtures", []),
                         self.committed_unpinned())
        self.assertEqual(PINS.render(out),
                         PINS.fixture_path(self.POOL).read_text())

    def test_membership_is_the_crate_tests_substring_rule(self) -> None:
        elite = _committed("elite")
        key = "DECIMILLIPEDE"
        self.assertIn(key, elite["registered_keys"])
        self.manifest["fights"].append(
            {"id": "ffff00000000eeee", "seed": "NEWSEED", "node": 24,
             "encounter": f"ENCOUNTER.{key}_ELITE"})
        out = PINS.relabel(elite, self.manifest, "elite")
        self.assertEqual(out["unpinned_fixtures"], sorted(
            self.committed_unpinned(elite) + ["ffff00000000eeee"]))


class RelabelRefusesRatherThanGuesses(unittest.TestCase):
    def test_a_synthetic_root_that_became_a_fixture_refuses(self) -> None:
        pins = _committed("normal_c")
        root = next(c for c in pins["cases"] if c.get("synthetic"))
        manifest = copy.deepcopy(MANIFEST)
        manifest["fights"].append(
            {"id": "ffff00000000ffff", "seed": root["seed"],
             "node": root["node_index"], "encounter": root["encounter"]})
        with self.assertRaises(SystemExit) as raised:
            PINS.relabel(pins, manifest, "normal_c")
        self.assertIn("ffff00000000ffff", str(raised.exception))

    def test_an_ambiguous_dispatch_refuses(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            PINS.dispatch("ENCOUNTER.SLIMES_WEAK", ["SLIMES", "SLIMES_WEAK"])
        self.assertIn("claimed by", str(raised.exception))

    def test_a_pinned_case_without_a_node_refuses(self) -> None:
        pins = _committed("weak")
        case = next(c for c in pins["cases"] if not c.get("synthetic"))
        case["node_index"] = None
        with self.assertRaises(SystemExit) as raised:
            PINS.relabel(pins, MANIFEST, "weak")
        self.assertIn("a pinned case has no node", str(raised.exception))

    def test_two_fixtures_at_one_fight_refuse(self) -> None:
        fight = dict(MANIFEST["fights"][0], id="ffff000000001111")
        manifest = dict(MANIFEST, fights=MANIFEST["fights"] + [fight])
        with self.assertRaises(SystemExit) as raised:
            PINS.relabel(_committed("weak"), manifest, "weak")
        self.assertIn("two fixtures", str(raised.exception))


class CheckModeAndPurity(unittest.TestCase):
    def test_check_fails_on_a_stale_pool_and_write_fixes_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rust = pathlib.Path(tmp) / "rust"
            shutil.copytree(RUST_DIR / "fixtures", rust / "fixtures",
                            ignore=shutil.ignore_patterns("*.mcr", "*.save"))
            manifest = copy.deepcopy(MANIFEST)
            manifest["fights"][0]["id"] = "ffff000000002222"
            path = pathlib.Path(tmp) / "manifest.json"
            path.write_text(json.dumps(manifest))
            command = [sys.executable, str(TOOLS_DIR / "gen_roster_pins.py"),
                       "--relabel", "--manifest", str(path),
                       "--rust-dir", str(rust)]
            stale = subprocess.run(command + ["--check"], capture_output=True,
                                   text=True, check=False)
            fixed = subprocess.run(command, capture_output=True, text=True,
                                   check=False)
            fresh = subprocess.run(command + ["--check"], capture_output=True,
                                   text=True, check=False)
        self.assertEqual(stale.returncode, 1, stale)
        self.assertIn("STALE", stale.stderr)
        self.assertEqual(fixed.returncode, 0, fixed)
        self.assertEqual(fresh.returncode, 0, fresh)

    def test_the_tool_imports_no_simulator(self) -> None:
        probe = (
            "import runpy, sys\n"
            f"sys.argv = [{str(TOOLS_DIR / 'gen_roster_pins.py')!r}, "
            "'--relabel', '--check']\n"
            "try:\n"
            f"    runpy.run_path({str(TOOLS_DIR / 'gen_roster_pins.py')!r}, "
            "run_name='__main__')\n"
            "except SystemExit as exc:\n"
            "    assert not exc.code, exc.code\n"
            "bad = sorted(m for m in sys.modules if m in {'combat_sim', "
            "'solve_fight', 'content', 'mcr_replay', 'project_state'})\n"
            "print(bad)\n")
        result = subprocess.run([sys.executable, "-c", probe],
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "[]")


if __name__ == "__main__":
    unittest.main()

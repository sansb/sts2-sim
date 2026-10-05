#!/usr/bin/env python3
"""Controls for `verify_search_suite.py`, and the check that keeps it true (#3683).

The committed search reports went stale within days of being written and no
lane ran the verifier, so a reader who ran the published tool on the published
reports got 291 failures of 293. Three things are held here, on whatever
engine binary the lane just built:

* **the committed reports check green by default**: every report under
  `eval/search` is either registered as historical in
  `historical-reports.json` (byte-identical to what was frozen, not replayed)
  or replays. A report added later without a registry entry is replayed, so a
  new report cannot rot unnoticed;
* **the round trip the README documents still works**: `search_suite.py`
  searches two committed fixtures on the current engine with a fixed playout
  count, and `verify_search_suite.py` replays every witness it kept in fresh
  processes to the same final digest. Nothing committed is compared, so an
  engine change cannot stale it; what it catches is a search whose witness
  does not replay, or a tool that no longer speaks the engine's protocol;
* **the verifier can fail**, and names why: a report damaged in each of the
  ways `FAILURE_KINDS` lists is reported as that kind, per run, without
  stopping at the first; a historical report that was edited is refused.

Standard library plus an engine binary. Run directly:

    python3 test_verify_search_suite.py [--binary PATH/TO/sts-sim]

The default binary is `target/release/sts-sim`. See
`.github/workflows/rust-port.yml` for the lane that runs it.
"""

from __future__ import annotations

import contextlib
import copy
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

TOOLS = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import verify_search_suite as vss  # noqa: E402

BINARY = vss.RUST / "target/release/sts-sim"

#: Two committed eval fixtures (Mawler, The Insatiable). `tests/recorded_line.rs`
#: reads the same two, so a re-seed that drops either is already a named break.
FIXTURES = ("f04442cd475cdc72", "fc78829c88941121")
METHODS = ("random", "uct")
SEEDS = (1, 2)
PLAYOUTS = 40


def run_verifier(*args):
    """`(exit code, summary rows, stderr)` of one in-process verifier run."""
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = vss.main(["--binary", str(BINARY), *map(str, args)])
    return code, json.loads(out.getvalue()), err.getvalue()


class CommittedReports(unittest.TestCase):
    def test_default_run_is_green_and_replays_no_historical_report(self):
        code, rows, err = run_verifier()
        self.assertEqual(code, 0, err)
        registry = vss.load_registry(vss.REGISTRY)
        self.assertEqual([row["report"] for row in rows if row.get("historical")], list(registry))
        for row in rows:
            if row.get("historical"):
                self.assertNotIn("witnesses_replayed", row)
                self.assertIn(f"{row['report']}: historical, not replayed", err)

    def test_every_committed_report_is_registered_or_replays(self):
        registry = vss.load_registry(vss.REGISTRY)
        reports = vss.committed_reports(vss.REGISTRY, registry)
        self.assertEqual({path.name for path in reports if path.name in registry}, set(registry))
        for name, record in registry.items():
            self.assertTrue((vss.SEARCH / name).is_file(), name)
            report = json.loads((vss.SEARCH / name).read_text())
            self.assertEqual(sum(1 for run in report["runs"] if run.get("best") is not None),
                             record["witnesses"], name)
            self.assertTrue(record["reason"].strip(), name)

    def test_an_edited_historical_report_is_refused(self):
        name = next(iter(vss.load_registry(vss.REGISTRY)))
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            (tmp / vss.REGISTRY.name).write_bytes(vss.REGISTRY.read_bytes())
            (tmp / name).write_bytes((vss.SEARCH / name).read_bytes() + b" ")
            code, rows, err = run_verifier(tmp / name, "--registry", tmp / vss.REGISTRY.name)
            self.assertEqual(code, 1)
            self.assertEqual(rows[0]["failed"], 1)
            self.assertIn("historical_report_changed", err)
            (tmp / name).unlink()
            code, _, err = run_verifier(tmp / name, "--registry", tmp / vss.REGISTRY.name)
            self.assertEqual(code, 1)
            self.assertIn("the file is missing", err)


class RoundTrip(unittest.TestCase):
    """Search on the current engine, then replay what the search kept."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        tmp = pathlib.Path(cls.tmp.name)
        manifest = {"schema": "search-suite-round-trip", "seconds_per_search": 60,
                    "seeds": list(SEEDS), "methods": list(METHODS),
                    "fights": [{"id": fixture} for fixture in FIXTURES]}
        (tmp / "manifest.json").write_text(json.dumps(manifest))
        cls.report_path = tmp / "report.json"
        done = subprocess.run(
            [sys.executable, str(TOOLS / "search_suite.py"), "--manifest", str(tmp / "manifest.json"),
             "--engine", str(BINARY), "--playouts", str(PLAYOUTS), "--out", str(cls.report_path)],
            capture_output=True, text=True, timeout=600)
        assert done.returncode == 0, done.stderr
        cls.report = json.loads(cls.report_path.read_text())

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def damaged(self, mutate):
        """Verify a copy of the fresh report with run 0 changed by `mutate`."""
        report = copy.deepcopy(self.report)
        mutate(report)
        path = pathlib.Path(self.tmp.name) / "damaged.json"
        path.write_text(json.dumps(report))
        code, rows, err = run_verifier(path, "--out", pathlib.Path(self.tmp.name) / "summary.json")
        written = json.loads((pathlib.Path(self.tmp.name) / "summary.json").read_text())
        return code, rows[0], written[0]["failures"], err

    def test_every_search_kept_a_witness_that_replays(self):
        runs = self.report["runs"]
        self.assertEqual(len(runs), len(FIXTURES) * len(METHODS) * len(SEEDS))
        for run in runs:
            self.assertEqual(run["playouts"], PLAYOUTS, run["fight_id"])
            self.assertIsNotNone(run.get("best"), run["fight_id"])
        code, rows, err = run_verifier(self.report_path)
        self.assertEqual(code, 0, err)
        self.assertEqual(rows, [{"report": "report.json", "witnesses_replayed": len(runs),
                                 "wins": sum(int(run["best"]["won"]) for run in runs),
                                 "refused_or_failed": 0, "failed": 0}])

    def test_a_run_without_a_witness_is_counted_not_failed(self):
        code, row, failures, _ = self.damaged(lambda report: report["runs"][0].update(best=None))
        self.assertEqual((code, row["refused_or_failed"], failures), (0, 1, []))

    def test_each_failure_kind_is_named_per_run_and_the_rest_still_replay(self):
        def missing(report):
            report["manifest"]["fights"].append({"id": "0000000000000000"})
            report["runs"][0]["fight_id"] = "0000000000000000"

        def frozen_entry_moved(report):
            report["manifest"]["fights"][0]["entry_sha256"] = "0" * 64

        def illegal(report):
            report["runs"][0]["best"]["actions"].insert(0, {"kind": "play", "uid": 10 ** 9})

        cases = {
            "entry_missing": missing,
            "entry_changed": lambda report: report["runs"][0].update(entry_digest="0" * 64),
            "line_does_not_replay": illegal,
            "final_hp_differs": lambda report: report["runs"][0]["best"].update(
                combat_hp=report["runs"][0]["best"]["combat_hp"] + 1),
            "final_digest_differs": lambda report: report["runs"][0]["best"].update(final_digest="0" * 64),
        }
        for kind, mutate in cases.items():
            with self.subTest(kind=kind):
                code, row, failures, err = self.damaged(mutate)
                self.assertEqual(code, 1)
                self.assertEqual([(failure["run"], failure["kind"]) for failure in failures], [(0, kind)])
                self.assertEqual(row["witnesses_replayed"], len(self.report["runs"]) - 1)
                self.assertIn(f"damaged.json run 0 ({failures[0]['fight_id']}", err)
                self.assertIn(kind, vss.FAILURE_KINDS)
        with self.subTest(kind="entry_missing (frozen entry moved)"):
            fight = self.report["manifest"]["fights"][0]["id"]
            code, row, failures, _ = self.damaged(frozen_entry_moved)
            self.assertEqual(code, 1)
            self.assertEqual({failure["kind"] for failure in failures}, {"entry_missing"})
            self.assertEqual({failure["fight_id"] for failure in failures}, {fight})
            self.assertEqual(row["failed"], len(METHODS) * len(SEEDS))

    def test_an_entry_the_engine_does_not_round_trip_is_refused(self):
        entry = json.loads(vss.entry_path({"id": FIXTURES[0]}).read_text())
        entry["player"]["hp"] = "not a number"
        with self.assertRaises(vss.WitnessFailure) as raised:
            vss.verify(entry, [], BINARY)
        self.assertEqual(raised.exception.kind, "entry_refused")


if __name__ == "__main__":
    if "--binary" in sys.argv:
        at = sys.argv.index("--binary")
        BINARY = pathlib.Path(sys.argv[at + 1]).resolve()
        del sys.argv[at:at + 2]
    if not BINARY.exists():
        sys.exit(f"{BINARY}: no engine binary; build it with "
                 "`cargo build --locked --release --bin sts-sim`, or pass --binary")
    unittest.main()

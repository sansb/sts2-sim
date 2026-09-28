#!/usr/bin/env python3
"""Focused, explicitly invoked contract for the Rust performance-floor tool."""

from __future__ import annotations

import importlib.util
import hashlib
import json
import pathlib
import tomllib
import unittest


TOOL = pathlib.Path(__file__).with_name("perf_floor.py")
SPEC = importlib.util.spec_from_file_location("perf_floor", TOOL)
assert SPEC is not None and SPEC.loader is not None
PERF = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PERF)
MANIFEST = TOOL.parent.parent / "Cargo.toml"
FLOORS = TOOL.parent.parent / "benchmarks" / "floors.json"
WORKFLOW = TOOL.parents[4] / ".github" / "workflows" / "rust-port-perf.yml"


class PerfFloorContractTests(unittest.TestCase):
    def test_rss_comparison_uses_unmodified_source_and_the_floor_allocator(self) -> None:
        import profile_intervals
        import profile_rss_origin

        profile = json.loads((FLOORS.parent /
            "2026-09-14-issue-2444-rss-origin.json").read_text())
        self.assertEqual(profile["harness_sha256"], hashlib.sha256(
            pathlib.Path(profile_rss_origin.__file__).read_bytes()).hexdigest())
        self.assertEqual(profile["common_harness_sha256"], hashlib.sha256(
            pathlib.Path(profile_intervals.__file__).read_bytes()).hexdigest())
        self.assertEqual([row["ref"] for row in profile["checkpoints"]],
                         profile_rss_origin.REFS)
        for row in profile["checkpoints"]:
            self.assertEqual(row["source_status"], "")
            self.assertEqual(row["vmmap"]["observer_returncode"], 0)
            for mode in ("plain", "allocation-counting"):
                self.assertEqual(len(row["measurements"][mode]), 5)
                for observation in row["measurements"][mode]:
                    report = observation["report"]
                    profile_intervals.identity(report)
                    self.assertEqual(report["allocation"]["instrumented"],
                                     mode == "allocation-counting")
                    self.assertEqual(json.loads(observation["raw"]["stdout"]), report)

    def test_historical_component_profile_reconciles_with_its_raw_results(self) -> None:
        """The cost justification must retain every interval and identical work."""
        import profile_intervals

        artifact = FLOORS.parent / "2026-09-14-issue-2444-component-profile.json"
        profile = json.loads(artifact.read_text())
        self.assertTrue(profile["diagnostic_only"])
        self.assertEqual(profile["harness_sha256"], hashlib.sha256(
            pathlib.Path(profile_intervals.__file__).read_bytes()).hexdigest())
        self.assertEqual([row["ref"] for row in profile["checkpoints"]],
                         profile_intervals.REFS)
        for row in profile["checkpoints"]:
            self.assertEqual(len(row["source_sha"]), 40)
            self.assertEqual(row["fixture_sha256"], profile_intervals.FIXTURE_SHA)
            for mode in ("plain", "diagnostic"):
                self.assertEqual(len(row["measurements"][mode]), 5)
                for observation in row["measurements"][mode]:
                    report = observation["report"]
                    profile_intervals.identity(report)
                    self.assertEqual(json.loads(observation["raw"]["stdout"]), report)
                    self.assertEqual(observation["raw"]["returncode"], 0)
                    self.assertIn(str(observation["peak_rss_bytes"]),
                                  observation["raw"]["stderr"])
                    if mode == "diagnostic":
                        self.assertLessEqual(sum(part["nanos"] for part in
                            report["components"].values()), report["timing"]["wall_nanos"])
                        self.assertGreaterEqual(report["heap"]["requested_peak_since_warmup"],
                                                report["heap"]["requested_live_after_warmup"])
            for observer in ("sample", "vmmap"):
                self.assertEqual(row[observer]["observer_returncode"], 0)
                profile_intervals.identity(json.loads(row[observer]["stdout"]))

    def test_measured_release_binary_omits_unwinding(self) -> None:
        """I5 refusals are Results; release panic unwinding is dead RSS weight."""
        manifest = tomllib.loads(MANIFEST.read_text())
        self.assertEqual(manifest["profile"]["release"]["panic"], "abort")

    def test_repeat_and_best_of_n_statistic_are_unchanged(self) -> None:
        """Control the host first; changing the statistic needs later evidence."""
        self.assertEqual(PERF.DEFAULT_REPEAT, 5)
        observations = {
            "timed": {
                "example": {
                    "best_transitions_per_second": 101.0,
                    "median_transitions_per_second": 90.0,
                    "worst_transitions_per_second": 80.0,
                    "relative_spread": 21.0 / 101.0,
                    "throughputs": [80.0, 85.0, 90.0, 95.0, 101.0],
                    "identity": {
                        "entry_sha256": "entry",
                        "checksum": "checksum",
                        "transitions": 10,
                    },
                }
            },
            "instrumented": {
                "example": {
                    "peak_rss_bytes": 100,
                    "run": {
                        "allocation": {
                            "allocations_per_transition": 1.0,
                            "allocated_bytes_per_transition": 1.0,
                        }
                    },
                }
            },
        }
        floors = {
            "workloads": {
                "example": {
                    "evidence": "fixture.json",
                    "transitions_per_second_floor": 100.0,
                    "peak_rss_bytes_ceiling": 101,
                    "allocations_per_transition_ceiling": 2.0,
                    "allocated_bytes_per_transition_ceiling": 2.0,
                    "observed_at_baseline": {
                        "entry_sha256": "entry",
                        "checksum": "checksum",
                        "transitions": 10,
                    },
                }
            }
        }
        breaches, drifts, lines = PERF.compare(
            observations, floors, ["example"])
        self.assertEqual(breaches, [])
        self.assertEqual(drifts, [])
        self.assertTrue(any(
            "best 101/s of 5 runs, median 90/s" in line for line in lines))

    def test_artifact_hashes_the_byte_pinned_v1_entry(self) -> None:
        """Evidence must name the workload input, not a mutable projection fixture."""
        fixture = PERF.BENCH_V1_FIXTURE
        self.assertEqual(fixture.name, "bench_ironclad_toadpoles_uniform_v1.json")
        self.assertEqual(
            PERF.artifact_hashes()["fixture_sha256"],
            hashlib.sha256(fixture.read_bytes()).hexdigest(),
        )

    def test_ci_rebaseline_returns_reviewable_candidates(self) -> None:
        """CI-timed floors must be downloadable rather than copied from logs."""
        workflow = WORKFLOW.read_text()
        self.assertIn("rebaseline_args=(--rebaseline --write-floors)", workflow)
        self.assertIn("git ls-files --others --exclude-standard", workflow)
        self.assertIn("uses: actions/upload-artifact@v4", workflow)
        self.assertIn("versions/v0.111.0/rust/benchmarks/floors.json", workflow)
        self.assertIn("steps.rebaseline-artifact.outputs.evidence", workflow)

    def test_committed_floors_are_exactly_derived_from_their_evidence(self) -> None:
        """A cited file must exist and reproduce every committed limit."""
        floors = json.loads(FLOORS.read_text())
        workloads = list(floors["workloads"])
        self.assertEqual(workloads, ["ironclad_toadpoles_uniform_v1"])
        evidence_name = floors["workloads"][workloads[0]]["evidence"]
        evidence_path = FLOORS.parent.parent / evidence_name
        evidence = json.loads(evidence_path.read_text())
        self.assertEqual(evidence["schema"], PERF.EVIDENCE_SCHEMA)
        self.assertFalse(evidence["git"]["dirty"])
        self.assertEqual(evidence["artifact_hashes"], PERF.artifact_hashes())
        tolerance = floors["workloads"][workloads[0]]["tolerance"]
        derived = PERF.proposed_floors(
            evidence,
            workloads,
            evidence_name,
            tolerance["throughput"],
            tolerance["peak_rss"],
            tolerance["allocation"],
        )
        self.assertEqual(floors, derived)


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/env python3
"""Post-submit performance floors for the v0.111.0 Rust port (PORT_PLAN §6/§7).

This is the measured half of the port's performance discipline. The
deterministic half — hot-type size assertions and the allocations-per-
transition pin — runs on every PR at zero noise. Throughput and peak RSS
cannot: they are real-hardware measurements, so they run **post-submit on the
self-hosted `mac-solver` runner**, per merge, where a red run is attributable
to a single commit. The workflow holds the host-wide exclusive workload lock
around this entire tool; a manual invocation outside that wrapper is not
controlled CI evidence.

What one invocation does:

1. builds the crate twice, exactly as `queen_benchmark.py` does — a plain
   release build for the timed passes, then an `allocation-counting` build for
   the instrumented pass (the counting allocator is not free, so it must never
   be inside the timed window);
2. runs `sts-sim bench` `--repeat` times per workload and takes **best-of-N**
   throughput. Best-of-N remains the statistic of record while the serialized
   lane accumulates evidence; residual host/OS noise is predominantly one-sided
   (other work only slows a run down), so the maximum is the closest available
   estimate of what the machine can actually do;
3. asserts every repeat did **identical work** — same entry sha256, same action
   checksum, same transition count. Comparing throughput across different work
   is meaningless, and this is what catches an accidentally nondeterministic
   workload before it is mistaken for a regression;
4. measures peak RSS with `/usr/bin/time -l` around the instrumented pass;
5. compares everything against `benchmarks/floors.json` and exits nonzero on
   any breach, printing which floor moved and by how much.

`--rebaseline` writes a full evidence artifact (environment, rustc, load
averages, artifact hashes, every run's number — not just the best) to
`benchmarks/` and prints the floors block those numbers justify.
**floors.json must name its evidence artifact**: a floor that moved without one
is a floor nobody can audit.

Exit codes: 0 green, 1 floor breach, 2 refusal (bad invocation, broken
environment, nondeterministic workload).

Coupling to #1283: the workload is the synthetic stand-in described in
`src/bench.rs` — the eval suite does not exist yet. When it lands, each eval
fight becomes another workload row and another floors.json entry; this tool
does not change shape.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import statistics
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[4]
RUST = ROOT / "versions/v0.111.0/rust"
BINARY = RUST / "target/release/sts-sim"
MANIFEST = RUST / "Cargo.toml"
FLOORS = RUST / "benchmarks/floors.json"
BENCHMARKS = RUST / "benchmarks"
BENCH_SOURCE = RUST / "src/bench.rs"
BENCH_V1_FIXTURE = RUST / "fixtures/bench_ironclad_toadpoles_uniform_v1.json"
ENGINE_SOURCE = RUST / "src/engine/mod.rs"

FLOORS_SCHEMA = "sts-sim-perf-floors/v1"
EVIDENCE_SCHEMA = "sts-sim-perf-floor-evidence/v1"
BENCH_SCHEMA = "sts-sim-bench/v1"

DEFAULT_REPEAT = 5

# Tolerances the initial floors were derived with. See `--rebaseline` output and
# PERF.md for the derivation; the short version is that best-of-5 on this
# hardware class reproduces within ~2%, and the floors sit five to ten times
# further out than that so the lane is a creep detector rather than a
# microbenchmark.
DEFAULT_THROUGHPUT_TOLERANCE = 0.15
DEFAULT_RSS_TOLERANCE = 0.50
DEFAULT_ALLOCATION_TOLERANCE = 0.10


class Refusal(Exception):
    """A typed refusal: the tool cannot produce a verdict, so it produces none."""


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command, cwd=ROOT, check=True, capture_output=True, text=True, **kwargs)


def system_value(*command: str) -> str | None:
    try:
        return run(list(command)).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError, OSError):
        return None


def git(*args: str) -> str:
    return run(["git", *args]).stdout.strip()


def build_release(*, allocation_counting: bool = False) -> list[str]:
    """Build the binary the next passes measure.

    Both variants land on the same `target/release/sts-sim` path, so the caller
    must finish every timed pass before asking for the instrumented build.
    """
    command = [
        "cargo", "build", "--release", "--locked", "--manifest-path", str(MANIFEST),
    ]
    if allocation_counting:
        command.extend(["--features", "allocation-counting"])
    subprocess.run(command, cwd=ROOT, check=True)
    return command


def bench_command(workload: str, overrides: dict[str, int]) -> list[str]:
    command = [str(BINARY), "bench", "--workload", workload]
    for flag, value in sorted(overrides.items()):
        command.extend([f"--{flag}", str(value)])
    return command


def bench(workload: str, overrides: dict[str, int]) -> dict[str, Any]:
    command = bench_command(workload, overrides)
    report = json.loads(run(command).stdout)
    if report.get("schema") != BENCH_SCHEMA:
        raise Refusal(
            f"bench reported schema {report.get('schema')!r}, expected {BENCH_SCHEMA!r}")
    return report


def bench_with_rss(workload: str, overrides: dict[str, int]) -> tuple[dict[str, Any], int]:
    """One instrumented pass, with peak RSS read from `/usr/bin/time -l`."""
    command = ["/usr/bin/time", "-l", *bench_command(workload, overrides)]
    completed = run(command)
    match = re.search(r"(\d+)\s+maximum resident set size", completed.stderr)
    if match is None:
        raise Refusal("/usr/bin/time -l did not report a peak resident set size")
    report = json.loads(completed.stdout)
    if not report["allocation"]["instrumented"]:
        raise Refusal(
            "the instrumented pass ran a plain binary; the allocation-counting "
            "build did not take effect")
    return report, int(match.group(1))


def workload_identity(report: dict[str, Any]) -> dict[str, Any]:
    """The facts that must match for two throughput numbers to be comparable."""
    return {
        "entry_sha256": report["entry_sha256"],
        "checksum": report["work"]["checksum"],
        "transitions": report["work"]["transitions"],
        "config": report["config"],
    }


def cargo_pin_ceiling() -> int | None:
    """The allocations-per-transition ceiling the cargo-test pin asserts.

    Read out of `src/engine/mod.rs` by regex, deliberately as an **advisory**
    cross-check rather than a gate: that file belongs to the engine slices, and
    a rename there must not stop the line on a lane that does not own it. The
    enforced allocation ceiling is the one in floors.json; this only answers
    "is the benchmark's whole-loop number consistent with the per-apply pin",
    which is how a silently broken allocation counter would show up.
    """
    try:
        source = ENGINE_SOURCE.read_text()
    except OSError:
        return None
    match = re.search(r"const CEILING:\s*u64\s*=\s*(\d+)\s*;", source)
    return int(match.group(1)) if match else None


def measure(
    workloads: list[str],
    repeat: int,
    overrides: dict[str, int],
) -> dict[str, Any]:
    """Run every pass and return the raw observations, with no verdict attached."""
    load_start = os.getloadavg()
    release_build = build_release()
    # Hashed here rather than at report time: the two builds share one output
    # path (cargo uplifts the right artifact per feature set), so by the end of
    # the run the path holds the instrumented binary. Recording both hashes is
    # what lets a reader of the evidence artifact tell which binary produced
    # which number.
    plain_binary_sha256 = sha256(BINARY)
    timed: dict[str, Any] = {}
    for workload in workloads:
        runs = [bench(workload, overrides) for _ in range(repeat)]
        identities = [workload_identity(report) for report in runs]
        if any(identity != identities[0] for identity in identities[1:]):
            raise Refusal(
                f"workload {workload!r} did not do identical work across "
                f"{repeat} runs; throughput numbers are not comparable. "
                f"Identities: {json.dumps(identities, sort_keys=True)}")
        throughputs = [report["timing"]["transitions_per_second"] for report in runs]
        timed[workload] = {
            "command": bench_command(workload, overrides),
            "identity": identities[0],
            "runs": runs,
            "throughputs": throughputs,
            "best_transitions_per_second": max(throughputs),
            "median_transitions_per_second": statistics.median(throughputs),
            "worst_transitions_per_second": min(throughputs),
            "relative_spread": (max(throughputs) - min(throughputs)) / max(throughputs),
        }

    allocation_build = build_release(allocation_counting=True)
    instrumented_binary_sha256 = sha256(BINARY)
    if instrumented_binary_sha256 == plain_binary_sha256:
        raise Refusal(
            "the allocation-counting build produced the same binary as the "
            "plain build; the feature did not take effect and the allocation "
            "numbers would be meaningless")
    instrumented: dict[str, Any] = {}
    for workload in workloads:
        report, peak_rss = bench_with_rss(workload, overrides)
        instrumented[workload] = {
            "command": ["/usr/bin/time", "-l", *bench_command(workload, overrides)],
            "identity": workload_identity(report),
            "run": report,
            "peak_rss_bytes": peak_rss,
        }
        # The instrumented binary is slower but must still be the same workload;
        # if it is not, one of the two builds is measuring something else.
        if instrumented[workload]["identity"] != timed[workload]["identity"]:
            raise Refusal(
                f"workload {workload!r} differs between the plain and "
                f"allocation-counting builds; the two passes are not describing "
                f"the same work")
    load_end = os.getloadavg()

    return {
        "load_average_start": load_start,
        "load_average_end": load_end,
        "build_commands": {
            "release": release_build,
            "allocation_counting": allocation_build,
        },
        "binary_hashes": {
            "release_sha256": plain_binary_sha256,
            "allocation_counting_sha256": instrumented_binary_sha256,
        },
        "timed": timed,
        "instrumented": instrumented,
    }


def environment() -> dict[str, Any]:
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
        "rustc": system_value("rustc", "--version"),
        "cargo": system_value("cargo", "--version"),
        "cpu": system_value("sysctl", "-n", "machdep.cpu.brand_string"),
        "logical_cpus": os.cpu_count(),
        "memory_bytes": system_value("sysctl", "-n", "hw.memsize"),
        "hostname": platform.node(),
    }


def artifact_hashes() -> dict[str, str]:
    return {
        "bench_source_sha256": sha256(BENCH_SOURCE),
        "perf_floor_tool_sha256": sha256(pathlib.Path(__file__).resolve()),
        # The named v1 workload owns this byte-pinned input.  The canonical
        # projection fixture is deliberately allowed to evolve independently.
        "fixture_sha256": sha256(BENCH_V1_FIXTURE),
    }


def git_state() -> dict[str, Any]:
    dirty = git("status", "--short")
    return {
        "head_sha": git("rev-parse", "HEAD"),
        "dirty": bool(dirty),
        "dirty_paths": dirty.splitlines(),
    }


def load_floors(path: pathlib.Path) -> dict[str, Any]:
    if not path.exists():
        raise Refusal(
            f"no floors file at {path}; run with --rebaseline to derive one")
    floors = json.loads(path.read_text())
    if floors.get("schema") != FLOORS_SCHEMA:
        raise Refusal(
            f"{path} reports schema {floors.get('schema')!r}, "
            f"expected {FLOORS_SCHEMA!r}")
    return floors


def compare(
    observations: dict[str, Any],
    floors: dict[str, Any],
    workloads: list[str],
) -> tuple[list[str], list[str], list[str]]:
    """Return (breaches, drifts, lines) — lines being the human report body."""
    breaches: list[str] = []
    drifts: list[str] = []
    lines: list[str] = []
    for workload in workloads:
        floor = floors["workloads"].get(workload)
        if floor is None:
            raise Refusal(
                f"floors.json carries no entry for workload {workload!r}; "
                f"add one via --rebaseline in a reviewed diff")
        timed = observations["timed"][workload]
        instrumented = observations["instrumented"][workload]
        best = timed["best_transitions_per_second"]
        rss = instrumented["peak_rss_bytes"]
        allocation = instrumented["run"]["allocation"]
        per_transition = allocation["allocations_per_transition"]
        bytes_per_transition = allocation["allocated_bytes_per_transition"]

        lines.append(f"workload {workload}")
        lines.append(
            f"  evidence for these floors: {floor.get('evidence', '(none named)')}")

        def check(label: str, measured: float, limit: float, higher_is_better: bool) -> None:
            ok = measured >= limit if higher_is_better else measured <= limit
            margin = (measured - limit) / limit if limit else float("inf")
            relation = "floor" if higher_is_better else "ceiling"
            lines.append(
                f"  [{'PASS' if ok else 'FAIL'}] {label}: measured {measured:,.4g} "
                f"vs {relation} {limit:,.4g} ({margin:+.1%})")
            if not ok:
                breaches.append(
                    f"{workload}: {label} measured {measured:,.4g}, "
                    f"{relation} {limit:,.4g} ({margin:+.1%})")

        check("transitions/s", best, floor["transitions_per_second_floor"], True)
        check("peak RSS (bytes)", rss, floor["peak_rss_bytes_ceiling"], False)
        check(
            "allocations/transition", per_transition,
            floor["allocations_per_transition_ceiling"], False)
        check(
            "allocated bytes/transition", bytes_per_transition,
            floor["allocated_bytes_per_transition_ceiling"], False)

        lines.append(
            f"  observed: best {best:,.0f}/s of {len(timed['throughputs'])} runs, "
            f"median {timed['median_transitions_per_second']:,.0f}/s, "
            f"worst {timed['worst_transitions_per_second']:,.0f}/s, "
            f"spread {timed['relative_spread']:.1%}")

        # Work identity against the baseline is a DRIFT NOTE, never a breach.
        # A slice that adds a step kind legitimately changes the action space
        # and therefore the checksum; that must not stop the line. It does make
        # the throughput comparison approximate, so it is printed loudly and
        # recorded, and the next reviewed rebaseline refreshes it.
        baseline_identity = floor.get("observed_at_baseline", {})
        for key in ("entry_sha256", "checksum", "transitions"):
            if key in baseline_identity and baseline_identity[key] != timed["identity"][key]:
                drift = (
                    f"{workload}: {key} moved since the baseline "
                    f"({baseline_identity[key]} -> {timed['identity'][key]}); "
                    f"the workload changed, so the throughput comparison is "
                    f"approximate until the next reviewed rebaseline")
                drifts.append(drift)
                lines.append(f"  [DRIFT] {drift}")

        pin = cargo_pin_ceiling()
        if pin is None:
            lines.append(
                "  [NOTE] cargo-test allocation pin not located in "
                "src/engine/mod.rs; cross-check skipped (advisory only)")
        elif per_transition < pin:
            lines.append(
                f"  [NOTE] allocations/transition {per_transition:.2f} is BELOW "
                f"the cargo-test per-apply pin ceiling {pin}; the bench window "
                f"is strictly larger than the pin's, so this suggests the "
                f"allocation counter is not recording (advisory only)")
        else:
            lines.append(
                f"  [NOTE] allocations/transition {per_transition:.2f} "
                f"is consistent with the cargo-test per-apply pin ceiling {pin} "
                f"(bench brackets the whole playout loop, so it must be larger)")
    return breaches, drifts, lines


def proposed_floors(
    observations: dict[str, Any],
    workloads: list[str],
    evidence_name: str,
    throughput_tolerance: float,
    rss_tolerance: float,
    allocation_tolerance: float,
) -> dict[str, Any]:
    entries: dict[str, Any] = {}
    for workload in workloads:
        timed = observations["timed"][workload]
        instrumented = observations["instrumented"][workload]
        best = timed["best_transitions_per_second"]
        rss = instrumented["peak_rss_bytes"]
        allocation = instrumented["run"]["allocation"]
        entries[workload] = {
            "evidence": evidence_name,
            "tolerance": {
                "throughput": throughput_tolerance,
                "peak_rss": rss_tolerance,
                "allocation": allocation_tolerance,
            },
            "measured_at_baseline": {
                "best_transitions_per_second": round(best, 1),
                "median_transitions_per_second": round(
                    timed["median_transitions_per_second"], 1),
                "worst_transitions_per_second": round(
                    timed["worst_transitions_per_second"], 1),
                "relative_spread": round(timed["relative_spread"], 4),
                "runs": len(timed["throughputs"]),
                "peak_rss_bytes": rss,
                "allocations_per_transition": round(
                    allocation["allocations_per_transition"], 4),
                "allocated_bytes_per_transition": round(
                    allocation["allocated_bytes_per_transition"], 4),
            },
            "observed_at_baseline": {
                "entry_sha256": timed["identity"]["entry_sha256"],
                "checksum": timed["identity"]["checksum"],
                "transitions": timed["identity"]["transitions"],
            },
            "transitions_per_second_floor": round(best * (1.0 - throughput_tolerance)),
            "peak_rss_bytes_ceiling": int(rss * (1.0 + rss_tolerance)),
            "allocations_per_transition_ceiling": round(
                allocation["allocations_per_transition"] * (1.0 + allocation_tolerance), 2),
            "allocated_bytes_per_transition_ceiling": round(
                allocation["allocated_bytes_per_transition"] * (1.0 + allocation_tolerance), 1),
        }
    return {
        "schema": FLOORS_SCHEMA,
        "notes": [
            "Floors are enforced post-submit only (.github/workflows/rust-port-perf.yml).",
            "A floor moves only in a reviewed diff that names its evidence artifact.",
            "throughput floor = best-of-N baseline * (1 - throughput tolerance).",
            "peak_rss / allocation ceilings = baseline * (1 + their tolerance).",
            "The workload is the #1283 synthetic stand-in; eval fights append rows.",
        ],
        "workloads": entries,
    }


def evidence_artifact(
    observations: dict[str, Any],
    workloads: list[str],
    repeat: int,
    overrides: dict[str, int],
) -> dict[str, Any]:
    return {
        "schema": EVIDENCE_SCHEMA,
        "recorded_utc": datetime.datetime.now(datetime.UTC).isoformat(),
        "git": git_state(),
        "repeat": repeat,
        "overrides": overrides,
        "workloads": workloads,
        "environment": environment(),
        "artifact_hashes": artifact_hashes(),
        "load_average_start": observations["load_average_start"],
        "load_average_end": observations["load_average_end"],
        "build_commands": observations["build_commands"],
        "binary_hashes": observations["binary_hashes"],
        # The full distribution, not just the best: a floor derived from one
        # number nobody can see the spread of is not evidence.
        "timed": {
            workload: {
                "command": observations["timed"][workload]["command"],
                "identity": observations["timed"][workload]["identity"],
                "throughputs": observations["timed"][workload]["throughputs"],
                "wall_seconds": [
                    report["timing"]["wall_seconds"]
                    for report in observations["timed"][workload]["runs"]
                ],
                "best_transitions_per_second":
                    observations["timed"][workload]["best_transitions_per_second"],
                "median_transitions_per_second":
                    observations["timed"][workload]["median_transitions_per_second"],
                "worst_transitions_per_second":
                    observations["timed"][workload]["worst_transitions_per_second"],
                "relative_spread": observations["timed"][workload]["relative_spread"],
                "reports": observations["timed"][workload]["runs"],
            }
            for workload in workloads
        },
        "instrumented": observations["instrumented"],
        "cargo_test_allocation_pin_ceiling": cargo_pin_ceiling(),
        "caveats": [
            "Measured on the shared developer/runner Mac. CI measurements hold "
            "the host-wide exclusive workload lock; manual invocations must "
            "provide their own quiet-host discipline.",
            "Load averages are recorded honestly at both ends of the run; a "
            "baseline taken under heavy load understates the machine and "
            "therefore sets a conservative floor.",
            "The allocation and RSS pass uses the allocation-counting build, "
            "which is slower than the timed build by design; its throughput "
            "number is not comparable to the timed passes.",
            "The workload is a seeded uniform-random policy over one canonical "
            "fixture, not a search and not a real eval fight (#1283).",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--workload", action="append", dest="workloads",
        help="workload to measure (repeatable); default: every workload the binary lists")
    parser.add_argument(
        "--repeat", type=int, default=DEFAULT_REPEAT,
        help=f"timed passes per workload, best-of-N (default {DEFAULT_REPEAT})")
    parser.add_argument("--floors", type=pathlib.Path, default=FLOORS)
    parser.add_argument(
        "--rebaseline", action="store_true",
        help="write an evidence artifact and print the floors block it justifies")
    parser.add_argument(
        "--write-floors", action="store_true",
        help="with --rebaseline, also write the floors file (still a reviewed diff)")
    parser.add_argument(
        "--evidence", type=pathlib.Path, default=None,
        help="path for the --rebaseline evidence artifact (default: benchmarks/DATE-perf-floor-*.json)")
    parser.add_argument(
        "--slug", default="perf-floor-baseline-v1",
        help="slug used in the default evidence artifact filename")
    parser.add_argument(
        "--allow-dirty", action="store_true",
        help="permit --rebaseline from a dirty tree (the artifact records it either way)")
    parser.add_argument(
        "--trajectories", type=int, default=None,
        help="override the workload's trajectory count; makes the run ADVISORY "
             "(no floor verdict), because it is no longer the floored workload")
    parser.add_argument(
        "--json", type=pathlib.Path, default=None,
        help="also write the full observations to this path")
    args = parser.parse_args()

    if args.repeat < 1:
        print("refusal: --repeat must be at least 1", file=sys.stderr)
        return 2

    overrides: dict[str, int] = {}
    if args.trajectories is not None:
        overrides["trajectories"] = args.trajectories

    try:
        build_release()
        listed = json.loads(run([str(BINARY), "bench", "--list"]).stdout)
        available = [row["name"] for row in listed["workloads"]]
        workloads = args.workloads or available
        unknown = [name for name in workloads if name not in available]
        if unknown:
            raise Refusal(
                f"unknown workload(s) {unknown}; the binary offers {available}")

        if args.rebaseline and not args.allow_dirty and git_state()["dirty"]:
            raise Refusal(
                "--rebaseline from a dirty tree: the evidence artifact would "
                "name a SHA that does not describe what was measured. Commit "
                "first, or pass --allow-dirty and say so in the PR body.")

        observations = measure(workloads, args.repeat, overrides)
    except Refusal as refusal:
        print(f"refusal: {refusal}", file=sys.stderr)
        return 2
    except subprocess.CalledProcessError as error:
        print(f"refusal: {error.cmd} failed ({error.returncode})", file=sys.stderr)
        print(error.stderr, file=sys.stderr)
        return 2

    if args.json is not None:
        args.json.write_text(json.dumps(observations, indent=2, sort_keys=True) + "\n")

    print(f"host: {platform.node()} / {platform.machine()} / {environment()['cpu']}")
    print(f"load average: start {observations['load_average_start']} "
          f"end {observations['load_average_end']}")
    print(f"repeat: {args.repeat} (best-of-N throughput)")
    print()

    if args.rebaseline:
        evidence_path = args.evidence
        if evidence_path is None:
            today = datetime.datetime.now(datetime.UTC).date().isoformat()
            evidence_path = BENCHMARKS / f"{today}-{args.slug}.json"
        evidence_path.parent.mkdir(parents=True, exist_ok=True)
        artifact = evidence_artifact(observations, workloads, args.repeat, overrides)
        evidence_path.write_text(json.dumps(artifact, indent=2, sort_keys=True) + "\n")
        block = proposed_floors(
            observations, workloads,
            f"benchmarks/{evidence_path.name}",
            DEFAULT_THROUGHPUT_TOLERANCE,
            DEFAULT_RSS_TOLERANCE,
            DEFAULT_ALLOCATION_TOLERANCE)
        print(f"evidence artifact: {evidence_path}")
        for workload in workloads:
            timed = observations["timed"][workload]
            print(f"  {workload}: best {timed['best_transitions_per_second']:,.0f}/s, "
                  f"median {timed['median_transitions_per_second']:,.0f}/s, "
                  f"spread {timed['relative_spread']:.2%} over {args.repeat} runs")
        print()
        print("proposed floors block (paste into a reviewed diff):")
        print(json.dumps(block, indent=2, sort_keys=True))
        if args.write_floors:
            args.floors.parent.mkdir(parents=True, exist_ok=True)
            args.floors.write_text(json.dumps(block, indent=2, sort_keys=True) + "\n")
            print(f"\nwrote {args.floors}")
        return 0

    if overrides:
        print("ADVISORY ONLY: the workload was overridden "
              f"({overrides}), so it is not the floored workload and no verdict "
              "is issued.")
        for workload in workloads:
            timed = observations["timed"][workload]
            print(f"  {workload}: best {timed['best_transitions_per_second']:,.0f}/s "
                  f"over {args.repeat} runs, "
                  f"peak RSS {observations['instrumented'][workload]['peak_rss_bytes']:,} bytes")
        return 0

    try:
        floors = load_floors(args.floors)
        breaches, drifts, lines = compare(observations, floors, workloads)
    except Refusal as refusal:
        print(f"refusal: {refusal}", file=sys.stderr)
        return 2

    print("\n".join(lines))
    print()
    if breaches:
        print("PERFORMANCE FLOOR BREACH — this lane is stop-the-line.")
        for breach in breaches:
            print(f"  - {breach}")
        print()
        print("Fix or revert the merge that caused it. If the new numbers are "
              "correct and intended, move the floor in a reviewed diff that "
              "names a fresh --rebaseline evidence artifact; never widen a "
              "floor to make a red run green.")
        return 1
    if drifts:
        print("floors green, with workload drift noted above.")
    else:
        print("floors green.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

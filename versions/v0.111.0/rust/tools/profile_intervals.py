#!/usr/bin/env python3
"""Reproduce #2444 diagnostics without changing the floor-setting sources.

Creates sparse, dedicated worktrees at explicit SHAs. Only those temporary
worktrees receive the fixture normalization and diagnostic instrumentation.
Builds hold the host-wide shared lock; measurement holds its exclusive lock.
This is a local diagnostic, never an input to perf_floor.py or floors.json.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[4]
CRATE = Path("versions/v0.111.0/rust")
FIXTURE_SHA = "6a5f557d8ae8b3966aeea58c623d2bdb6fdda9069ec3f1a4a05af32049f16ee2"
REFS = ["49c0f490", "7e6f39f3", "323d75a1", "79034c24", "1bdc5797", "571f8b4b", "74ed4a23"]
sys.path.insert(0, str(ROOT / "solver/tools"))
from mac_solver_workload_lock import acquire  # noqa: E402


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def run(argv, *, cwd=ROOT, env=None):
    started = datetime.now(timezone.utc).isoformat()
    result = subprocess.run(list(map(str, argv)), cwd=cwd, env=env,
                            text=True, capture_output=True)
    record = dict(command=list(map(str, argv)), cwd=str(cwd), started_utc=started,
                  returncode=result.returncode, stdout=result.stdout, stderr=result.stderr)
    if result.returncode:
        raise RuntimeError(json.dumps(record, indent=2))
    return record


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"expected exactly one instrumentation anchor: {old!r}")
    return source.replace(old, new)


def instrument_bench(source):
    source = replace_once(source, "struct Measurement {", """
#[derive(Default)]
struct ComponentCost { calls: u64, nanos: u128, allocations: u64, bytes: u64 }
impl ComponentCost {
    fn finish(&mut self, start: Instant, before: (u64, u64)) {
        let nanos = start.elapsed().as_nanos();
        let after = sts_sim::allocation::snapshot();
        self.calls += 1;
        self.nanos += nanos;
        self.allocations += after.0 - before.0;
        self.bytes += after.1 - before.1;
    }
    fn report(&self) -> serde_json::Value {
        serde_json::json!({"calls": self.calls, "nanos": self.nanos,
            "allocations": self.allocations, "allocated_bytes": self.bytes})
    }
}
struct Measurement {
    clone_cost: ComponentCost,
    legal_cost: ComponentCost,
    apply_cost: ComponentCost,
    drop_cost: ComponentCost,
""")
    source = source.replace("        transitions: 0,", """        clone_cost: ComponentCost::default(),
        legal_cost: ComponentCost::default(),
        apply_cost: ComponentCost::default(),
        drop_cost: ComponentCost::default(),
        transitions: 0,""")
    source = replace_once(source, "    let mut state = entry.clone();", """    let before = sts_sim::allocation::snapshot();
    let started = Instant::now();
    let mut state = std::hint::black_box(entry).clone();
    measurement.clone_cost.finish(started, before);""")
    source = replace_once(source,
        "        let actions = engine::legal_actions_into(&state, catalog, legal_actions);", """        let before = sts_sim::allocation::snapshot();
        let started = Instant::now();
        let actions = engine::legal_actions_into(&state, catalog, legal_actions);
        measurement.legal_cost.finish(started, before);""")
    source = replace_once(source,
        "        match engine::apply_action_into(&state, catalog, &action, events) {", """        let before = sts_sim::allocation::snapshot();
        let started = Instant::now();
        let result = engine::apply_action_into(&state, catalog, &action, events);
        measurement.apply_cost.finish(started, before);
        match result {""")
    source = replace_once(source, "                state = next;", """                let before = sts_sim::allocation::snapshot();
                let started = Instant::now();
                state = next;
                measurement.drop_cost.finish(started, before);""")
    source = replace_once(source, "        \"allocation\": allocation,", """        "allocation": allocation,
        "components": {
            "trajectory_clone": measurement.clone_cost.report(),
            "legal_actions_into": measurement.legal_cost.report(),
            "apply_action_into_inclusive": measurement.apply_cost.report(),
            "replace_and_drop_predecessor": measurement.drop_cost.report(),
        },
        "hot_type_sizes": sts_sim::hot::issue2444_type_sizes(),""")
    source = replace_once(source, "    let started = Instant::now();\n    for trajectory in 0..workload.trajectories {", """    sts_sim::allocation::issue2444_mark_warm();
    let started = Instant::now();
    for trajectory in 0..workload.trajectories {""")
    # Calibrate the exact finish boundary, without subtracting noisy estimates.
    source = replace_once(source, "    Ok(report(&config.workload, &measurement).to_string())", """    let heap = sts_sim::allocation::issue2444_live_heap();
    let mut output = report(&config.workload, &measurement);
    output["heap"] = heap;
    let mut timer = ComponentCost::default();
    for _ in 0..1_000_000 {
        let before = sts_sim::allocation::snapshot();
        let started = Instant::now();
        std::hint::black_box(());
        timer.finish(started, before);
    }
    output["empty_timer"] = timer.report();
    Ok(output.to_string())""")
    return source


def instrument_allocation(source):
    source = replace_once(source,
        "        unsafe { System.alloc(layout) }", """        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = ISSUE2444_LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed)
                + layout.size() as u64;
            ISSUE2444_PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr""")
    source = replace_once(source,
        "        unsafe { System.dealloc(ptr, layout) }", """        ISSUE2444_LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }""")
    return source + """
static ISSUE2444_LIVE: AtomicU64 = AtomicU64::new(0);
static ISSUE2444_PEAK: AtomicU64 = AtomicU64::new(0);
static ISSUE2444_WARM: AtomicU64 = AtomicU64::new(0);
pub fn issue2444_mark_warm() {
    let live = ISSUE2444_LIVE.load(Ordering::Relaxed);
    ISSUE2444_WARM.store(live, Ordering::Relaxed);
    ISSUE2444_PEAK.store(live, Ordering::Relaxed);
}
pub fn issue2444_live_heap() -> serde_json::Value {
    let warm = ISSUE2444_WARM.load(Ordering::Relaxed);
    let peak = ISSUE2444_PEAK.load(Ordering::Relaxed);
    let live = ISSUE2444_LIVE.load(Ordering::Relaxed);
    serde_json::json!({
        "requested_live_after_warmup": warm,
        "requested_peak_since_warmup": peak,
        "requested_live_after_loop": live,
    })
}
"""


def prepare(ref, work, fixture, environment):
    full = run(["git", "rev-parse", f"{ref}^{{commit}}"])["stdout"].strip()
    directory = work / full[:8]
    branch = f"codex/issue2444-profile-{work.name}-{full[:8]}"
    commands = [run(["git", "worktree", "add", "--no-checkout", "-b", branch, directory, full])]
    commands += [run(["git", "sparse-checkout", "set", "versions/v0.111.0/rust"], cwd=directory)]
    commands += [run(["git", "checkout", branch], cwd=directory)]
    crate = directory / CRATE
    bench_path = crate / "src/bench.rs"
    bench = bench_path.read_text()
    fixture_relative = re.search(r'include_str!\("(../fixtures/[^\"]+)"\)', bench).group(1)
    fixture_path = bench_path.parent / fixture_relative
    original_fixture_sha = sha(fixture_path.read_bytes())
    fixture_path.write_bytes(fixture)
    row = dict(ref=ref, source_sha=full, source_tree=run(["git", "rev-parse", f"{full}:{CRATE}"])["stdout"].strip(),
               branch=branch, directory=str(directory), original_bench_sha256=sha(bench.encode()),
               original_fixture_sha256=original_fixture_sha, fixture_sha256=sha(fixture),
               cargo_lock_sha256=sha((crate / "Cargo.lock").read_bytes()),
               cargo_manifest=(crate / "Cargo.toml").read_text(), commands=commands, binaries={})
    env = {**os.environ, "CARGO_TARGET_DIR": str(work / "target")}
    # Ambient compiler settings would silently change historical comparisons.
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_RELEASE_LTO",
                "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_PANIC"):
        if key in os.environ:
            raise RuntimeError(f"unset {key} before profiling")
    for mode in ("plain", "diagnostic"):
        if mode == "diagnostic":
            bench_path.write_text(instrument_bench(bench))
            allocation = crate / "src/allocation.rs"
            allocation.write_text(instrument_allocation(allocation.read_text()))
            hot = crate / "src/hot.rs"
            hot_source = hot.read_text()
            names = [name for name in ("HotState", "HotMonster", "HotCard", "HotHistory",
                     "FanoutState", "ColdFanoutState", "CardStateStore", "FrameStore", "CardInstanceState")
                     if re.search(rf"\bstruct {name}\b", hot_source)]
            pairs = ",".join(f'"{name}": std::mem::size_of::<{name}>()' for name in names)
            hot.write_text(hot_source + f"\npub fn issue2444_type_sizes() -> serde_json::Value {{ serde_json::json!({{{pairs}}}) }}\n")
        command = ["cargo", "build", "--release", "--locked", "--manifest-path", crate / "Cargo.toml"]
        if mode == "diagnostic":
            command += ["--features", "allocation-counting"]
        print(f"Building {full[:8]} {mode}", flush=True)
        with acquire("shared", timeout_seconds=7200):
            build = run(command, cwd=directory, env=env)
        binary = work / f"{full[:8]}-{mode}"
        shutil.copy2(work / "target/release/sts-sim", binary)
        row["binaries"][mode] = dict(path=str(binary), sha256=sha(binary.read_bytes()),
            file_bytes=binary.stat().st_size, build=build,
            source_diff=run(["git", "diff", "--", str(CRATE)], cwd=directory)["stdout"],
            sections=run(["size", "-m", binary]))
    return row


def identity(report):
    expected = dict(transitions=4078800, legal_action_enumerations=4078800,
                    legal_actions_seen=14332026, checksum="5d4f992689b53874",
                    stops=dict(terminal=240000, budget=0, no_legal_actions=0, refused=0))
    assert report["entry_sha256"] == FIXTURE_SHA
    assert report["config"] == dict(trajectories=240000, actions=64, seed=6004515678751904326, warmup=2000)
    assert all(report["work"][key] == value for key, value in expected.items()), report["work"]
    if "components" in report:
        assert report["components"]["trajectory_clone"]["calls"] == 240000
        assert all(report["components"][key]["calls"] == 4078800 for key in
                   ("legal_actions_into", "apply_action_into_inclusive", "replace_and_drop_predecessor"))
        assert sum(c["allocations"] for c in report["components"].values()) == report["allocation"]["allocations"]
        assert sum(c["allocated_bytes"] for c in report["components"].values()) == report["allocation"]["allocated_bytes"]


def observe(binary, observer, seconds):
    command = [str(binary), "bench"]
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    time.sleep(0.15)
    tool_command = (["sample", str(process.pid), str(seconds), "1"] if observer == "sample"
                    else ["vmmap", "-summary", str(process.pid)])
    tool = subprocess.run(tool_command, capture_output=True, text=True)
    stdout, stderr = process.communicate()
    if process.returncode:
        raise RuntimeError(stderr)
    identity(json.loads(stdout))
    return dict(command=command, pid=process.pid, stdout=stdout, stderr=stderr,
                observer_command=tool_command, observer_returncode=tool.returncode,
                observer_stdout=tool.stdout, observer_stderr=tool.stderr)


def measure(evidence, repeat, output):
    print("Waiting for exclusive diagnostic measurement lane", flush=True)
    with acquire("exclusive", timeout_seconds=7200):
        print("Acquired exclusive diagnostic measurement lane", flush=True)
        evidence["measurement_started_utc"] = datetime.now(timezone.utc).isoformat()
        evidence["load_average_start"] = os.getloadavg()
        for row in evidence["checkpoints"]:
            row["measurements"] = {"plain": [], "diagnostic": []}
        # Alternate chronological/reverse sweeps, instead of timing each ref in one batch.
        for repetition in range(repeat):
            rows = evidence["checkpoints"] if repetition % 2 == 0 else list(reversed(evidence["checkpoints"]))
            for row in rows:
                for mode in ("plain", "diagnostic"):
                    print(f"Measuring {row['source_sha'][:8]} {mode} {repetition + 1}/{repeat}", flush=True)
                    raw = run(["/usr/bin/time", "-l", row["binaries"][mode]["path"], "bench"])
                    report = json.loads(raw["stdout"])
                    identity(report)
                    rss = int(re.search(r"(\d+)\s+maximum resident set size", raw["stderr"]).group(1))
                    row["measurements"][mode].append(dict(repetition=repetition, peak_rss_bytes=rss, report=report, raw=raw))
            output.write_text(json.dumps(evidence, indent=2) + "\n")
        for row in evidence["checkpoints"]:
            print(f"Sampling CPU and resident regions {row['source_sha'][:8]}", flush=True)
            row["sample"] = observe(row["binaries"]["plain"]["path"], "sample", 2)
            row["vmmap"] = observe(row["binaries"]["plain"]["path"], "vmmap", 0)
            output.write_text(json.dumps(evidence, indent=2) + "\n")
        evidence["load_average_end"] = os.getloadavg()
        evidence["measurement_finished_utc"] = datetime.now(timezone.utc).isoformat()
    output.write_text(json.dumps(evidence, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--refs", nargs="+", default=REFS)
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--resume", type=Path, help="reuse an existing completed build manifest")
    parser.add_argument("--build-only", action="store_true")
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    if args.resume:
        evidence = json.loads(args.resume.read_text())
    else:
        fixture = (ROOT / CRATE / "fixtures/bench_ironclad_toadpoles_uniform_v1.json").read_bytes()
        assert sha(fixture) == FIXTURE_SHA
        work = Path(tempfile.mkdtemp(prefix="sts2444-profile-"))
        evidence = dict(schema="sts-sim-interval-profile/v1", diagnostic_only=True,
            command=sys.argv, work_directory=str(work), harness_sha256=sha(Path(__file__).read_bytes()),
            environment=dict(platform=platform.platform(), machine=platform.machine(),
                python=platform.python_version(), cpu=run(["sysctl", "-n", "machdep.cpu.brand_string"])["stdout"].strip(),
                rustc=run(["rustc", "-Vv"]), cargo=run(["cargo", "-V"]),
                target_directory=str(work / "target")), checkpoints=[])
        print(f"Temporary worktrees and builds: {work}", flush=True)
        for ref in args.refs:
            evidence["checkpoints"].append(prepare(ref, work, fixture, evidence["environment"]))
            args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    if not args.build_only:
        measure(evidence, args.repeat, args.output)


if __name__ == "__main__":
    main()

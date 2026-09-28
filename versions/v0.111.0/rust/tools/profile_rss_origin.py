#!/usr/bin/env python3
"""Compare the original RSS baseline, September anchor, and recovery image.

Unlike profile_intervals.py's diagnostic build, both binaries here have their
original source and allocator: plain release and the exact allocation-counting
feature used by the floor lane. Each ref has its own sparse temporary worktree.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import tempfile

from profile_intervals import ROOT, CRATE, FIXTURE_SHA, acquire, identity, observe, run, sha

REFS = ["b5b1ddd5", "49c0f490", "74ed4a23"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    work = Path(tempfile.mkdtemp(prefix="sts2444-rss-"))
    evidence = dict(schema="sts-sim-rss-origin/v1", diagnostic_only=True, command=sys.argv,
        work_directory=str(work), harness_sha256=sha(Path(__file__).read_bytes()),
        common_harness_sha256=sha((Path(__file__).parent / "profile_intervals.py").read_bytes()),
        environment=dict(platform=platform.platform(), python=platform.python_version(),
            cpu=run(["sysctl", "-n", "machdep.cpu.brand_string"]),
            rustc=run(["rustc", "-Vv"]), cargo=run(["cargo", "-V"])), checkpoints=[])
    env = {**os.environ, "CARGO_TARGET_DIR": str(work / "target")}
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_RELEASE_LTO",
                "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_PANIC"):
        if key in os.environ:
            raise RuntimeError(f"unset {key} before profiling")
    print(f"Temporary original-source builds: {work}", flush=True)
    for ref in REFS:
        full = run(["git", "rev-parse", f"{ref}^{{commit}}"])["stdout"].strip()
        directory = work / ref
        branch = f"codex/issue2444-rss-{work.name}-{ref}"
        commands = [run(["git", "worktree", "add", "--no-checkout", "-b", branch, directory, full])]
        commands += [run(["git", "sparse-checkout", "set", "versions/v0.111.0/rust"], cwd=directory)]
        commands += [run(["git", "checkout", branch], cwd=directory)]
        crate = directory / CRATE
        row = dict(ref=ref, source_sha=full, branch=branch, directory=str(directory),
            commands=commands, source_tree=run(["git", "rev-parse", f"{full}:{CRATE}"])["stdout"].strip(),
            original_bench_sha256=sha((crate / "src/bench.rs").read_bytes()),
            cargo_manifest=(crate / "Cargo.toml").read_text(),
            cargo_lock_sha256=sha((crate / "Cargo.lock").read_bytes()), binaries={}, measurements={})
        for mode in ("plain", "allocation-counting"):
            print(f"Building original {ref} {mode}", flush=True)
            command = ["cargo", "build", "--release", "--locked", "--manifest-path", crate / "Cargo.toml"]
            if mode == "allocation-counting":
                command += ["--features", mode]
            with acquire("shared", timeout_seconds=7200):
                build = run(command, cwd=directory, env=env)
            binary = work / f"{ref}-{mode}"
            shutil.copy2(work / "target/release/sts-sim", binary)
            row["binaries"][mode] = dict(path=str(binary), sha256=sha(binary.read_bytes()),
                file_bytes=binary.stat().st_size, build=build,
                sections=run(["size", "-m", binary]))
            row["measurements"][mode] = []
        row["source_status"] = run(["git", "status", "--porcelain"], cwd=directory)["stdout"]
        assert not row["source_status"]
        evidence["checkpoints"].append(row)
        args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    print("Waiting for exclusive original-source RSS lane", flush=True)
    with acquire("exclusive", timeout_seconds=7200):
        print("Acquired exclusive original-source RSS lane", flush=True)
        evidence["measurement_started_utc"] = datetime.now(timezone.utc).isoformat()
        evidence["load_average_start"] = os.getloadavg()
        for repetition in range(5):
            rows = evidence["checkpoints"] if repetition % 2 == 0 else list(reversed(evidence["checkpoints"]))
            for row in rows:
                for mode in ("plain", "allocation-counting"):
                    raw = run(["/usr/bin/time", "-l", row["binaries"][mode]["path"], "bench"])
                    report = json.loads(raw["stdout"])
                    identity(report)
                    rss = int(re.search(r"(\d+)\s+maximum resident set size", raw["stderr"]).group(1))
                    row["measurements"][mode].append(dict(repetition=repetition, peak_rss_bytes=rss, report=report, raw=raw))
                    print(f"RSS {row['ref']} {mode} {repetition + 1}/5: {rss}", flush=True)
            args.output.write_text(json.dumps(evidence, indent=2) + "\n")
        for row in evidence["checkpoints"]:
            row["vmmap"] = observe(row["binaries"]["allocation-counting"]["path"], "vmmap", 0)
        evidence["load_average_end"] = os.getloadavg()
        evidence["measurement_finished_utc"] = datetime.now(timezone.utc).isoformat()
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()

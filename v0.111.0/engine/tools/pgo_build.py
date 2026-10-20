#!/usr/bin/env python3
"""Build a profile-guided (PGO) release `sts-sim` (#3979).

PGO changes no semantics: it reorders code and re-weighs inlining from a
training run's branch counts. On 2026-10-19 it made UCT search on held-out
eval fights 1.6-1.7x faster on the ordinary route and 1.25-1.4x on the
action-replay route, with identical results (same best-line digests), and
`sts-sim bench` about 1.7x faster (the bench is in the training set, so that
number is optimistic).

Steps:

1. build with `-Cprofile-generate` into its own target directory;
2. train: `sts-sim bench`, then `sts-sim search` (UCT, a few seconds each)
   over a deterministic sample of the eval fixtures, both routes;
3. merge the raw profiles with an `llvm-profdata` whose LLVM major version
   matches rustc's (it fails closed otherwise: a mismatched merger writes a
   profile rustc rejects or silently ignores);
4. build with `-Cprofile-use` into a second target directory and print the
   binary's path.

The certification census and every pinned measurement stay on the ordinary
release build; this binary is for running searches faster. Usage:

    python3 tools/pgo_build.py [--out DIR] [--fights N] [--seconds S]
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys

ENGINE = pathlib.Path(__file__).resolve().parents[1]
FIGHTS = ENGINE.parent / "eval" / "fights"


def rustc_llvm_major() -> int:
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    match = re.search(r"LLVM version: (\d+)", out)
    if not match:
        sys.exit("refusal: pgo_build: rustc -vV reports no LLVM version")
    return int(match.group(1))


def find_profdata(major: int) -> str:
    candidates = []
    if os.environ.get("LLVM_PROFDATA"):
        candidates.append(os.environ["LLVM_PROFDATA"])
    candidates += [
        f"/opt/homebrew/opt/llvm@{major}/bin/llvm-profdata",
        "/opt/homebrew/opt/llvm/bin/llvm-profdata",
        f"/usr/local/opt/llvm@{major}/bin/llvm-profdata",
        shutil.which("llvm-profdata") or "",
    ]
    for candidate in candidates:
        if not candidate or not pathlib.Path(candidate).exists():
            continue
        out = subprocess.run([candidate, "--version"], capture_output=True, text=True).stdout
        found = re.search(r"LLVM version (\d+)", out)
        if found and int(found.group(1)) == major:
            return candidate
    sys.exit(
        f"refusal: pgo_build: no llvm-profdata for LLVM {major} "
        f"(rustc's); `brew install llvm@{major}` or set LLVM_PROFDATA"
    )


def cargo_build(target_dir: pathlib.Path, rustflags: str) -> pathlib.Path:
    env = dict(os.environ, RUSTFLAGS=rustflags)
    subprocess.run(
        ["cargo", "build", "--locked", "--release", "--bin", "sts-sim",
         "--target-dir", str(target_dir)],
        cwd=ENGINE, env=env, check=True,
    )
    return target_dir / "release" / "sts-sim"


def training_fights(count: int) -> list[pathlib.Path]:
    entries = sorted(FIGHTS.glob("*/entry.canonical.json"))
    if not entries:
        sys.exit(f"refusal: pgo_build: no eval fixtures under {FIGHTS}")
    stride = max(1, len(entries) // count)
    return entries[::stride][:count]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--out", type=pathlib.Path,
                        default=ENGINE / "target" / "pgo")
    parser.add_argument("--fights", type=int, default=24)
    parser.add_argument("--seconds", type=float, default=3.0)
    args = parser.parse_args()

    profdata = find_profdata(rustc_llvm_major())
    raw = args.out / "raw"
    if raw.exists():
        shutil.rmtree(raw)
    raw.mkdir(parents=True)
    generate = cargo_build(args.out / "generate", f"-Cprofile-generate={raw}")

    subprocess.run([str(generate), "bench", "--trajectories", "60000"],
                   check=True, stdout=subprocess.DEVNULL)
    for entry in training_fights(args.fights):
        # A refused or failing search still contributes the counts it ran.
        subprocess.run([str(generate), "search", str(entry), "uct", "7",
                        str(args.seconds)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    merged = args.out / "merged.profdata"
    subprocess.run([profdata, "merge", "-o", str(merged), str(raw)], check=True)
    binary = cargo_build(args.out / "use", f"-Cprofile-use={merged}")
    print(binary)


if __name__ == "__main__":
    main()

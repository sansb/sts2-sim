#!/usr/bin/env python3
"""Build and check the browser engine for `wasm32-unknown-unknown` (#3469, #3470).

Two crates target wasm32: the `sts-sim` library itself, and `wasm/`, the
handle-based C-ABI module the browser loads (#3470). Nothing else in the
`rust port` lane builds that target, so without this step the next 64-bit
assumption (an exact pointer-width layout pin, a `usize` overflow, a
`std::time::Instant::now()` on a path the solve reaches) would re-break the
build silently. #3412 found five of those on the first try.

Toolchain: option (a) of #3469, decided by Sean on 2026-09-29. The
`mac-solver` runners use Homebrew Rust, which ships no wasm32 standard
library, and `tools/check_rust_toolchain.py` keeps PATH on it. This step alone
uses the rustup toolchain installed beside Homebrew, by absolute path:

    $RUSTUP_HOME/toolchains/<channel>-<host>/bin/{cargo,rustc}

(`RUSTUP_HOME` defaults to `~/.rustup`). It never puts rustup on PATH and
never goes through the `~/.cargo/bin` proxies, so no later step and no
Homebrew command can pick it up, and nothing is downloaded mid-job. It fails
closed, with the command that fixes it, when that toolchain is absent, is not
the `rust-toolchain.toml` channel, or lacks the wasm32 standard library. It
also fails closed, with its `brew install`, when `wasm-opt` (binaryen) or
`node` is missing.

**Toolchain bumps.** A channel change in `rust-toolchain.toml` now needs, on
the runner host and in addition to the Homebrew upgrade:

    rustup toolchain install <channel> --profile minimal \
        --component clippy,rustfmt --target wasm32-unknown-unknown

Output goes to a separate target directory (`target/wasm32-rustup`). The two
1.97.1 compilers are different builds, and a proc-macro dylib (`serde_derive`)
must be loaded by the compiler that built it, so the two installations must
never share artifacts. Every command runs with the same `RUSTFLAGS`
(`--remap-path-prefix` for the repository, cargo home and toolchain), so the
steps share artifacts and the shipped module names no build host's paths.
#3412 found 54 absolute paths in the spike's module.

Before anything builds, `wasm/Cargo.lock` must pin every package at exactly
the version and checksum the engine crate's own `Cargo.lock` pins. The shipped
module has to be the certified engine, and a separately resolved lockfile
silently took newer crates, including `rust_decimal`, whose decimal semantics
are parity-bearing. The API crate may add only itself. To refresh after the
engine's lockfile moves: `cp Cargo.lock wasm/Cargo.lock`, then run any cargo
command in `wasm/` without `--locked`.

Steps, in order, stopping at the first failure:

1. `sts-sim` lib: clippy `-D warnings` for wasm32, where an import used only
   by one side of a `cfg(target_arch)` split surfaces.
2. `sts-sim` lib: release build for wasm32, where codegen and link errors
   surface.
3. `wasm/`: clippy `-D warnings` for wasm32.
4. `wasm/`: native `cargo test --release` of the same API (`tests/api.rs`).
5. `wasm/`: release cdylib for wasm32.
6. `wasm-opt -Oz --strip-debug --strip-producers`, the module a browser
   loads, at `target/wasm32-rustup/sts_sim_wasm.wasm`.
7. The optimized module must not contain the build host's home directory.
8. `node wasm/js/test_parity.mjs` on the optimized module: every certified
   eval fixture's human line, step digest by step digest, plus undo/branch,
   refusal, solve and replay checks.

It then prints the module's size (raw and gzip) and sha256 fingerprint.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import os
import pathlib
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Callable, Mapping, Sequence

CRATE = pathlib.Path(__file__).resolve().parents[1]
REPO = CRATE.parents[2]
PIN = REPO / "rust-toolchain.toml"
TARGET = "wasm32-unknown-unknown"
TARGET_DIR = CRATE / "target" / "wasm32-rustup"
API = CRATE / "wasm"
MODULE = TARGET_DIR / "sts_sim_wasm.wasm"

Run = Callable[..., subprocess.CompletedProcess]


class WasmToolchainError(RuntimeError):
    """The rustup wasm32 toolchain, binaryen or node is absent or off-pin."""


def pinned_channel(path: pathlib.Path = PIN) -> str:
    return tomllib.loads(path.read_text())["toolchain"]["channel"]


def host_triple(run: Run = subprocess.run) -> str:
    """The host triple of the PATH `rustc` (Homebrew, already validated)."""
    output = run(["rustc", "-vV"], check=True, text=True,
                 stdout=subprocess.PIPE).stdout
    for line in output.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ").strip()
    raise WasmToolchainError("rustc -vV printed no host triple")


def install_hint(channel: str) -> str:
    return (f"rustup toolchain install {channel} --profile minimal "
            f"--component clippy,rustfmt --target {TARGET}")


def locate(channel: str, host: str, env: Mapping[str, str],
           run: Run = subprocess.run) -> pathlib.Path:
    """The pinned rustup toolchain's `bin/`, or a refusal naming the fix."""
    rustup_home = pathlib.Path(
        env.get("RUSTUP_HOME") or pathlib.Path(env.get("HOME", "~")) / ".rustup"
    ).expanduser()
    toolchain = rustup_home / "toolchains" / f"{channel}-{host}"
    bin_dir = toolchain / "bin"
    for tool in ("cargo", "rustc", "cargo-clippy"):
        if not (bin_dir / tool).is_file():
            raise WasmToolchainError(
                f"rustup toolchain {toolchain} has no bin/{tool}; on the runner "
                f"host run: {install_hint(channel)}")
    version = run([str(bin_dir / "rustc"), "--version"], check=True, text=True,
                  stdout=subprocess.PIPE).stdout.strip()
    if not version.startswith(f"rustc {channel} "):
        raise WasmToolchainError(
            f"{bin_dir / 'rustc'} reports {version!r}, not the pinned "
            f"{channel} (rust-toolchain.toml)")
    std = toolchain / "lib" / "rustlib" / TARGET / "lib"
    if not std.is_dir() or not any(std.glob("libstd-*.rlib")):
        raise WasmToolchainError(
            f"rustup toolchain {toolchain} has no {TARGET} standard library; "
            f"on the runner host run: {install_hint(channel)}")
    return bin_dir


def host_tool(name: str, formula: str, env: Mapping[str, str]) -> str:
    """A Homebrew tool the post-build steps need, from `env`'s PATH, or a
    refusal naming the install."""
    found = shutil.which(name, path=env.get("PATH", ""))
    if found is None:
        raise WasmToolchainError(
            f"{name} is not on PATH; on the runner host run: brew install {formula}")
    return found


def commands(bin_dir: pathlib.Path) -> list[list[str]]:
    """The cargo steps (1-5 above), all through the located toolchain."""
    cargo = str(bin_dir / "cargo")
    target_dir = ["--target-dir", str(TARGET_DIR)]
    lib = ["--manifest-path", str(CRATE / "Cargo.toml"), "--lib"]
    api = ["--manifest-path", str(API / "Cargo.toml")]
    wasm = ["--target", TARGET]
    return [
        [cargo, "clippy", "--locked", *lib, *wasm, *target_dir, "--", "-D", "warnings"],
        [cargo, "build", "--release", "--locked", *lib, *wasm, *target_dir],
        [cargo, "clippy", "--locked", *api, *wasm, *target_dir, "--", "-D", "warnings"],
        [cargo, "test", "--release", "--locked", *api, *target_dir],
        [cargo, "build", "--release", "--locked", *api, "--lib", *wasm, *target_dir],
    ]


def cdylib() -> pathlib.Path:
    return TARGET_DIR / TARGET / "release" / "sts_sim_wasm.wasm"


def lock_drift(engine_lock: pathlib.Path, api_lock: pathlib.Path) -> list[str]:
    """Where `wasm/Cargo.lock` departs from the engine's (see module docs)."""
    def packages(path: pathlib.Path) -> dict[str, tuple[str, str | None]]:
        return {
            package["name"]: (package["version"], package.get("checksum"))
            for package in tomllib.loads(path.read_text())["package"]
        }
    engine, api = packages(engine_lock), packages(api_lock)
    drift = [f"{name}: engine {pin[0]}, wasm/ {api[name][0] if name in api else 'absent'}"
             for name, pin in sorted(engine.items()) if api.get(name) != pin]
    drift += [f"{name}: only in wasm/ ({api[name][0]})"
              for name in sorted(set(api) - set(engine) - {"sts-sim-wasm"})]
    return drift


def remap_flags(bin_dir: pathlib.Path, env: Mapping[str, str]) -> str:
    """`--remap-path-prefix` for every host path a panic location can name."""
    home = pathlib.Path(env.get("HOME", "~")).expanduser()
    cargo_home = pathlib.Path(env.get("CARGO_HOME") or home / ".cargo").expanduser()
    prefixes = [(REPO, "/sts"), (cargo_home, "/cargo"), (bin_dir.parent, "/rustc")]
    return " ".join(f"--remap-path-prefix={path}={alias}" for path, alias in prefixes)


def child_env(bin_dir: pathlib.Path, env: Mapping[str, str]) -> dict[str, str]:
    """The cargo subprocesses' environment: this toolchain first on PATH,
    `RUSTC` pinned to its compiler, and the path remapping. The parent
    environment is untouched, so the change ends with this step."""
    child = dict(env)
    child["PATH"] = os.pathsep.join([str(bin_dir), env.get("PATH", "")])
    child["RUSTC"] = str(bin_dir / "rustc")
    child["RUSTFLAGS"] = remap_flags(bin_dir, env)
    child.pop("RUSTUP_TOOLCHAIN", None)
    child.pop("RUSTC_WRAPPER", None)
    child.pop("CARGO_ENCODED_RUSTFLAGS", None)
    return child


def leaked_paths(module: bytes, env: Mapping[str, str]) -> list[str]:
    """Host paths that survived remapping (step 7)."""
    home = str(pathlib.Path(env.get("HOME", "~")).expanduser())
    return [path for path in (home, str(REPO)) if path.encode() in module]


def main(argv: Sequence[str] | None = None, *, env: Mapping[str, str] | None = None,
         run: Run = subprocess.run) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--dry-run", action="store_true",
                        help="locate and validate the tools, print the commands")
    args = parser.parse_args(argv)
    env = os.environ if env is None else env
    channel = pinned_channel()
    try:
        bin_dir = locate(channel, host_triple(run), env, run)
        wasm_opt = host_tool("wasm-opt", "binaryen", env)
        node = host_tool("node", "node", env)
    except WasmToolchainError as error:
        print(f"wasm32 build refused: {error}", file=sys.stderr)
        return 2
    drift = lock_drift(CRATE / "Cargo.lock", API / "Cargo.lock")
    if drift:
        print("wasm32 build refused: wasm/Cargo.lock drifted from the engine's "
              "Cargo.lock (cp Cargo.lock wasm/Cargo.lock, then any cargo command "
              "in wasm/ without --locked):\n  " + "\n  ".join(drift), file=sys.stderr)
        return 2
    print(f"wasm32 toolchain: {bin_dir} (pinned {channel})", flush=True)
    child = child_env(bin_dir, env)
    post = [
        [wasm_opt, "--all-features", "-Oz", "--strip-debug", "--strip-producers",
         str(cdylib()), "-o", str(MODULE)],
        None,  # step 7, in-process
        [node, str(API / "js" / "test_parity.mjs"), str(MODULE)],
    ]
    for command in commands(bin_dir) + post:
        if command is None:
            if args.dry_run:
                continue
            leaked = leaked_paths(MODULE.read_bytes(), env)
            if leaked:
                print(f"wasm32 module names host paths: {leaked}", file=sys.stderr)
                return 1
            continue
        print("+ " + " ".join(command), flush=True)
        if args.dry_run:
            continue
        status = run(command, env=child).returncode
        if status != 0:
            return status
    if not args.dry_run:
        module = MODULE.read_bytes()
        print(f"module {MODULE.name}: {len(module)} bytes raw, "
              f"{len(gzip.compress(module, 9))} gzip -9, "
              f"sha256 {hashlib.sha256(module).hexdigest()}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

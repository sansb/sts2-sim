#!/usr/bin/env python3
"""Controls for `wasm_build.py` (#3469): it must refuse, never guess.

Each refusal case mutates a synthetic rustup home that is otherwise valid, so
a control that passes did so because of the mutation it names. Stdlib only,
no cargo; runs in the `rust port` step before the build it guards.
"""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import wasm_build  # noqa: E402

CHANNEL = wasm_build.pinned_channel()
HOST = "aarch64-apple-darwin"


def _toolchain(root: pathlib.Path, *, version: str = CHANNEL, std: bool = True,
               missing: str | None = None) -> pathlib.Path:
    toolchain = root / "toolchains" / f"{CHANNEL}-{HOST}"
    bin_dir = toolchain / "bin"
    bin_dir.mkdir(parents=True)
    for tool in ("cargo", "cargo-clippy"):
        if tool != missing:
            (bin_dir / tool).write_text("#!/bin/sh\nexit 0\n")
    if missing != "rustc":
        rustc = bin_dir / "rustc"
        rustc.write_text(f"#!/bin/sh\necho 'rustc {version} (8bab26f4f 2026-07-14)'\n")
        rustc.chmod(0o755)
    if std:
        lib = toolchain / "lib" / "rustlib" / wasm_build.TARGET / "lib"
        lib.mkdir(parents=True)
        (lib / "libstd-0123456789abcdef.rlib").write_bytes(b"")
    return bin_dir


def _refuses(expect: str, **mutation) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        _toolchain(pathlib.Path(tmp), **mutation)
        try:
            wasm_build.locate(CHANNEL, HOST, {"RUSTUP_HOME": tmp})
        except wasm_build.WasmToolchainError as error:
            assert expect in str(error), (expect, str(error))
            return
    raise AssertionError(f"accepted a toolchain with {mutation}")


def test_accepts_the_pinned_toolchain() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        bin_dir = _toolchain(pathlib.Path(tmp))
        assert wasm_build.locate(CHANNEL, HOST, {"RUSTUP_HOME": tmp}) == bin_dir


def test_refuses_what_it_must() -> None:
    _refuses("has no bin/cargo", missing="cargo")
    _refuses("has no bin/cargo-clippy", missing="cargo-clippy")
    _refuses("has no bin/rustc", missing="rustc")
    _refuses("not the pinned", version="1.96.0")
    _refuses(f"no {wasm_build.TARGET} standard library", std=False)
    # An absent toolchain names the command that installs it.
    with tempfile.TemporaryDirectory() as tmp:
        try:
            wasm_build.locate(CHANNEL, HOST, {"RUSTUP_HOME": tmp})
        except wasm_build.WasmToolchainError as error:
            assert wasm_build.install_hint(CHANNEL) in str(error), str(error)
        else:
            raise AssertionError("accepted an empty rustup home")


def test_default_home_is_dot_rustup() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        bin_dir = _toolchain(pathlib.Path(tmp) / ".rustup")
        assert wasm_build.locate(CHANNEL, HOST, {"HOME": tmp}) == bin_dir


def _host_tools(root: pathlib.Path) -> str:
    """A PATH holding stand-in wasm-opt and node, for main()'s tool checks."""
    tools = root / "host-bin"
    tools.mkdir()
    for tool in ("wasm-opt", "node"):
        (tools / tool).write_text("#!/bin/sh\nexit 0\n")
        (tools / tool).chmod(0o755)
    return str(tools)


def test_commands_use_the_located_toolchain_and_their_own_target_dir() -> None:
    bin_dir = pathlib.Path("/x/bin")
    steps = wasm_build.commands(bin_dir)
    assert [step[1] for step in steps] == ["clippy", "build", "clippy", "test", "build"]
    lib_manifest = str(wasm_build.CRATE / "Cargo.toml")
    api_manifest = str(wasm_build.API / "Cargo.toml")
    for index, command in enumerate(steps):
        assert command[0] == "/x/bin/cargo", command
        target_dir = command[command.index("--target-dir") + 1]
        # Never the native `target/` itself: the two compilers must not share
        # proc-macro artifacts.
        assert pathlib.Path(target_dir) == wasm_build.CRATE / "target" / "wasm32-rustup"
        assert "--locked" in command
        manifest = command[command.index("--manifest-path") + 1]
        assert manifest == (lib_manifest if index < 2 else api_manifest), command
        if index != 3:
            # Everything but the API's native tests builds for wasm32.
            assert command[command.index("--target") + 1] == "wasm32-unknown-unknown"
        else:
            assert "--target" not in command
    for clippy in (steps[0], steps[2]):
        assert clippy[-3:] == ["--", "-D", "warnings"]
    for build in (steps[1], steps[4]):
        assert build[1:3] == ["build", "--release"] and "--lib" in build
    assert steps[3][1:3] == ["test", "--release"]
    assert wasm_build.cdylib().name == "sts_sim_wasm.wasm"


def test_child_env_is_scoped_pins_rustc_and_remaps_paths() -> None:
    parent = {"PATH": "/opt/homebrew/bin:/usr/bin", "RUSTUP_TOOLCHAIN": "stable",
              "RUSTC_WRAPPER": "sccache", "HOME": "/home/builder",
              "CARGO_ENCODED_RUSTFLAGS": "-Cfoo"}
    before = dict(parent)
    bin_dir = pathlib.Path("/home/builder/.rustup/toolchains/x/bin")
    child = wasm_build.child_env(bin_dir, parent)
    assert parent == before, "the parent environment must not change"
    assert child["PATH"].split(os.pathsep)[0] == str(bin_dir)
    assert child["RUSTC"] == str(bin_dir / "rustc")
    for dropped in ("RUSTUP_TOOLCHAIN", "RUSTC_WRAPPER", "CARGO_ENCODED_RUSTFLAGS"):
        assert dropped not in child, dropped
    flags = child["RUSTFLAGS"].split()
    assert f"--remap-path-prefix={wasm_build.REPO}=/sts" in flags
    assert "--remap-path-prefix=/home/builder/.cargo=/cargo" in flags
    assert "--remap-path-prefix=/home/builder/.rustup/toolchains/x=/rustc" in flags


def test_host_tools_refuse_with_their_install_command() -> None:
    try:
        wasm_build.host_tool("wasm-opt", "binaryen", {"PATH": ""})
    except wasm_build.WasmToolchainError as error:
        assert "brew install binaryen" in str(error), str(error)
    else:
        raise AssertionError("found wasm-opt on an empty PATH")
    with tempfile.TemporaryDirectory() as tmp:
        path = _host_tools(pathlib.Path(tmp))
        assert wasm_build.host_tool("node", "node", {"PATH": path}).endswith("/node")


def test_leaked_paths_names_the_home_and_repository() -> None:
    env = {"HOME": "/home/builder"}
    assert wasm_build.leaked_paths(b"panicked at /sts/src/x.rs", env) == []
    assert wasm_build.leaked_paths(b"at /home/builder/.cargo/registry", env) == ["/home/builder"]
    assert str(wasm_build.REPO) in wasm_build.leaked_paths(
        str(wasm_build.REPO).encode() + b"/src/hot.rs", {"HOME": "/elsewhere"})


def test_the_api_lockfile_pins_exactly_the_engine_crates() -> None:
    # The committed pair must agree today...
    assert wasm_build.lock_drift(wasm_build.CRATE / "Cargo.lock",
                                 wasm_build.API / "Cargo.lock") == []
    # ...and a bumped, missing or extra package must each be named.
    engine = (wasm_build.CRATE / "Cargo.lock").read_text()
    api = (wasm_build.API / "Cargo.lock").read_text()
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        (root / "engine.lock").write_text(engine)
        cases = {
            "bumped": (api.replace('name = "rust_decimal"\nversion = "',
                                   'name = "rust_decimal"\nversion = "9', 1),
                       ["rust_decimal: engine"]),
            "renamed": (api.replace('name = "sha2"', 'name = "sha2-renamed"', 1),
                        ["sha2: engine", "sha2-renamed: only in wasm/"]),
        }
        for label, (text, expected) in cases.items():
            assert text != api, label
            (root / "api.lock").write_text(text)
            drift = wasm_build.lock_drift(root / "engine.lock", root / "api.lock")
            for fragment in expected:
                assert any(line.startswith(fragment) for line in drift), (label, drift)


def test_main_refuses_with_exit_2_and_runs_nothing() -> None:
    calls = []

    def run(command, **kwargs):
        calls.append(command)
        if command == ["rustc", "-vV"]:
            return subprocess.CompletedProcess(command, 0, stdout=f"host: {HOST}\n")
        raise AssertionError(f"ran {command} after a refusal")

    with tempfile.TemporaryDirectory() as tmp:
        assert wasm_build.main([], env={"RUSTUP_HOME": tmp}, run=run) == 2
    assert calls == [["rustc", "-vV"]], calls

    # A valid toolchain but no binaryen: refused before any cargo command.
    calls.clear()

    def run_versions(command, **kwargs):
        calls.append(command)
        if command == ["rustc", "-vV"]:
            return subprocess.CompletedProcess(command, 0, stdout=f"host: {HOST}\n")
        if command[-1] == "--version":
            return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE)
        raise AssertionError(f"ran {command} after a refusal")

    with tempfile.TemporaryDirectory() as tmp:
        _toolchain(pathlib.Path(tmp))
        assert wasm_build.main([], env={"RUSTUP_HOME": tmp, "PATH": ""}, run=run_versions) == 2
    assert not any(c[0].endswith("/cargo") for c in calls), calls


def test_main_stops_at_the_first_failing_command() -> None:
    calls = []

    def run(command, **kwargs):
        calls.append(command)
        if command == ["rustc", "-vV"]:
            return subprocess.CompletedProcess(command, 0, stdout=f"host: {HOST}\n")
        if command[-1] == "--version":
            return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE)
        return subprocess.CompletedProcess(command, 101)

    with tempfile.TemporaryDirectory() as tmp:
        _toolchain(pathlib.Path(tmp))
        env = {"RUSTUP_HOME": tmp, "PATH": _host_tools(pathlib.Path(tmp))}
        assert wasm_build.main([], env=env, run=run) == 101
    assert [c[1] for c in calls if c[0].endswith("/cargo")] == ["clippy"], calls


if __name__ == "__main__":
    tests = [value for name, value in sorted(globals().items())
             if name.startswith("test_") and callable(value)]
    for test in tests:
        test()
    print(f"test_wasm_build: {len(tests)} controls passed")

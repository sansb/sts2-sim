#!/usr/bin/env python3
"""Detect and archive game builds before Steam deletes them (#1269, I11).

Archiving is a load-bearing product invariant, not hygiene. A build can only
ever be admitted for combat semantics if we hold its DLL — that is what makes
IL re-verification and the harness oracle possible. **One missed archive
permanently deletes a build's reviewability**: v0.108.0 is gone that way
(#309), which is why both ground-truth pins are stranded on a build that can
never be admitted, and why every run at or below v0.107.0 is unreviewable
forever.

Steam updates in place and deletes the old build, so the window between a
release and the loss is however long it takes someone to notice. `--check` is
meant to run on a schedule and fail loudly inside that window.

Identity is (version string, sts2.dll sha256). Both are checked: a depot can
be rebuilt under an unchanged version string, and that would otherwise archive
as "already have it".

    archive_build.py --check     # exit 1 if the installed build is unarchived
    archive_build.py --archive   # copy it in, verify, record the index entry

Run from the main checkout: a gitignored copy inside a worktree dies with the
worktree.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import shutil
import sys

HERE = pathlib.Path(__file__).resolve().parent
INDEX = HERE / "index.json"
STEAM = pathlib.Path.home() / (
    "Library/Application Support/Steam/steamapps/common/Slay the Spire 2/"
    "SlayTheSpire2.app/Contents/Resources")
RUNTIME_SUBDIR = "data_sts2_macos_arm64"


def _sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def load_index() -> dict:
    if not INDEX.exists():
        return {"schema": "sts-dll-archive-index/v1", "builds": {}}
    return json.loads(INDEX.read_text())


def installed() -> tuple[str, str, dict]:
    """(version, sts2.dll sha256, release_info) for the current Steam install."""
    info_path = STEAM / "release_info.json"
    if not info_path.exists():
        raise SystemExit(
            f"no Steam install at {STEAM} — run this on the machine that has "
            "the game, or pass --steam-root")
    info = json.loads(info_path.read_text())
    dll = STEAM / RUNTIME_SUBDIR / "sts2.dll"
    if not dll.exists():
        raise SystemExit(f"install is missing {dll}")
    return info["version"], _sha256(dll), info


def check() -> int:
    version, sha, _info = installed()
    builds = load_index()["builds"]
    row = builds.get(version)
    if row is None:
        print(f"::error::game build {version} is INSTALLED BUT NOT ARCHIVED. "
              "Steam deletes the previous build on update, so this window is "
              "the only chance to keep it. Archive it now from the main "
              "checkout:\n"
              "  python3 sim/dll-archive/archive_build.py --archive\n"
              "Losing it permanently forecloses ever admitting this build "
              "(#1269, SOLVER_INVARIANTS.md I11).")
        return 1
    if row["dll_sha256"] != sha:
        print(f"::error::game build {version} is archived, but the INSTALLED "
              f"sts2.dll does not match it.\n"
              f"  archived: {row['dll_sha256']}\n"
              f"  installed: {sha}\n"
              "A depot rebuilt under an unchanged version string is a "
              "different build. Archive it under a distinguishing name and "
              "decide which one the admitted set means.")
        return 1
    print(f"ok: {version} archived, installed sts2.dll matches "
          f"({sha[:16]}...)")
    return 0


def archive() -> int:
    version, sha, info = installed()
    index = load_index()
    row = index["builds"].get(version)
    if row and row["dll_sha256"] == sha:
        print(f"ok: {version} already archived with a matching sha256")
        return 0
    if row:
        raise SystemExit(
            f"{version} is already archived with sha {row['dll_sha256']}, but "
            f"the install has {sha}. Refusing to overwrite an archived build "
            "-- resolve which is authoritative first.")

    dest = HERE / version
    if dest.exists():
        raise SystemExit(f"{dest} exists but is not in the index; resolve by "
                         "hand rather than overwriting provenance")
    dest.mkdir(parents=True)
    shutil.copytree(STEAM / RUNTIME_SUBDIR, dest / RUNTIME_SUBDIR)
    shutil.copy2(STEAM / "release_info.json", dest / "release_info.json")

    copied = _sha256(dest / RUNTIME_SUBDIR / "sts2.dll")
    if copied != sha:
        raise SystemExit(
            f"copy verification FAILED: installed {sha}, archived {copied}. "
            "The archive is not trustworthy; delete it and retry.")

    index["builds"][version] = {
        "dll_sha256": sha,
        "archived_at": dt.datetime.now(dt.UTC).strftime("%Y-%m-%d"),
        # release_info's build timestamp is the closest thing we have to a
        # release date, and #1271's straddling-run detector needs one per
        # build: a run whose start_time precedes its own tagged build's
        # release necessarily straddled.
        "built_at": info.get("date"),
        "commit": info.get("commit"),
        "release_info": info,
    }
    INDEX.write_text(json.dumps(index, indent=2, sort_keys=True) + "\n")
    print(f"archived {version} ({sha[:16]}...) and recorded the index entry.\n"
          "Add the prose entry to README.md with the version-impact summary.")
    return 0


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--check", action="store_true",
                   help="exit 1 if the installed build is unarchived or drifted")
    g.add_argument("--archive", action="store_true",
                   help="archive the installed build and index it")
    args = ap.parse_args(argv)
    return check() if args.check else archive()


if __name__ == "__main__":
    sys.exit(main())

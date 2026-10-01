#!/usr/bin/env python3
"""Dev-side capture watcher for STS2 live play (issue #118).

Archives two game files whenever they change, because the game keeps no
history of either:

  saves/current_run.save   full run snapshot as JSON, written continuously
                           during play, so it is the finer-grained record of
                           the cumulative per-stream RNG counters. (The
                           .mcr's rng dict carries them too, once per
                           combat — the old claim that it did not was a
                           decoder bug, see MCR_FORMAT.md "Counter
                           caveat".) Deleted when the run ends.
  replays/latest.mcr       CombatReplay bitstream, rewritten at each
                           combat's END with the combat-start snapshot
                           plus the full input log (cards played with
                           stable instance indexes + targets, potions,
                           end-turns). Overwritten by the next combat.

Captures land OUTSIDE the repo (default ~/sts2-captures) — nothing leaves
the dev machine; only distilled fixtures get checked in (see issue #118
scope). tools/mcr_validate.py consumes a capture directory.

Usage:
    python3 python/tools/mcr_watch.py [--out DIR] [--profile DIR]
                                      [--interval SECONDS] [--once]

Run it in a terminal before playing, or keep it running via launchd:
print a ready-to-install LaunchAgent with --print-launchd (installation
is left to you deliberately; this script never touches launchd itself).
"""
from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import shutil
import sys
import time
from datetime import datetime
from pathlib import Path

DEFAULT_OUT = Path.home() / "sts2-captures"
PROFILE_GLOB = str(Path.home() / "Library/Application Support/SlayTheSpire2"
                   / "steam" / "*" / "profile1")

WATCHED = {
    # kind -> (relative path, archive extension)
    "save": ("saves/current_run.save", "save"),
    "mcr": ("replays/latest.mcr", "mcr"),
}


def discover_profile() -> Path:
    hits = sorted(glob.glob(PROFILE_GLOB))
    if not hits:
        sys.exit(f"no profile dir matches {PROFILE_GLOB}; pass --profile")
    if len(hits) > 1:
        print(f"note: multiple profiles, using {hits[0]}", file=sys.stderr)
    return Path(hits[0])


def file_sig(path: Path):
    """(mtime_ns, size) or None if missing."""
    try:
        st = path.stat()
    except FileNotFoundError:
        return None
    return (st.st_mtime_ns, st.st_size)


def capture_name(kind: str, ext: str, digest: str) -> str:
    ts = datetime.now().strftime("%H%M%S.%f")[:-3]
    return f"{ts}_{kind}_{digest[:8]}.{ext}"


class Watcher:
    def __init__(self, profile: Path, out: Path, interval: float):
        self.profile = profile
        self.out = out
        self.interval = interval
        # per kind: last seen (mtime, size) and last archived content hash
        self.pending: dict[str, tuple] = {}
        self.last_sig: dict[str, tuple | None] = {}
        self.last_hash: dict[str, str | None] = {
            k: self._last_archived_hash(k) for k in WATCHED}
        self.log_path = out / "capture.log"

    def _last_archived_hash(self, kind: str) -> str | None:
        """Digest prefix of the newest existing capture of this kind, so a
        watcher restart doesn't re-archive an unchanged file. Filenames
        embed sha1[:8]; comparing prefixes is fine for dedupe."""
        newest = None
        for f in sorted(self.out.glob(f"*/*_{kind}_*.{WATCHED[kind][1]}")):
            newest = f
        if newest is None:
            return None
        return newest.stem.rsplit("_", 1)[-1]

    def log(self, msg: str):
        line = f"{datetime.now().isoformat(timespec='milliseconds')} {msg}"
        print(line, flush=True)
        self.out.mkdir(parents=True, exist_ok=True)
        with open(self.log_path, "a") as fh:
            fh.write(line + "\n")

    def day_dir(self) -> Path:
        d = self.out / datetime.now().strftime("%Y-%m-%d")
        d.mkdir(parents=True, exist_ok=True)
        return d

    def archive(self, kind: str, path: Path):
        try:
            data = path.read_bytes()
        except FileNotFoundError:
            return
        digest = hashlib.sha1(data).hexdigest()[:8]
        if digest == self.last_hash[kind]:
            return
        self.last_hash[kind] = digest
        ext = WATCHED[kind][1]
        dest = self.day_dir() / capture_name(kind, ext, digest)
        tmp = dest.with_suffix(dest.suffix + ".tmp")
        tmp.write_bytes(data)
        os.replace(tmp, dest)
        extra = ""
        if kind == "save":
            try:
                run = json.loads(data)
                # global node index — visited_map_coords resets per act,
                # prior acts' lengths live in map_point_history
                act = run.get("current_act_index") or 0
                prior = sum(len(a) for a in
                            (run.get("map_point_history") or [])[:act])
                nodes = len(run.get("visited_map_coords") or [])
                extra = (f" seed={run['rng']['seed']}"
                         f" node={prior + max(0, nodes - 1)}"
                         f" shuffle={run['rng']['counters'].get('shuffle')}")
            except (ValueError, KeyError):
                extra = " (unparseable JSON — partial write?)"
        self.log(f"captured {kind} -> {dest.name}"
                 f" ({len(data)} bytes){extra}")

    def poll_once(self):
        for kind, (rel, _ext) in WATCHED.items():
            path = self.profile / rel
            sig = file_sig(path)
            prev = self.last_sig.get(kind)
            if sig is None:
                if prev is not None:
                    self.log(f"{kind} deleted ({rel}) — run ended?")
                self.last_sig[kind] = None
                self.pending.pop(kind, None)
                continue
            if sig != prev:
                # changed this poll: debounce until stable one interval
                self.pending[kind] = sig
                self.last_sig[kind] = sig
                continue
            if kind in self.pending and self.pending[kind] == sig:
                del self.pending[kind]
                self.archive(kind, path)

    def run(self, once: bool = False):
        self.log(f"watching {self.profile} -> {self.out} "
                 f"(interval {self.interval}s)")
        # archive whatever exists right now (marks sig as pending so the
        # next stable poll captures it)
        for kind, (rel, _e) in WATCHED.items():
            path = self.profile / rel
            sig = file_sig(path)
            self.last_sig[kind] = sig
            if sig is not None:
                self.pending[kind] = sig
        if once:
            self.poll_once()
            return
        while True:
            self.poll_once()
            time.sleep(self.interval)


LAUNCHD_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
 "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.relaythespire.mcrwatch</string>
  <key>ProgramArguments</key><array>
    <string>{python}</string>
    <string>{script}</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{out}/launchd.log</string>
  <key>StandardErrorPath</key><string>{out}/launchd.err</string>
</dict></plist>
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--profile", type=Path, default=None)
    ap.add_argument("--interval", type=float, default=0.5)
    ap.add_argument("--once", action="store_true",
                    help="single poll (archive current files) and exit")
    ap.add_argument("--print-launchd", action="store_true",
                    help="print a LaunchAgent plist + install instructions")
    args = ap.parse_args()

    if args.print_launchd:
        plist = LAUNCHD_PLIST.format(python=sys.executable,
                                     script=str(Path(__file__).resolve()),
                                     out=str(args.out))
        print(plist)
        print("# install with:", file=sys.stderr)
        print("#   cat > ~/Library/LaunchAgents/"
              "com.relaythespire.mcrwatch.plist   # paste the above",
              file=sys.stderr)
        print("#   launchctl load ~/Library/LaunchAgents/"
              "com.relaythespire.mcrwatch.plist", file=sys.stderr)
        return

    profile = args.profile or discover_profile()
    Watcher(profile, args.out, args.interval).run(once=args.once)


if __name__ == "__main__":
    main()

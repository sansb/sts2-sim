#!/usr/bin/env python3
"""Run a preselected fight/seed matrix without hiding refusals or cutoffs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

RUST = Path(__file__).resolve().parents[1]


def entry_path(fight):
    base = (RUST.parent / "eval").resolve()
    path = (base / fight.get("entry", f"fights/{fight['id']}/entry.canonical.json")).resolve()
    if not path.is_relative_to(base):
        raise ValueError("entry must remain inside the version's eval directory")
    if fight.get("entry_sha256") and hashlib.sha256(path.read_bytes()).hexdigest() != fight["entry_sha256"]:
        raise ValueError("generated entry changed since the manifest was frozen")
    return path


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest", type=Path, default=RUST.parent / "eval/search/pilot-v1.json")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--binary", type=Path, default=RUST / "target/release/examples/search_experiment")
    p.add_argument("--playouts", type=int, help="optional equal-playout cap in addition to wall time")
    a = p.parse_args()
    manifest = json.loads(a.manifest.read_text())
    report = {"manifest": manifest, "binary_sha256": hashlib.sha256(a.binary.read_bytes()).hexdigest(),
              "playout_cap": a.playouts, "runs": []}
    a.out.parent.mkdir(parents=True, exist_ok=True)
    for fight in manifest["fights"]:
        entry = entry_path(fight)
        for seed in manifest["seeds"]:
            for method in manifest["methods"]:
                command = [str(a.binary), str(entry), method, str(seed), str(manifest["seconds_per_search"])]
                if a.playouts is not None:
                    command += ["1.414", "0.05", str(a.playouts)]
                try:
                    result = subprocess.run(command, text=True, capture_output=True,
                                            timeout=manifest["seconds_per_search"] + 15)
                    row = json.loads(result.stdout) if result.returncode == 0 else {
                        "status": "refused_or_failed", "exit_code": result.returncode,
                        "diagnostic": result.stderr}
                except subprocess.TimeoutExpired:
                    row = {"status": "process_timeout"}
                row.update(fight_id=fight["id"], mode=method, seed=seed)
                report["runs"].append(row)
                a.out.write_text(json.dumps(report, indent=2) + "\n")
                best = row.get("best") or {}
                print(f"{fight['id']} {method} seed={seed}: {row['status']} won={best.get('won')} hp={best.get('combat_hp')}", flush=True)


if __name__ == "__main__":
    main()

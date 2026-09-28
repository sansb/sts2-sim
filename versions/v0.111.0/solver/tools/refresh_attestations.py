#!/usr/bin/env python3
"""Deliberately refresh versions/v0.111.0/solver/attestations.json to the live artifacts.

Prints every changed entry so the PR diff and the refresh intent stay
reviewable. Adding a NEW shared artifact: add its name to the manifest
with any placeholder value, then run this tool.
"""

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import attestations


def main() -> None:
    manifest = attestations.manifest()
    changed = []
    for name, old in sorted(manifest.items()):
        new = attestations.live_sha256(name)
        if new != old:
            changed.append((name, old, new))
            manifest[name] = new
    path = pathlib.Path(attestations.__file__).parent / "attestations.json"
    path.write_text(json.dumps(manifest, indent=1, sort_keys=True) + "\n")
    for name, old, new in changed:
        print(f"{name}: {old[:12]} -> {new[:12]}")
    if not changed:
        print("manifest already current")


if __name__ == "__main__":
    main()

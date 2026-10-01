#!/usr/bin/env python3
"""Enchantment census for the coverage swarm (#138).

Enumerates every class in the Enchantments namespace (and .Mocks), keyed by
the id form the (deleted, #2827) combat_sim / coverage_report used for enchantments
(ENCHANTMENT.<UPPER_SNAKE of the class name>; combat_sim.KNOWN_ENCHANTMENTS
stores the bare UPPER form and coverage_report prefixes it).

The DLL carries no per-enchantment rarity/tier/act: EnchantmentModel's ctor
takes no args and there is no EnchantmentRarity enum (checked 2026-07-15),
so this census is deliberately just {id, class, kind}. kind classifies the
non-real rows so nothing is silently dropped:

    kind = enchantment | deprecated | mock

Output: JSON {ENCHANTMENT.ID: {...}} on stdout, sorted. Regenerate with:
    python3 tools/census_enchantments.py > enchantments_census.json
"""
import json
import os
import sys
from pathlib import Path

import dnfile

sys.path.insert(0, str(Path(__file__).parent.parent))
from sts2_rng import snake_case  # noqa: E402

DLL = Path(os.environ.get(
    "STS2_DLL",
    Path.home() / ("Library/Application Support/Steam/steamapps/common/"
                   "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
                   "data_sts2_macos_arm64/sts2.dll"),
))

ENCH_NS = "MegaCrit.Sts2.Core.Models.Enchantments"


def main():
    pe = dnfile.dnPE(str(DLL))
    md = pe.net.mdtables

    out = {}
    for t in md.TypeDef.rows:
        ns, name = str(t.TypeNamespace), str(t.TypeName)
        if not ns.startswith(ENCH_NS):
            continue
        if name == "EnchantmentModel":
            continue
        if ns.endswith(".Mocks") or name.startswith("Mock"):
            kind = "mock"
        elif name == "DeprecatedEnchantment" or name.startswith("Deprecated"):
            kind = "deprecated"
        else:
            kind = "enchantment"
        ench_id = "ENCHANTMENT." + snake_case(name).upper()
        out[ench_id] = {"class": name, "kind": kind}

    print(json.dumps(out, indent=1, sort_keys=True))


if __name__ == "__main__":
    main()

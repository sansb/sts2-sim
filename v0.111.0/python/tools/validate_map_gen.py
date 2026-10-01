#!/usr/bin/env python3
"""Validate map_gen.py against the saved_map graphs in testdata/*.save.

For every fixture save and every act whose saved_map is present, regenerate
the map from (seed, act index, act id, ascension) and diff node-by-node.
"""
import json
import sys
from pathlib import Path

SOLVER = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(SOLVER))

import map_gen  # noqa: E402


def norm_point(p):
    # can_modify is deliberately excluded: fixture dumps disagree on it
    # (older dumper omits false; post-reload saves reset it to true).
    return (p["coord"]["col"], p["coord"]["row"], p["type"],
            tuple(sorted((c["col"], c["row"])
                         for c in p.get("children", []))))


def children_order(p):
    return [(c["col"], c["row"]) for c in p["children"]]


def compare(saved, gen, label):
    diffs = []
    for k in ("width", "height"):
        if saved[k] != gen[k]:
            diffs.append(f"{k}: saved={saved[k]} gen={gen[k]}")
    for k in ("boss", "start"):
        if norm_point(saved[k]) != norm_point(gen[k]):
            diffs.append(f"{k}: saved={norm_point(saved[k])} "
                         f"gen={norm_point(gen[k])}")
    sb = saved.get("second_boss") or saved.get("second_boss_point")
    gb = gen.get("second_boss")
    if (sb is None) != (gb is None):
        diffs.append(f"second_boss presence: saved={sb is not None} "
                     f"gen={gb is not None}")
    elif sb is not None and norm_point(sb) != norm_point(gb):
        diffs.append("second_boss mismatch")

    s_pts = {(p["coord"]["col"], p["coord"]["row"]): p
             for p in saved["points"]}
    g_pts = {(p["coord"]["col"], p["coord"]["row"]): p
             for p in gen["points"]}
    for coord in sorted(set(s_pts) | set(g_pts)):
        sp, gp = s_pts.get(coord), g_pts.get(coord)
        if sp is None:
            diffs.append(f"extra generated point {coord} "
                         f"{gp['type']}")
        elif gp is None:
            diffs.append(f"missing point {coord} {sp['type']}")
        elif norm_point(sp) != norm_point(gp):
            diffs.append(f"point {coord}: saved={norm_point(sp)} "
                         f"gen={norm_point(gp)}")

    s_starts = sorted((c["col"], c["row"])
                      for c in saved["start_coords"])
    g_starts = sorted((c["col"], c["row"]) for c in gen["start_coords"])
    if s_starts != g_starts:
        diffs.append(f"start_coords: saved={s_starts} gen={g_starts}")

    # order-sensitive checks, reported separately
    order_diffs = []
    for coord in sorted(set(s_pts) & set(g_pts)):
        so = children_order(s_pts[coord])
        go = children_order(g_pts[coord])
        if so != go and sorted(so) == sorted(go):
            order_diffs.append(f"child order {coord}: saved={so} gen={go}")
    return diffs, order_diffs


def main():
    td = SOLVER / "testdata"
    total = ok = 0
    for save_path in sorted(td.glob("*.save")):
        s = json.loads(save_path.read_text())
        if "acts" not in s or "ascension" not in s:
            print(f"SKIP {save_path.name}: not a full save dump")
            continue
        seed = s["rng"]["seed"]
        asc = s["ascension"]
        neow = s["extra_fields"]["started_with_neow"]
        mp = len(s["players"]) > 1
        for i, act in enumerate(s["acts"]):
            sm = act.get("saved_map")
            if sm is None:
                continue
            total += 1
            label = f"{save_path.name} act{i + 1} {act['id']}"
            second = sm.get("second_boss") is not None or \
                sm.get("second_boss_point") is not None
            try:
                m = map_gen.generate_act_map(
                    seed, i, act["id"], ascension=asc,
                    started_with_neow=neow, multiplayer=mp,
                    has_second_boss=second)
            except Exception as e:  # noqa: BLE001
                print(f"FAIL {label}: generator raised {e!r}")
                continue
            gen = map_gen.to_save_shape(m)
            diffs, order_diffs = compare(sm, gen, label)
            if not diffs:
                ok += 1
                od = f" ({len(order_diffs)} child-order diffs)" \
                    if order_diffs else ""
                print(f"OK   {label}: {len(sm['points'])} points{od}")
            else:
                print(f"FAIL {label}: {len(diffs)} diffs")
                for d in diffs[:12]:
                    print(f"     {d}")
    print(f"\n{ok}/{total} act maps matched")


if __name__ == "__main__":
    main()

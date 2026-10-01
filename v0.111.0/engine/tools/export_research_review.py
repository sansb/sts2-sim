#!/usr/bin/env python3
"""Replay retained research lines in Rust and export a draft site document.

The generated document says modeled / achieved / awaiting in-game replay.
No database writes, native-certification claims, or synthesized text events.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

RUST = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RUST / "tools"))
from diff_serve_client import DiffServe


def name(card):
    text = card["id"] + ("+" if card.get("upgrade") else "")
    if card.get("enchantment"):
        text += " [" + card["enchantment"][0] + " " + str(card["enchantment"][1]) + "]"
    return text


def snapshot(doc):
    return {"hp": doc["player"].get("hp", 0), "block": doc["player"].get("block", 0),
            "energy": doc["player"].get("energy", 0),
            "hand": [name(c) for c in doc["piles"].get("hand", [])],
            "enemies": [{"name": m["kind"], "hp": m.get("hp", 0),
                         "maxSeen": m.get("max_hp", 0),
                         "intent": m.get("next_move") or None} for m in doc["monsters"]]}


def replay(entry, actions, binary, expected_digest):
    client = DiffServe(binary)
    turns = []
    before = entry
    try:
        loaded = client.load(entry)
        if "ok" not in loaded:
            raise ValueError(loaded)
        for wire in actions:
            old = snapshot(before)
            turn = before["player"].get("turn", 1)
            if not turns or turns[-1]["turn"] != turn:
                turns.append({"turn": turn, "hp": old["hp"], "block": old["block"],
                              "hand": old["hand"], "actions": []})
            response = client.apply(wire)
            if "ok" not in response:
                raise ValueError(response)
            after = client.project()["ok"]["state"]
            action = {"kind": wire["kind"], "state_after": snapshot(after),
                      "wire": wire, "digest_after": response["ok"]["digest"]}
            if wire["kind"] == "play":
                cards = before["piles"]["hand"]
                card = next(c for c in cards if c["uid"] == wire["uid"])
                action.update(card=name(card), hand_position=cards.index(card) + 1)
                if wire.get("target") is not None:
                    target = before["monsters"][wire["target"]]
                    action["target"] = f"{target.get('hp', 0)}hp {target['kind']}"
            elif wire["kind"] == "potion":
                action["potion"] = before["player"]["potion_slots"][wire["slot"]]
            elif wire["kind"] == "select":
                pending = before["player"].get("pending", [])
                if len(pending) != 3 or pending[0] != "frame_select":
                    raise ValueError("selection needs an explicit presentation adapter")
                op = pending[2]
                if op[:1] != ["select"] or op[1] not in before["piles"] or op[2:4] != [1, 1]:
                    raise ValueError("only one-card physical selections are presented")
                candidates = before["piles"][op[1]]
                remaining = {c["uid"] for c in after["piles"].get(op[1], [])}
                selected = [c for c in candidates if c["uid"] not in remaining]
                if len(selected) != 1:
                    raise ValueError("selection's physical identity is ambiguous")
                action.update(cards=[name(selected[0])], selection_uid=selected[0]["uid"],
                              selection_source=op[1], selection_position=candidates.index(selected[0]) + 1,
                              selection_candidates=[dict(c, label=name(c)) for c in candidates])
            turns[-1]["actions"].append(action)
            before = after
        if response["ok"]["digest"] != expected_digest:
            raise ValueError("retained line's terminal digest changed")
        return turns, before
    finally:
        client.close()


def write_guide(out, outcome):
    rows = [f"# {outcome['display_name']}", "",
            "Use the exact floor-start replay, with the original RNG. Follow the order below, including End Turn; extra plays can change later draws.", "",
            "The listed hand position is counted left-to-right in the modeled hand before playing that card. For selections, the guide includes the modeled source-pile position and physical identity; check the card's upgrade/enchantment and the next hand checkpoint if the game's selector sorts differently.", ""]
    for group in outcome["line"]:
        rows += [f"## Turn {group['turn']} — start at {group['hp']} HP", "",
                 "Opening hand: " + " · ".join(group["hand"]), ""]
        for i, action in enumerate(group["actions"], 1):
            kind = action["kind"]
            label = (f"Play {action['card']} (hand position {action['hand_position']})" if kind == "play"
                     else f"Drink {action['potion']}" if kind == "potion"
                     else "End Turn" if kind == "end"
                     else f"Select {', '.join(action['cards'])} from {action['selection_source']} (pile position {action['selection_position']}, physical uid {action['selection_uid']})")
            state = action["state_after"]
            rows.append(f"{i}. {label}. → **{state['hp']} HP**, {state['block']} block; boss {sum(max(0, e['hp']) for e in state['enemies'])} HP.")
            if kind == "select":
                rows.append("   Candidates in modeled pile order: " + " · ".join(
                    f"{j + 1}: {c['label']} (uid {c['uid']})" for j, c in enumerate(action["selection_candidates"])))
        rows.append("")
    rows += [f"Expected finish: **win, {outcome['final_hp']} HP, turn {outcome['turns']}**.", "",
             "If anything differs, stop at the first mismatch and record the turn/action, hand, HP, block and enemy HP. Keep the game's latest replay before starting another fight."]
    (out / (outcome["line_id"] + ".md")).write_text("\n".join(rows) + "\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--fight-index", type=int, required=True)
    p.add_argument("--snapshot-id", required=True)
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    source = RUST / "benchmarks/2026-09-17-insatiable-search"
    entry = json.loads((source / "entry.json").read_text())
    binary = RUST / "target/release/sts-sim"
    outcomes = []
    for filename, line_id, display in [("uct-2", "best", "55 HP · 11 turns"),
                                       ("random-2", "rollout-54", "54 HP · 11 turns"),
                                       ("uct-1", "crosscheck-43", "43 HP · 14 turns")]:
        result = json.loads((source / (filename + ".json")).read_text())
        best = result["best"]
        line, final = replay(entry, best["actions"], binary, best["final_digest"])
        if not best["won"] or final["player"].get("hp", 0) != best["combat_hp"]:
            raise ValueError("winning result does not match its replay")
        outcome = {"line_id": line_id, "display_name": display, "line": line,
                   "result": "win", "won": True, "final_hp": best["combat_hp"],
                   "hp_lost": 66 - best["combat_hp"], "potions_used": 1,
                   "turns": best["turn"], "exact": False, "deadline_hit": True,
                   "bound": "achieved_lower_bound", "claim": {"kind": "achieved", "display": "achieved (lower bound)"},
                   "verification_note": "Replayed in the simulator; awaiting in-game verification.",
                   "verification": {"rust_replay": True, "in_game": "pending",
                                    "entry_digest": result["entry_digest"], "final_digest": best["final_digest"],
                                    "wire_actions_sha256": hashlib.sha256(json.dumps(best["actions"], sort_keys=True).encode()).hexdigest()},
                   "search": {"method": result["mode"], "seed": result["seed"], "playouts": result["playouts"], "seconds": result["budget_seconds"]}}
        outcomes.append(outcome)
        write_guide(a.out, outcome)
    human = json.loads((source / "human-replay.json").read_text())
    recorded, _ = replay(entry, [row["action"] for row in human["trace"]], binary, human["final_digest"])
    document = {"schema_version": 8, "status": "ok", "fight": {
                    "fight_index": a.fight_index, "floor": 33, "encounter_id": "ENCOUNTER.THE_INSATIABLE_BOSS", "node_type": "boss"},
                "actual": {"entry_hp": 66, "final_hp": 0, "hp_lost": 66, "won": False,
                           "turns": 9, "potions_used": {"count": 1, "names": ["CLARITY"]}},
                "recorded_line": recorded,
                "best_actual_seed": outcomes[0], "alternative_actual_seeds": outcomes[1:],
                "derived": {"skill_gap": {"available": True, "kind": "lower_bound", "value": 55, "display": "at least 55"}},
                "metadata": {"trust_tier": "modeled", "game_build": "v0.111.0", "source": "research_import",
                             "entry_digest": outcomes[0]["verification"]["entry_digest"],
                             "engine_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                             "snapshot_id": a.snapshot_id, "native_full_checksum": "incomplete",
                             "in_game_verification": "pending", "assumed_fully_unlocked": False}}
    digest = hashlib.sha256(json.dumps(document, sort_keys=True).encode()).hexdigest()
    row = {"run_id": a.run_id, "fight_index": a.fight_index, "document": document,
           "status": "ok", "schema_version": 8, "solver_build": "v0.111.0",
           "generator_config_hash": digest, "solver_identity_digest": digest}
    (a.out / "review-row.draft.json").write_text(json.dumps(row, indent=2) + "\n")
    print(f"Exported {len(outcomes)} replay guides and draft review row to {a.out}")


if __name__ == "__main__":
    main()

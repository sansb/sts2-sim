"""What outlived the Python fight review: the capture run and the shared labels.

The review-document producer that lived here (the bounded Rust search, the
recorded-line witness and the play log built from them) was deleted with the
rest of the Python review path on 2026-10-11. Uploaded fights are replayed by
the Rust recorded-line resolver (`engine/src/recorded.rs`), which the site
runs as wasm, and certified by `engine/tools/eval_suite.py`.

Three things had other callers and stay:

* `replay_capture_run`, the document `sts-sim entry --capture-run` roots.
  The eval suite's upload import writes it beside each capture.
* `_with_entry_reset_witness`, the floor-save root's AfterEnergyReset order
  (`tools/live_coach.py`).
* `card_name` and `snapshot`, the card and state labels `rust_replay` and the
  live coach print.
"""
import copy
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "tools"))

import live_coach  # noqa: E402  (the save adapter, `build_entry`)
import provenance_entry  # noqa: E402
import relay_parser  # noqa: E402
import review_provenance  # noqa: E402
from admission import require_admitted  # noqa: E402


def card_name(card):
    label = card["id"] + ("+" if card.get("upgrade") else "")
    if card.get("enchantment"):
        label += f" [{card['enchantment'][0]} {card['enchantment'][1]}]"
    return label


def snapshot(doc, intents=None):
    # `intent` is the move the monster will act with, which is what the game
    # shows as its intent during the player's turn. It comes from the Rust
    # `intents` query (#2806), which names pattern-driven monsters such as The
    # Insatiable as well as random-table ones, and adds `intent_damage` (the
    # per-hit number the game displays, after Strength, Weak, Vulnerable and
    # the other modifiers) and `intent_hits` only where the engine proves the
    # number exact. Where the query names nothing, `intent` falls back to the
    # projected `next_move` (random-table monsters only), else None, so an
    # older binary still snapshots (Sean, 2026-09-17: "we don't have anything
    # that shows cards drawn and enemy intents"). The frontend humanizes ids.
    monsters = doc.get("monsters", [])
    if intents is not None and (
            len(intents) != len(monsters)
            or any("uid" in m and m["uid"] != i.get("uid") for m, i in zip(monsters, intents))):
        raise ValueError("Rust intents do not match the projected monster roster")
    enemies = []
    for index, m in enumerate(monsters):
        enemy = {"name": m["kind"], "hp": m.get("hp", 0),
                 "maxSeen": m.get("max_hp", m.get("hp", 0)),
                 "intent": m.get("next_move") or None}
        described = intents[index] if intents is not None else {}
        if described.get("intent"):
            enemy["intent"] = described["intent"]
        if described.get("intent_damage") is not None and described.get("intent_hits"):
            enemy["intent_damage"] = described["intent_damage"]
            enemy["intent_hits"] = described["intent_hits"]
        enemies.append(enemy)
    return {"hp": doc["player"].get("hp", 0), "block": doc["player"].get("block", 0),
            "energy": doc["player"].get("energy", 3),
            "hand": [card_name(c) for c in doc["piles"].get("hand", [])],
            "enemies": enemies}


def _load_refusal(binary, document):
    """The refusal `sts-sim diff-serve` gives this document, or None."""
    completed = subprocess.run(
        [str(binary), "diff-serve"],
        input=json.dumps({"cmd": "load", "entry": document}) + "\n"
        + json.dumps({"cmd": "quit"}) + "\n",
        text=True, capture_output=True, timeout=60, check=False)
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    if len(lines) < 2:
        raise ValueError("diff-serve answered no load line: "
                         + completed.stderr[-300:])
    answer = json.loads(lines[1])
    if "ok" in answer:
        return None
    return json.dumps(answer.get("refusal", answer))


def _with_entry_reset_witness(binary, document):
    """Carry the entry's empty AfterEnergyReset listener order as a fact.

    Without `player.after_energy_reset_order` the engine treats the order as
    legacy-unknown and later refuses a mid-fight listener whose order it
    cannot place. The Python adapter wrote `[]` exactly when no listener was
    live at entry. The boundary is the authority on that claim: it rejects an
    order that does not exactly match the live listeners (`boundary.rs`,
    "must exactly match live reset listeners"), so `[]` is proposed and kept
    unless that specific rejection comes back, in which case the root stays
    legacy-unknown, as the Python adapter left it. Any other load refusal is
    search's to report on the witnessed root.
    """
    candidate = copy.deepcopy(document)
    candidate["player"]["after_energy_reset_order"] = []
    refusal = _load_refusal(binary, candidate)
    if refusal is not None and "after_energy_reset_order" in refusal:
        return document
    return candidate


def replay_capture_run(run_path, fight_index, replay, projection, lineage=()):
    """The capture run `sts-sim entry --capture-run` roots, as `(run, fight, capture)`.

    The capture's embedded run is taken as-is, except that the separately
    resolved save projection supplies unlock provenance only. The capture
    supplies actual HP, deck order, potion capacity and all nine RNG streams;
    no run-history counter predictions enter this path. The eval suite's
    upload import (#2915) writes this document beside the capture, and the
    certification roots the upload from it.

    ``lineage`` is a branch run's ancestry (#3373): a pre-branch fight's
    capture carries the SOURCE run's ``start_time``, admitted by exactly the
    rule ``review_provenance.analyze_bundle`` associated it under.
    """
    raw = json.loads(Path(run_path).read_text())
    run = relay_parser.parse_run(str(run_path))
    require_admitted(run.build_id)
    if run.build_id != "v0.111.0" or replay.get("version") != run.build_id:
        raise ValueError("replay belongs to another build")
    if not 0 <= fight_index < len(run.fights):
        raise ValueError("replay fight index out of range")
    fight = run.fights[fight_index]
    saved = replay["run"]
    if (saved["rng"]["seed"] != run.seed
            or not review_provenance._encounter_matches(replay, fight)
            or review_provenance._history_depth(saved) != fight.node_index
            or not review_provenance.capture_identity_admitted(
                saved["start_time"], raw["start_time"], fight.node_index,
                lineage)):
        raise ValueError("replay belongs to another run or fight")
    if review_provenance.candidate_counter_map({"entry": projection}) != review_provenance._replay_counter_map(saved):
        raise ValueError("entry projection does not match replay RNG counters")
    # Existing complete-envelope, character, deck/relic and persistent-field
    # validation remains binding. It also resolves captured upgrade copies.
    provenance_entry.provenance_entry_for_fight(
        fight, projection, expected_character=run.character, replay=replay)
    player = provenance_entry._complete_projected_player(projection)
    capture = copy.deepcopy(saved)
    if len(capture["players"]) != 1:
        raise ValueError("replay requires one player")
    capture["players"][0]["unlock_state"] = copy.deepcopy(player["unlock_state"])
    # MCR uses PascalCase stream names; the save adapter uses snake_case.
    # CombatOrbs is the native capture spelling (not CombatOrbGeneration).
    names = {**live_coach.STREAMS, "combat_orbs": "CombatOrbs"}
    rng = saved["rng"]
    streams = {}
    for name, native in names.items():
        words, counter = rng["states"][native], rng["counters"][native]
        if (not isinstance(words, list) or len(words) != 4
                or any(type(w) is not int or not 0 <= w < 2**64 for w in words)
                or type(counter) is not int or counter < 0):
            raise ValueError("malformed captured RNG stream: " + native)
        streams[name] = {"counter": counter, **dict(zip(("s0", "s1", "s2", "s3"), words))}
    if live_coach.verify_stream_seeding({"rng": {"seed": run.seed, "rngs": streams}}, run.build_id):
        raise ValueError("captured RNG states disagree with seed/counters")
    return run, fight, capture

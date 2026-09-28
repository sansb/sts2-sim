"""
Tests for the .mcr combat-replay decoder (mcr_parser.py).

Validation layers:
1. Real replays must decode end-to-end with ZERO unconsumed bits — the
   decoder raises if any padding bit is nonzero or any section over/
   under-reads. Two game builds are pinned:
   - testdata/latest.mcr (seed ZPJHU3WSH2, Ironclad A10, recorded July 9
     2026 on v0.108.0), decoded with the frozen v0.108.0 tables;
   - testdata/7XDBEBWZ1REL_byrdonis_v109.mcr (recorded July 26 2026 on
     v0.109.1), decoded with the current tables.
2. The modelIdHash header must equal the XxHash32 our table builder computes
   from sts2.dll's model registry; decode() enforces this, proving the
   net-id -> card/relic name mapping is byte-exact. The v0.109.1 value the
   GAME wrote is pinned as a literal below, so a builder change that makes
   the two sides agree on a wrong value cannot pass (issue #639).
3. Spot checks of known run facts (seed, character, deck contents, picked
   card from the floor-2 reward) and of the recorded inputs; on v0.109 also
   the checksum section's full combat state (hand and draw-pile order,
   monster HP and powers).

Run: python3 -m pytest versions/v0.111.0/solver/test_mcr_parser.py
"""

import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from mcr_parser import BitReader, decode, tables_for_version  # noqa: E402

TESTDATA = pathlib.Path(__file__).parent / "testdata"
MCR = TESTDATA / "latest.mcr"
MCR_109 = TESTDATA / "7XDBEBWZ1REL_byrdonis_v109.mcr"

# Ground truth: the modelIdHash these builds actually wrote into their
# replay headers. Do NOT relax these to match a rebuilt mcr_tables.json —
# a mismatch means the net-id reconstruction is wrong (that is exactly how
# #639 hid for a release: both sides were self-consistently wrong).
HASH_V108 = 0x75EE2DEF
HASH_V109 = 0xFC40A98D


def test_bitreader_lsb_first():
    r = BitReader(bytes([0b00100100, 0xFF]))
    assert r.read_bits(3) == 0b100
    assert r.read_bits(5) == 0b00100
    assert r.read_bits(8) == 0xFF


def test_bitreader_signed_int32():
    r = BitReader((0xFFFFFFFF).to_bytes(4, "little"))
    assert r.i(32) == -1


def test_tables_match_the_hash_the_game_wrote():
    # The tables for each build must reproduce that build's own header hash.
    assert tables_for_version("v0.108.0")["model_id_hash"] == HASH_V108
    assert tables_for_version("v0.109.1")["model_id_hash"] == HASH_V109
    # ...and the two builds really are different tables, so a stale file
    # cannot satisfy both.
    assert HASH_V108 != HASH_V109


def test_full_decode_consumes_every_bit():
    # decode() raises McrError on hash mismatch, unconsumed bits, or
    # nonzero padding — reaching here means all three checks passed.
    replay = decode(MCR)
    assert replay["version"] == "v0.108.0"
    assert replay["git_commit"] == "58694f64"
    assert replay["model_id_hash"] == HASH_V108


def test_run_snapshot_matches_known_run():
    run = decode(MCR)["run"]
    assert run["rng"]["seed"] == "ZPJHU3WSH2"
    assert run["ascension"] == 10
    assert run["game_mode"] == "Custom"

    p = run["players"][0]
    assert p["character"] == "IRONCLAD"
    assert p["current_hp"] == 56 and p["max_hp"] == 80

    deck = [(c["id"], c["upgrade_level"], c["floor_added_to_deck"])
            for c in p["deck"]]
    # starter deck + the floor-2 reward pick (BLOOD_WALL+, upgraded by
    # SILVER_CRUCIBLE at pickup)
    assert deck.count(("STRIKE_IRONCLAD", 0, 1)) == 5
    assert deck.count(("DEFEND_IRONCLAD", 0, 1)) == 4
    assert ("BASH", 0, 1) in deck
    assert ("BLOOD_WALL", 1, 2) in deck
    assert [x["id"] for x in p["relics"]] == \
        ["BURNING_BLOOD", "SILVER_CRUCIBLE"]


def test_rng_counters_are_the_full_cumulative_set():
    # The .mcr DOES carry every stream, cumulative. The old "Counter caveat"
    # (MCR_FORMAT.md) was a decoder bug, not a game quirk: in
    # `counters[self.enum(...)] = r.i(32)` Python evaluates the value first,
    # so the 32-bit counter was read where the 5-bit stream id lives. The bit
    # total was unchanged, so the file still decoded "cleanly" — into garbage
    # pairs ({"UpFront": 11}). This pin is the fixed reading.
    run = decode(MCR)["run"]
    counters = run["rng"]["counters"]
    assert len(counters) == 12
    # Shuffle 18 entering ZPJHU3WSH2 fight 2 was established independently,
    # by replaying the recorded inputs, before the decoder could read it.
    assert counters["Shuffle"] == 18
    assert counters["UpFront"] == 406 and counters["Niche"] == 2
    assert run["players"][0]["player_rng"]["counters"] == \
        {"Rewards": 13, "Shops": 0, "Transformations": 0}
    # v0.108 has no per-stream xoshiro state on the wire
    assert "states" not in run["rng"]


def test_v108_counters_match_the_paired_json_save():
    # Cross-artifact pin: the boss replay's snapshot counters must equal the
    # counters distilled from current_run.save during that same fight
    # (testdata/HYHM8WP1E5_boss_entry.save). Nothing in the decode path
    # produced those numbers, so this is an external oracle for the fix
    # above — and it is exactly the comparison the "unexplained
    # contradiction" was built on.
    import json

    replay = decode(TESTDATA / "HYHM8WP1E5_boss.mcr")["run"]
    save = json.loads((TESTDATA / "HYHM8WP1E5_boss_entry.save").read_text())

    def snake(name):
        return "".join(("_" + c.lower()) if c.isupper() and i else c.lower()
                       for i, c in enumerate(name))

    assert replay["rng"]["seed"] == save["rng"]["seed"]
    assert {snake(k): v for k, v in replay["rng"]["counters"].items()} == \
        save["rng"]["counters"]
    saved_player = save["players"][0]["rng"]
    player = replay["players"][0]["player_rng"]
    assert player["seed"] == saved_player["seed"]
    assert {snake(k): v for k, v in player["counters"].items()} == \
        saved_player["counters"]


def test_map_points_read_coord_before_point_type():
    # SerializableMapPoint::Serialize (v0.111.0 RVA 0x40d80) writes Coord
    # (IL_0013) before PointType (IL_0020). Reading PointType first put
    # Coord.col in `point_type` (the boss at col 3 decoded as "Treasure") and
    # [row, PointType] in `coord`, so no child edge landed on a real point.
    for path in (MCR, MCR_109):
        saved = decode(path)["run"]["acts"][0]["saved_map"]
        assert saved["boss_point"]["point_type"] == "Boss"
        assert saved["starting_point"]["point_type"] == "Ancient"
        width, height = saved["grid_width"], saved["grid_height"]
        coords = {tuple(p["coord"]) for p in saved["points"]}
        assert all(col < width and row < height for col, row in coords)
        targets = coords | {tuple(saved["boss_point"]["coord"])}
        for point in saved["points"] + [saved["starting_point"]]:
            assert {tuple(c) for c in point["child_coords"]} <= targets


def test_events_are_turn1_inputs():
    events = decode(MCR)["events"]
    assert [e["event_type"] for e in events] == ["GameAction"] * 5
    acts = [e["action"] for e in events]
    assert [a["type"] for a in acts] == [
        "NetPlayCardAction", "NetPlayCardAction", "NetPlayCardAction",
        "NetEndPlayerTurnAction", "NetReadyToBeginEnemyTurnAction",
    ]
    assert [a.get("card_id") for a in acts[:3]] == \
        ["STRIKE_IRONCLAD", "STRIKE_IRONCLAD", "DEFEND_IRONCLAD"]
    # both strikes targeted creature 1; the defend has no target
    assert [a.get("target_id") for a in acts[:3]] == [1, 1, None]


def test_map_history_records_previous_fight():
    run = decode(MCR)["run"]
    rooms = [r for mp in run["map_point_history"] for e in mp
             for r in e["rooms"]]
    fights = [(r["model_id"], r["monster_ids"], r["turns_taken"])
              for r in rooms if r["room_type"] == "Monster"]
    assert fights == [("ENCOUNTER.TOADPOLES_WEAK",
                       ["TOADPOLE", "TOADPOLE"], 3)]


# --- v0.109.1 -----------------------------------------------------------
# One fight from the 2026-07-26 live session: Ironclad A10, seed
# 7XDBEBWZ1REL, Byrdonis, one recorded card play (Bash on the bird).


def test_v109_full_decode_consumes_every_bit():
    replay = decode(MCR_109)
    assert replay["version"] == "v0.109.1"
    assert replay["git_commit"] == "c8c577f6"
    assert replay["model_id_hash"] == HASH_V109


def test_v109_run_snapshot():
    run = decode(MCR_109)["run"]
    assert run["rng"]["seed"] == "7XDBEBWZ1REL"
    assert run["ascension"] == 10
    assert run["game_mode"] == "Standard"

    p = run["players"][0]
    assert p["character"] == "IRONCLAD"
    assert p["current_hp"] == 64 and p["max_hp"] == 80
    assert [x["id"] for x in p["relics"]] == \
        ["BURNING_BLOOD", "SILVER_CRUCIBLE"]
    # SavedProperties: name/value pairing (the game writes the name FIRST)
    assert p["relics"][1]["props"] == {"TimesUsed": 3,
                                       "TreasureRoomsEntered": 0}
    assert [x["id"] for x in p["potions"]] == \
        ["DEXTERITY_POTION", "BLOOD_POTION"]


def test_v109_rng_carries_xoshiro_state():
    # v0.109 replaced the bare per-stream counter with SerializableRng =
    # counter + the four xoshiro256** state words.
    rng = decode(MCR_109)["run"]["rng"]
    assert rng["counters"]["Shuffle"] == 61
    assert rng["counters"]["UpFront"] == 414
    state = rng["states"]["Shuffle"]
    assert len(state) == 4 and all(0 <= w < 1 << 64 for w in state)
    assert state == [0x27C03C6810FEE9E4, 0x9B95FBEFE76A8840,
                     0xB147AB6B6D1C26F2, 0x4864A73D308D3A5E]


def test_v109_events():
    events = decode(MCR_109)["events"]
    assert [e["event_type"] for e in events] == ["GameAction"]
    act = events[0]["action"]
    assert act["type"] == "NetPlayCardAction"
    assert act["card_id"] == "BASH" and act["target_id"] == 1
    assert act["combat_card_index"] == 1


def test_v109_checksum_full_combat_state():
    # New in v0.109: every replay carries NetFullCombatState snapshots, so
    # the engine's own hand and draw-pile ORDER is ground truth (the exact
    # thing #636 needed and screen-reading hides).
    checksums = decode(MCR_109)["checksums"]
    assert len(checksums) == 2
    first = checksums[0]
    assert first["context"] == "After player turn start"

    creatures = first["full_state"]["creatures"]
    assert [c["monster_id"] for c in creatures] == [None, "BYRDONIS"]
    assert creatures[0]["player_id"] is not None  # the player creature
    assert (creatures[1]["current_hp"], creatures[1]["max_hp"]) == (90, 90)
    assert [(pw["id"], pw["amount"]) for pw in creatures[1]["powers"]] == \
        [("TERRITORIAL_POWER", 1)]

    player = first["full_state"]["players"][0]
    assert player["turn_number"] == 1 and player["phase"] == "Start"
    assert player["energy"] == 3 and player["gold"] == 38
    piles = {pile["pile_type"]: [c["card"]["id"] for c in pile["cards"]]
             for pile in player["piles"]}
    assert piles["Hand"] == ["STRIKE_IRONCLAD", "BASH", "ANGER",
                             "DEFEND_IRONCLAD", "DEFEND_IRONCLAD"]
    assert piles["Draw"][:3] == ["SETUP_STRIKE", "DEFEND_IRONCLAD",
                                 "STRIKE_IRONCLAD"]
    assert piles["Discard"] == [] and piles["Play"] == []

    # ...and the post-action snapshot shows Bash's damage and Vulnerable.
    last = checksums[-1]["full_state"]
    assert last["creatures"][1]["current_hp"] == 82
    assert [(pw["id"], pw["amount"])
            for pw in last["creatures"][1]["powers"]] == \
        [("TERRITORIAL_POWER", 1), ("VULNERABLE_POWER", 2)]
    assert last["players"][0]["phase"] == "Play"
    assert last["players"][0]["energy"] == 1


if __name__ == "__main__":
    import pytest
    raise SystemExit(pytest.main([__file__, "-v"]))

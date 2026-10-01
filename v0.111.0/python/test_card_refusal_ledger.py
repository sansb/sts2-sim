"""DLL-free freshness and validation guards for the card refusal ledger (#423).

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""
import copy
import importlib.util
import json
import pathlib

import pytest


HERE = pathlib.Path(__file__).parent
_spec = importlib.util.spec_from_file_location(
    "card_templates", HERE / "tools" / "card_templates.py")
ct = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ct)


def _load(name):
    return json.loads((HERE / name).read_text())


def test_composed_card_templates_are_fresh_without_a_dll():
    """The checked output is exactly raw snapshot + reviewed ledger, never seed data."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    census = _load("cards_census.json")
    composed = ct.compose_templates(raw, ledger, census)
    assert composed == checked
    # The comparison is intentionally textual as well as structural: these
    # generated inputs/output have one documented canonical representation.
    assert (HERE / "card_templates_raw.json").read_text() == ct.canonical_json(raw)
    assert (HERE / "card_refusal_ledger.json").read_text() == ct.canonical_json(ledger)
    assert (HERE / "card_templates.json").read_text() == ct.canonical_json(composed)

    # This is the full pre-#423 checked-only delta, including every reason;
    # it makes a lost migration loud instead of silently treating it as raw.
    checked_only = {cid: reason for cid, reason in checked["refused"].items()
                    if cid not in raw["refused"]}
    # #351/#352/#353 replaced Dark Embrace, Battle Trance, and Clash
    # #354 removes Rebound, Nostalgia, and Shining Strike; Batch 44 removes
    # Apparition and Buffer after landing their shared HP-loss pipeline:
    # Batch 52 removes Demesne, Machine Learning, and Pyre after their shared
    # hand-draw/max-energy modifier pipeline lands; Batch 60 retires the five
    # AutoPre/AutoPost card rows after the W161 phase primitive; Batch 69
    # retires Danse Macabre and Spirit of Ash; W173 retires Infinite Blades;
    # W197 retires Juggling after its exact listener lands; W198 retires
    # Biased Cognition and Fasten after their exact power hooks land; W199
    # retires Pale Blue Dot and Wraith Form; W200 retires Borrowed Time;
    # W229 retires Drum of Battle; Batch 128 retires Colossus; Batch 129
    # retires Cruelty; Batch 130 retires Corruption; Batch 131 retires
    # Vicious; Batch 134 retires Hellraiser; Batch 144 retires Calamity;
    # Batch 147 retires Entropy after its exact turn-start transform;
    # Batch 149 retires Debilitate, Knockdown, and Tracking; Batch 151
    # retires Arsenal; Batch 152 retires Rocket Punch; Batch 156 retires
    # Strangle; Batch 162 retires Stratagem; Batch 169 retires Underworld
    # after its exact teammate event boundary; Batch 222 retires Hello World
    # and Sentry Mode after their exact BeforeHandDraw generation
    # continuations land; Batch 223 retires Pillar of Creation and Smokestack
    # after their exact generated-card listeners land; Batch 224 retires
    # Shroud and Sleight of Flesh after their exact acquisition-ordered
    # AfterPowerAmountChanged listener walk lands; Batch 226 retires Pagestorm
    # and Iteration after the recursive AfterCardDrawn cursor lands; Batch 227
    # retires Trash to Treasure after CombatOrbGeneration/channel continuations;
    # Batch 231 retires Creative AI after its exact recursive serial
    # generation; Batch 243 retires Tools of the Trade and Tyranny after
    # their keyed post-draw choice continuations; Batch 244 retires Calcify,
    # Haunt, and Lethality after their exact damage hooks; Batch 245 retires
    # Serpent Form and Shadowmeld after their exact per-card pending-damage
    # and owner-block multiplier hooks; Batch 246 retires Tag Team after its
    # exact target-matched play-count power lands; Batch 247 retires Call of
        # the Void after exact Necrobinder pool closure; Batch 254 retires
        # Spectrum Shift after exact Regent owner-pool closure; Batch 267
        # retires Signal Boost after exact generic Power replay; Batch 268
        # retires Coordinate/Fade behind quarantined raw target templates;
        # Batch 269 retires Blaze behind the same exact target quarantine;
        # Batch 271 retires Flanking and Intercept behind exact multiplayer
        # power projections (Intercept keeps the raw-target quarantine);
        # Batch 277 retires Beacon behind its exact multiplayer hook model;
        # Batch 290 retires Concoct behind exact selected-player ownership;
        # Batch 299 retires Hammer Time behind exact serial multiplayer
        # Forge fan-out; issue #1751 retires Kingly Kick and Pinpoint after
        # their exact physical cost listener lifecycles land: 14 checked-only
        # refusals remain.
    assert len(checked_only) == 14
    assert ledger["explicit_refusals"] == checked_only


def test_issue1751_kingly_kick_and_pinpoint_leave_the_reviewed_refusal_ledger():
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    for cid in ("CARD.KINGLY_KICK", "CARD.PINPOINT"):
        assert cid in raw["cards"]
        assert cid not in ledger["explicit_refusals"]
        assert cid not in checked["refused"]
        assert checked["cards"][cid] == raw["cards"][cid]


def test_batch129_cruelty_is_admitted_from_the_raw_template():
    """Only the reviewed refusal leaves; the exact raw power body stays source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    cruelty = raw["cards"]["CARD.CRUELTY"]
    assert cruelty["levels"] == {
        "0": [["power", "CrueltyPower", "self", 25]],
        "1": [["power", "CrueltyPower", "self", 50]],
    }
    assert "CARD.CRUELTY" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.CRUELTY"] == cruelty
    assert "CARD.CRUELTY" not in checked["refused"]


def test_batch130_corruption_is_admitted_from_the_raw_template():
    """Only the reviewed refusal leaves; the exact raw power body stays source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    corruption = raw["cards"]["CARD.CORRUPTION"]
    assert corruption["cost_by_level"] == {"0": 3, "1": 2}
    assert corruption["levels"] == {
        "0": [["power", "CorruptionPower", "self", 1]],
        "1": [["power", "CorruptionPower", "self", 1]],
    }
    assert "CARD.CORRUPTION" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.CORRUPTION"] == corruption
    assert "CARD.CORRUPTION" not in checked["refused"]


def test_batch131_vicious_is_admitted_from_the_raw_template():
    """Only the reviewed refusal leaves; the exact raw power body stays source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    vicious = raw["cards"]["CARD.VICIOUS"]
    assert vicious["levels"] == {
        "0": [["power", "ViciousPower", "self", 1]],
        "1": [["power", "ViciousPower", "self", 2]],
    }
    assert "CARD.VICIOUS" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.VICIOUS"] == vicious
    assert "CARD.VICIOUS" not in checked["refused"]


def test_batch71_infinite_blades_leaves_only_the_reviewed_ledger_stop():
    """The raw declarative body remains authoritative after W173 review."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    cid = "CARD.INFINITE_BLADES"
    assert raw["cards"][cid]["levels"] == {
        "0": [["power", "InfiniteBladesPower", "self", 1]],
        "1": [["power", "InfiniteBladesPower", "self", 1]],
    }
    assert cid not in ledger["explicit_refusals"]
    assert checked["cards"][cid] == raw["cards"][cid]
    assert cid not in checked["refused"]


def test_batch44_hp_loss_cards_are_admitted_from_raw_templates():
    """Only reviewed ledger rows leave; current raw card bodies stay source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    expected = {
        "CARD.APPARITION": {
            "0": [["power", "IntangiblePower", "self", 1]],
            "1": [["power", "IntangiblePower", "self", 1]],
        },
        "CARD.BUFFER": {
            "0": [["power", "BufferPower", "self", 1]],
            "1": [["power", "BufferPower", "self", 2]],
        },
    }
    for cid, levels in expected.items():
        assert raw["cards"][cid]["levels"] == levels
        assert cid not in ledger["explicit_refusals"]
        assert checked["cards"][cid] == raw["cards"][cid]
        assert cid not in checked["refused"]


def test_352_battle_trance_is_admitted_from_the_raw_template():
    """Only the reviewed refusal is removed; raw ordered IL stays source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    battle_trance = raw["cards"]["CARD.BATTLE_TRANCE"]
    assert battle_trance["levels"] == {
        "0": [["draw", 3], ["power", "NoDrawPower", "self", 1]],
        "1": [["draw", 4], ["power", "NoDrawPower", "self", 1]],
    }
    assert "CARD.BATTLE_TRANCE" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.BATTLE_TRANCE"] == battle_trance
    assert "CARD.BATTLE_TRANCE" not in checked["refused"]


def test_353_clash_is_admitted_from_the_raw_template():
    """Only the reviewed refusal is removed; raw IL remains source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    clash = raw["cards"]["CARD.CLASH"]
    assert clash["levels"] == {
        "0": [["attack", 14, 1]],
        "1": [["attack", 18, 1]],
    }
    assert "CARD.CLASH" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.CLASH"] == clash
    assert "CARD.CLASH" not in checked["refused"]


def test_351_dark_embrace_is_admitted_from_the_raw_template():
    """Only the reviewed refusal is removed; raw power shell stays source."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    dark = raw["cards"]["CARD.DARK_EMBRACE"]
    assert dark["cost_by_level"] == {"0": 2, "1": 1}
    assert dark["levels"] == {
        "0": [["power", "DarkEmbracePower", "self", 1]],
        "1": [["power", "DarkEmbracePower", "self", 1]],
    }
    assert "CARD.DARK_EMBRACE" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.DARK_EMBRACE"] == dark
    assert "CARD.DARK_EMBRACE" not in checked["refused"]


def test_354_result_location_cards_leave_the_reviewed_refusal_ledger():
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    for cid in ("CARD.REBOUND", "CARD.NOSTALGIA", "CARD.SHINING_STRIKE"):
        assert cid in raw["cards"]
        assert cid not in ledger["explicit_refusals"]
        assert cid not in checked["refused"]


def test_anticipate_is_admitted_from_the_raw_template():
    """#172: only its former reviewed refusal was removed; the raw
    Apply<AnticipatePower> template is now an engine-supported step."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    anticipate = raw["cards"]["CARD.ANTICIPATE"]
    assert anticipate["levels"] == {
        "0": [["power", "AnticipatePower", "self", 2]],
        "1": [["power", "AnticipatePower", "self", 4]],
    }
    assert "CARD.ANTICIPATE" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.ANTICIPATE"] == anticipate
    assert "CARD.ANTICIPATE" not in checked["refused"]


def test_334_mangle_is_admitted_from_the_raw_template():
    """#334 removes only the reviewed refusal; raw IL-derived levels stay
    the source for the attack plus negative temporary enemy Strength."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    mangle = raw["cards"]["CARD.MANGLE"]
    assert mangle["levels"] == {
        "0": [["attack", 20, 1],
              ["power", "ManglePower", "enemy", 10]],
        "1": [["attack", 26, 1],
              ["power", "ManglePower", "enemy", 15]],
    }
    assert "CARD.MANGLE" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.MANGLE"] == mangle
    assert "CARD.MANGLE" not in checked["refused"]


def test_439_full_multiplayer_target_census_and_raw_shapes_are_pinned():
    """All 37 current overrides stay visible, including raw refusals."""
    raw = _load("card_templates_raw.json")
    checked = _load("card_templates.json")
    assert len(raw["targeting"]) == len(_load("cards_census.json")) == 596
    by_target = {
        "Self": {
            "CARD.BEACON_OF_HOPE", "CARD.CACOPHONY", "CARD.HAMMER_TIME",
            "CARD.HIBERNATE", "CARD.SNEAKY", "CARD.TANK", "CARD.UNDERWORLD",
        },
        "AnyEnemy": {
            "CARD.FLANKING", "CARD.GANG_UP", "CARD.KNOCKDOWN",
            "CARD.MIDNIGHT", "CARD.OUTRAGE", "CARD.TAG_TEAM", "CARD.THE_BALL",
        },
        "AnyAlly": {
            "CARD.BELIEVE_IN_YOU", "CARD.BLAZE", "CARD.CONCOCT",
            "CARD.CONSTELLATION", "CARD.COORDINATE", "CARD.DEMONIC_SHIELD",
            "CARD.FADE", "CARD.IGNITION", "CARD.IMITATION_LEARNING",
            "CARD.INTERCEPT", "CARD.LARGESSE", "CARD.LIFT", "CARD.MIMIC",
            "CARD.SOULBOUND", "CARD.TUTOR",
        },
        "AllAllies": {
            "CARD.BLADE_SYMPHONY", "CARD.ENERGY_SURGE",
            "CARD.GLIMPSE_BEYOND", "CARD.HUDDLE_UP", "CARD.LEGION_OF_BONE",
            "CARD.ONE_FOR_ALL", "CARD.PLOT", "CARD.RALLY",
        },
    }
    actual = {}
    for cid, meta in raw["targeting"].items():
        constraint = meta["multiplayer_constraint"]
        if constraint.get("name") == "MultiplayerOnly":
            assert constraint["value"] == 1
            assert constraint["source"] == "getter"
            target = meta["effective_target_type"]
            assert target["kind"] == "static"
            actual.setdefault(target["name"], set()).add(cid)
    assert actual == by_target
    assert sum(map(len, actual.values())) == 37
    assert checked["targeting"] == raw["targeting"]

    translated = set(raw["cards"]) & set().union(*by_target.values())
    assert translated == {
        "CARD.BEACON_OF_HOPE", "CARD.BLAZE", "CARD.CACOPHONY",
        "CARD.CONCOCT", "CARD.COORDINATE", "CARD.FADE", "CARD.FLANKING",
        "CARD.HAMMER_TIME", "CARD.INTERCEPT", "CARD.KNOCKDOWN",
        "CARD.MIDNIGHT", "CARD.SNEAKY", "CARD.TAG_TEAM", "CARD.UNDERWORLD",
    }
    enemy_shaped = {
        cid for cid in translated
        if any(step[0] == "attack"
               or (step[0] == "power" and step[2] == "enemy")
               for steps in raw["cards"][cid]["levels"].values()
               for step in steps)
    }
    assert enemy_shaped == {
        "CARD.BLAZE", "CARD.CONCOCT", "CARD.COORDINATE", "CARD.FADE",
        "CARD.FLANKING", "CARD.INTERCEPT", "CARD.KNOCKDOWN",
        "CARD.MIDNIGHT", "CARD.TAG_TEAM",
    }
    unsafe_translated = {
        cid for cid in translated
        if ct.template_target_refusal(
            raw["targeting"][cid], raw["cards"][cid]["levels"])
    }
    assert unsafe_translated == {
        "CARD.BLAZE", "CARD.CONCOCT", "CARD.COORDINATE", "CARD.FADE",
        "CARD.INTERCEPT",
    }
    ledger = _load("card_refusal_ledger.json")
    assert unsafe_translated <= (
        set(ledger["explicit_refusals"])
        | set(ledger["template_admission_exclusions"]))


def test_439_composition_rejects_an_unreviewed_unsafe_target():
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    ledger["template_admission_exclusions"].pop("CARD.BLAZE")
    with pytest.raises(ValueError, match="reviewed refusal or hand-model"):
        ct.compose_templates(raw, ledger, _load("cards_census.json"))


def test_389_oblivion_is_admitted_from_the_raw_template():
    """#389 removes only the reviewed latch refusal; the raw 3/4 enemy
    OblivionPower application remains the source of template metadata."""
    raw = _load("card_templates_raw.json")
    ledger = _load("card_refusal_ledger.json")
    checked = _load("card_templates.json")
    oblivion = raw["cards"]["CARD.OBLIVION"]
    assert oblivion["levels"] == {
        "0": [["power", "OblivionPower", "enemy", 3]],
        "1": [["power", "OblivionPower", "enemy", 4]],
    }
    assert "CARD.OBLIVION" not in ledger["explicit_refusals"]
    assert checked["cards"]["CARD.OBLIVION"] == oblivion
    assert "CARD.OBLIVION" not in checked["refused"]


@pytest.mark.parametrize("mutation, match", [
    (lambda ledger: ledger["explicit_refusals"].update({"CARD.NOPE": "I5"}),
     "unknown"),
    (lambda ledger: ledger["explicit_refusals"].update(
        {"CARD.ABUNDANCE": "I5 duplicate raw"}), "duplicate raw"),
    (lambda ledger: ledger["translator_refusal_exclusions"].update(
        {"CARD.STRIKE_IRONCLAD": "not a raw refusal"}), "must name current raw"),
    (lambda ledger: ledger["template_exclusions"].update(
        {"CARD.STRIKE_IRONCLAD": "not explicitly refused"}),
     "template exclusions must be explicit"),
    (lambda ledger: ledger["template_admission_exclusions"].update(
        {"CARD.NOPE": "unknown admission"}), "unknown"),
            (lambda ledger: ledger["template_admission_exclusions"].update(
                {"CARD.FURNACE": "conflicts with refusal"}),
             "conflict with a refusal"),
])
def test_ledger_rejects_unknown_stale_and_invalid_overlap(mutation, match):
    ledger = copy.deepcopy(_load("card_refusal_ledger.json"))
    mutation(ledger)
    with pytest.raises(ValueError, match=match):
        ct.compose_templates(_load("card_templates_raw.json"), ledger,
                             _load("cards_census.json"))

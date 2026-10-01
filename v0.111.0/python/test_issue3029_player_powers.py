"""#3029: native checkpoints compare the player's powers, reported first.

`rust_replay.native_player_power_mismatches` maps every native hero power
row onto Rust's canonical player fields. The production review keeps its
verdicts (`check_native_snapshot(..., powers=False)` is the default), and the
census reports power drift beside an unchanged `lockstep` unless
`--powers-verdict` is passed.
"""
import pathlib
import sys

import pytest

import mcr_native
import rust_replay as replay
from rust_replay import PowerMismatch

HERE = pathlib.Path(__file__).resolve().parent
RUST_DIR = HERE.parent / "engine"
sys.path.insert(0, str(RUST_DIR / "tools"))

import eval_suite  # noqa: E402


def _native(*powers):
    return {"creatures": [{"player_id": 1, "powers": [
        {"id": name, "amount": amount} for name, amount in powers]}]}


def _state(**player):
    return {"player": player}


def _rows(state, native):
    return [m.as_row() for m in replay.native_player_power_mismatches(state, native)]


# ---------------------------------------------------------------------------
# The mapping
# ---------------------------------------------------------------------------


def test_player_side_debuffs_map_to_their_player_slots():
    fields = replay.NATIVE_PLAYER_POWER_FIELDS
    assert fields["WEAK_POWER"] == "player_weak"
    assert fields["VULNERABLE_POWER"] == "player_vuln"
    assert fields["FRAIL_POWER"] == "player_frail"
    assert fields["DOOM_POWER"] == "player_doom"
    assert fields["RITUAL_POWER"] == "player_ritual"
    assert fields["HEX_POWER"] == "hex_power"
    assert fields["THE_SEALED_THRONE_POWER"] == "sealed_throne"
    assert fields["DRAW_CARDS_NEXT_TURN_POWER"] == "draw_next_turn"
    assert fields["VEILPIERCER_POWER"] == "free_ethereal"
    assert fields["NOXIOUS_FUMES_POWER"] == "noxious_fumes"


def test_curious_power_is_compared_as_its_own_field():
    # #3427: Mad Science's Curious rider applies CuriousPower, which Rust
    # carries as `PowerId::Curious` (`curious`).
    assert replay.NATIVE_PLAYER_POWER_FIELDS["CURIOUS_POWER"] == "curious"
    assert _rows(_state(curious=1), _native(("CURIOUS_POWER", 1))) == []
    assert _rows(_state(), _native(("CURIOUS_POWER", 1))) == [
        {"class": "amount", "power": "CURIOUS_POWER", "native": 1, "rust": 0}]


def test_every_native_id_has_exactly_one_comparison():
    groups = [set(replay.NATIVE_PLAYER_POWER_FIELDS),
              set(replay.NATIVE_PLAYER_POWER_PROJECTIONS),
              set(replay._NATIVE_TEMPORARY_POWER),
              set(replay.NATIVE_INSTANCED_PLAYER_POWERS)]
    for i, left in enumerate(groups):
        for right in groups[i + 1:]:
            assert not left & right
    assert replay.NATIVE_SCALAR_INSTANCED_PLAYER_POWERS <= set(
        replay.NATIVE_PLAYER_POWER_FIELDS)
    assert all(name.endswith("_POWER") for group in groups for name in group)


def test_weak_drift_is_an_amount_mismatch():
    # The #3013 shape: a duration debuff Rust ticked differently.
    assert _rows(_state(player_weak=1), _native(("WEAK_POWER", 2))) == [
        {"class": "amount", "power": "WEAK_POWER", "native": 2, "rust": 1}]
    assert _rows(_state(player_weak=2), _native(("WEAK_POWER", 2))) == []


def test_a_power_rust_holds_and_native_lacks_is_a_mismatch():
    assert _rows(_state(player_frail=1), _native()) == [
        {"class": "amount", "power": "FRAIL_POWER", "native": 0, "rust": 1}]


def test_bool_slots_compare_as_one():
    assert _rows(_state(barricade=True), _native(("BARRICADE_POWER", 1))) == []


def test_stat_rows_include_their_temporary_parts():
    # Flex: native STRENGTH 5 with FLEX_POTION 5; Rust strength 0 + temp 5.
    native = _native(("STRENGTH_POWER", 5), ("FLEX_POTION_POWER", 5))
    assert _rows(_state(temp_strength=5), native) == []
    native = _native(("DEXTERITY_POWER", 4), ("ANTICIPATE_POWER", 2))
    assert _rows(_state(dexterity=2, temp_dexterity=2), native) == []
    native = _native(("FOCUS_POWER", 2), ("HOTFIX_POWER", 2))
    assert _rows(_state(temp_focus=2), native) == []
    # A leftover temporary part with no native row is both a stat and a
    # family disagreement.
    assert _rows(_state(temp_focus=1), _native()) == [
        {"class": "amount", "power": "FOCUS_POWER", "native": 0, "rust": 1},
        {"class": "amount", "power": "temp_focus", "native": 0, "rust": 1}]


def test_temporary_families_are_signed_sums():
    native = _native(("FOCUSED_STRIKE_POWER", 3), ("HYPERBEAM_FOCUS_DOWN_POWER", 1),
                     ("FOCUS_POWER", 2))
    assert _rows(_state(temp_focus=2), native) == []
    assert replay.NATIVE_TEMPORARY_POWER_FAMILIES["temp_strength"][
        "PIERCING_WAIL_POWER"] == -1


def test_red_skull_strength_is_the_rust_scalar():
    # #3044: Rust stores Red Skull's 3 in `strength`, so the projection adds
    # nothing on top of it, whatever the HP.
    native = _native(("STRENGTH_POWER", 3))
    assert _rows(_state(red_skull=True, strength=3, hp=20, max_hp=40), native) == []
    assert _rows(_state(red_skull=True, hp=20, max_hp=40), native) == [
        {"class": "amount", "power": "STRENGTH_POWER", "native": 3, "rust": 0}]
    assert _rows(_state(red_skull=True, hp=21, max_hp=40), _native()) == []


def test_shrink_sentinel_is_the_native_infinite_amount():
    assert _rows(_state(player_shrink=1), _native(("SHRINK_POWER", -1))) == []
    assert _rows(_state(), _native(("SHRINK_POWER", -1))) == [
        {"class": "amount", "power": "SHRINK_POWER", "native": -1, "rust": 0}]


def test_constrict_dampen_and_surrounded_projections():
    assert _rows(_state(constrict_sources=[[1, 3]]),
                 _native(("CONSTRICT_POWER", 3))) == []
    assert _rows(_state(dampen_casters=[2]), _native(("DAMPEN_POWER", 1))) == []
    assert _rows(_state(kaiser_facing=0), _native(("SURROUNDED_POWER", 1))) == []
    assert _rows(_state(), _native(("SURROUNDED_POWER", 1))) == [
        {"class": "amount", "power": "SURROUNDED_POWER", "native": 1, "rust": 0}]


def test_instanced_powers_compare_per_object_amounts():
    # #3021: two Automation objects, amounts 1 then 2.
    native = _native(("AUTOMATION_POWER", 1), ("AUTOMATION_POWER", 2))
    assert _rows(_state(automation=3, automation_later_instances=[[2, 10]]),
                 native) == []
    # The same total as one object is an instance-count disagreement.
    assert _rows(_state(automation=3), native) == [
        {"class": "instances", "power": "AUTOMATION_POWER",
         "native": [1, 2], "rust": [3]}]
    assert _rows(_state(panache_instances=[[4, 10, 5, False]]),
                 _native(("PANACHE_POWER", 10))) == []
    assert _rows(_state(the_bomb_instances=[[1, 3, 40], [2, 2, 50]]),
                 _native(("THE_BOMB_POWER", 3), ("THE_BOMB_POWER", 2))) == []
    assert _rows(_state(monologue_instances=[[7, 1, 0, 0]]),
                 _native()) == [
        {"class": "instances", "power": "MONOLOGUE_POWER",
         "native": [], "rust": [1]}]


def test_an_unprojected_power_fails_loudly_by_name():
    # ImitationLearningPower shares its name with a list-valued spec, so it
    # is deliberately left unprojected (`_NATIVE_PLAYER_POWER_DIRECT`).
    assert _rows(_state(block_next_turn=3),
                 _native(("IMITATION_LEARNING_POWER", 3))) == [
        {"class": "power_not_projected", "power": "IMITATION_LEARNING_POWER",
         "native": 3, "rust": None},
        {"class": "amount", "power": "BLOCK_NEXT_TURN_POWER",
         "native": 0, "rust": 3}]


def test_self_forming_clay_power_is_its_own_field():
    # #3159: Rust carries SelfFormingClayPower as `self_forming_clay_power`,
    # never folded into Block Next Turn; the bare `self_forming_clay` field
    # is the relic-ownership flag and is not read.
    assert _rows(_state(self_forming_clay=True, self_forming_clay_power=3),
                 _native(("SELF_FORMING_CLAY_POWER", 3))) == []
    assert _rows(_state(self_forming_clay=True, block_next_turn=3),
                 _native(("SELF_FORMING_CLAY_POWER", 3))) == [
        {"class": "amount", "power": "BLOCK_NEXT_TURN_POWER",
         "native": 0, "rust": 3},
        {"class": "amount", "power": "SELF_FORMING_CLAY_POWER",
         "native": 3, "rust": 0}]


def test_several_objects_of_a_scalar_instanced_power_are_not_compared():
    rows = _rows(_state(orbit=2), _native(("ORBIT_POWER", 1), ("ORBIT_POWER", 1)))
    assert rows == [{"class": "power_instances_not_projected",
                     "power": "ORBIT_POWER", "native": None, "rust": None}]
    assert _rows(_state(orbit=1), _native(("ORBIT_POWER", 1))) == []


def test_malformed_power_lists_raise():
    with pytest.raises(ValueError, match="malformed"):
        replay.native_player_power_mismatches(_state(), _native(("WEAK_POWER", True)))
    with pytest.raises(ValueError, match="repeats"):
        replay.native_player_power_mismatches(
            _state(), _native(("WEAK_POWER", 1), ("WEAK_POWER", 1)))
    with pytest.raises(ValueError, match="one player power list"):
        replay.native_player_power_mismatches(_state(), {"creatures": [{"player_id": 1}]})


# ---------------------------------------------------------------------------
# check_native_snapshot: the review keeps its verdicts
# ---------------------------------------------------------------------------


def _snapshot_pair(player_powers, rust_player):
    words = [1, 2, 3, 4]
    rng_states = {s: words for s in mcr_native._REPRESENTED_RNG_STREAMS.values()}
    rng_counters = {s: 0 for s in mcr_native._REPRESENTED_RNG_STREAMS.values()}
    native = {
        "players": [{"energy": 3, "turn_number": 1, "piles": [], "gold": 0,
                     "stars": 0, "max_potion_count": 0, "potions": [],
                     "orbs": []}],
        "creatures": [{"player_id": 1, "current_hp": 50, "max_hp": 80, "block": 0,
                       "powers": [{"id": n, "amount": a} for n, a in player_powers]}],
        "rng": {"states": rng_states, "counters": rng_counters},
    }
    state = {"player": dict({"hp": 50, "max_hp": 80}, **rust_player),
             "monsters": [], "piles": {},
             "rng": {attr: {"words": words, "counter": 0}
                     for attr in mcr_native._REPRESENTED_RNG_STREAMS}}
    return state, native


def test_snapshot_ignores_powers_unless_asked():
    state, native = _snapshot_pair([("WEAK_POWER", 2)], {"player_weak": 1})
    replay.check_native_snapshot(state, native)  # the review's default
    with pytest.raises(PowerMismatch,
                       match="differs from native player powers: WEAK_POWER") as caught:
        replay.check_native_snapshot(state, native, powers=True)
    assert caught.value.as_row() == {"class": "amount", "power": "WEAK_POWER",
                                     "native": 2, "rust": 1}
    state, native = _snapshot_pair([("WEAK_POWER", 2)], {"player_weak": 2})
    replay.check_native_snapshot(state, native, powers=True)


def test_native_checks_default_is_the_pre_3029_review():
    checks = replay.NativeChecks({"checksums": []})
    assert not hasattr(checks, "powers")


# ---------------------------------------------------------------------------
# The census: reported beside lockstep, verdict behind a flag
# ---------------------------------------------------------------------------


_MISMATCH = [{"class": "amount", "power": "WEAK_POWER", "native": 2, "rust": 1}]


def test_power_check_fields():
    assert eval_suite.power_check_fields({}, None) == {
        "power_check": "no_native_checkpoints"}
    row = {"checksummed": True}
    assert eval_suite.power_check_fields(row, None) == {"power_check": "match"}
    assert eval_suite.power_check_fields(row, {"step": 4, "mismatches": _MISMATCH}) == {
        "power_check": "mismatch", "power_mismatch_step": 4,
        "power_mismatch_class": "amount", "power_mismatch_power": "WEAK_POWER",
        "player_powers": _MISMATCH}
    opening = dict(row, opening_power_mismatches=_MISMATCH)
    assert eval_suite.power_check_fields(
        opening, {"step": 4, "mismatches": []})["power_mismatch_step"] == 0


def _certifiable():
    row = {"checksummed": True, "opening_checkpoint": "match",
           "power_check": "mismatch", "power_mismatch_step": 3,
           "player_powers": _MISMATCH}
    line = {"actions": [{}] * 6, "checkpoint_mismatch": None,
            "native_checkpoints": 5}
    return row, line


def test_power_mismatch_is_report_only_by_default():
    row, line = _certifiable()
    assert eval_suite.POWERS_VERDICT_DEFAULT is False
    assert eval_suite.certification_verdict(row, line) == {
        "lockstep": "lockstep_ok", "lockstep_step": 6}


def test_powers_verdict_fails_certification_at_the_first_step():
    row, line = _certifiable()
    verdict = eval_suite.certification_verdict(row, line, powers_verdict=True)
    assert verdict["lockstep"] == "checkpoint_mismatch"
    assert verdict["lockstep_step"] == 3
    assert eval_suite.checkpoint_mismatch_field(
        verdict["lockstep_detail"]) == "player_powers"
    # An earlier ordinary mismatch keeps its own step and field.
    line["checkpoint_mismatch"] = {
        "step": 2, "detail": "recorded replay differs from native monsters"}
    verdict = eval_suite.certification_verdict(row, line, powers_verdict=True)
    assert (verdict["lockstep_step"], verdict["lockstep_detail"]) == (
        2, "recorded replay differs from native monsters")
    # An unprojected power is its own field.
    row["player_powers"] = [{"class": "power_not_projected",
                             "power": "SELF_FORMING_CLAY_POWER",
                             "native": 3, "rust": None}]
    line["checkpoint_mismatch"] = None
    detail = eval_suite.certification_verdict(
        row, line, powers_verdict=True)["lockstep_detail"]
    assert eval_suite.checkpoint_mismatch_field(detail) == "power_not_projected"
    assert eval_suite.checkpoint_mismatch_field(
        str(PowerMismatch("power_instances_not_projected", "ORBIT_POWER", None, None))
    ) == "power_instances_not_projected"


def test_the_census_tallies_power_mismatches():
    rows = [
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "lockstep_ok", "power_check": "mismatch",
         "power_mismatch_class": "amount", "power_mismatch_power": "WEAK_POWER"},
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "lockstep_ok", "power_check": "match"},
        {"stage": "rooted", "rust": "admitted", "human": "diverged",
         "power_check": "mismatch", "power_mismatch_class": "power_not_projected",
         "power_mismatch_power": "SELF_FORMING_CLAY_POWER"},
    ]
    summary = eval_suite.census_summary(rows)
    assert summary["power_check"] == {"match": 1, "mismatch": 1}
    assert summary["power_check_before_divergence"] == {"mismatch": 1}
    assert summary["power_mismatch_classes"] == {"amount": 1, "power_not_projected": 1}
    assert summary["power_mismatch_powers"] == {"SELF_FORMING_CLAY_POWER": 1,
                                                "WEAK_POWER": 1}
    assert summary["lockstep_ok_with_power_mismatch"] == 1


class _PowerChecks:
    """A census checker whose second completed checkpoint disagrees on powers."""

    def __init__(self, _replay):
        self.validated = 0
        self.power_mismatch = None

    def start(self, _event):
        pass

    def stage(self, _applied):
        return None  # no checkpoint inside the apply (#3242)

    def no_decision(self, _event):
        pass

    def completed(self, _state):
        self.validated += 1
        if self.validated == 2 and self.power_mismatch is None:
            self.power_mismatch = _MISMATCH

    def finish(self):
        pass


class _ScriptedSession:
    """Answers `load`/`legal`/`apply`/`project` for potions then an end."""

    def __init__(self):
        self.state = {"player": {"turn": 1, "hp": 50}, "monsters": [],
                      "piles": {"hand": []}}

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        if request["cmd"] == "legal":
            return {"actions": [{"kind": "potion", "slot": 0}, {"kind": "end"}]}
        if request["cmd"] == "apply":
            return {"digest": "d"}
        if request["cmd"] == "project":
            return {"state": self.state}
        raise AssertionError(request)


def test_the_line_walk_carries_the_first_power_mismatch(monkeypatch):
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "_CensusChecks", _PowerChecks)
    events = [{"event_type": "Action", "action": {
        "type": "NetUsePotionAction", "potion_index": 0}}] * 2 + [
        {"event_type": "Action", "action": {
            "type": "NetEndPlayerTurnAction", "turn_number": 7}}]
    root = {"player": {"turn": 1}, "monsters": [], "piles": {"hand": []}}
    with pytest.raises(eval_suite.LineDiverged) as caught:
        eval_suite.replay_recorded_line(_ScriptedSession(), root, {"events": events})
    assert caught.value.power_mismatch == {"step": 2, "mismatches": _MISMATCH}
    fields = eval_suite.power_check_fields(
        {"checksummed": True}, caught.value.power_mismatch)
    assert fields["power_check"] == "mismatch"
    assert fields["power_mismatch_step"] == 2

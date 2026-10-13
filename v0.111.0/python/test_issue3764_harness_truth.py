"""#3764: three census verdicts that blamed the engine for the harness.

* the deal map sliced the Innate front before dropping the cards the opening
  created, so a created card ahead of it read as `entry_deal`;
* a capture that stops at a player turn start was `combat_not_complete`
  instead of truncated;
* three player powers were `power_not_projected` on certified lines.

Every fixture here is synthetic. None of the three fixes may certify a
partial line or hide a disagreement, and each has a control for that.
"""
import pathlib
import sys

import pytest

import rust_replay as replay

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "engine" / "tools"))

import eval_suite  # noqa: E402


# ---------------------------------------------------------------------------
# 1. The deal map: created cards leave before the Innate split
# ---------------------------------------------------------------------------


def _card(cid, uid, **more):
    return dict({"id": cid, "uid": uid, "upgrade": 0}, **more)


def test_a_created_card_ahead_of_the_innate_front_is_not_counted_in_it():
    """Radiant Pearl's shape: the opening adds a card to the Hand before the
    turn-one draw, so it sits at Hand index 0, ahead of the Innate card.
    `innate_min_draw` is 1 and names the deck's Innate card, not the created
    one."""
    root = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card("LUMINESCE", 3), _card("INNATE", 2), _card("A", 0)],
        "draw": [_card("B", 1)]}}
    # The raw shuffle had the Innate card last; the rewrite pulled it forward.
    recorded = [("A", 0), ("B", 0), ("INNATE", 0)]
    assert eval_suite._dealt_uids(root, recorded) == [0, 1, 2]


def test_two_innate_cards_behind_a_created_card_keep_their_reversed_order():
    root = {"player": {"innate_min_draw": 2}, "piles": {
        "hand": [_card("LUMINESCE", 4), _card("I2", 3), _card("I1", 1),
                 _card("A", 0)],
        "draw": [_card("B", 2)]}}
    # Front reversed is I1, I2: their relative order in the raw shuffle.
    recorded = [("A", 0), ("I1", 0), ("B", 0), ("I2", 0)]
    assert eval_suite._dealt_uids(root, recorded) == [0, 1, 2, 3]


def test_a_created_card_among_the_draw_is_dropped_the_same_way():
    """A created card shuffled into the Draw inside the Innate front."""
    root = {"player": {"innate_min_draw": 2}, "piles": {
        "hand": [],
        "draw": [_card("I2", 2), _card("DAZED", 3), _card("I1", 0),
                 _card("A", 1)]}}
    recorded = [("I1", 0), ("A", 0), ("I2", 0)]
    assert eval_suite._dealt_uids(root, recorded) == [0, 1, 2]


def test_the_deal_still_refuses_what_it_cannot_read():
    """The fix reads more deals; it does not accept a wrong one."""
    root = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card("LUMINESCE", 3), _card("INNATE", 2), _card("A", 0)],
        "draw": [_card("B", 1)]}}
    # A recorded cycle naming a card the root does not hold.
    with pytest.raises(ValueError, match="identities differ"):
        eval_suite._dealt_uids(root, [("A", 0), ("X", 0), ("INNATE", 0)])
    # More Innate cards claimed than deck cards dealt: only created cards
    # could fill the front, and they are not deck cards.
    over = {"player": {"innate_min_draw": 2}, "piles": {
        "hand": [_card("LUMINESCE", 1), _card("A", 0)], "draw": []}}
    with pytest.raises(ValueError, match="innate_min_draw is out of range"):
        eval_suite._dealt_uids(over, [("A", 0)])
    # Two complete readings stay a refusal.
    twins = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [_card("LUMINESCE", 2), _card("A", 1), _card("A", 0)],
        "draw": []}}
    with pytest.raises(ValueError, match="identities differ"):
        eval_suite._dealt_uids(twins, [("A", 0), ("A", 0)])


# ---------------------------------------------------------------------------
# 2. A capture that stops at a player turn start
# ---------------------------------------------------------------------------


class _LiveSession:
    """A fight Rust never ends: a potion and an end turn, both no-ops."""

    def __init__(self):
        self.state = {"player": {"turn": 1, "hp": 40}, "monsters": [],
                      "piles": {"hand": []}}

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        if request["cmd"] == "legal":
            return {"actions": [{"kind": "potion", "slot": 0},
                                {"kind": "end"}]}
        if request["cmd"] == "apply":
            if request["action"]["kind"] == "end":
                self.state = dict(self.state, player=dict(
                    self.state["player"],
                    turn=self.state["player"]["turn"] + 1))
            return {"digest": "d", "native_checkpoints": []}
        if request["cmd"] == "project":
            return {"state": self.state}
        raise AssertionError(request)


_POTION = {"event_type": "GameAction", "action": {
    "type": "NetUsePotionAction", "potion_index": 0}}
_END = {"event_type": "GameAction", "action": {
    "type": "NetEndPlayerTurnAction", "turn_number": 1}}
_READY = {"event_type": "GameAction", "action": {
    "type": "NetReadyToBeginEnemyTurnAction"}}
_POTION_DONE = "finished action execution UsePotionAction 1  index: 0 target: "


def _checksum(context, enemy_hp=30, player_hp=40, powers=()):
    return {"checksum": {"id": 1, "value": 0}, "context": context,
            "full_state": {"marker": context, "creatures": [
                {"monster_id": None, "player_id": 7, "current_hp": player_hp,
                 "max_hp": 50, "block": 0,
                 "powers": [{"id": name, "amount": amount}
                            for name, amount in powers]},
                {"monster_id": "QUEEN", "player_id": None,
                 "current_hp": enemy_hp, "max_hp": 419, "block": 0,
                 "powers": []}]}}


def _turn_boundary(final_context=eval_suite.PLAYER_TURN_START_CONTEXT, **final):
    """potion, end turn, ready marker; then the native turn-boundary records."""
    return ([_POTION, _END, _READY], [
        _checksum(_POTION_DONE),
        _checksum("After player turn phase one end"),
        _checksum("after player turn phase two end"),
        _checksum("After enemy turn start"),
        _checksum("After enemy turn end"),
        _checksum(final_context, **final)])


@pytest.fixture
def compared(monkeypatch):
    """The real `_CensusChecks`; every state comparison agrees and is logged."""
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    seen = []
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: seen.append(
                            (native["marker"], state["player"]["turn"])))
    monkeypatch.setattr(eval_suite, "power_mismatch_rows",
                        lambda state, native: [])
    return seen


def _replay(events, checksums):
    root = {"player": {"turn": 1, "hp": 40}, "monsters": [],
            "piles": {"hand": []}}
    return eval_suite.replay_recorded_line(
        _LiveSession(), root, {"events": events, "checksums": checksums})


def test_a_capture_that_stops_at_a_player_turn_start_is_truncated(compared):
    events, checksums = _turn_boundary()
    with pytest.raises(eval_suite.CaptureTruncated) as caught:
        _replay(events, checksums)
    exc = caught.value
    assert exc.shape == eval_suite.TRUNCATED_AT_PLAYER_TURN_START
    assert (exc.check, exc.step) == ("capture_truncated", 2)
    # The final turn-start checksum WAS compared, against Rust's state after
    # the end turn (turn 2), not assumed.
    assert compared == [(_POTION_DONE, 1),
                        (eval_suite.PLAYER_TURN_START_CONTEXT, 2)]

    # Truncated is uncertified: no lockstep verdict, and out of `eligible`.
    row = {"checksummed": True, "opening_checkpoint": "match"}
    row.update(eval_suite.truncated_row_fields(row, exc))
    assert row["human"] == "truncated"
    assert row["truncation_shape"] == "at_player_turn_start"
    assert "lockstep" not in row
    assert eval_suite.unusable_reason(row) == eval_suite.UNUSABLE_TRUNCATED
    summary = eval_suite.census_summary(
        [dict(row, stage="rooted", rust="admitted")])
    assert (summary["certified"], summary["eligible"]) == (0, 0)
    assert summary["capture_truncated_shapes"] == {"at_player_turn_start": 1}


def test_a_turn_start_checksum_rust_disagrees_with_is_not_truncated(
        monkeypatch, compared):
    """The disagreement the class must not hide: the game's state at the
    next turn start is not Rust's. The row stays `combat_not_complete` and
    says what was compared."""
    def check(state, native):
        if native["marker"] == eval_suite.PLAYER_TURN_START_CONTEXT:
            raise ValueError("recorded replay differs from native player state")
    monkeypatch.setattr(eval_suite, "check_native_checkpoint", check)
    events, checksums = _turn_boundary()
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay(events, checksums)
    exc = caught.value
    assert not isinstance(exc, eval_suite.CaptureTruncated)
    assert exc.check == "combat_not_complete"
    assert "final turn-start checksum differs" in exc.detail
    assert "native player state" in exc.detail


def test_a_dead_side_at_the_final_turn_start_is_not_truncated(compared):
    """Agreement is not enough: the capture must show the combat live."""
    for final in ({"player_hp": 0}, {"enemy_hp": 0}):
        events, checksums = _turn_boundary(**final)
        with pytest.raises(eval_suite.LineDiverged) as caught:
            _replay(events, checksums)
        assert not isinstance(caught.value, eval_suite.CaptureTruncated)
        assert caught.value.check == "combat_not_complete"


def test_a_capture_that_stops_inside_the_enemy_turn_is_not_claimed(compared):
    """`After enemy turn start` is also how a LOST fight's capture ends (the
    player dies inside the enemy turn), with the player alive in it. Rust
    surviving that turn is then a real disagreement, so nothing is claimed."""
    events, checksums = _turn_boundary()
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay(events, checksums[:4])
    assert not isinstance(caught.value, eval_suite.CaptureTruncated)
    assert caught.value.check == "combat_not_complete"
    assert "turn-start" not in caught.value.detail
    # Only the completed action was compared: no turn-boundary record is.
    assert compared == [(_POTION_DONE, 1)]


def test_the_turn_start_shape_needs_the_line_to_end_on_the_end_turn(compared):
    # A capture whose last checksum is a turn start but whose line ends on a
    # completed action is the #3277 rule's to read, and it claims nothing.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay([_POTION], [_checksum(_POTION_DONE), _checksum(
            eval_suite.PLAYER_TURN_START_CONTEXT)])
    assert caught.value.check == "combat_not_complete"
    # An unconsumed completed-action checkpoint: the game did something the
    # line does not hold.
    events, checksums = _turn_boundary()
    checksums.insert(1, _checksum(_POTION_DONE))
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay(events, checksums)
    assert caught.value.check == "combat_not_complete"


# ---------------------------------------------------------------------------
# 3. Player powers: compared, or named as not compared
# ---------------------------------------------------------------------------


def _native(*powers):
    return {"creatures": [{"player_id": 1, "powers": [
        {"id": name, "amount": amount} for name, amount in powers]}]}


def _rows(player, native):
    return [m.as_row() for m in
            replay.native_player_power_mismatches({"player": player}, native)]


def test_foregone_conclusion_is_compared_as_rusts_foregone_slot():
    assert replay.NATIVE_PLAYER_POWER_FIELDS[
        "FOREGONE_CONCLUSION_POWER"] == "foregone"
    native = _native(("FOREGONE_CONCLUSION_POWER", 2))
    assert _rows({"foregone": 2}, native) == []
    # Really compared: either side's amount moving is an `amount` row.
    assert _rows({"foregone": 3}, native) == [
        {"class": "amount", "power": "FOREGONE_CONCLUSION_POWER",
         "native": 2, "rust": 3}]
    assert _rows({}, native) == [
        {"class": "amount", "power": "FOREGONE_CONCLUSION_POWER",
         "native": 2, "rust": 0}]
    assert _rows({"foregone": 2}, _native()) == [
        {"class": "amount", "power": "FOREGONE_CONCLUSION_POWER",
         "native": 0, "rust": 2}]


def test_post_combat_only_powers_are_named_not_compared():
    assert replay.NATIVE_POST_COMBAT_ONLY_PLAYER_POWERS == {
        "FORBIDDEN_GRIMOIRE_POWER", "ROYALTIES_POWER"}
    native = _native(("ROYALTIES_POWER", 30), ("FORBIDDEN_GRIMOIRE_POWER", 1),
                     ("WEAK_POWER", 2))
    # No row for either: Rust has no value to disagree with.
    assert _rows({"player_weak": 2}, native) == []
    assert replay.native_player_powers_not_compared(native) == [
        "FORBIDDEN_GRIMOIRE_POWER", "ROYALTIES_POWER"]
    assert replay.native_player_powers_not_compared(_native(("WEAK_POWER", 2))) == []
    # They hide nothing beside them: the Weak row still disagrees.
    assert _rows({"player_weak": 1}, native) == [
        {"class": "amount", "power": "WEAK_POWER", "native": 2, "rust": 1}]
    # A power nobody has read stays `power_not_projected`, never left out.
    assert _rows({}, _native(("NOT_A_REAL_POWER", 1))) == [
        {"class": "power_not_projected", "power": "NOT_A_REAL_POWER",
         "native": 1, "rust": None}]
    assert replay.native_player_powers_not_compared(
        _native(("NOT_A_REAL_POWER", 1))) == []
    with pytest.raises(ValueError, match="one player power list"):
        replay.native_player_powers_not_compared({"creatures": []})
    # The helper the census uses never raises on a malformed checkpoint.
    assert eval_suite.powers_not_compared({"creatures": []}) == []


def test_the_sets_of_compared_and_not_compared_powers_are_disjoint():
    compared_ids = (set(replay.NATIVE_PLAYER_POWER_FIELDS)
                    | set(replay.NATIVE_PLAYER_POWER_PROJECTIONS)
                    | set(replay._NATIVE_TEMPORARY_POWER)
                    | set(replay.NATIVE_INSTANCED_PLAYER_POWERS))
    assert not compared_ids & replay.NATIVE_POST_COMBAT_ONLY_PLAYER_POWERS


def test_power_check_fields_name_what_was_left_out():
    row = {"checksummed": True}
    assert eval_suite.power_check_fields(row, None, ["ROYALTIES_POWER"]) == {
        "power_check": "match",
        "player_powers_not_compared": ["ROYALTIES_POWER"]}
    mismatch = [{"class": "amount", "power": "WEAK_POWER",
                 "native": 2, "rust": 1}]
    fields = eval_suite.power_check_fields(
        dict(row, opening_powers_not_compared=["FORBIDDEN_GRIMOIRE_POWER"]),
        {"step": 4, "mismatches": mismatch}, ["ROYALTIES_POWER"])
    assert fields["power_check"] == "mismatch"
    assert fields["player_powers_not_compared"] == [
        "FORBIDDEN_GRIMOIRE_POWER", "ROYALTIES_POWER"]
    # Nothing left out: the key is absent, as before.
    assert eval_suite.power_check_fields(row, None, []) == {
        "power_check": "match"}


def test_a_power_left_out_does_not_end_the_comparison(monkeypatch):
    """Before #3764 the first checkpoint with any power row was the only one
    kept, so a Forbidden Grimoire played early masked every later row."""
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: None)
    checks = eval_suite._CensusChecks({"checksums": [
        _checksum(_POTION_DONE, powers=[("FORBIDDEN_GRIMOIRE_POWER", 1)]),
        _checksum(_POTION_DONE, powers=[("FORBIDDEN_GRIMOIRE_POWER", 1),
                                        ("WEAK_POWER", 2)])]})
    for _ in range(2):
        checks.start(_POTION)
        checks.completed({"player": {"player_weak": 0}})
    assert checks.not_compared == {"FORBIDDEN_GRIMOIRE_POWER"}
    assert checks.power_mismatch == [
        {"class": "amount", "power": "WEAK_POWER", "native": 2, "rust": 0}]
    assert checks.validated == 2


def test_the_line_and_the_census_carry_the_powers_left_out(monkeypatch):
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: None)
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay([_POTION], [_checksum(
            _POTION_DONE, enemy_hp=0, powers=[("ROYALTIES_POWER", 30)])])
    assert caught.value.check == "combat_not_complete"
    assert caught.value.powers_not_compared == ["ROYALTIES_POWER"]
    assert caught.value.power_mismatch is None

    rows = [
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "lockstep_ok", "power_check": "match",
         "player_powers_not_compared": ["FORBIDDEN_GRIMOIRE_POWER",
                                        "ROYALTIES_POWER"]},
        {"stage": "rooted", "rust": "admitted", "human": "diverged",
         "power_check": "match",
         "player_powers_not_compared": ["FORBIDDEN_GRIMOIRE_POWER"]},
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "lockstep_ok", "power_check": "match"},
    ]
    summary = eval_suite.census_summary(rows)
    assert summary["powers_not_compared"] == {
        "FORBIDDEN_GRIMOIRE_POWER": 2, "ROYALTIES_POWER": 1}
    assert summary["lockstep_ok_with_powers_not_compared"] == 1
    assert summary["lockstep_ok_with_power_mismatch"] == 0
    markdown = eval_suite.census_markdown({
        "captures": "x", "build": "v0.111.0", "summary": summary})
    assert "| certified, with a player power not compared (#3764) | 1 |" in markdown
    assert "| `FORBIDDEN_GRIMOIRE_POWER` | 2 |" in markdown

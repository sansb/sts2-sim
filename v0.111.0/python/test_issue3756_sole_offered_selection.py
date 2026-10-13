"""#3756: a selecting play recorded with no choice.

A card whose body selects from Hand opens no choice when the candidates do
not exceed its minimum (`rust_replay.sole_offered_selection` carries the IL),
so the capture holds the play and no `PlayerChoice`. Rust's `legal` still
spells the play with its one `selection`. The rule takes that selection only
when it is one physical card, and the census's
`eval_suite.replay_recorded_line` is held to it here over a scripted engine
session. The Rust port's rule is tested in
`engine/src/recorded.rs` and `engine/tests/recorded_line.rs`.

Every fixture is synthetic.
"""
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "engine" / "tools"))

import eval_suite  # noqa: E402
import rust_replay  # noqa: E402

PACT, DEFEND, TWIN = 6, 5, 4
PACT_CHECKPOINT = ("finished action execution PlayCardAction card: "
                   "CARD.BURNING_PACT (1) index: 6 targetid: ")


def _hand(*cards):
    return {"piles": {"hand": [{"id": name, "uid": uid, "upgrade": 0}
                               for name, uid in cards]}}


def test_the_one_offered_selection_of_one_physical_card_is_taken():
    before = _hand(("BURNING_PACT", PACT), ("DEFEND_IRONCLAD", DEFEND))
    legal = [{"kind": "play", "uid": PACT, "selection": DEFEND},
             {"kind": "play", "uid": DEFEND}, {"kind": "end"}]
    wire = {"kind": "play", "uid": PACT}
    assert rust_replay.sole_offered_selection(before, legal, wire) == {
        "kind": "play", "uid": PACT, "selection": DEFEND}
    # The census's name for the rule is the same function.
    assert eval_suite._sole_offered_selection(before, legal, wire) == {
        "kind": "play", "uid": PACT, "selection": DEFEND}
    # A played copy `legal` does not list keeps its recorded uid.
    twins = _hand(("BURNING_PACT", PACT), ("BURNING_PACT", 7),
                  ("DEFEND_IRONCLAD", DEFEND))
    assert rust_replay.sole_offered_selection(
        twins, legal[:1], {"kind": "play", "uid": 7}) == {
            "kind": "play", "uid": 7, "selection": DEFEND}


def test_anything_else_stays_not_legal():
    before = _hand(("BURNING_PACT", PACT), ("DEFEND_IRONCLAD", DEFEND))
    legal = [{"kind": "play", "uid": PACT, "selection": DEFEND},
             {"kind": "play", "uid": DEFEND}]
    wire = {"kind": "play", "uid": PACT}
    sole = rust_replay.sole_offered_selection
    # Two offered selections for the play.
    assert sole(before, legal + [
        {"kind": "play", "uid": PACT, "selection": 7}], wire) is None
    # The offered play differs in its target; no selecting play for the card.
    assert sole(before, legal, dict(wire, target=0)) is None
    assert sole(before, legal, {"kind": "play", "uid": DEFEND}) is None
    # One offered selection standing for two identical Hand cards: the game
    # would have asked which.
    twins = _hand(("BURNING_PACT", PACT), ("DEFEND_IRONCLAD", DEFEND),
                  ("DEFEND_IRONCLAD", TWIN))
    assert sole(twins, legal, wire) is None
    # A different upgrade is a different card, and the selection is taken.
    upgraded = _hand(("BURNING_PACT", PACT), ("DEFEND_IRONCLAD", DEFEND))
    upgraded["piles"]["hand"].append(
        {"id": "DEFEND_IRONCLAD", "uid": TWIN, "upgrade": 1})
    assert sole(upgraded, legal, wire) == dict(wire, selection=DEFEND)
    # The offered selection is not a Hand card: FromHand is the selector read.
    assert sole(before, [{"kind": "play", "uid": PACT, "selection": 9}],
                wire) is None


class _PactSession:
    """Rust holding a Burning Pact and `others` in Hand. `legal` offers the
    pact once per group of identical other cards, as the engine does."""

    def __init__(self, others):
        hand = [{"id": "BURNING_PACT", "uid": PACT, "upgrade": 0}] + [
            {"id": name, "uid": uid, "upgrade": 0} for name, uid in others]
        self.state = {"player": {"turn": 1, "hp": 40}, "monsters": [],
                      "piles": {"hand": hand}}
        self.applied = []

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        command = request["cmd"]
        if command == "legal":
            seen, actions = set(), []
            for card in self.state["piles"]["hand"]:
                if card["uid"] != PACT and card["id"] not in seen:
                    seen.add(card["id"])
                    actions.append({"kind": "play", "uid": PACT,
                                    "selection": card["uid"]})
            return {"actions": actions + [{"kind": "end"}]}
        if command == "apply":
            wire = request["action"]
            self.applied.append(wire)
            player = dict(self.state["player"])
            if wire["kind"] == "end":
                player["over"] = True
            self.state = dict(self.state, player=player)
            return {"digest": "d", "native_checkpoints": []}
        if command == "project":
            return {"state": self.state}
        raise AssertionError(request)


def _pact():
    return {"event_type": "GameAction", "action": {
        "type": "NetPlayCardAction", "combat_card_index": PACT,
        "card_id": "CARD.BURNING_PACT"}}


def _resume():
    return {"event_type": "ResumeAction", "action_id": 0}


def _choice(*indexes):
    return {"event_type": "PlayerChoice", "choice_id": 0, "result": {
        "type": "CombatCard", "combat_card_indexes": list(indexes)}}


def _end():
    return {"event_type": "GameAction", "action": {
        "type": "NetEndPlayerTurnAction", "turn_number": 1}}


def _replay(events):
    return {"version": "v0.111.0", "events": events, "checksums": [
        {"checksum": {"id": 0, "value": 0}, "context": PACT_CHECKPOINT,
         "full_state": {"creatures": []}}]}


@pytest.fixture
def census(monkeypatch):
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: None)
    monkeypatch.setattr(eval_suite, "power_mismatch_rows",
                        lambda state, native: [])

    def run(others, events):
        session = _PactSession(others)
        line = eval_suite.replay_recorded_line(
            session, session.state, _replay(events))
        return line["actions"], session
    return run


@pytest.fixture
def resolver(census):
    return census


def test_a_play_with_no_recorded_choice_takes_its_one_selection(resolver):
    """Burning Pact with one other card in Hand: play, resume, nothing
    between. The resolver applies the play with the one offered selection."""
    actions, session = resolver(
        [("DEFEND_IRONCLAD", DEFEND)], [_pact(), _resume(), _end()])
    assert session.applied[0] == {
        "kind": "play", "uid": PACT, "selection": DEFEND}
    assert [a["kind"] for a in actions] == ["play", "end"]
    assert actions[0]["selection"] == DEFEND


def test_a_recorded_choice_still_resolves_the_play(resolver):
    actions, session = resolver(
        [("DEFEND_IRONCLAD", DEFEND), ("BASH", TWIN)],
        [_pact(), _choice(TWIN), _resume(), _end()])
    assert session.applied[0] == {
        "kind": "play", "uid": PACT, "selection": TWIN}


@pytest.mark.parametrize("others, events", [
    # Two different candidates and no recorded choice.
    ([("DEFEND_IRONCLAD", DEFEND), ("BASH", TWIN)],
     [_pact(), _resume(), _end()]),
    # Two identical candidates: one offered selection, two physical cards.
    ([("DEFEND_IRONCLAD", DEFEND), ("DEFEND_IRONCLAD", TWIN)],
     [_pact(), _resume(), _end()]),
    # A recorded choice the play cannot take is not replaced by the sole
    # selection.
    ([("DEFEND_IRONCLAD", DEFEND)], [_pact(), _choice(DEFEND, TWIN), _end()]),
])
def test_nothing_is_guessed(resolver, others, events):
    with pytest.raises((eval_suite.LineDiverged, ValueError)) as refused:
        resolver(others, events)
    assert "not legal" in str(refused.value)

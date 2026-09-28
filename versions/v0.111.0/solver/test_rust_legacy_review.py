"""The legacy (`.run`-only) review on the Rust root (#2827 item C2).

`rust_legacy_review` roots a `.run` fight with `sts-sim entry --facts
--opening` from the parsed facts plus the `sts-sim run-counters` prediction,
then searches and replays it in Rust. These tests pin the facts projection,
the counter-source precedence, the unknown-stream (placeholder) contract, the
document shape and refusal names, and that no Python simulator entry point is
called. The equivalence against the retired Python producer is measured in
the invariant walk, not here.
"""

import copy
import json
import pathlib

import pytest

import relay_parser
import review_summary as phase1
import review_summary_v2 as v2
import rust_legacy_review as legacy
import rust_review
import sts2_rng

HERE = pathlib.Path(__file__).parent
RUN = HERE / "testdata" / "6P96T755CNZ3.run"
PENDING_RUN = HERE / "testdata" / "85920V7XQFSN.run"
BINARY = HERE.parent / "rust" / "target" / "release" / "sts-sim"


def _prediction(**overrides):
    rows = {name: {"value": 0, "status": "exact", "caveats": []}
            for name, _key in legacy.COMBAT_STREAMS}
    rows["monster_ai"] = {"value": 0, "status": "assumed", "caveats": []}
    rows["combat_potion_generation"] = {
        "value": None, "status": "not_predicted", "caveats": []}
    rows["shuffle"] = {"value": 27, "status": "baseline",
                       "caveats": ["fight 0 adds cards"]}
    rows.update(overrides)
    return {"schema": "sts-sim-run-counters-v1", "counters": rows,
            "caveats": ["fight 0 adds cards"]}


def _entry(**changes):
    entry = {
        "encounter_id": "ENCOUNTER.SLIMES_WEAK", "node_index": 1,
        "node_type": "monster",
        "deck_entering": [
            {"id": "CARD.STRIKE_IRONCLAD", "upgrade_level": 0,
             "floor_added": 1, "enchantment": None},
            {"id": "CARD.BASH", "upgrade_level": 1, "floor_added": 1,
             "enchantment": "ENCHANTMENT.SHARP"},
            {"id": "CARD.THE_SCYTHE", "upgrade_level": 0, "floor_added": 3,
             "enchantment": None, "props": {"ints": []}},
            {"id": "CARD.FEED", "upgrade_level": 0, "floor_added": 4,
             "enchantment": None, "props": {"ints": []}},
        ],
        "relics_entering": ["RELIC.BURNING_BLOOD", "RELIC.HAPPY_FLOWER"],
        "potions_entering": [], "hp_entering": 70, "max_hp_entering": 80,
        "gold_entering": 99, "relic_counters": {"RELIC.HAPPY_FLOWER": 1},
        "tea_set_charged": None, "fake_tea_set_charged": None,
        "fur_coat_active": False, "entry_ambiguous": False,
        "potion_entry_ambiguous": False,
    }
    entry.update(changes)
    return entry


def _run():
    return relay_parser.RunSummary(
        seed="6P96T755CNZ3", build_id="v0.111.0", schema_version=8,
        character="CHARACTER.IRONCLAD", ascension=7, win=True, killed_by=None)


def _resolved(prediction=None, **kw):
    return legacy.resolve_counters(prediction or _prediction(),
                                   kw.get("overrides", {}),
                                   kw.get("observed", {}))


# --- counter sources -------------------------------------------------------

def test_counter_sources_rank_override_then_capture_then_prediction():
    resolved = _resolved(overrides={"combat_targets": 4},
                         observed={"combat_targets": 9, "monster_ai": 3})
    assert resolved["combat_targets"] == {
        "value": 4, "source": "override", "status": "exact"}
    assert resolved["monster_ai"] == {
        "value": 3, "source": "capture", "status": "exact"}
    assert resolved["shuffle"] == {
        "value": 27, "source": "prediction", "status": "baseline"}
    assert resolved["niche"]["source"] == "prediction"


@pytest.mark.parametrize("status", ["unknown", "assumed", "not_predicted"])
def test_every_unclaimed_status_enters_at_a_placeholder(status):
    value = None if status != "assumed" else 0
    resolved = _resolved(_prediction(
        combat_card_generation={"value": value, "status": status,
                                "caveats": []}))
    assert resolved["combat_card_generation"] == {
        "value": None, "source": "placeholder", "status": status}


def test_an_exact_first_combat_monster_ai_is_used():
    resolved = _resolved(_prediction(
        monster_ai={"value": 0, "status": "exact", "caveats": []}))
    assert resolved["monster_ai"]["source"] == "prediction"
    assert resolved["monster_ai"]["value"] == 0


def test_a_missing_stream_row_refuses_by_name():
    prediction = _prediction()
    del prediction["counters"]["niche"]
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        _resolved(prediction)
    assert refused.value.reason == "run_counters_incomplete"


def test_capture_counters_read_the_capture_spelling_and_skip_malformed():
    replay = {"run": {"rng": {"counters": {
        "Shuffle": 12, "CombatOrbs": 2, "MonsterAi": 5, "Niche": -1,
        "CombatTargets": True, "UpFront": 7}}}}
    assert legacy.capture_counters(replay) == {
        "shuffle": 12, "combat_orbs": 2, "monster_ai": 5}
    assert legacy.capture_counters(None) == {}
    assert legacy.capture_counters({"run": {}}) == {}


# --- the facts document ----------------------------------------------------

def test_the_facts_are_the_fights_run_facts_in_the_save_vocabulary():
    counters = _resolved()
    facts = legacy.entry_facts(_run(), _entry(), counters,
                               fully_unlocked_card_pool=True)
    assert facts["schema"] == "sts-sim-entry-v1"
    assert facts["game_build"] == "v0.111.0"
    assert facts["character"] == "CHARACTER.IRONCLAD"
    entry = facts["entry"]
    assert entry["ascension"] == 7
    assert entry["deck_entering"] == [
        {"id": "CARD.STRIKE_IRONCLAD", "upgrade_level": 0},
        # `start_combat` read a missing amount as 1.
        {"id": "CARD.BASH", "upgrade_level": 1,
         "enchantment": "ENCHANTMENT.SHARP", "enchant_amount": 1},
        # Props ride on the three per-instance cards only.
        {"id": "CARD.THE_SCYTHE", "upgrade_level": 0, "props": {"ints": []}},
        {"id": "CARD.FEED", "upgrade_level": 0},
    ]
    assert entry["relic_counters"] == {"RELIC.HAPPY_FLOWER": 1}
    assert entry["potions_entering"] == []
    assert entry["potion_slots_entering"] == []
    assert entry["max_potion_slot_count"] is None
    assert entry["fur_coat_active"] is None
    assert entry["relics_entering_dispatch_ordered"] is False
    assert entry["entry_ambiguous"] is False
    assert facts["unlocks"] == {
        "unlocked_card_pool_epochs": None,
        "fully_unlocked_card_pool": True,
        "fully_unlocked_potion_pool": None,
    }
    assert legacy.entry_facts(
        _run(), _entry(relic_counters={}), counters,
        fully_unlocked_card_pool=False)["entry"]["relic_counters"] is None


def test_stream_words_are_the_seeded_state_and_placeholders_sit_at_zero():
    counters = _resolved()
    facts = legacy.entry_facts(_run(), _entry(), counters,
                               fully_unlocked_card_pool=False)
    assert facts["counters"]["shuffle"] == 27
    for name in ("monster_ai", "combat_potion_generation"):
        assert facts["counters"][name] == legacy.PLACEHOLDER_COUNTER
    run_set = sts2_rng.RunRngSet(
        "6P96T755CNZ3", {"Shuffle": 27}, build="v0.111.0")
    words = run_set.rngs["Shuffle"]._random
    assert facts["streams"]["shuffle"] == {
        "counter": 27, "s0": words.s0, "s1": words.s1, "s2": words.s2,
        "s3": words.s3}
    assert set(facts["streams"]) == {name for name, _ in legacy.COMBAT_STREAMS}


@pytest.mark.parametrize("change, reason", [
    ({"entry_ambiguous": True}, "entry_ambiguous"),
    ({"potion_entry_ambiguous": True}, "potion_entry_ambiguous"),
    ({"potions_entering": ["POTION.FIRE_POTION"]},
     "potion_belt_slots_unrecorded"),
    ({"relics_entering": ["RELIC.FUR_COAT"]},
     "fur_coat_membership_unrepresentable"),
    ({"deck_entering": [{"id": "CARD.THE_SCYTHE", "upgrade_level": 0,
                         "props": {}, "props_ambiguous": True}]},
     "entry_props_ambiguous"),
    ({"deck_entering": [{"id": "CARD.BASH", "upgrade_level": 1,
                         "upgrade_ambiguous": True}]},
     "copy_assignment_ambiguous"),
])
def test_each_unrepresentable_run_fact_refuses_by_name(change, reason):
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        legacy.entry_facts(_run(), _entry(**change), _resolved(),
                           fully_unlocked_card_pool=True)
    assert refused.value.reason == reason


# --- the placeholder contract ----------------------------------------------

def _state(**counters):
    return {"rng": {key: {"counter": value, "words": [1, 2, 3, 4]}
                    for key, value in counters.items()}}


def test_the_placeholder_check_passes_only_an_undrawn_stream():
    check = legacy.placeholder_check({"monster_ai": 0, "combat_orbs": 0})
    assert check.withholds is legacy.UnknownStreamConsumed
    check(_state(ai=0, combat_orbs=0, rng=40))
    for state in (_state(ai=1, combat_orbs=0), _state(combat_orbs=0)):
        with pytest.raises(legacy.UnknownStreamConsumed) as consumed:
            check(state)
        assert consumed.value.stream == "monster_ai"


def test_build_root_refuses_an_opening_that_draws_a_placeholder(monkeypatch):
    monkeypatch.setattr(legacy, "rust_opening",
                        lambda binary, facts: _state(ai=1, potion_generation=0))
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        legacy.build_root(pathlib.Path("sts-sim"), _run(), _entry(),
                          _prediction(), overrides={}, replay=None,
                          fully_unlocked_card_pool=True)
    assert refused.value.reason == "monster_ai_counter_unknown"
    assert refused.value.details == {"stream": "monster_ai",
                                     "status": "assumed"}


def test_search_outcomes_withholds_a_line_that_draws_a_placeholder(
        monkeypatch):
    results = iter([
        {"entry_digest": "d", "best": {"actions": [], "combat_hp": 50,
                                       "turn": 3, "won": True,
                                       "final_digest": "a"}},
        {"entry_digest": "d", "best": {"actions": [], "combat_hp": 40,
                                       "turn": 4, "won": True,
                                       "final_digest": "b"}},
    ])
    monkeypatch.setattr(rust_review.subprocess, "run",
                        lambda *a, **k: type("Done", (), {
                            "returncode": 0,
                            "stdout": json.dumps(next(results))})())
    monkeypatch.setattr(rust_review.canonical_document, "differential_digest",
                        lambda _entry: "d")

    def replay(binary, entry, best, final_check=None):
        if best["final_digest"] == "a":
            final_check(_state(ai=3))
        final_check(_state(ai=0))
        return [{"turn": 1, "actions": []}]

    monkeypatch.setattr(rust_review, "replay_line", replay)
    withheld = []
    outcomes, diagnostics = rust_review.search_outcomes(
        "sts-sim", {"player": {"hp": 70}}, phase1.ReviewConfig(),
        final_check=legacy.placeholder_check({"monster_ai": 0}),
        withheld=withheld)
    assert [o["line_id"] for o in outcomes] == ["random"]
    assert len(diagnostics) == 2
    assert withheld == [{"line_id": "uct", "stream": "monster_ai",
                         "reason": str(legacy.UnknownStreamConsumed(
                             "monster_ai"))[:300]}]


# --- generate --------------------------------------------------------------

@pytest.fixture
def fake_rust(monkeypatch, tmp_path):
    """Replace every Rust call with a recorded answer; the rest is real."""
    binary = tmp_path / "sts-sim"
    binary.write_bytes(b"fake")
    binary.chmod(0o755)
    calls = {"placeholders": {"monster_ai": 0}, "withheld": [],
             "outcomes": None}
    monkeypatch.setattr(legacy, "history_prediction",
                        lambda *a: _prediction())

    def build_root(binary, run, entry, prediction, **kw):
        calls["build_root"] = kw
        counters = legacy.resolve_counters(prediction, kw["overrides"], {})
        return {"player": {"hp": 70}, "rng": {}}, counters, calls["placeholders"]

    monkeypatch.setattr(legacy, "build_root", build_root)

    def search(binary, entry, config, *, final_check, withheld):
        withheld.extend(calls["withheld"])
        return calls["outcomes"] if calls["outcomes"] is not None else [{
            "line_id": "uct", "won": True, "final_hp": 50, "exact": False,
            "claim": {"kind": "achieved", "display": "achieved (lower bound)"},
            "line": [{"turn": 1, "actions": []}]}], [{"method": "uct"}]

    monkeypatch.setattr(rust_review, "search_outcomes", search)
    monkeypatch.setattr(rust_review.canonical_document, "differential_digest",
                        lambda _entry: "digest")
    calls["config"] = phase1.ReviewConfig(
        horizon=20, actual_deadline=0.5, rust_exact_solver_binary=binary)
    return calls


def _generate(calls, fight=0, **config):
    return legacy.generate(RUN, fight, _replace(calls["config"], **config))


def _replace(config, **changes):
    import dataclasses
    return dataclasses.replace(config, **changes)


def test_a_card_is_the_rust_review_shape_plus_the_legacy_disclosures(fake_rust):
    document = _generate(fake_rust, targets_counter=3)
    assert document["status"] == "ok"
    assert document["schema_version"] == v2.SCHEMA_VERSION
    assert document["fight"] == {
        "run_file": RUN.name, "fight_index": 0, "run_seed": "6P96T755CNZ3",
        "encounter_id": "ENCOUNTER.SLIMES_WEAK", "node_type": "monster",
        "floor": 2}
    assert document["best_actual_seed"]["line_id"] == "uct"
    assert document["alternative_actual_seeds"] == []
    assert document["degraded_reasons"] == []
    metadata = document["metadata"]
    assert metadata["entry_state_source"] == "run_history_prediction"
    assert metadata["opening_adapter"] == "rust_entry_facts_opening"
    assert metadata["search_engine"] == metadata["replay_engine"] == "rust"
    assert metadata["entry_caveats"] == ["fight 0 adds cards"]
    assert metadata["unverified_streams"] == ["monster_ai"]
    assert metadata["assumed_fully_unlocked"] is True
    assert metadata["counter_overrides"] == {"combat_targets": 3}
    assert metadata["entry_counters"]["combat_targets"]["source"] == "override"
    assert fake_rust["build_root"]["overrides"] == {"combat_targets": 3}
    assert fake_rust["build_root"]["fully_unlocked_card_pool"] is True


def test_every_witness_drawing_a_placeholder_refuses_naming_it(fake_rust):
    fake_rust["outcomes"] = []
    fake_rust["withheld"] = [{"line_id": "uct", "stream": "monster_ai",
                              "reason": "drawn"}]
    document = _generate(fake_rust)
    assert document["status"] == "refused"
    assert document["refusal"]["reason"] == "simulation_refusal"
    assert document["refusal"]["details"]["cause"] == \
        "monster_ai_counter_unknown"
    assert document["refusal"]["details"]["withheld_lines"] == \
        fake_rust["withheld"]


def test_an_opening_that_draws_a_placeholder_refuses_in_the_old_vocabulary(
        fake_rust, monkeypatch):
    def build_root(*_a, **_k):
        raise legacy.LegacyRootRefusal(
            "combat_orbs_counter_unknown", "drawn", stream="combat_orbs",
            status="unknown")

    monkeypatch.setattr(legacy, "build_root", build_root)
    details = _generate(fake_rust)["refusal"]["details"]
    assert details["cause"] == "combat_orb_generation_counter_unknown"
    assert details["prediction_status"] == "unknown"


def test_a_multiset_only_entry_refuses_where_the_benchmark_used_to_run(
        fake_rust, monkeypatch):
    monkeypatch.setattr(legacy, "fight_entry", lambda *a: (
        {}, ["upgrade_copy_assignment_ambiguous"]))
    refusal = _generate(fake_rust)["refusal"]
    assert refusal["details"]["cause"] == "benchmark_only_unavailable"
    assert refusal["details"]["degraded_reasons"] == [
        "upgrade_copy_assignment_ambiguous"]


def test_a_search_without_a_witness_refuses_by_name(fake_rust):
    fake_rust["outcomes"] = []
    assert _generate(fake_rust)["refusal"]["details"]["cause"] == \
        "search_budget_exhausted"


def test_a_capture_that_does_not_replay_refuses_and_writes_the_status(
        fake_rust, monkeypatch, tmp_path):
    def witness(*_args):
        raise ValueError("recorded replay differs from native RNG")

    monkeypatch.setattr(rust_review, "recorded_witness", witness)
    status = tmp_path / "line-status.json"
    document = _generate(fake_rust, recorded_replay={"version": "v0.111.0"},
                         line_replay_status_path=status)
    assert document["refusal"]["reason"] == "recorded_line_replay_failed"
    assert json.loads(status.read_text()) == {
        "turn": None, "check": "rust_recorded_replay:ValueError"}


def test_a_capture_that_replays_publishes_the_players_line(
        fake_rust, monkeypatch):
    monkeypatch.setattr(rust_review, "recorded_witness", lambda *a: {
        "final_digest": "f", "native_checkpoints": 4})
    seen = []
    monkeypatch.setattr(
        rust_review, "replay_line",
        lambda binary, root, witness, check: seen.append(check) or [
            {"turn": 1, "actions": []}])
    document = _generate(fake_rust, recorded_replay={"version": "v0.111.0"})
    assert document["recorded_line"] == [{"turn": 1, "actions": []}]
    assert document["metadata"]["opening_validation"] == "recorded_replay"
    assert document["metadata"]["recorded_replay"]["native_checkpoints"] == 4
    assert seen[0].withholds is legacy.UnknownStreamConsumed


def test_admission_and_range_refusals_are_the_legacy_documents(fake_rust):
    pending = legacy.generate(PENDING_RUN, 0, fake_rust["config"])
    assert pending["status"] == "deferred"
    assert pending["deferral"]["reason"] == "game_build_pending_admission"
    out_of_range = _generate(fake_rust, fight=99)
    assert out_of_range["refusal"]["reason"] == "fight_index_out_of_range"
    assert out_of_range["degraded_reasons"] == []
    assert out_of_range["metadata"]["entry_state_source"] == \
        "run_history_prediction"


def test_a_multiplayer_parse_refusal_is_a_structured_document(
        fake_rust, monkeypatch):
    def refuse(_path):
        raise NotImplementedError(".run contains 2 players")

    monkeypatch.setattr(legacy.relay_parser, "parse_run", refuse)
    document = _generate(fake_rust)
    assert document["status"] == "refused"
    assert document["refusal"]["reason"] == "simulation_refusal"
    assert "2 players" in document["refusal"]["message"]


def test_the_public_producers_route_reviews_to_rust(fake_rust):
    for module in (phase1, v2):
        document = module.generate_review_document(
            RUN, 0, config=fake_rust["config"])
        assert document["metadata"]["opening_adapter"] == \
            "rust_entry_facts_opening"


# --- no Python simulator on the path ---------------------------------------

# The frozen Python simulator and its adapters (#2827 item F deletes them).
_SIMULATOR_MODULES = frozenset({
    "combat_sim", "solve_fight", "content", "mcr_replay", "project_state",
    "python_fight_context", "infoset_sample"})


def _poison(monkeypatch):
    """Make the simulator unimportable, and the old `.run` counter path fatal.

    The simulator's modules are dropped from `sys.modules` for the test (the
    monkeypatch restores them) and a finder refuses any import of them, so
    the review cannot reach one directly or through a module it imports.
    `replay_fight.predict`, the Python shuffle-counter prediction, is still
    importable and simulator-free; the legacy review must not call it either.
    """
    import importlib.abc
    import sys

    import replay_fight

    class Refuse(importlib.abc.MetaPathFinder):
        def find_spec(self, name, path=None, target=None):
            if name.split(".")[0] in _SIMULATOR_MODULES:
                raise ImportError(f"the legacy review imported {name}")
            return None

    for name in list(sys.modules):
        if name.split(".")[0] in _SIMULATOR_MODULES:
            monkeypatch.delitem(sys.modules, name)
    monkeypatch.setattr(sys, "meta_path", [Refuse(), *sys.meta_path])

    def forbidden(*_args, **_kwargs):
        raise AssertionError("the legacy review called the Python predictor")

    monkeypatch.setattr(replay_fight, "predict", forbidden)


def test_generate_calls_no_python_simulator_entry_point(fake_rust, monkeypatch):
    _poison(monkeypatch)
    assert _generate(fake_rust)["status"] == "ok"


@pytest.mark.skipif(not BINARY.exists(),
                    reason="build versions/v0.111.0/rust/target/release/sts-sim")
def test_the_real_binary_reviews_the_fixture_run_without_python(monkeypatch):
    _poison(monkeypatch)
    config = phase1.ReviewConfig(horizon=20, actual_deadline=0.5,
                                 rust_exact_solver_binary=BINARY)
    documents = [legacy.generate(RUN, index, config) for index in range(9)]
    statuses = [document["status"] for document in documents]
    # Fights 0-2 enter with an empty belt; 3-8 carry potions, whose slots a
    # `.run` does not record.
    assert statuses[:3] == ["ok", "ok", "ok"], documents[:3]
    for document in documents[3:]:
        assert document["refusal"]["details"]["cause"] == \
            "potion_belt_slots_unrecorded"
    first = documents[0]
    # MonsterAi is exact entering the first combat, so only the unpredicted
    # potion stream enters at a placeholder.
    assert first["metadata"]["unverified_streams"] == [
        "combat_potion_generation"]
    for document in documents[:3]:
        best = document["best_actual_seed"]
        assert best["claim"] == {"kind": "achieved",
                                 "display": "achieved (lower bound)"}
        assert best["line"] and best["verification"]["rust_replay"] is True
    # The facts document itself is what Rust opens: a copy re-opens to the
    # same root.
    run = relay_parser.parse_run(str(RUN))
    entry, degraded = legacy.fight_entry(RUN, run, 0)
    assert degraded == []
    counters = legacy.resolve_counters(
        legacy.history_prediction(BINARY, RUN, 0, "v0.111.0"), {}, {})
    facts = legacy.entry_facts(run, entry, counters,
                               fully_unlocked_card_pool=True)
    assert legacy.rust_opening(BINARY, facts) == legacy.rust_opening(
        BINARY, copy.deepcopy(facts))


# --- refusal branches not reached above (review of #3009) -------------------

def _script(tmp_path, body):
    binary = tmp_path / "fake-sts-sim"
    binary.write_text("#!/bin/sh\n" + body)
    binary.chmod(0o755)
    return binary


@pytest.mark.parametrize("stdout, refusal_class", [
    # `entry --facts` refusing the document itself (top-level class).
    ({"refusal": {"detail": "facts.game_build is absent",
                  "kind": "entry_not_buildable", "site": "entry"},
      "refusal_class": "unknown_facts_field"}, "unknown_facts_field"),
    # A built entry whose opening refuses keeps the v1 schema.
    ({"schema": "sts-sim-entry-v1",
      "opening": {"built": False, "refusal_class": "opening_refused",
                  "detail": "unmodeled"}}, "opening_refused"),
])
def test_a_refused_opening_raises_its_class_by_name(tmp_path, stdout,
                                                     refusal_class):
    binary = _script(tmp_path, f"cat <<'JSON'\n{json.dumps(stdout)}\nJSON\n")
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        legacy.rust_opening(binary, {"schema": "sts-sim-entry-v1"})
    assert refused.value.reason == "rust_opening_refused"
    assert refused.value.details == {"refusal_class": refusal_class}


def test_an_opening_cli_failure_refuses_with_its_stderr(tmp_path):
    binary = _script(tmp_path, "echo 'refusal: entry: bad flag' >&2\nexit 2\n")
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        legacy.rust_opening(binary, {})
    assert refused.value.reason == "rust_opening_cli_failed"
    assert "bad flag" in refused.value.message


def test_a_run_counters_refusal_keeps_its_code(monkeypatch):
    import rust_run_counters

    def refuse(*_args):
        raise rust_run_counters.RunCountersRefusal(
            "fight_out_of_range", "refusal: run-counters: fight_out_of_range")

    monkeypatch.setattr(rust_run_counters, "predict", refuse)
    with pytest.raises(legacy.LegacyRootRefusal) as refused:
        legacy.history_prediction(pathlib.Path("sts-sim"), RUN, 0, "v0.111.0")
    assert refused.value.reason == "run_counters_fight_out_of_range"


def test_a_missing_binary_refuses_by_name(fake_rust, monkeypatch):
    import rust_exact_solve
    monkeypatch.setattr(rust_exact_solve, "default_binary", lambda: None)
    refusal = _generate(fake_rust, rust_exact_solver_binary=None)["refusal"]
    assert refusal["reason"] == "simulation_refusal"
    assert refusal["details"]["cause"] == "rust_binary_unavailable"


def test_a_search_whose_every_playout_refused_names_the_refusal(
        fake_rust, monkeypatch):
    monkeypatch.setattr(rust_review, "search_outcomes", lambda *a, **k: (
        [], [{"method": "uct", "first_refusal": {"kind": "unmodeled"}}]))
    refusal = _generate(fake_rust)["refusal"]
    assert refusal["details"]["cause"] == "rust_search_refused"
    assert "unmodeled" in refusal["message"]


def test_an_unreadable_run_is_an_invalid_run_document(fake_rust, tmp_path):
    broken = tmp_path / "broken.run"
    broken.write_text("{")
    document = legacy.generate(broken, 0, fake_rust["config"])
    assert document["status"] == "refused"
    assert document["refusal"]["reason"] == "invalid_run"
    assert document["metadata"]["entry_state_source"] == \
        "run_history_prediction"

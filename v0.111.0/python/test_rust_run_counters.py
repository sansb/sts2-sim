"""The run-history counter adapter (#2827 item C1).

`rust_run_counters` projects a parsed `.run` into the `sts-sim-run-history-v1`
document `sts-sim run-counters` reads. These tests pin the projection and that
the adapter imports no simulator; the Rust side's own fixture pins the answer.
"""

import pathlib
import subprocess
import sys

import pytest

import rust_run_counters as rrc

HERE = pathlib.Path(__file__).parent
RUN = HERE / "testdata" / "6P96T755CNZ3.run"
BINARY = HERE.parent / "engine" / "target" / "release" / "sts-sim"


def test_the_adapter_imports_no_simulator():
    probe = subprocess.run(
        [sys.executable, "-c",
         "import sys; import rust_run_counters; "
         "print(sorted(m for m in ('combat_sim', 'solve_fight', 'replay_fight', "
         "'review_summary', 'rust_review') if m in sys.modules))"],
        cwd=HERE, capture_output=True, text=True, check=True)
    assert probe.stdout.strip() == "[]"


def test_the_history_carries_the_raw_run_and_the_read_fields_only():
    history = rrc.load_history(RUN)
    assert history["schema"] == "sts-sim-run-history-v1"
    assert history["run"]["build_id"] == "v0.111.0"
    assert len(history["fights"]) == 9
    for fight in history["fights"]:
        assert set(fight) == {"node_index", "encounter_id", "monster_ids",
                              "turns_taken", "relics_entering", "potions_used",
                              "deck_entering"}
        for row in fight["deck_entering"]:
            assert set(row) == {"id", "upgrade_level", "enchantment",
                                "upgrade_ambiguous"}
            assert type(row["upgrade_ambiguous"]) is bool


def test_an_ambiguous_row_projects_true_and_an_absent_flag_false():
    assert rrc._deck_row({"id": "CARD.X", "upgrade_level": 1,
                          "upgrade_ambiguous": True})["upgrade_ambiguous"] is True
    assert rrc._deck_row({"id": "CARD.X", "upgrade_level": 1}) == {
        "id": "CARD.X", "upgrade_level": 1, "enchantment": None,
        "upgrade_ambiguous": False}


@pytest.mark.skipif(not BINARY.exists(),
                    reason="build sim/v0.111.0/engine/target/release/sts-sim")
def test_the_binary_answers_and_refuses_by_name():
    history = rrc.load_history(RUN)
    document = rrc.predict(BINARY, history, 1, "v0.111.0")
    assert rrc.counter_values(document)["shuffle"] == 27
    assert document["counters"]["shuffle"]["status"] == "baseline"
    with pytest.raises(rrc.RunCountersRefusal) as refused:
        rrc.predict(BINARY, history, 99, "v0.111.0")
    assert refused.value.code == "fight_index_out_of_range"

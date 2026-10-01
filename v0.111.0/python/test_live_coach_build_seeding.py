"""
Regression tests for the live-coach build/seeding bug (2026-07-26 session).

The bug: sim/v0.111.0/python/tools/live_coach.py never passed a game build, so
combat_sim's `RunRngSet(seed)` and `Rng(rs[...].seed, counter=...)` both
fell back to `GAME_BUILD_V0_108` — the 32-bit djb2 scheme — while solving a
v0.109 save. Every stream derived wrong: the Niche roll produced a 48 HP
Nibbit against the game's real 44, and the shuffle diverged as soon as the
first reshuffle was exercised. It went unnoticed for two turns because
turn 1 happened to match by hand CONTENTS with the wrong draw ORDER.

Save schema >= 19 persists each stream's xoshiro state (s0..s3), so the
derivation is falsifiable instead of merely assumed — which is what
`verify_stream_seeding` exploits to refuse (I5) before solving. That half is
asserted here and imports no simulator module. The other half, that
`start_combat(build=...)` really reaches both streams, pinned the frozen Python
simulator: it moved to `test_combat_sim_build_seeding.py` when the live coach
started rooting in Rust (`sts-sim entry --save`, #2827 item F1), and item F
deleted it with the simulator.

Run: python3 -m pytest sim/v0.111.0/python/test_live_coach_build_seeding.py
"""

import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE / "tools"))

import live_coach  # noqa: E402
from sts2_rng import (  # noqa: E402
    GAME_BUILD_V0_108, RUN_RNG_STREAMS, RunRngSet, snake_case,
)

V109 = "v0.109.0"
V1091 = "v0.109.1"
V1101 = "v0.110.1"
V1110 = "v0.111.0"
SEED = "7XDBEBWZ1REL"          # the 2026-07-26 live run


def _schema19_save(seed=SEED, build=V109, counters=None):
    """A minimal schema-19 rng block whose states are, by construction,
    the ones `build` derives — i.e. a save genuinely recorded under it."""
    counters = counters or {}
    run_set = RunRngSet(seed, counters, build=build)
    rngs = {}
    for name in RUN_RNG_STREAMS:
        m = run_set.rngs[name]._random
        rngs[snake_case(name)] = {
            "counter": counters.get(name, 0),
            "s0": m.s0, "s1": m.s1, "s2": m.s2, "s3": m.s3,
        }
    return {"rng": {"seed": seed, "rngs": rngs}}


# --------------------------------------------------------------------------
# stream_counters: both save schemas
# --------------------------------------------------------------------------

def test_stream_counters_reads_schema_18():
    save = {"rng": {"seed": SEED, "counters": {"shuffle": 61, "niche": 5}}}
    assert live_coach.stream_counters(save) == {"shuffle": 61, "niche": 5}


def test_stream_counters_reads_schema_19():
    save = {"rng": {"seed": SEED, "rngs": {
        "shuffle": {"counter": 61, "s0": 1, "s1": 2, "s2": 3, "s3": 4},
        "niche": {"counter": 5, "s0": 5, "s1": 6, "s2": 7, "s3": 8}}}}
    assert live_coach.stream_counters(save) == {"shuffle": 61, "niche": 5}


def test_stream_counters_refuses_unknown_shape():
    """I5: an unrecognized rng block must refuse, not default to zeros —
    silently reading no counters would solve from a fresh-combat state."""
    with pytest.raises(NotImplementedError, match="neither 'counters'"):
        live_coach.stream_counters({"rng": {"seed": SEED}})


# --------------------------------------------------------------------------
# verify_stream_seeding: the check that makes the bug visible
# --------------------------------------------------------------------------

def test_verify_stream_seeding_accepts_the_recording_build():
    save = _schema19_save(build=V109)
    assert live_coach.verify_stream_seeding(save, V109) == []


def test_verify_stream_seeding_accepts_v0_109_1_as_v109():
    """v0.109.1 is a pure rebuild of v0.109.0 (PR #627: zero CIL delta), so
    seeding_scheme maps both to the v109 route and a v0.109.0-recorded save
    must verify under v0.109.1."""
    assert live_coach.verify_stream_seeding(
        _schema19_save(build=V109), V1091) == []


def test_verify_stream_seeding_accepts_v0_110_1_rng_route():
    """#793 proved the v0.110.1 RNG route exact independently of content."""
    assert live_coach.verify_stream_seeding(
        _schema19_save(build=V109), V1101) == []


def test_verify_stream_seeding_accepts_v0_111_0_rng_route():
    """#1214 proved v0.111.0 seed derivation exact independently of content."""
    assert live_coach.verify_stream_seeding(
        _schema19_save(build=V109), V1110) == []


def test_verify_stream_seeding_rejects_the_v0_108_default():
    """The actual bug. A v0.109 save checked under the v0.108 default must
    report every stream as mismatched rather than proceeding."""
    save = _schema19_save(build=V109)
    bad = live_coach.verify_stream_seeding(save, GAME_BUILD_V0_108)
    assert set(bad) == {snake_case(n) for n in RUN_RNG_STREAMS}


def test_verify_stream_seeding_honours_nonzero_counters():
    counters = {"Shuffle": 61, "Niche": 5, "MonsterAi": 5}
    save = _schema19_save(build=V109, counters=counters)
    assert live_coach.verify_stream_seeding(save, V109) == []
    assert live_coach.verify_stream_seeding(save, GAME_BUILD_V0_108)


def test_verify_stream_seeding_skips_schema_18():
    """Schema <= 18 carries no states, so there is nothing to verify — the
    check must return clean rather than fabricate a mismatch."""
    save = {"rng": {"seed": SEED, "counters": {"shuffle": 0}}}
    assert live_coach.verify_stream_seeding(save, V109) == []


# --------------------------------------------------------------------------
# current_node: global numbering (#826)
# --------------------------------------------------------------------------

def test_current_node_is_global_across_acts():
    # visited_map_coords resets per act; prior acts' node lists stay in
    # map_point_history. The entry derives the per-fight Encounter
    # stream's total_floor from current_node + 1, so a per-act index
    # silently reseeded every act-2+ random-starter encounter (the #826
    # MonsterAi chain drift on TQM88QFMHSQR fights 12/19).
    act1 = {"current_act_index": 0,
            "visited_map_coords": [[0, 0]] * 5,
            "map_point_history": [[None] * 5]}
    assert live_coach.current_node(act1) == 4
    act3 = {"current_act_index": 2,
            "visited_map_coords": [[0, 0]] * 10,
            "map_point_history": [[None] * 17, [None] * 16, [None] * 9]}
    # 17 + 16 prior nodes + per-act index 9 = global node 42
    assert live_coach.current_node(act3) == 42

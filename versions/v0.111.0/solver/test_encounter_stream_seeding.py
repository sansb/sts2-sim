"""#637 — the PER-FIGHT Encounter stream's seed derivation, per build.

`EncounterModel::GenerateMonstersWithSlots` (v0.109.1 RVA 0x22bc0c,
IL_002d-0054) lazily creates an ad-hoc Rng from three addends:

    ldarg.1; callvirt IRunState::get_Rng
             callvirt RunRngSet::get_Seed        // UInt64 (u8, rid 3288)
    ldarg.1; callvirt IRunState::get_TotalFloor  // Int32  (i4, rid 2899)
             conv.i8
             add                                 // unchecked, 64-bit
    ldarg.0; call     AbstractModel::get_Id
             callvirt ModelId::get_Entry         // String (rid 19751)
             call     StringHelper::GetDeterministicHashCode
                                                 // UInt64 (u8, rid 32749)
             add                                 // unchecked, 64-bit
             newobj   Rng::.ctor                 // (UInt64), rid 4166
                                                 //  -> MegaRandom(UInt64)

Every part is 64-bit under v0.109: `RunRngSet::get_Seed` returns u8 (the
XxHash64 run seed), `StringHelper::GetDeterministicHashCode` is the
XxHash64 one (the surviving djb2 is `GetDeterministicHashCodeOld`, rid
32750, which this site does not call), and `Rng(UInt64)` (RVA 0x61b81)
hands the sum straight to `MegaRandom(UInt64)` with no truncation. The
derivation the six roster builders used — int32 wraparound plus djb2 —
was therefore wrong on all three counts for a v0.109 solve.

This stream is NOT one of the 12 persisted run streams, so unlike #636's
`verify_stream_seeding` there is no recorded save state to compare against.
The vectors below are ENGINE ground truth instead, taken with the headless
harness (local-only; versions/v0.111.0/solver/harness/README.md) against build v0.109.1
commit c8c577f6, sts2.dll sha256
2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f:
`EncounterModel.MutableClone()` -> `GenerateMonstersWithSlots(run)` on a
`RunState.CreateForTest` run, then `_rng._counter` and `_rng._random`'s
xoshiro quadruple read by reflection. Solving those states back to a seed
selects exactly ONE of five candidate derivations in all 42 cases — see
test_v109_states_reject_every_other_derivation for the four rejected.

Nonzero TotalFloor is measured, not assumed: `RunState::get_TotalFloor`
(v0.109.1 RVA 0x5134e) = Sum(MapPointHistory, act => act.Count) over a
List<List<MapPointHistoryEntry>>, so the probe appended one inner list of
N entries to reach TotalFloor == N (0, 1, 3, 8 and 30 appear below).

The v0.108 route is unchanged and stays the default, so every pre-#309
capture and pin keeps the scheme it was recorded under (I8).

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""
import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from sts2_rng import (GAME_BUILD_V0_108, MASK64, Rng,  # noqa: E402
                      RunRngSet, _wrap_i32, deterministic_hash_code,
                      deterministic_hash_code_v109)

V109 = "v0.109.0"
V1091 = "v0.109.1"          # the build the engine vectors were taken under


# --------------------------------------------------------------------------
# Engine-observed (counter, s0, s1, s2, s3) of the per-fight Encounter Rng
# right after GenerateMonstersWithSlots, keyed by
# (run seed string, Id.Entry, TotalFloor). The counter IS the number of
# Encounter-stream draws the roster construction makes: SLIMES_WEAK takes 3
# (two smalls, one medium), every other builder takes exactly 1.
# --------------------------------------------------------------------------
V109_ENCOUNTER_STATES = {
    ("7XDBEBWZ1REL", "SLIMES_WEAK", 0):
        (3, 16829205524269576970, 11834216939161919039,
         3004761610728837737, 15915298846030386246),
    ("7XDBEBWZ1REL", "BOWLBUGS_WEAK", 0):
        (1, 16857215048810718546, 1843399061536386291,
         9655045284448200404, 7281403398352126758),
    ("7XDBEBWZ1REL", "TWO_TAILED_RATS_NORMAL", 0):
        (1, 15120954662956269085, 4710487543489507409,
         7982445877594225449, 17249011320292737644),
    ("7XDBEBWZ1REL", "SLIMES_NORMAL", 0):
        (1, 4457436315302165755, 12073918942832624988,
         12992528041802511055, 5415382042842121043),
    ("7XDBEBWZ1REL", "SCROLLS_OF_BITING_NORMAL", 0):
        (1, 15644609482105770472, 4346300472740557958,
         11319303021543221075, 11013336981887217149),
    ("7XDBEBWZ1REL", "DECIMILLIPEDE_ELITE", 0):
        (1, 1459816651727543233, 5862282522673444731,
         13042519373129380230, 8054307308202890244),
    ("ZPJHU3WSH2", "SLIMES_WEAK", 0):
        (3, 18025208222997967989, 9148393205872488620,
         16557185994593105507, 4273433674354344487),
    ("ZPJHU3WSH2", "BOWLBUGS_WEAK", 0):
        (1, 9534203579257073391, 12051005801361624465,
         5927823101921149775, 18311822924203716770),
    ("ZPJHU3WSH2", "TWO_TAILED_RATS_NORMAL", 0):
        (1, 1475236499849792740, 6061722069570583041,
         7032326106168251317, 15207189672169750054),
    ("ZPJHU3WSH2", "SLIMES_NORMAL", 0):
        (1, 1367121417289966584, 11174505082993385687,
         6717112857999958880, 16926498467305196495),
    ("ZPJHU3WSH2", "SCROLLS_OF_BITING_NORMAL", 0):
        (1, 15757356884969254701, 13006332113177130210,
         138180934315868308, 2837455254741203829),
    ("ZPJHU3WSH2", "DECIMILLIPEDE_ELITE", 0):
        (1, 9574146015688589752, 106522252328093104,
         14793562855668389322, 12485544867064810517),
    ("7XDBEBWZ1REL", "SLIMES_WEAK", 3):
        (3, 1384548653478200527, 16120169711793838386,
         16562912836972592928, 13700001040920174467),
    ("7XDBEBWZ1REL", "BOWLBUGS_WEAK", 3):
        (1, 3039618303159043167, 5128985502604038782,
         8200990048327043013, 15184440362692132165),
    ("7XDBEBWZ1REL", "TWO_TAILED_RATS_NORMAL", 3):
        (1, 15953194154254135109, 8920139168196377491,
         11941503755905818978, 17467453940856865782),
    ("7XDBEBWZ1REL", "SLIMES_NORMAL", 3):
        (1, 12195350120384311145, 9640239625640031742,
         10798415985347609037, 7075478428425139937),
    ("7XDBEBWZ1REL", "SCROLLS_OF_BITING_NORMAL", 3):
        (1, 3369620393185854090, 5823677363154638751,
         16319457755586003345, 4401959630140830917),
    ("7XDBEBWZ1REL", "DECIMILLIPEDE_ELITE", 3):
        (1, 6445193142968647764, 11900124736402893985,
         14667542239851944373, 3616617039004061269),
    ("7XDBEBWZ1REL", "SLIMES_WEAK", 8):
        (3, 12286949327472156976, 15621741904755590045,
         7576522819286052236, 6576712663063415184),
    ("7XDBEBWZ1REL", "BOWLBUGS_WEAK", 8):
        (1, 14118511245685501369, 16026635078108417363,
         12995746211657172540, 4784230173856921726),
    ("7XDBEBWZ1REL", "TWO_TAILED_RATS_NORMAL", 8):
        (1, 7395604481328861128, 992070764732857842,
         7407318539989403513, 9714789087139279267),
    ("7XDBEBWZ1REL", "SLIMES_NORMAL", 8):
        (1, 10583744952526990413, 8145085496987466688,
         14451108643867872265, 8680885622372321251),
    ("7XDBEBWZ1REL", "SCROLLS_OF_BITING_NORMAL", 8):
        (1, 13488394311280310431, 1955048323662104298,
         1873898141879018974, 16249663040670021696),
    ("7XDBEBWZ1REL", "DECIMILLIPEDE_ELITE", 8):
        (1, 8167963998657887337, 11935610335308400432,
         8741434143077157690, 1359010119485047670),
    ("ZPJHU3WSH2", "SLIMES_WEAK", 8):
        (3, 8085143568074184235, 3700782701199053421,
         11291951598337804447, 10666952615771309899),
    ("ZPJHU3WSH2", "BOWLBUGS_WEAK", 8):
        (1, 18326413875097929054, 2180987089504778159,
         10682835530742377749, 10295706156565281257),
    ("ZPJHU3WSH2", "TWO_TAILED_RATS_NORMAL", 8):
        (1, 5073199220046893919, 15818362241158473984,
         17763538445535532569, 3228487663798977313),
    ("ZPJHU3WSH2", "SLIMES_NORMAL", 8):
        (1, 10738368164932887119, 10700844933680506748,
         13379831900717366496, 15754149091008106881),
    ("ZPJHU3WSH2", "SCROLLS_OF_BITING_NORMAL", 8):
        (1, 12787142906231180282, 8905313462479912370,
         10625217080952986675, 16460906683428851187),
    ("ZPJHU3WSH2", "DECIMILLIPEDE_ELITE", 8):
        (1, 12890017284377105032, 4640278584535318196,
         17635347023372297582, 1889205005388187479),
    ("ZPJHU3WSH2", "SLIMES_WEAK", 30):
        (3, 3606470234134232752, 14811330402570724699,
         10479245646005523517, 2002849917397904040),
    ("ZPJHU3WSH2", "BOWLBUGS_WEAK", 30):
        (1, 14392098911785391541, 15883399443848800208,
         4797452660719592725, 5587072444004595654),
    ("ZPJHU3WSH2", "TWO_TAILED_RATS_NORMAL", 30):
        (1, 3914333079946259223, 2628567372057083706,
         3457842497130666210, 15483829496166329777),
    ("ZPJHU3WSH2", "SLIMES_NORMAL", 30):
        (1, 1955557783420795293, 2525466942645692497,
         8923692037324998998, 3806244269616382557),
    ("ZPJHU3WSH2", "SCROLLS_OF_BITING_NORMAL", 30):
        (1, 791987565936336114, 16902650635491704781,
         6821196601802694992, 12562136498253672220),
    ("ZPJHU3WSH2", "DECIMILLIPEDE_ELITE", 30):
        (1, 9405074556749916074, 7774356704851043247,
         2926347355243522274, 17328655778353341635),
    ("HQPAXCBS6P", "SLIMES_WEAK", 1):
        (3, 679985265474341631, 14353683161420778444,
         7494951473369659663, 4530359689612510667),
    ("HQPAXCBS6P", "BOWLBUGS_WEAK", 1):
        (1, 2376376855972819993, 16304152757025892661,
         5847209539259473875, 10537175702640945486),
    ("HQPAXCBS6P", "TWO_TAILED_RATS_NORMAL", 1):
        (1, 11422466240806900981, 4331575104812667990,
         10927653164804593338, 3702273343613418203),
    ("HQPAXCBS6P", "SLIMES_NORMAL", 1):
        (1, 9425786968869716113, 11408703136676948856,
         1189732431978187043, 4220292538561092530),
    ("HQPAXCBS6P", "SCROLLS_OF_BITING_NORMAL", 1):
        (1, 14027739131669925127, 1942229392281988590,
         10569435253404085009, 18139566725813315134),
    ("HQPAXCBS6P", "DECIMILLIPEDE_ELITE", 1):
        (1, 5073937763576533804, 946020417193118406,
         8687435527340086545, 16773953099774334711),
}


# --------------------------------------------------------------------------
# The derivation itself
# --------------------------------------------------------------------------


def _engine_state(seed, entry, floor, build=V1091):
    """Re-derive the engine's post-generation Encounter-stream state."""
    counter = V109_ENCOUNTER_STATES[(seed, entry, floor)][0]
    rng = Rng.for_encounter(RunRngSet(seed, build=build).seed, floor, entry,
                            build=build)
    rng.fast_forward(counter)
    m = rng._random
    return (rng.counter, m.s0, m.s1, m.s2, m.s3)


def test_v109_encounter_seed_matches_the_engine():
    """42 (seed, encounter, floor) cases from the v0.109.1 harness probe."""
    assert len(V109_ENCOUNTER_STATES) == 42
    for key, expected in V109_ENCOUNTER_STATES.items():
        assert _engine_state(*key) == expected, key


def test_v109_1_and_v109_0_derive_the_same_encounter_seed():
    for seed, entry, floor in V109_ENCOUNTER_STATES:
        a = Rng.for_encounter(RunRngSet(seed, build=V109).seed, floor, entry,
                              build=V109)
        b = Rng.for_encounter(RunRngSet(seed, build=V1091).seed, floor, entry,
                              build=V1091)
        assert a.seed == b.seed, (seed, entry, floor)


def test_v109_states_reject_every_other_derivation():
    """The engine states are not merely CONSISTENT with the IL read — they
    exclude the alternatives, including the derivation the six roster
    builders used before #637 (a hybrid: the v0.109 64-bit set seed, the
    v0.108 djb2 entry hash, and an int32 truncation on top)."""
    rejected = 0
    for (seed, entry, floor), expected in V109_ENCOUNTER_STATES.items():
        counter = expected[0]
        s109 = RunRngSet(seed, build=V1091).seed
        s108 = RunRngSet(seed).seed
        djb2, xxh = (deterministic_hash_code(entry),
                     deterministic_hash_code_v109(entry))
        candidates = {
            # what the six sites computed after #636 threaded the build
            "hybrid_i32": _wrap_i32(s109 + floor + djb2),
            "hybrid_u64": (s109 + floor + djb2) & MASK64,
            "pure_v108": _wrap_i32(s108 + floor + djb2),
            "xxh_truncated_i32": _wrap_i32(s109 + floor + xxh),
        }
        for name, cand in candidates.items():
            rng = Rng(cand, counter, build=V1091)
            m = rng._random
            assert (rng.counter, m.s0, m.s1, m.s2, m.s3) != expected, \
                (name, seed, entry, floor)
            rejected += 1
    assert rejected == 42 * 4


def test_v109_addends_are_all_64_bit():
    """No truncation anywhere: a set seed above 2**32 stays intact, and the
    entry hash is XxHash64 rather than djb2."""
    seed_string, entry, floor = "ZPJHU3WSH2", "SLIMES_WEAK", 8
    set_seed = RunRngSet(seed_string, build=V1091).seed
    assert set_seed > 2 ** 32                       # XxHash64 run seed
    rng = Rng.for_encounter(set_seed, floor, entry, build=V1091)
    assert rng.seed == (set_seed + floor
                        + deterministic_hash_code_v109(entry)) & MASK64
    assert rng.seed > 2 ** 32
    assert rng.counter == 0                         # ad-hoc stream, no
    #                                                 counter accounting


def test_v108_route_is_the_documented_int32_djb2_derivation():
    set_seed = RunRngSet("7MA0PY7AD4").seed
    rng = Rng.for_encounter(set_seed, 5, "SLIMES_WEAK")
    assert rng.seed == _wrap_i32(
        set_seed + 5 + deterministic_hash_code("SLIMES_WEAK"))
    assert rng.build == GAME_BUILD_V0_108           # the default


def test_for_encounter_default_build_is_v0_108():
    a = Rng.for_encounter(1234, 7, "SLIMES_WEAK")
    b = Rng.for_encounter(1234, 7, "SLIMES_WEAK", build=GAME_BUILD_V0_108)
    assert a.seed == b.seed
    assert a.seed != Rng.for_encounter(1234, 7, "SLIMES_WEAK",
                                       build=V1091).seed


def test_for_encounter_refuses_an_unverified_future_build():
    with pytest.raises(NotImplementedError):
        Rng.for_encounter(1234, 7, "SLIMES_WEAK", build="v0.110.0")


def test_for_encounter_v110_1_matches_engine_verified_v109_route():
    # Batch 202 / #793 reran all 42 live-engine vectors on v0.110.1.
    old = Rng.for_encounter(1234, 7, "SLIMES_WEAK", build=V1091)
    new = Rng.for_encounter(1234, 7, "SLIMES_WEAK", build="v0.110.1")
    old_state = tuple(getattr(old._random, f"s{i}") for i in range(4))
    new_state = tuple(getattr(new._random, f"s{i}") for i in range(4))
    assert (new.seed, new.counter, new_state) == \
        (old.seed, old.counter, old_state)

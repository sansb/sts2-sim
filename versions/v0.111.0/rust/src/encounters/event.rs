//! Event-pool encounter rosters — E4b family F7 (#2537).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/event.py`, builder
//! for builder. Seven registered keys: the three Battleworn Dummy variants,
//! Dense Vegetation, the Fake Merchant, the Mysterious Knight and the Punch
//! Off. Every one of them is entered from an event rather than from a map
//! node's encounter pool.
//!
//! Every number below is read from a generated table. HP bands and spawn-time
//! power amounts come from `content_tables::MONSTER_MODELS`; starting rotation
//! positions come from `content_tables::LOOPS`, looked up **by move name**
//! (PORT_PLAN §5); and the Punch Off's two `StartingHpReduction` draw bounds
//! come from `content_tables::ENCOUNTER_RNG_DRAWS`, which `tools/dll_content.py`
//! reads out of `PunchOffEventEncounter::GenerateMonsters` itself (#2537 added
//! that fact, because `event.py` spells the bound as a call-site literal
//! `rng.next_int(2, 10)` that no module constant carries).
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs (v0.111.0 `sts2.dll` sha256 `9cb4f1ad…`, read with
//! `versions/v0.111.0/solver/tools/dump_il.py`). Python's own evidence tuples
//! (`combat_sim._B240_BATTLEWORN_DUMMY_EVIDENCE`) cite a previous build's
//! addresses — `BattlewornDummyEventV1Encounter::GenerateMonsters` at
//! `0xD6501` there, `0xd2f85` here:
//!
//! | class | method | RVA |
//! |---|---|---|
//! | `BattlewornDummyEventV1Encounter` | `GenerateMonsters` | `0xd2f85` |
//! | `BattlewornDummyEventV2Encounter` | `GenerateMonsters` | `0xd2fb6` |
//! | `BattlewornDummyEventV3Encounter` | `GenerateMonsters` | `0xd2fe7` |
//! | `BattleFriendV1` / `V2` / `V3` | `<AfterAddedToRoom>d__6::MoveNext` | `0x3536cc` / `0x3537c0` / `0x3538b4` |
//! | `DenseVegetationEventEncounter` | `GenerateMonsters` / `get_Slots` | `0xd37d0` / `0xd3793` |
//! | `Wriggler` | `GenerateMoveStateMachine` | `0xc5630` |
//! | `FakeMerchantEventEncounter` | `GenerateMonsters` | `0xd3c19` |
//! | `FakeMerchantMonster` | `GenerateMoveStateMachine` | `0xb40c4` |
//! | `MysteriousKnightEventEncounter` | `GenerateMonsters` | `0xd4421` |
//! | `MysteriousKnight` | `<AfterAddedToRoom>d__0::MoveNext` | `0x3642a0` |
//! | `FlailKnight` | `GenerateMoveStateMachine` | `0xb47bc` |
//! | `PunchOffEventEncounter` | `GenerateMonsters` | `0xd491c` |
//! | `PunchConstruct` | `GenerateMoveStateMachine` | `0xbbf9c` |
//! | `PunchConstruct` | `<AfterAddedToRoom>d__24::MoveNext` | `0x366c04` |
//!
//! # `raw_niche_draw`: the two single-creature fixed-HP builders
//!
//! `_fake_merchant` and `_mysterious_knight` spend their `Niche` draw with a
//! bare `ctx.niche.next_int(0, 1)` rather than through `ctx.fixed`, because
//! they also set a random-AI opener `ctx.fixed` has no keyword for. The draw
//! is still exactly the one `Creature::SetUniqueMonsterHpValue` spends for a
//! single-value set, so both are [`fixed`] here — which spends the same draw
//! and additionally refuses if the native band were a range. The pins measure
//! the `Niche` counter and words after creation, so this is checked rather
//! than assumed.

use crate::content_tables::ENCOUNTER_RNG_DRAWS;
use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position,
};
use crate::ids::MonsterKind;

/// `ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER` —
/// `event.py::_battleworn_dummy_v1`.
///
/// `BattlewornDummyEventV1Encounter::GenerateMonsters` (`0xd2f85`) is one
/// `Monster<BattleFriendV1>().ToMutable()` with a `ldnull` slot. See
/// [`battleworn_dummy`].
pub fn build_battleworn_dummy_event_v1_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    battleworn_dummy(ctx, MonsterKind::BattleFriendV1)
}

/// `ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V2_ENCOUNTER` —
/// `event.py::_battleworn_dummy_v2`. `GenerateMonsters` `0xd2fb6`, the V1 body
/// over `BattleFriendV2`.
pub fn build_battleworn_dummy_event_v2_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    battleworn_dummy(ctx, MonsterKind::BattleFriendV2)
}

/// `ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V3_ENCOUNTER` —
/// `event.py::_battleworn_dummy_v3`. `GenerateMonsters` `0xd2fe7`, the V1 body
/// over `BattleFriendV3`.
pub fn build_battleworn_dummy_event_v3_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    battleworn_dummy(ctx, MonsterKind::BattleFriendV3)
}

/// The shared Battleworn Dummy roster: one fixed-HP training dummy carrying
/// `BattlewornDummyTimeLimitPower`.
///
/// Each `BattleFriendV<n>::get_MinInitialHp`/`get_MaxInitialHp` is one untiered
/// `ldc.i4.s; ret` pair (V1 `0xaf2cf`/`0xaf2d3`), so the creation spends the
/// single-value `SetUniqueMonsterHpValue` draw [`fixed`] spends. The three
/// `<AfterAddedToRoom>d__6::MoveNext` bodies apply exactly one power:
///
/// ```text
/// IL_0028: ldc.i4.3
/// IL_0029: newobj   System.Decimal::.ctor
/// IL_0031: call     Apply<BattlewornDummyTimeLimitPower>
/// ```
///
/// The oracle carries it as `Monster.battleworn_time_limit`, an `int` counter
/// (`combat_sim.py` `battleworn_time_limit: int = 0`), so it is an amount, read
/// from `MONSTER_MODELS` rather than typed in. Slot 0: the native slot is
/// `ldnull`, and the oracle pins `slot=0`, which is the default.
///
/// The machine is a single self-following `NOTHING_MOVE` (`GenerateMoveStateMachine`
/// V1 `0xaf304`), the rotation's only position, so `loop_pos` stays 0.
fn battleworn_dummy(
    ctx: &mut dyn EncounterCtx,
    kind: MonsterKind,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let time_limit = initial_power(kind, "BattlewornDummyTimeLimitPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, kind)?.with_state("battleworn_time_limit", time_limit),
    ])
}

/// `ENCOUNTER.DENSE_VEGETATION_EVENT_ENCOUNTER` — `event.py::_dense_vegetation`.
///
/// Four ordinary Wrigglers. `DenseVegetationEventEncounter::GenerateMonsters`
/// (`0xd37d0`) walks `get_Slots` (`0xd3793`: the four literals `wriggler1` ..
/// `wriggler4`, in that order) and creates one Wriggler per slot:
///
/// ```text
/// IL_0027: call     Monster<Wriggler>
/// IL_002c: callvirt MonsterModel::ToMutable
/// IL_0037: ldc.i4.0
/// IL_0039: callvirt Wriggler::set_StartStunned       // false: not a spawn
/// IL_0041: newobj   (wriggler, slotName)
/// ```
///
/// So creation order is slot order, and the four creations share one band, so
/// each excludes its predecessors' rolled max HP (`SetUniqueMonsterHpValue`).
/// The count, four, is structural — it is the length of the slot array — in
/// the same sense the scroll count is in [`super::normal_a`].
///
/// # Opening positions
///
/// `Wriggler::GenerateMoveStateMachine` (`0xc5630`) opens on its `INIT_MOVE`
/// `ConditionalBranchState` whenever `StartStunned` is false
/// (`IL_0120`-`IL_012c`), and that branch picks by slot name:
///
/// ```text
/// IL_009e: ldloc.1 (NASTY_BITE_MOVE)  <- b__16_0: SlotName == "wriggler1"
/// IL_00b2: ldloc.2 (WRIGGLE_MOVE)     <- b__16_1: SlotName == "wriggler2"
/// IL_00c6: ldloc.1 (NASTY_BITE_MOVE)  <- b__16_2: SlotName == "wriggler3"
/// IL_00da: ldloc.2 (WRIGGLE_MOVE)     <- b__16_3: SlotName == "wriggler4"
/// ```
///
/// That is the oracle's `loop_pos=slot % 2`; here the two positions are read
/// out of the generated rotation by the oracle's move names instead.
pub fn build_dense_vegetation_event_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const OPENERS: [&str; 4] = ["NASTY_BITE", "WRIGGLE", "NASTY_BITE", "WRIGGLE"];
    let kind = MonsterKind::Wriggler;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(OPENERS.len());
    for (slot, opener) in OPENERS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(kind, hp)
                .slot(slot as i32)
                .loop_pos(loop_position(kind, opener)?),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.FAKE_MERCHANT_EVENT_ENCOUNTER` — `event.py::_fake_merchant`.
///
/// `FakeMerchantEventEncounter::GenerateMonsters` (`0xd3c19`) creates one
/// `FakeMerchantMonster` in the named slot `merchant`; the oracle pins
/// `slot=0`. Its band is a single value per tier (`get_MinInitialHp` `0xb4072`
/// and `get_MaxInitialHp` `0xb4084` are the same `GetValueIfAscension` triple),
/// so the creation spends one `Niche` draw — the oracle's bare
/// `ctx.niche.next_int(0, 1)` (see the module doc's `raw_niche_draw` note).
///
/// `FakeMerchantMonster::GenerateMoveStateMachine` (`0xb40c4`) hands
/// `MonsterMoveStateMachine::.ctor` its `SWIPE_MOVE` `MoveState` (`ldloc.1`,
/// stored at `IL_003c`) as the initial state at `IL_0166`, not one of its two
/// `RandomBranchState`s, so turn 1 is used as-is with no `MonsterAi` draw and
/// logged like a rolled move. It applies no spawn-time power.
pub fn build_fake_merchant_event_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const SWIPE: &str = "SWIPE_MOVE";
    Ok(vec![
        fixed(ctx, MonsterKind::FakeMerchantMonster)?
            .slot(0)
            .opening_move(SWIPE, &[SWIPE]),
    ])
}

/// `ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER` —
/// `event.py::_mysterious_knight`.
///
/// `MysteriousKnightEventEncounter::GenerateMonsters` (`0xd4421`) creates one
/// `MysteriousKnight` with a `ldnull` slot. Its band is a single value per tier
/// (`MONSTER_MODELS`), so the creation spends one `Niche` draw (the oracle's
/// bare `ctx.niche.next_int(0, 1)`).
///
/// `MysteriousKnight` is a `FlailKnight` subclass whose only override is
/// `AfterAddedToRoom`. `<AfterAddedToRoom>d__0::MoveNext` (`0x3642a0`) awaits
/// the base hook (`IL_002d`), then applies two powers to itself in order:
///
/// ```text
/// IL_0092: ldc.i4.6; newobj Decimal::.ctor; IL_00a0: call Apply<StrengthPower>
/// IL_0106: ldc.i4.6; newobj Decimal::.ctor; IL_0114: call Apply<PlatingPower>
/// ```
///
/// The oracle carries them as `Monster.strength` and `Monster.mplating`, in
/// that (dataclass) order; both amounts are read from `MONSTER_MODELS`.
///
/// The inherited `FlailKnight::GenerateMoveStateMachine` (`0xb47bc`) hands the
/// machine its `RAM_MOVE` `MoveState` (`ldloc.3`, `IL_00eb`) as the initial
/// state rather than its `RAND` branch, so the opener consumes no `MonsterAi`
/// draw and is logged like a rolled move — the same opener
/// [`super::elite::build_knights_elite`] gives the Flail Knight.
pub fn build_mysterious_knight_event_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const RAM: &str = "RAM_MOVE";
    let kind = MonsterKind::MysteriousKnight;
    let strength = initial_power(kind, "StrengthPower", ctx.ascension())?;
    let plating = initial_power(kind, "PlatingPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, kind)?
            .opening_move(RAM, &[RAM])
            .with_state("strength", strength)
            .with_state("mplating", plating),
    ])
}

/// The Punch Off's registered key and ModelId entry: the row of
/// `content_tables::ENCOUNTER_RNG_DRAWS` its builder reads.
const PUNCH_OFF_ENTRY: &str = "PUNCH_OFF_EVENT_ENCOUNTER";

/// `ENCOUNTER.PUNCH_OFF_EVENT_ENCOUNTER` — `event.py::_punch_off`.
///
/// Two Punch Constructs. `PunchOffEventEncounter::GenerateMonsters`
/// (`0xd491c`), in body order:
///
/// ```text
/// IL_000c: Monster<PunchConstruct>.ToMutable -> loc0
/// IL_001d: ldc.i4.1; callvirt PunchConstruct::set_StartsWithFastPunch
/// IL_0025: call EncounterModel::get_Rng
/// IL_002a: ldc.i4.2; IL_002b: ldc.i4.s 10; callvirt Rng::NextInt    // draw 1
/// IL_0032: callvirt PunchConstruct::set_StartingHpReduction
/// IL_0037: Monster<PunchConstruct>.ToMutable -> loc1
/// IL_0049: call EncounterModel::get_Rng
/// IL_004e: ldc.i4.2; IL_004f: ldc.i4.s 10; callvirt Rng::NextInt    // draw 2
/// IL_0056: callvirt PunchConstruct::set_StartingHpReduction
/// IL_005b: newarr [ (loc0, null), (loc1, null) ]
/// ```
///
/// Both reductions are drawn from the per-fight `Encounter` stream while the
/// models are still being configured; the creatures — and with them the two
/// `Niche` HP draws — come afterwards, in array order. The oracle draws both
/// reductions first and rolls HP after, which is the native order; the two
/// streams are disjoint either way. The bounds are
/// `content_tables::ENCOUNTER_RNG_DRAWS`' row for this class, read from those
/// two `ldc` pairs.
///
/// # Opening positions
///
/// `PunchConstruct::GenerateMoveStateMachine` (`0xbbf9c`) starts the machine on
/// `FAST_PUNCH_MOVE` (`ldloc.3`) when `get_StartsWithFastPunch` is true
/// (`IL_00c8`-`IL_00cd`) and on `READY_MOVE` (`ldloc.1`) otherwise, so the
/// first construct opens on `FAST_PUNCH` and the second on `READY`, each read
/// out of the generated rotation by name.
///
/// # Spawn state
///
/// `<AfterAddedToRoom>d__24::MoveNext` (`0x366c04`) awaits the base hook, then
/// applies `ArtifactPower` with `Decimal.One` (`IL_008a`-`IL_0097`), then — when
/// `StartingHpReduction > 0` (`IL_00f2`-`IL_00f9`, always true for a draw over
/// `[2, 10)`) — sets current HP to `Math.Max(1, CurrentHp - StartingHpReduction)`
/// (`IL_0101`-`IL_011e`). Max HP keeps the rolled value. The `1` is the clamp
/// in that `Math.Max`, not a content number. Each fixed-HP creation spends its
/// `SetUniqueMonsterHpValue` draw over an empty uniqueness set, as the oracle's
/// `ctx.unique_hp(PUNCH_HP, PUNCH_HP, set())` does; over a single-value band the
/// set cannot change the pick, only the draw count, which is one either way.
pub fn build_punch_off_event_encounter(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const OPENERS: [&str; 2] = ["FAST_PUNCH", "READY"];
    const ROLL: &str = "PunchConstruct StartingHpReduction rolls";
    let kind = MonsterKind::PunchConstruct;
    let draws = punch_off_draws();
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let artifact = initial_power(kind, "ArtifactPower", ctx.ascension())?;
    let mut reductions = Vec::with_capacity(draws.len());
    for &(draw_lo, draw_hi) in draws {
        reductions.push(ctx.encounter_next_int(ROLL, draw_lo, draw_hi)?);
    }
    let mut roster = Vec::with_capacity(OPENERS.len());
    for (slot, (opener, reduction)) in OPENERS.iter().zip(reductions).enumerate() {
        let max_hp = ctx.unique_hp(lo, hi, &[]);
        let mut spec = MonsterSpec::new(kind, max_hp)
            .slot(slot as i32)
            .loop_pos(loop_position(kind, opener)?)
            .with_state("artifact", artifact);
        spec.hp = (max_hp - reduction).max(1);
        roster.push(spec);
    }
    Ok(roster)
}

/// The Punch Off's two `StartingHpReduction` draw bounds, from codegen.
///
/// A missing or refused row is a generator/content disagreement rather than a
/// fact about a fight, so it is a programming error here, and
/// `the_punch_off_draw_bounds_are_generated` pins that it cannot happen on
/// this build.
fn punch_off_draws() -> &'static [(i32, i32)] {
    let row = ENCOUNTER_RNG_DRAWS
        .iter()
        .find(|row| row.entry == PUNCH_OFF_ENTRY)
        .expect("ENCOUNTER_RNG_DRAWS carries PunchOffEventEncounter's draws");
    assert!(
        row.refused.is_empty(),
        "PunchOffEventEncounter's draw bounds were refused at codegen: {}",
        row.refused
    );
    row.draws
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::opening::roster::make_monsters;
    use crate::rng::{Xoshiro256StarStar, run_set_seed_v109};

    /// One monster: `(hp, max_hp, slot, loop_pos, spawn state)`.
    type Row = (i32, i32, i32, i32, &'static [(&'static str, i64)]);

    /// One oracle-printed opening: `(seed, total floor, ascension, Niche
    /// seed)`, then its [`Row`]s, then the `Niche` counter and words after
    /// creation.
    type Vector = (&'static str, i64, u8, u64, &'static [Row], u64, [u64; 4]);

    /// The corpus has no Punch Off, Battleworn V1 or Battleworn V3 fight
    /// (#2537's table: zero demand), so `fixtures/event_rosters_v1.json` cannot
    /// measure these three builders. They are measured here against vectors the
    /// oracle printed, through the engine's own `make_monsters` — the live
    /// `Niche` carrier and the one per-fight `Encounter` stream derivation —
    /// at three ascensions on either side of the A8 HP gate:
    ///
    /// ```text
    /// cd versions/v0.111.0/solver && python3.12 -c "
    /// import combat_sim as s
    /// for e in ('ENCOUNTER.PUNCH_OFF_EVENT_ENCOUNTER',
    ///           'ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER',
    ///           'ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V3_ENCOUNTER'):
    ///     for seed, floor, asc in (('ZPJHU3WSH2', 12, 10),
    ///                              ('AW56G6M5C6C7', 30, 7),
    ///                              ('KBC46J02JWT3', 5, 0)):
    ///         rs = s.RunRngSet(seed, build='v0.111.0')
    ///         ms, n = s.make_monsters(e, seed, 0, floor, build='v0.111.0',
    ///                                 ascension=asc)
    ///         print(e, seed, rs['Niche'].seed,
    ///               [(m.kind, m.hp, m.max_hp, m.slot, m.loop_pos, m.artifact,
    ///                 m.battleworn_time_limit) for m in ms], n[4], n[:4])"
    /// ```
    fn assert_vectors(encounter: &str, kind: MonsterKind, vectors: &[Vector]) {
        for &(seed, floor, ascension, niche_seed, monsters, counter, words) in vectors {
            let roster = make_monsters(
                encounter,
                Xoshiro256StarStar::from_seed(niche_seed),
                run_set_seed_v109(seed),
                Some(floor),
                ascension,
            )
            .expect("the id dispatches to a registered builder")
            .expect("the builder has every fact it needs");
            let got: Vec<_> = roster
                .monsters
                .iter()
                .map(|m| {
                    assert_eq!(m.kind, kind, "{encounter} {seed}");
                    let state: Vec<(&str, i64)> = m
                        .initial_state
                        .iter()
                        .map(|(field, value)| {
                            (*field, value.as_i64().expect("an integer spawn value"))
                        })
                        .collect();
                    (m.hp, m.max_hp, m.slot, m.loop_pos, state)
                })
                .collect();
            let want: Vec<_> = monsters
                .iter()
                .map(|&(hp, max_hp, slot, loop_pos, state)| {
                    (hp, max_hp, slot, loop_pos, state.to_vec())
                })
                .collect();
            assert_eq!(got, want, "{encounter} {seed} A{ascension}");
            assert_eq!(
                (roster.niche.counter, roster.niche.words),
                (counter, words),
                "{encounter} {seed}: Niche after creation"
            );
        }
    }

    #[test]
    fn the_punch_off_matches_the_oracle() {
        const ART: &[(&str, i64)] = &[("artifact", 1)];
        assert_vectors(
            "ENCOUNTER.PUNCH_OFF_EVENT_ENCOUNTER",
            MonsterKind::PunchConstruct,
            &[
                (
                    "ZPJHU3WSH2",
                    12,
                    10,
                    12_800_230_507_224_994_239,
                    &[(54, 60, 0, 1, ART), (52, 60, 1, 0, ART)],
                    2,
                    [
                        4_858_694_066_161_000_806,
                        1_694_859_406_351_784_433,
                        15_951_007_701_312_378_353,
                        1_312_339_868_190_625_827,
                    ],
                ),
                (
                    "AW56G6M5C6C7",
                    30,
                    7,
                    15_076_828_399_546_096_497,
                    &[(50, 55, 0, 1, ART), (53, 55, 1, 0, ART)],
                    2,
                    [
                        5_860_744_530_295_211_721,
                        8_932_657_921_787_358_912,
                        15_518_652_502_656_047_525,
                        7_072_626_738_014_414_780,
                    ],
                ),
                (
                    "KBC46J02JWT3",
                    5,
                    0,
                    16_991_662_252_921_997_635,
                    &[(51, 55, 0, 1, ART), (50, 55, 1, 0, ART)],
                    2,
                    [
                        3_939_940_860_462_487_938,
                        9_054_486_849_414_010_176,
                        16_025_136_593_240_977_675,
                        13_920_580_266_016_560_301,
                    ],
                ),
            ],
        );
    }

    /// The single-draw words the two zero-demand dummies leave behind: the
    /// same three streams after one `NextInt(0, 1)` each, whichever dummy it is.
    const DUMMY_NICHE_AFTER: [(&str, u64, [u64; 4]); 3] = [
        (
            "ZPJHU3WSH2",
            12_800_230_507_224_994_239,
            [
                12_838_685_206_236_606_676,
                6_331_443_712_751_389_696,
                17_470_794_500_245_103_909,
                12_005_591_843_614_350_770,
            ],
        ),
        (
            "AW56G6M5C6C7",
            15_076_828_399_546_096_497,
            [
                8_433_111_396_613_117_937,
                5_989_787_344_473_339_749,
                6_764_614_404_004_384_340,
                8_593_463_895_452_395_101,
            ],
        ),
        (
            "KBC46J02JWT3",
            16_991_662_252_921_997_635,
            [
                11_049_443_700_710_559_996,
                17_328_630_755_599_803_467,
                1_480_510_607_649_939_959,
                6_881_869_713_002_032_437,
            ],
        ),
    ];

    fn dummy_vectors(hp: i32, state: &'static [(&'static str, i64)]) -> Vec<Vector> {
        let floors = [(12, 10), (30, 7), (5, 0)];
        DUMMY_NICHE_AFTER
            .iter()
            .zip(floors)
            .map(|(&(seed, niche_seed, words), (floor, ascension))| {
                let monsters: &'static [Row] = Box::leak(Box::new([(hp, hp, 0, 0, state)]));
                (seed, floor, ascension, niche_seed, monsters, 1, words)
            })
            .collect()
    }

    #[test]
    fn the_zero_demand_battleworn_dummies_match_the_oracle() {
        const LIMIT: &[(&str, i64)] = &[("battleworn_time_limit", 3)];
        assert_vectors(
            "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER",
            MonsterKind::BattleFriendV1,
            &dummy_vectors(75, LIMIT),
        );
        assert_vectors(
            "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V3_ENCOUNTER",
            MonsterKind::BattleFriendV3,
            &dummy_vectors(300, LIMIT),
        );
    }

    #[test]
    fn the_punch_off_draw_bounds_are_generated() {
        let draws = punch_off_draws();
        assert_eq!(draws.len(), 2, "two StartingHpReduction draws");
        assert!(draws.iter().all(|(lo, hi)| 0 < *lo && lo < hi));
    }
}

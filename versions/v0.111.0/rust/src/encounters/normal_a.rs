//! Normal-pool encounter rosters, demand rank 1-15 — E4b family F3 (#2533).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/normal.py`, builder
//! for builder, under the same rules as [`super::weak`] and [`super::elite`]:
//! every number in a roster comes from a generated table
//! (`content_tables::MONSTER_MODELS` for HP bands and spawn-time power amounts,
//! `content_tables::LOOPS` for rotation lengths and positions, looked up **by
//! move name**, `content_tables::encounter_pool_constants::normal` for the
//! pool's flat constants, and `content_tables::MONSTER_CTOR_INTS` for the one
//! constructor-initialised field a roster carries), and the control flow
//! around them is hand-ported with current-build IL citations (PORT_PLAN §5).
//!
//! # Fifteen builders, sixteen entry points
//!
//! `_scrolls_of_biting` was pulled forward out of the wave (7ff1ff16): it is
//! the one Python builder that serves two registered keys across two families
//! (`SCROLLS_OF_BITING_WEAK` is F3's, `SCROLLS_OF_BITING_NORMAL` is F6's,
//! #2536), and this module owns both entry points. Its acceptance lives in the
//! `scrolls_of_biting` pin file; the other fourteen are pinned in
//! `fixtures/normal_a_rosters_v1.json`.
//!
//! `_corpse_slugs_normal` delegates to F1's [`super::weak::corpse_slugs`],
//! which is where the Corpse Slug Encounter-stream draw lives; it is called,
//! not copied.
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs (v0.111.0 `sts2.dll` sha256 `9cb4f1ad…`, read with
//! `versions/v0.111.0/solver/tools/dump_il.py`):
//!
//! | class | method | RVA |
//! |---|---|---|
//! | `ThievingHopperWeak` | `GenerateMonsters` | `0xd5823` |
//! | `MytesNormal` | `GenerateMonsters` / `get_Slots` | `0xd4488` / `0xd4440` |
//! | `CultistsNormal` | `GenerateMonsters` | `0xd3615` |
//! | `InkletsNormal` | `GenerateMonsters` | `0xd3fd4` |
//! | `OvicopterNormal` | `GenerateMonsters` / `get_Slots` | `0xd46cc` / `0xd4643` |
//! | `TwoTailedRatsNormal` | `GenerateMonsters` / `get_Slots` | `0xd5a50` / `0xd59f2` |
//! | `TheObscuraNormal` | `GenerateMonsters` / `get_Slots` | `0xd57e6` / `0xd57a9` |
//! | `GremlinMercNormal` | `GenerateMonsters` | `0xd3e94` |
//! | `CorpseSlugsNormal` | `GenerateMonsters` | `0xd3464` |
//! | `HunterKillerNormal` | `GenerateMonsters` | `0xd3f67` |
//! | `ExoskeletonsNormal` | `GenerateMonsters` / `get_Slots` | `0xd390c` / `0xd38cd` |
//! | `CubexConstructNormal` | `GenerateMonsters` | `0xd35ec` |
//! | `FossilStalkerNormal` | `GenerateMonsters` | `0xd3da4` |
//! | `MawlerNormal` | `GenerateMonsters` | `0xd43ad` |
//! | `ScrollsOfBitingWeak` | `GenerateMonsters` | `0xd4c78` |
//! | `ScrollsOfBitingNormal` | `GenerateMonsters` | `0xd4b90` |
//!
//! and, on the monster side, the `GenerateMoveStateMachine` initial state each
//! opener below is read from:
//!
//! | class | `GenerateMoveStateMachine` | initial state |
//! |---|---|---|
//! | `Myte` | `0xba07c` | `INIT_MOVE` conditional on `Creature::get_SlotName` |
//! | `Inklet` | `0xb6d30` | `_middleInklet ? WHIRLWIND_MOVE : JAB_MOVE` (IL_012f-IL_013b) |
//! | `TwoTailedRat` | `0xc4010` | `StarterMoveIndex % 3` (IL_016e-IL_0192) |
//! | `TheObscura` | `0xc1d2c` | `ILLUSION_MOVE` (IL_012a) |
//! | `HunterKiller` | `0xb6604` | `TENDERIZING_GOOP_MOVE` (IL_00e3) |
//! | `FossilStalker` | `0xb5050` | `LATCH_MOVE` (IL_00ff) |
//! | `Mawler` | `0xb9634` | `CLAW_MOVE` (IL_00fb) |
//! | `CubexConstruct` | `0xb23c0` | `CHARGE_UP_MOVE` (IL_0106), rotation position 0 |
//! | `CalcifiedCultist` / `DampCultist` | `0xb0a14` / `0xb2874` | `INCANTATION_MOVE` (IL_007f), rotation position 0 |
//! | `ThievingHopper` | `0xc21e0` | `THIEVERY_MOVE` (IL_0135), rotation position 0 |
//! | `Ovicopter` | `0xbab10` | `LAY_EGGS_MOVE` (IL_0135), rotation position 0 |
//! | `GremlinMerc` | `0xb5f88` | `GIMME_MOVE` (IL_00db), rotation position 0 |
//!
//! # Spawn-time powers that are deliberately not roster state
//!
//! Parity here is with `make_monsters`' output, so a power the oracle models as
//! behaviour keyed on the kind rather than as a `Monster` field is not carried,
//! for the same reason `weak.rs` leaves `BowlbugRock`'s `ImbalancedPower` out:
//! `GremlinMerc`'s `SurprisePower` (`<AfterAddedToRoom>d__21::MoveNext`
//! `0x35d898`, IL_009f) and the Scroll's `PaperCutsPower`. Every spawn-time
//! power this family *does* carry is read from `MONSTER_MODELS`:
//! `ThievingHopper`'s `EscapeArtistPower` (`0x370cf0` IL_0098), `Inklet`'s
//! `SlipperyPower` (`0x35f500` IL_0097), `FossilStalker`'s `SuckPower`
//! (`0x35c1dc` IL_0098), `CubexConstruct`'s `ArtifactPower` (`0x357a60`
//! IL_010a), `Exoskeleton`'s `HardToKillPower` and `CorpseSlug`'s
//! `RavenousPower` (both via F1).

use crate::content_tables::encounter_pool_constants::normal as constants;
use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position,
    rotation_len,
};
use crate::ids::MonsterKind;

/// `ENCOUNTER.THIEVING_HOPPER_WEAK` — `normal.py::_thieving_hopper`.
///
/// One hopper (`ThievingHopperWeak::GenerateMonsters` `0xd5823`: one
/// creation, null slot), fixed band, carrying `EscapeArtistPower`.
pub fn build_thieving_hopper_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::ThievingHopper;
    let escape_artist = initial_power(kind, "EscapeArtistPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, kind)?.with_state("escape_artist", escape_artist),
    ])
}

/// `ENCOUNTER.MYTES_NORMAL` — `normal.py::_mytes`.
///
/// Two mytes in the named slots `first` and `second`
/// (`MytesNormal::GenerateMonsters` `0xd4488`), one shared uniqueness set.
/// `Myte`'s INIT node is a `ConditionalBranchState` over the slot name:
/// `<GenerateMoveStateMachine>b__17_0` (`0xba31f`) is `SlotName == "first"` and
/// routes to `TOXIC_MOVE` (IL_00a4), `b__17_1` (`0xba336`) is `SlotName ==
/// "second"` and routes to `SUCK_MOVE` (IL_00b8). The oracle writes those as
/// the bare rotation indices `(0, 2)`; they are read here by move name.
pub fn build_mytes_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTS: [&str; 2] = ["TOXIC_MOVE", "SUCK_MOVE"];
    let kind = MonsterKind::Myte;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(STARTS.len());
    for (slot, start) in STARTS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(kind, hp)
                .slot(slot as i32)
                .loop_pos(loop_position(kind, start)?),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.CULTISTS_NORMAL` — `normal.py::_cultists`.
///
/// `CultistsNormal::GenerateMonsters` (`0xd3615`) creates a Calcified Cultist
/// then a Damp Cultist, both with null slots, so creation order is roster order
/// and `Niche` roll order. The two bands are disjoint, so each rolls against
/// an empty uniqueness set — as the oracle passes them. Both machines open on
/// `INCANTATION_MOVE`, their rotation's first position.
pub fn build_cultists_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![
        rolled(ctx, MonsterKind::CalcifiedCultist, &[])?.slot(0),
        rolled(ctx, MonsterKind::DampCultist, &[])?.slot(1),
    ])
}

/// `ENCOUNTER.INKLETS_NORMAL` — `normal.py::_inklets_normal`.
///
/// Three inklets, no Encounter-stream draw. `InkletsNormal::GenerateMonsters`
/// (`0xd3fd4`) creates all three, then sets `Inklet::set_MiddleInklet(true)` on
/// the second only (IL_003c-IL_003e). `Inklet::GenerateMoveStateMachine`
/// (`0xb6d30`) opens on `_middleInklet ? WHIRLWIND_MOVE : JAB_MOVE`
/// (IL_012f-IL_013b) — a concrete state, so the opener is used as-is and
/// logged like a rolled move, with no turn-1 `MonsterAi` draw. One shared
/// uniqueness set; each carries `SlipperyPower`.
pub fn build_inklets_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTERS: [(&str, &[&str]); 3] = [
        ("JAB_MOVE", &["JAB_MOVE"]),
        ("WHIRLWIND_MOVE", &["WHIRLWIND_MOVE"]),
        ("JAB_MOVE", &["JAB_MOVE"]),
    ];
    let kind = MonsterKind::Inklet;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let slippery = initial_power(kind, "SlipperyPower", ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(STARTERS.len());
    for (slot, (opener, log)) in STARTERS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(kind, hp)
                .slot(slot as i32)
                .opening_move(opener, log)
                .with_state("slippery", slippery),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.OVICOPTER_NORMAL` — `normal.py::_ovicopter`.
///
/// `OvicopterNormal::get_Slots` (`0xd4643`) is `egg1`..`egg5` then
/// `ovicopter`, and `GenerateMonsters` (`0xd46cc`) creates only the parent,
/// into `ovicopter` — the last slot, which is `OVICOPTER_SLOTS - 1`. The eggs
/// are laid mid-fight by the `LAY_EGGS_MOVE` opener, which is engine.
pub fn build_ovicopter_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let parent_slot = (constants::OVICOPTER_SLOTS - 1) as i32;
    Ok(vec![
        rolled(ctx, MonsterKind::Ovicopter, &[])?.slot(parent_slot),
    ])
}

/// `ENCOUNTER.TWO_TAILED_RATS_NORMAL` — `normal.py::_two_tailed_rats`.
///
/// `TwoTailedRatsNormal::GenerateMonsters` (`0xd5a50`), in body order:
///
/// ```text
/// IL_000c..IL_003b  three Monster<TwoTailedRat> ToMutable creations -> loc0, loc1, loc2
/// IL_003d: call     EncounterModel::get_Rng
/// IL_0042: ldc.i4.3
/// IL_0043: callvirt Rng::NextInt                      // ONE draw, 0..3
/// IL_004b: loc0.StarterMoveIndex = k
/// IL_0056: loc1.StarterMoveIndex = (k + 1) % 3
/// IL_0061: loc2.StarterMoveIndex = (k + 2) % 3
/// ```
///
/// and `TwoTailedRat::GenerateMoveStateMachine` (`0xc4010`) resolves the
/// initial state as `StarterMoveIndex % 3` over `SCRATCH_MOVE`,
/// `DISEASE_BITE_MOVE`, `SCREECH_MOVE` (IL_016e-IL_0192) — the three
/// non-summon states, in the order the machine declares them. Every one is a
/// concrete state, so the opener is used as-is and logged like a rolled move.
/// The `3` in the draw bound, both moduli and the initial-state switch is the
/// size of that starter set, so it is [`RAT_STARTERS`]' length rather than a
/// literal; each name is checked against the generated random-AI move table.
///
/// `TwoTailedRat::.ctor` (`0xc445f`) initialises `_turnsUntilSummonable` to 2
/// (IL_0009-IL_000a) — the counter `CanSummon` reads — which `combat_sim`
/// carries as `Monster.tus`; it is read from the generated
/// `MONSTER_CTOR_INTS`.
///
/// The slots are the oracle's `0, 1, 2`. The native puts the three rats in
/// `get_Slots` (`0xd59f2`) positions 2, 3, 4 (`third`..`fifth`, IL_0075,
/// IL_008e, IL_00a7); parity here is with `make_monsters`, and the difference
/// is recorded in the family walk rather than resolved in a content PR.
pub fn build_two_tailed_rats_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::TwoTailedRat;
    let starters = rat_starters()?;
    let count = starters.len() as i32;
    let turns_until_summonable = ctor_int(kind, "_turnsUntilSummonable")?;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let k = ctx.encounter_next_int("Encounter-stream starter roll", 0, count)?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(starters.len());
    for slot in 0..count {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        let starter = starters[((k + slot) % count) as usize];
        roster.push(
            MonsterSpec::new(kind, hp)
                .slot(slot)
                .opening_move(starter.0, starter.1)
                .with_state("tus", turns_until_summonable),
        );
    }
    Ok(roster)
}

/// The Two-Tailed Rat's starter set, in `GenerateMoveStateMachine` order,
/// each with the one-entry move log its opener writes. Spelled as the oracle's
/// `RAT_ORDER` spells them (the random-AI table's names).
const RAT_STARTERS: [(&str, &[&str]); 3] = [
    ("SCRATCH", &["SCRATCH"]),
    ("DISEASE_BITE", &["DISEASE_BITE"]),
    ("SCREECH", &["SCREECH"]),
];

/// [`RAT_STARTERS`], each name confirmed to be a move of the rat's generated
/// random-AI table — a name that does not resolve is a codegen/content
/// disagreement and refuses rather than opening on a move the engine does not
/// have.
fn rat_starters() -> Result<&'static [(&'static str, &'static [&'static str])], RosterRefusal> {
    let kind = MonsterKind::TwoTailedRat;
    let moves = crate::content_tables::random_moves(kind)
        .ok_or(RosterRefusal::MonsterLoopAbsent { kind })?;
    for (name, _) in RAT_STARTERS {
        if !moves.iter().any(|entry| entry.name == name) {
            return Err(RosterRefusal::MonsterLoopMoveAbsent {
                kind,
                move_name: name,
            });
        }
    }
    Ok(&RAT_STARTERS)
}

/// `ENCOUNTER.THE_OBSCURA_NORMAL` — `normal.py::_the_obscura`.
///
/// `TheObscuraNormal::get_Slots` (`0xd57a9`) is `illusion`, `obscura`, and
/// `GenerateMonsters` (`0xd57e6`) creates only the primary, into `obscura`
/// (IL_000b). The Parafright is summoned into `illusion` by the
/// `ILLUSION_MOVE` opener, which is engine. Fixed band.
pub fn build_the_obscura_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const SLOTS: [&str; 2] = ["illusion", "obscura"];
    const ILLUSION: &str = "ILLUSION_MOVE";
    Ok(vec![
        fixed(ctx, MonsterKind::TheObscura)?
            .slot(slot_named(&SLOTS, "obscura"))
            .opening_move(ILLUSION, &[ILLUSION]),
    ])
}

/// `ENCOUNTER.GREMLIN_MERC_NORMAL` — `normal.py::_gremlin_merc`.
///
/// `GremlinMercNormal::GenerateMonsters` (`0xd3e94`) creates only the merc,
/// into slot `merc`; the Sneaky and Fat gremlins do not exist until
/// `SurprisePower` fires at its death, which is engine. One `Niche` roll over
/// its band.
pub fn build_gremlin_merc_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::GremlinMerc, &[])?.slot(0)])
}

/// `ENCOUNTER.CORPSE_SLUGS_NORMAL` — `normal.py::_corpse_slugs_normal`.
///
/// `CorpseSlugsNormal::GenerateMonsters` (`0xd3464`) is the weak body with a
/// third creation: three null-slot slugs, then
/// `CorpseSlug::EnsureCorpseSlugsStartWithDifferentMoves` with the
/// encounter's own `Rng` (IL_00a9-IL_00af). The oracle delegates to
/// `weak.build_corpse_slugs(ctx, 3)`, and so does this: the Encounter-stream
/// draw lives in F1's primitive.
pub fn build_corpse_slugs_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    super::weak::corpse_slugs(ctx, 3)
}

/// `ENCOUNTER.HUNTER_KILLER_NORMAL` — `normal.py::_hunter_killer`.
///
/// One default-slot Hunter Killer (`HunterKillerNormal::GenerateMonsters`
/// `0xd3f67`), fixed band — still one `Niche` draw. Its machine's initial
/// state is the concrete `TENDERIZING_GOOP_MOVE`, used as-is and logged.
pub fn build_hunter_killer_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const GOOP: &str = "TENDERIZING_GOOP_MOVE";
    Ok(vec![
        fixed(ctx, MonsterKind::HunterKiller)?.opening_move(GOOP, &[GOOP]),
    ])
}

/// `ENCOUNTER.EXOSKELETONS_NORMAL` — `normal.py::_exoskeletons_normal`.
///
/// Four exoskeletons in the named slots `first`..`fourth`
/// (`ExoskeletonsNormal::GenerateMonsters` `0xd390c`, `get_Slots` `0xd38cd`),
/// one shared uniqueness set, each carrying `HardToKillPower`. Slots 0-2 open
/// on the concrete state their INIT node resolves to, exactly as in
/// [`super::weak::build_exoskeletons_weak`]. The fourth slot has its own INIT
/// branch: `Exoskeleton::GenerateMoveStateMachine` (`0xb3658`) adds
/// `<GenerateMoveStateMachine>b__22_3` (`0xb39c5`, `SlotName == "fourth"`)
/// routing to the `RAND` `RandomBranchState` (IL_0103-IL_0113), so its opener
/// is a turn-1 `MonsterAi` roll the engine owns (#2528). Its `next_move` stays
/// empty here, as the oracle leaves it.
pub fn build_exoskeletons_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTERS: [Option<(&str, &[&str])>; 4] = [
        Some(("SKITTER_MOVE", &["SKITTER_MOVE"])),
        Some(("MANDIBLES_MOVE", &["MANDIBLES_MOVE"])),
        Some(("ENRAGE_MOVE", &["ENRAGE_MOVE"])),
        None,
    ];
    let kind = MonsterKind::Exoskeleton;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let hard_to_kill = initial_power(kind, "HardToKillPower", ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(STARTERS.len());
    for (slot, starter) in STARTERS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        let mut spec = MonsterSpec::new(kind, hp).slot(slot as i32);
        if let Some((opener, log)) = starter {
            spec = spec.opening_move(opener, log);
        }
        roster.push(spec.with_state("hard_to_kill", hard_to_kill));
    }
    Ok(roster)
}

/// `ENCOUNTER.CUBEX_CONSTRUCT_NORMAL` — `normal.py::_cubex_construct`.
///
/// One construct (`CubexConstructNormal::GenerateMonsters` `0xd35ec`), fixed
/// band, spawning with `ArtifactPower` 1 and **no Block**.
///
/// `CubexConstruct/<AfterAddedToRoom>d__27::MoveNext` (`0x357a60`) does call
/// `CreatureCmd::GainBlock` for a flat 13 (`ldc.i4.s 13` at IL_008d, call at
/// IL_0097) before `Apply<ArtifactPower>` (IL_00fd-IL_010a), but that Block is
/// a no-op in the game (#3048). The hook runs from
/// `CombatManager/<StartCombatInternal>d__98::MoveNext` (`0x3f71b0`) via
/// `CombatManager::AfterCreatureAdded` at IL_0157, which awaits
/// `Creature::AfterAddedToRoom` (`<AfterCreatureAdded>d__113` IL_001c) —
/// before `CombatTurnState::set_IsInProgress(true)` at IL_020f.
/// `CreatureCmd/<GainBlock>d__18::MoveNext` (`0x3eaec0`) returns 0 at
/// IL_002d-IL_0041 when `CombatManager::get_IsOverOrEnding` (`0x1358be`) is
/// true, and that getter is `IsEnding || !IsInProgress` (IL_0002-IL_0010), so
/// it is true for the whole pre-combat hook. `PowerCmd::Apply` carries no such
/// gate: the Artifact lands (captures show `ARTIFACT_POWER` 1 and 0 Block).
/// The pool's generated `CUBEX_CONSTRUCT_BLOCK` is therefore not read.
pub fn build_cubex_construct_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::CubexConstruct;
    let artifact = initial_power(kind, "ArtifactPower", ctx.ascension())?;
    Ok(vec![fixed(ctx, kind)?.with_state("artifact", artifact)])
}

/// `ENCOUNTER.FOSSIL_STALKER_NORMAL` — `normal.py::_fossil_stalker`.
///
/// One stalker (`FossilStalkerNormal::GenerateMonsters` `0xd3da4`), rolled
/// over its band, carrying `SuckPower`, opening on the concrete
/// `LATCH_MOVE`.
pub fn build_fossil_stalker_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const LATCH: &str = "LATCH_MOVE";
    let kind = MonsterKind::FossilStalker;
    let suck = initial_power(kind, "SuckPower", ctx.ascension())?;
    Ok(vec![
        rolled(ctx, kind, &[])?
            .slot(0)
            .with_state("suck", suck)
            .opening_move(LATCH, &[LATCH]),
    ])
}

/// `ENCOUNTER.MAWLER_NORMAL` — `normal.py::_mawler`.
///
/// One Mawler (`MawlerNormal::GenerateMonsters` `0xd43ad`), fixed band, opening
/// on the concrete `CLAW_MOVE`.
pub fn build_mawler_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const CLAW: &str = "CLAW_MOVE";
    Ok(vec![
        fixed(ctx, MonsterKind::Mawler)?.opening_move(CLAW, &[CLAW]),
    ])
}

/// One creation whose band is a real range: one `Niche` draw over the band
/// minus `taken`, then the spec. (`weak.rs` has the same private helper; it is
/// three lines, and reaching into another family's module for it would be a
/// shared surface for nothing.)
fn rolled(
    ctx: &mut dyn EncounterCtx,
    kind: MonsterKind,
    taken: &[i32],
) -> Result<MonsterSpec, RosterRefusal> {
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let hp = ctx.unique_hp(lo, hi, taken);
    Ok(MonsterSpec::new(kind, hp))
}

/// The index of `name` in an encounter's `get_Slots` list — a slot index read
/// by name, for the reason [`loop_position`] reads a rotation index by name.
fn slot_named(slots: &[&str], name: &str) -> i32 {
    slots
        .iter()
        .position(|slot| *slot == name)
        .expect("the slot name is one of the listed slots") as i32
}

/// One integer a monster class's `.ctor` stores into its own field before
/// calling `MonsterModel::.ctor`, from the generated `MONSTER_CTOR_INTS`.
///
/// A field the table does not carry refuses as a missing model fact rather
/// than defaulting, since the whole point of reading it is that the default
/// (`0`) is wrong.
fn ctor_int(kind: MonsterKind, field: &'static str) -> Result<i64, RosterRefusal> {
    crate::content_tables::monster_ctor_int(kind, field).ok_or(RosterRefusal::MonsterModelUnread {
        kind,
        why: "the generated MONSTER_CTOR_INTS carries no such constructor field",
    })
}

/// `ENCOUNTER.SCROLLS_OF_BITING_WEAK` — `normal.py::_scrolls_of_biting`.
///
/// Three scrolls. `ScrollsOfBitingWeak::GenerateMonsters` (`0xd4c78`), in body
/// order:
///
/// ```text
/// IL_000c..IL_003b  three Monster<ScrollOfBiting> ToMutable creations -> loc0, loc1, loc2
/// IL_003d: call     EncounterModel::get_Rng
/// IL_0042: ldc.i4.3
/// IL_0043: callvirt Rng::NextInt                      // ONE draw, 0..3
/// IL_0049: loc0.StarterMoveIdx = start
/// IL_0050: loc1.StarterMoveIdx = (start + 1) % 3
/// IL_005b: loc2.StarterMoveIdx = (start + 2) % 3
/// IL_0066: newarr   [ (loc0, null), (loc1, null), (loc2, null) ]
/// ```
///
/// The slots are `ldnull`, so creation order is the roster's only order.
pub fn build_scrolls_of_biting_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    scrolls_of_biting(ctx, 3)
}

/// `ENCOUNTER.SCROLLS_OF_BITING_NORMAL` — the same `normal.py` builder, which
/// branches on `ctx.encounter_id`.
///
/// Four scrolls — **not** an up-to-four roster.
/// `ScrollsOfBitingNormal::GenerateMonsters` (`0xd4b90`) is the weak body with
/// one more creation, and the extra scroll's starting position is a constant
/// rather than a fourth rotation offset:
///
/// ```text
/// IL_000c..IL_004b  FOUR creations -> loc0, loc1, loc2, loc3
/// IL_004d: call     EncounterModel::get_Rng
/// IL_0052: ldc.i4.3
/// IL_0053: callvirt Rng::NextInt                      // still ONE draw, 0..3
/// IL_005a: loc0.StarterMoveIdx = start
/// IL_0062: loc1.StarterMoveIdx = (start + 1) % 3
/// IL_006e: loc2.StarterMoveIdx = (start + 2) % 3
/// IL_007a: ldc.i4.2; loc3.StarterMoveIdx = 2          // fixed, not (start + 3) % 3
/// IL_0081: newarr   [ (loc0, null), … , (loc3, null) ]
/// ```
///
/// A fourth *offset* is not available to it: the rotation has three positions,
/// so `(start + 3) % 3` would collide with the first scroll. The native picks
/// the constant instead, and [`scrolls_of_biting`] reads that constant back out
/// of the generated rotation by move name.
pub fn build_scrolls_of_biting_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    scrolls_of_biting(ctx, 4)
}

/// The shared Scroll of Biting roster: `count` scrolls off one Encounter draw.
///
/// # The rotation, and why `MORE_TEETH` is a name here and a `2` in the oracle
///
/// `ScrollOfBiting::GenerateMoveStateMachine` (`0xbcde4`) adds `CHOMP`, `CHEW`
/// and `MORE_TEETH` in that order, then selects the machine's initial state:
///
/// ```text
/// IL_00df: call     ScrollOfBiting::get_StarterMoveIdx
/// IL_00e5: ldc.i4.3; rem                              // idx % 3 — the rotation length
/// IL_00e9: brfalse  IL_00f4 -> loc1  (CHOMP)
/// IL_00ef: beq.s    IL_00fc -> loc2  (CHEW)
/// IL_00f2: br.s     IL_0104 -> loc3  (MORE_TEETH)
/// ```
///
/// So `StarterMoveIdx` indexes that rotation, 0/1/2 = `CHOMP`/`CHEW`/
/// `MORE_TEETH`, and the fourth scroll's native `ldc.i4.2` is `MORE_TEETH`. The
/// oracle carries it as a bare `2`; reading it by name out of
/// `content_tables::LOOPS` is what keeps the index out of the Rust source
/// (PORT_PLAN §5), exactly as [`super::weak::build_toadpoles_weak`] does.
///
/// The same literal `3` is the draw bound, the modulus, **and** the number of
/// scrolls that get a distinct offset, because all three are the rotation's
/// length — so it is read once from the generated rotation rather than typed in
/// three times. `count` itself is structural: it is how many `ToMutable`
/// creations each `GenerateMonsters` body contains, and it is the one thing the
/// two ids differ by.
///
/// # Draw order
///
/// The native creates every scroll first (each creation rolls its HP through
/// `Creature::SetUniqueMonsterHpValue`) and draws the starter offset
/// afterwards; the oracle draws first and rolls after. The two streams are
/// disjoint — the offset comes from the per-fight `Encounter` stream and the HP
/// rolls from `Niche` — so the observable is identical, and this mirrors the
/// oracle because parity with `make_monsters` is what E4b measures. It is the
/// same shape as [`super::weak::corpse_slugs`].
///
/// # One shared uniqueness set
///
/// Every scroll is the same kind and therefore shares one band (33-39 at A8+),
/// so each creation excludes its predecessors' rolled max HP. With four scrolls
/// and a seven-value band the set never empties.
///
/// # `PaperCutsPower` is deliberately not roster state
///
/// `ScrollOfBiting/<AfterAddedToRoom>d__26::MoveNext` (`0x368628`) applies
/// `Apply<PaperCutsPower>` with amount 2 at IL_0098, and
/// `content_tables::MONSTER_MODELS` records it. It is still not carried here,
/// for the reason `weak.rs` leaves `BowlbugRock`'s `ImbalancedPower` out: it is
/// not roster state in the oracle either. `combat_sim` models Paper Cuts as
/// behaviour keyed on the kind — the `m.kind == SCROLL_OF_BITING and hp_lost >
/// 0` retaliation in `combat_sim.py` — and `Monster` has no field for it, so
/// `make_monsters` sets nothing. Parity here is with `make_monsters`' output.
fn scrolls_of_biting(
    ctx: &mut dyn EncounterCtx,
    count: usize,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const TAIL_START: &str = "MORE_TEETH";
    let kind = MonsterKind::ScrollOfBiting;
    let rotation = rotation_len(kind)?;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    // The scrolls beyond the rotation's length share the native's constant.
    let tail = loop_position(kind, TAIL_START)?;
    let start = ctx.encounter_next_int("Encounter-stream starter roll", 0, rotation)?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(count);
    for slot in 0..count as i32 {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        let loop_pos = if slot < rotation {
            (start + slot) % rotation
        } else {
            tail
        };
        roster.push(MonsterSpec::new(kind, hp).slot(slot).loop_pos(loop_pos));
    }
    Ok(roster)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Xoshiro256StarStar;

    /// A context with a live `Niche` stream and no floor, so any
    /// Encounter-stream draw refuses.
    struct NoFloor(Xoshiro256StarStar);

    impl EncounterCtx for NoFloor {
        fn ascension(&self) -> u8 {
            crate::encounters::MODELED_ASCENSION
        }
        fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32 {
            self.0.next_bounded(hi - lo).expect("lo < hi") + lo
        }
        fn encounter_next_int(
            &mut self,
            roll: &'static str,
            _lo: i32,
            _hi: i32,
        ) -> Result<i32, RosterRefusal> {
            Err(RosterRefusal::EncounterStreamUnavailable { roll })
        }
    }

    /// `TwoTailedRat::.ctor` (`0xc445f`) IL_0009-IL_000a: the counter is read
    /// out of the generated table, and a field the table does not carry
    /// refuses rather than defaulting to `Monster.tus`'s `0`.
    #[test]
    fn the_rat_counter_is_the_generated_ctor_initialiser_and_an_unknown_field_refuses() {
        assert_eq!(
            ctor_int(MonsterKind::TwoTailedRat, "_turnsUntilSummonable"),
            Ok(2)
        );
        assert_eq!(
            ctor_int(MonsterKind::TwoTailedRat, "_noSuchField"),
            Err(RosterRefusal::MonsterModelUnread {
                kind: MonsterKind::TwoTailedRat,
                why: "the generated MONSTER_CTOR_INTS carries no such constructor field",
            })
        );
        // A kind whose constructor stores nothing has no row at all.
        assert_eq!(
            crate::content_tables::monster_ctor_int(MonsterKind::Myte, "_turnsUntilSummonable"),
            None
        );
    }

    /// Every rat starter is a move of the generated random-AI table, in the
    /// order `GenerateMoveStateMachine` declares them.
    #[test]
    fn the_rat_starters_are_the_first_three_random_ai_moves() {
        let moves = crate::content_tables::random_moves(MonsterKind::TwoTailedRat)
            .expect("the rat is random-AI");
        let starters = rat_starters().expect("every starter resolves");
        for (index, (name, log)) in starters.iter().enumerate() {
            assert_eq!(moves[index].name, *name);
            assert_eq!(*log, &[*name][..]);
        }
    }

    /// The rats and the Corpse Slugs draw on the per-fight Encounter stream,
    /// so without a floor they refuse — before spending a single `Niche` draw.
    #[test]
    fn the_encounter_stream_builders_refuse_without_a_floor() {
        for build in [build_two_tailed_rats_normal, build_corpse_slugs_normal] {
            let mut ctx = NoFloor(Xoshiro256StarStar::from_seed(1));
            assert!(matches!(
                build(&mut ctx),
                Err(RosterRefusal::EncounterStreamUnavailable { .. })
            ));
            assert_eq!(ctx.0.counter, 0, "no Niche draw before the refusal");
        }
    }

    /// #3048: every Cubex Construct spawns with its `ArtifactPower` and no
    /// Block, alone and in the Construct Menagerie. The `AfterAddedToRoom`
    /// `GainBlock(13)` (`0x357a60` IL_0097) runs before
    /// `CombatTurnState::set_IsInProgress(true)` and
    /// `<GainBlock>d__18::MoveNext` (`0x3eaec0`) returns on
    /// `IsOverOrEnding` at IL_002d-IL_0041.
    #[test]
    fn a_cubex_construct_spawns_with_artifact_and_no_block() {
        let solo = build_cubex_construct_normal(&mut NoFloor(Xoshiro256StarStar::from_seed(1)))
            .expect("the solo roster builds");
        let menagerie = crate::encounters::normal_c::build_construct_menagerie_normal(
            &mut NoFloor(Xoshiro256StarStar::from_seed(1)),
        )
        .expect("the menagerie roster builds");
        let cubexes: Vec<_> = solo
            .iter()
            .chain(&menagerie)
            .filter(|spec| spec.kind == MonsterKind::CubexConstruct)
            .collect();
        assert_eq!(cubexes.len(), 3, "one solo Cubex and two in the menagerie");
        for spec in cubexes {
            assert!(
                spec.initial_state
                    .iter()
                    .all(|(field, _)| *field != "block"),
                "a Cubex never keeps its pre-combat Block: {:?}",
                spec.initial_state
            );
            assert!(
                spec.initial_state
                    .iter()
                    .any(|(field, _)| *field == "artifact")
            );
        }
    }

    /// `TheObscuraNormal::get_Slots` (`0xd57a9`) lists `illusion` first, so
    /// the primary's slot is the second position, read by name.
    #[test]
    fn the_obscura_takes_the_obscura_slot() {
        let mut ctx = NoFloor(Xoshiro256StarStar::from_seed(1));
        let roster = build_the_obscura_normal(&mut ctx).expect("no Encounter draw");
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].slot, 1);
        assert_eq!(ctx.0.counter, 1, "a fixed band still spends its Niche draw");
    }
}

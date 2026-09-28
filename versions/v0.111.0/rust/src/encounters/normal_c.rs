//! Normal-pool encounter rosters, demand rank 31-44 — E4b family F6 (#2536).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/normal.py`, builder
//! for builder, under the same rules as [`super::weak`], [`super::elite`] and
//! [`super::normal_a`]: every number in a roster comes from a generated table
//! (`content_tables::MONSTER_MODELS` for HP bands and spawn-time power amounts,
//! `content_tables::LOOPS` for rotation positions looked up **by move name**,
//! `content_tables::encounter_pool_constants::normal` for the one flat
//! spawn-time amount the assembly reader does not carry), and the control flow
//! around them is hand-ported with current-build IL citations (PORT_PLAN §5).
//!
//! Thirteen of the fourteen ids in #2536. The fourteenth,
//! `SCROLLS_OF_BITING_NORMAL`, was pulled forward and lives in
//! [`super::normal_a`] beside its `_WEAK` sibling, because the two share one
//! Python builder (`_scrolls_of_biting`); it is not repeated here.
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs (v0.111.0 `sts2.dll` sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`, read
//! with `versions/v0.111.0/solver/tools/dump_il.py`):
//!
//! | class | `GenerateMonsters` | note |
//! |---|---|---|
//! | `GlobeHeadNormal` | `0xd3e4a` | one creation, null slot |
//! | `TheLostAndForgottenNormal` | `0xd5763` | `TheLost` then `TheForgotten`, null slots |
//! | `FlyconidNormal` | `0xd3cbc` | `NextItem(_mediumSlimes)` then `Flyconid`; `.cctor` `0xd3d15` |
//! | `SlitheringStranglerNormal` | `0xd519c` | `NextItem(GetValues<SecondaryEnemyType>)` switch; `.cctor` `0xd52d4` |
//! | `VineShamblerNormal` | `0xd5ba9` | one creation, null slot |
//! | `ConstructMenagerieNormal` | `0xd33e4` | `PunchConstruct`, `CubexConstruct` x2, null slots |
//! | `FabricatorNormal` | `0xd3b58` | one creation in slot `fabricator` (`get_Slots` `0xd3a53`) |
//! | `NibbitsNormal` | `0xd4510` | `set_IsFront(true)` `front`, then `back` (`get_Slots` `0xd44de`) |
//! | `OwlMagistrateNormal` | `0xd4711` | one creation, null slot |
//! | `FogmogNormal` | `0xd3d72` | one creation in slot `fogmog` (`get_Slots` `0xd3d35`) |
//! | `LivingFogNormal` | `0xd434d` | one creation in slot `livingFog` (`get_Slots` `0xd42db`) |
//! | `SlimedBerserkerNormal` | `0xd4e66` | one creation, null slot |
//! | `TunnelerNormal` | `0xd58f4` | `Chomper` with `set_ScreamFirst(true)`, then `Tunneler` |
//!
//! None of these bodies draws from the per-fight `Encounter` stream except
//! `FlyconidNormal` and `SlitheringStranglerNormal`; every creation spends one
//! `Niche` draw through `Creature::SetUniqueMonsterHpValue`, fixed bands
//! included.
//!
//! # Slots: where the oracle pins a number the IL spells as a name
//!
//! `make_monsters` sets `Monster.slot` on most of these rosters. Where the
//! native passes `null` the slot is the creation index, as in every other
//! family. Three bodies instead pass a slot **name** (`'fabricator'`,
//! `'fogmog'`, `'livingFog'`), and the oracle carries that name's position in
//! the encounter's `get_Slots` array as a bare integer (`slot=2`, `slot=1`,
//! `FOG_BOMB_SLOTS`). The position is read here from the IL's own slot-name
//! list by name, the same way [`super::loop_position`] reads a rotation
//! index, so no index is typed into this file.

use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position,
};
use crate::ids::MonsterKind;

/// `ENCOUNTER.GLOBE_HEAD_NORMAL` — `normal.py::_globe_head`.
///
/// One Globe Head on its single-value band, opening at the rotation's first
/// state. `GlobeHead`'s `AfterAddedToRoom` applies `GalvanicPower`, and
/// `MONSTER_MODELS` records it; it is not roster state, because the engine
/// reads it as a per-card-play hook keyed on the kind
/// (`play.rs::apply_globe_head_galvanic_after_card_played`, #3300) and
/// `HotMonster` has no field for it — so `make_monsters` sets nothing, exactly
/// as `normal_a.rs` leaves the Scroll's `PaperCutsPower` out.
pub fn build_globe_head_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::GlobeHead)?])
}

/// `ENCOUNTER.VINE_SHAMBLER_NORMAL` — `normal.py::_vine_shambler`.
pub fn build_vine_shambler_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::VineShambler)?])
}

/// `ENCOUNTER.OWL_MAGISTRATE_NORMAL` — `normal.py::_owl_magistrate`.
pub fn build_owl_magistrate_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::OwlMagistrate)?])
}

/// `ENCOUNTER.SLIMED_BERSERKER_NORMAL` — `normal.py::_slimed_berserker`.
pub fn build_slimed_berserker_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::SlimedBerserker)?])
}

/// `FabricatorNormal::get_Slots` (`0xd3a53`), IL_0009-IL_0029, in array order.
const FABRICATOR_SLOTS: [&str; 5] = ["bot1", "bot2", "fabricator", "bot3", "bot4"];

/// `ENCOUNTER.FABRICATOR_NORMAL` — `normal.py::_fabricator`.
///
/// `FabricatorNormal::GenerateMonsters` (`0xd3b58`) creates only the
/// Fabricator, in the slot named `'fabricator'` (IL_000b); the four bot slots
/// around it are filled mid-fight. The oracle rolls the single-value band
/// through `unique_hp(FABRICATOR_HP, FABRICATOR_HP, set())`, which is the same
/// one `Niche` draw [`fixed`] spends.
pub fn build_fabricator_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let slot = named_slot(&FABRICATOR_SLOTS, "fabricator");
    Ok(vec![fixed(ctx, MonsterKind::Fabricator)?.slot(slot)])
}

/// `FogmogNormal::get_Slots` (`0xd3d35`), IL_0009-IL_0011, in array order.
const FOGMOG_SLOTS: [&str; 2] = ["illusion", "fogmog"];

/// `ENCOUNTER.FOGMOG_NORMAL` — `normal.py::_fogmog`.
///
/// `FogmogNormal::GenerateMonsters` (`0xd3d72`) creates only the Fogmog, in
/// the slot named `'fogmog'` (IL_000b). The Eye With Teeth in the `illusion`
/// slot is summoned by the opener, which is engine behaviour, not roster.
pub fn build_fogmog_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let slot = named_slot(&FOGMOG_SLOTS, "fogmog");
    Ok(vec![fixed(ctx, MonsterKind::Fogmog)?.slot(slot)])
}

/// `LivingFogNormal::get_Slots` (`0xd42db`), IL_0009-IL_0031, in array order.
const LIVING_FOG_SLOTS: [&str; 6] = ["bomb1", "bomb2", "bomb3", "bomb4", "bomb5", "livingFog"];

/// `ENCOUNTER.LIVING_FOG_NORMAL` — `normal.py::_living_fog`.
///
/// `LivingFogNormal::GenerateMonsters` (`0xd434d`) creates only the Living
/// Fog, in the slot named `'livingFog'` (IL_000b), after the five bomb slots
/// the mid-fight Gas Bombs are inserted into. The oracle writes that position
/// as `combat_sim.FOG_BOMB_SLOTS` (lifted into
/// `encounter_pool_constants::normal`), because the engine's bomb spawn reads
/// the same number; the unit test below pins that the name lookup and the
/// lifted constant agree.
pub fn build_living_fog_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let slot = named_slot(&LIVING_FOG_SLOTS, "livingFog");
    Ok(vec![fixed(ctx, MonsterKind::LivingFog)?.slot(slot)])
}

/// `ENCOUNTER.THE_LOST_AND_FORGOTTEN_NORMAL` —
/// `normal.py::_the_lost_and_forgotten`.
///
/// `TheLostAndForgottenNormal::GenerateMonsters` (`0xd5763`) creates The Lost
/// (IL_0009) then The Forgotten (IL_0020), both with null slots, so creation
/// order is slot order and `Niche` roll order. Both bands are single values;
/// the oracle rolls the second against a uniqueness set holding the first's
/// HP, which the two disjoint single values can never hit, and each creation
/// still spends its one draw.
///
/// Both kinds' `AfterAddedToRoom` apply their `Possess*Power` (recorded in
/// `MONSTER_MODELS`); neither is roster state in the oracle — the debit and
/// restoration graph lives in `combat_sim`, keyed on the kinds.
pub fn build_the_lost_and_forgotten_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let lost = rolled(ctx, MonsterKind::TheLost, &[])?.slot(0);
    let forgotten = rolled(ctx, MonsterKind::TheForgotten, &[lost.hp])?.slot(1);
    Ok(vec![lost, forgotten])
}

/// `ENCOUNTER.NIBBITS_NORMAL` — `normal.py::_nibbits_normal`.
///
/// `NibbitsNormal::GenerateMonsters` (`0xd4510`) creates the front Nibbit
/// with `set_IsFront(true)` (IL_001e) in slot `'front'`, then a second one in
/// slot `'back'`; `IsAlone` is never set. So each takes the `!IsAlone` arm of
/// the INIT `ConditionalBranchState` in `Nibbit::GenerateMoveStateMachine`
/// (`0xba3cc`): IL_00bf adds `HISS_MOVE` under `b__25_1` (`0xba68b`,
/// `!IsFront`) and IL_00d3 adds `SLICE_MOVE` under `b__25_2` (`0xba6a5`,
/// `IsFront`). Front opens on `SLICE`, back on `HISS`, both read out of the
/// generated rotation by name. One band, so one shared uniqueness set.
pub fn build_nibbits_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTS: [&str; 2] = ["SLICE", "HISS"];
    let kind = MonsterKind::Nibbit;
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

/// `ENCOUNTER.TUNNELER_NORMAL` — `normal.py::_tunneler_normal`.
///
/// `TunnelerNormal::GenerateMonsters` (`0xd58f4`) creates the Chomper first
/// with `set_ScreamFirst(true)` (IL_001e), then the Tunneler, both with null
/// slots. `Chomper::GenerateMoveStateMachine` (`0xb14f0`) reads
/// `_screamFirst` at IL_0080 and starts the machine on `SCREECH_MOVE` when it
/// is set (IL_008a), so the Chomper's position is `SCREECH_MOVE`'s, read by
/// name. The Chomper carries `ArtifactPower` from its `AfterAddedToRoom`
/// (`<AfterAddedToRoom>d__15::MoveNext` `0x356460`, `Apply<ArtifactPower>` at
/// IL_0098). The bands are disjoint, so each creation rolls against its own
/// empty set; the Tunneler's single-value band still spends its draw.
pub fn build_tunneler_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let chomper = MonsterKind::Chomper;
    let artifact = initial_power(chomper, "ArtifactPower", ctx.ascension())?;
    let screech = loop_position(chomper, "SCREECH_MOVE")?;
    let first = rolled(ctx, chomper, &[])?
        .loop_pos(screech)
        .slot(0)
        .with_state("artifact", artifact);
    let second = fixed(ctx, MonsterKind::Tunneler)?.slot(1);
    Ok(vec![first, second])
}

/// `ENCOUNTER.CONSTRUCT_MENAGERIE_NORMAL` — `normal.py::_construct_menagerie`.
///
/// `ConstructMenagerieNormal::GenerateMonsters` (`0xd33e4`) is a fixed
/// three-element array with no `Rng` call: `PunchConstruct` (IL_0014), then
/// `CubexConstruct` twice (IL_002b, IL_0042), null slots — creation order is
/// slot order and `Niche` roll order, one draw each on single-value bands.
///
/// Spawn-time state, in `combat_sim.Monster` field order:
///
/// * Punch Construct — `ArtifactPower` from
///   `PunchConstruct/<AfterAddedToRoom>d__24::MoveNext` (`0x366c04`,
///   `Apply<ArtifactPower>` at IL_0097).
/// * Cubex Construct — `Apply<ArtifactPower>` (IL_010a) in
///   `CubexConstruct/<AfterAddedToRoom>d__27::MoveNext` (`0x357a60`), and
///   **no Block**: that hook's `CreatureCmd::GainBlock` of 13 (IL_008d-IL_0097)
///   runs before `CombatTurnState::set_IsInProgress(true)`
///   (`<StartCombatInternal>d__98` IL_0157 vs IL_020f), so
///   `<GainBlock>d__18::MoveNext` (`0x3eaec0`) returns at IL_002d-IL_0041 on
///   `CombatManager::get_IsOverOrEnding` (`0x1358be`, `IsEnding ||
///   !IsInProgress`). See `normal_a::build_cubex_construct_normal` (#3048).
pub fn build_construct_menagerie_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const CUBEXES: usize = 2;
    let punch_artifact = initial_power(
        MonsterKind::PunchConstruct,
        "ArtifactPower",
        ctx.ascension(),
    )?;
    let cubex_artifact = initial_power(
        MonsterKind::CubexConstruct,
        "ArtifactPower",
        ctx.ascension(),
    )?;
    let mut roster = vec![
        fixed(ctx, MonsterKind::PunchConstruct)?
            .slot(0)
            .with_state("artifact", punch_artifact),
    ];
    for _ in 0..CUBEXES {
        let slot = roster.len() as i32;
        roster.push(
            fixed(ctx, MonsterKind::CubexConstruct)?
                .slot(slot)
                .with_state("artifact", cubex_artifact),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.FLYCONID_NORMAL` — `normal.py::_flyconid_normal`.
///
/// `FlyconidNormal::GenerateMonsters` (`0xd3cbc`):
///
/// ```text
/// IL_0015: call     EncounterModel::get_Rng
/// IL_001a: ldsfld   FlyconidNormal::_mediumSlimes      // [LeafSlimeM, TwigSlimeM]
/// IL_001f: callvirt NextItem<MonsterModel>             // ONE Encounter draw
/// IL_0024: ToMutable -> slot 0 (null)
/// IL_0036: Monster<Flyconid>.ToMutable -> slot 1 (null)
/// ```
///
/// The pool is the `.cctor`'s (`0xd3d15`), in declaration order. The two
/// bands are disjoint and the oracle rolls each against its own empty set.
/// Twig Slime M opens on its concrete `STICKY_SHOT_MOVE`, used as-is on turn
/// 1 and logged like a rolled move (as in [`super::weak::slimes`]); the
/// Flyconid's INIT node is its `RandomBranchState`, so its opener is a turn-1
/// `MonsterAi` roll the engine owns (#2528) and stays empty here.
pub fn build_flyconid_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const MEDIUM: [MonsterKind; 2] = [MonsterKind::LeafSlimeM, MonsterKind::TwigSlimeM];
    let pick =
        ctx.encounter_next_int("Encounter-stream medium-slime roll", 0, MEDIUM.len() as i32)?;
    let medium = with_opener(rolled(ctx, MEDIUM[pick as usize], &[])?).slot(0);
    let flyconid = rolled(ctx, MonsterKind::Flyconid, &[])?.slot(1);
    Ok(vec![medium, flyconid])
}

/// `SlitheringStranglerNormal/SecondaryEnemyType`, in the order
/// `GenerateMonsters`' `switch` (IL_001e) dispatches its three arms.
#[derive(Clone, Copy)]
enum Secondary {
    /// Arm 0 (IL_0034): one `SnappingJaxfruit`.
    Jaxfruit,
    /// Arm 1 (IL_0063): one `NextItem(_mediumSlimes)`.
    MediumSlime,
    /// Arm 2 (IL_009b): two independent `NextItem(_smallSlimes)`.
    SmallSlimes,
}

/// `ENCOUNTER.SLITHERING_STRANGLER_NORMAL` —
/// `normal.py::_slithering_strangler`.
///
/// `SlitheringStranglerNormal::GenerateMonsters` (`0xd519c`), in body order:
///
/// ```text
/// IL_000d: get_Rng; GetValues<SecondaryEnemyType>; NextItem   // draw 1: the arm
/// IL_001e: switch [arm 0, arm 1, arm 2]; default throws (IL_00f2)
///   arm 0 (IL_0034): [Monster<SnappingJaxfruit>]
///   arm 1 (IL_0063): [get_Rng.NextItem(_mediumSlimes)]         // draw 2
///   arm 2 (IL_009b): [get_Rng.NextItem(_smallSlimes),          // draw 2
///                     get_Rng.NextItem(_smallSlimes)]          // draw 3
/// IL_00fa: list.Add(Monster<SlitheringStrangler>)
/// IL_0125: Select(m => (m.ToMutable(), null)); ToList          // creation order
/// ```
///
/// The sub-pools are the `.cctor`'s (`0xd52d4`): `_smallSlimes` is
/// `[LeafSlimeS, TwigSlimeS]` and `_mediumSlimes` `[LeafSlimeM, TwigSlimeM]`.
/// The two small picks are independent — both draw over the whole pool, so a
/// pair of the same kind is possible. Every creation is `ToMutable`'d in list
/// order, so the Strangler is created last, and all of them share one
/// `SetUniqueMonsterHpValue` taken set (the small bands overlap).
///
/// Openers: Twig Slime M as in [`build_flyconid_normal`];
/// `SlitheringStrangler::GenerateMoveStateMachine` (`0xbdf78`) constructs the
/// machine on its first state (IL_00e8 `ldloc.1`, the `CONSTRICT` state built
/// at IL_0012), logged like a rolled move under the oracle's `CONSTRICT_MOVE`
/// name — the generated `SLITHERING_STRANGLER_MOVES` spelling.
pub fn build_slithering_strangler_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const ARMS: [Secondary; 3] = [
        Secondary::Jaxfruit,
        Secondary::MediumSlime,
        Secondary::SmallSlimes,
    ];
    const SMALL: [MonsterKind; 2] = [MonsterKind::LeafSlimeS, MonsterKind::TwigSlimeS];
    const MEDIUM: [MonsterKind; 2] = [MonsterKind::LeafSlimeM, MonsterKind::TwigSlimeM];
    const CONSTRICT: &str = "CONSTRICT_MOVE";
    let roll = "Encounter-stream Slithering Strangler roster rolls";
    let arm = ARMS[ctx.encounter_next_int(roll, 0, ARMS.len() as i32)? as usize];
    let mut kinds: Vec<MonsterKind> = match arm {
        Secondary::Jaxfruit => vec![MonsterKind::SnappingJaxfruit],
        Secondary::MediumSlime => {
            vec![MEDIUM[ctx.encounter_next_int(roll, 0, MEDIUM.len() as i32)? as usize]]
        }
        Secondary::SmallSlimes => {
            let first = SMALL[ctx.encounter_next_int(roll, 0, SMALL.len() as i32)? as usize];
            let second = SMALL[ctx.encounter_next_int(roll, 0, SMALL.len() as i32)? as usize];
            vec![first, second]
        }
    };
    kinds.push(MonsterKind::SlitheringStrangler);
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(kinds.len());
    for (slot, kind) in kinds.into_iter().enumerate() {
        let spec = rolled(ctx, kind, &taken)?;
        taken.push(spec.hp);
        let spec = if kind == MonsterKind::SlitheringStrangler {
            spec.opening_move(CONSTRICT, &[CONSTRICT])
        } else {
            with_opener(spec)
        };
        roster.push(spec.slot(slot as i32));
    }
    Ok(roster)
}

/// Twig Slime M's concrete opener, as [`super::weak::slimes`] sets it: its
/// machine starts on `STICKY_SHOT_MOVE`, used as-is on turn 1 and logged like
/// a rolled move. Every other slime kind here is left untouched.
fn with_opener(spec: MonsterSpec) -> MonsterSpec {
    const STICKY_SHOT: &str = "STICKY_SHOT_MOVE";
    if spec.kind == MonsterKind::TwigSlimeM {
        spec.opening_move(STICKY_SHOT, &[STICKY_SHOT])
    } else {
        spec
    }
}

/// One creation over the kind's band at this fight's tier: one `Niche` draw
/// over the band minus `taken`, then the spec. A single-value band is not a
/// special case — its one candidate still costs the draw.
fn rolled(
    ctx: &mut dyn EncounterCtx,
    kind: MonsterKind,
    taken: &[i32],
) -> Result<MonsterSpec, RosterRefusal> {
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let hp = ctx.unique_hp(lo, hi, taken);
    Ok(MonsterSpec::new(kind, hp))
}

/// The position of the slot named `name` in an encounter's `get_Slots` array.
///
/// The arrays are the IL's own string tables, so a name that is not in one is
/// a transcription error in this file rather than a fact about a fight.
fn named_slot(slots: &[&str], name: &str) -> i32 {
    slots
        .iter()
        .position(|slot| *slot == name)
        .unwrap_or_else(|| panic!("{name} is not one of the encounter's slots {slots:?}"))
        as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LivingFogNormal`'s slot is read by name here and written as
    /// `combat_sim.FOG_BOMB_SLOTS` in the oracle, which the engine's Gas Bomb
    /// spawn also reads. The two must be the same number.
    #[test]
    fn the_living_fog_slot_name_is_the_oracles_bomb_slot_count() {
        use crate::content_tables::encounter_pool_constants::normal::FOG_BOMB_SLOTS;
        assert_eq!(
            i64::from(named_slot(&LIVING_FOG_SLOTS, "livingFog")),
            FOG_BOMB_SLOTS
        );
    }

    #[test]
    #[should_panic(expected = "is not one of the encounter's slots")]
    fn an_unknown_slot_name_panics_rather_than_defaulting() {
        named_slot(&FOGMOG_SLOTS, "eye");
    }
}

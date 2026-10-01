//! Boss-pool encounter rosters — E4b family F5 (#2535).
//!
//! Ported from `sim/v0.111.0/python/content/encounters/boss.py`, builder
//! for builder: twelve registered keys, each matched by `make_monsters` as
//! the whole entry of its wire id (`ENCOUNTER.VANTOM_BOSS` is built by the
//! builder registered under `VANTOM_BOSS`).
//!
//! Every number below is read from a generated table:
//!
//! * HP bands and spawn-time power amounts from
//!   `content_tables::MONSTER_MODELS` (`get_MinInitialHp` /
//!   `get_MaxInitialHp` and the `Apply<XPower>` sites of each class's
//!   `<AfterAddedToRoom>d__N::MoveNext`);
//! * Lagavulin Matriarch's Plating and Asleep amounts from
//!   `content_tables::encounter_pool_constants::boss`, because they are
//!   applied one call below the spawn hook, where the model reader does not
//!   look (see that builder);
//! * the Waterfall Giant's opening Pressure Gun damage from the DLL-sourced
//!   `content_tables::move_constants`, because it is a field write rather
//!   than a power;
//! * rotation starting positions from `content_tables::LOOPS`, **by move
//!   name** through [`loop_position`].
//!
//! Current-build IL, re-derived for this port on the archived v0.111.0
//! assembly (sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
//! with `sim/v0.111.0/python/tools/dump_il.py`, rather than inherited
//! from Python's frozen RVAs (several of which cite v0.109/v0.110 builds):
//!
//! | encounter | `GenerateMonsters` | monster | `get_MinInitialHp` | `<AfterAddedToRoom>` `MoveNext` |
//! |---|---|---|---|---|
//! | `AeonglassBoss` | `0xd2eb0` | `Aeonglass` | `0xae7cb` | `d__31` `0x3524e8` — `ArtifactPower` (and `WitheringPresencePower` on each opponent) |
//! | `SoulFyshBoss` | `0xd54b3` | `SoulFysh` | `0xbec8a` | — |
//! | `KnowledgeDemonBoss` | `0xd4257` | `KnowledgeDemon` | `0xb793d` | — |
//! | `LagavulinMatriarchBoss` | `0xd42b6` | `LagavulinMatriarch` | `0xb7eea` | `d__39` `0x360ea8` → `<Sleep>d__40` `0x3613b8` |
//! | `CeremonialBeastBoss` | `0xd332a` | `CeremonialBeast` | `0xb0c19` | — |
//! | `QueenBoss` | `0xd4a1c` | `TorchHeadAmalgam` / `Queen` | `0xc2c04` / `0xbc241` | `d__16` `0x371e7c` — `MinionPower` / `d__39` `0x36719c` — none |
//! | `TheKinBoss` | `0xd56c4` | `KinFollower` ×2 / `KinPriest` | `0xb707b` / `0xb7415` | `d__31` `0x35f954` — `MinionPower` / — |
//! | `VantomBoss` | `0xd5b6a` | `Vantom` | `0xc4535` | `d__35` `0x374420` — `SlipperyPower` |
//! | `WaterfallGiantBoss` | `0xd5c0c` | `WaterfallGiant` | `0xc4cfb` | `d__64` `0x3755f4` — a field write |
//! | `TheInsatiableBoss` | `0xd5617` | `TheInsatiable` | `0xc15da` | — |
//! | `TestSubjectBoss` | `0xd55a0` | `TestSubject` | `0xc0136` | `d__68` `0x36d628` — `AdaptablePower`, `EnragePower` |
//! | `KaiserCrabBoss` | `0xd40e0` | `Crusher` / `Rocket` | `0xb1e50` / `0xbc8f8` | `d__36` `0x3570b8` / `d__29` `0x367c94` — `CrabRagePower` |
//!
//! Every boss class's `get_MaxInitialHp` is `ldarg.0; callvirt
//! MonsterModel::get_MinInitialHp; ret` except `KinFollower`'s (`0xb7087`, a
//! real 62..=63 band at A8+), so the single-monster rosters are [`fixed`]:
//! one `Niche` draw for a one-value `SetUniqueMonsterHpValue`.
//!
//! # Spawn-time powers that are deliberately not roster state here
//!
//! Parity is with `make_monsters`' output, and these are not roster state in
//! the oracle either: `Crusher`'s `BackAttackLeftPower` and `Rocket`'s
//! `SurroundedPower` / `BackAttackRightPower` are the engine's facing model
//! (#223), and the `MinionPower` both secondary kinds carry is represented by
//! the oracle only as `Monster.secondary`, its `OwnerIsSecondaryEnemy`
//! projection — which is what the builders below set, and only after checking
//! the native row really applies the power.
//!
//! # Boss lifecycles are not this module's
//!
//! The roster is creation only. Test Subject's revive/respawn forms, the
//! Queen's amalgam-alive branches, the Kin's leader lifecycle and the Waterfall
//! Giant's death-to-ABOUT_TO_BLOW are engine behaviour (#2528), reached once
//! the fight is running.

use crate::content_tables::encounter_pool_constants::boss as constants;
use crate::content_tables::move_constants;
use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position, tier,
};
use crate::ids::MonsterKind;

/// `ENCOUNTER.AEONGLASS_BOSS` — `boss.py::_aeonglass`.
///
/// `Aeonglass/<AfterAddedToRoom>d__31::MoveNext` (`0x3524e8`) applies one
/// `ArtifactPower` to itself (`IL_01a2` `newobj Decimal::.ctor`, `IL_01af`
/// `Apply<ArtifactPower>`). Before that it walks `GetOpponentsOf` and applies
/// a `WitheringPresencePower` of 6 to each opponent (`IL_00d3`
/// `Power<WitheringPresencePower>`, `IL_00e8` `set_Target`, `IL_00fa`
/// `ldc.i4.6`, `IL_0108` the non-generic `PowerCmd::Apply`). That power is
/// player state — `combat_sim` seeds `State.withering_cards_left = 6` at
/// combat entry when any monster is `AEONGLASS`, not in `_aeonglass` — so it
/// is not roster state here either; the model reader, which sees only
/// `Apply<XPower>` sites, does not list it in `MONSTER_MODELS`. The oracle
/// also passes `additional_strength=0` and `wither_upgrade_count=0`, which
/// are `Monster`'s own defaults and so no roster state.
pub fn build_aeonglass_boss(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let artifact = initial_power(MonsterKind::Aeonglass, "ArtifactPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, MonsterKind::Aeonglass)?.with_state("artifact", artifact),
    ])
}

/// `ENCOUNTER.SOUL_FYSH_BOSS` — `boss.py::_soul_fysh`.
pub fn build_soul_fysh_boss(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::SoulFysh)?])
}

/// `ENCOUNTER.KNOWLEDGE_DEMON_BOSS` — `boss.py::_knowledge_demon`.
pub fn build_knowledge_demon_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::KnowledgeDemon)?])
}

/// `ENCOUNTER.LAGAVULIN_MATRIARCH_BOSS` — `boss.py::_lagavulin_matriarch`.
///
/// Plating then Asleep, in that order, both applied one call below the spawn
/// hook, which is why `MONSTER_MODELS` records no spawn-time powers for this
/// kind and the amounts come from the pool constants instead:
///
/// ```text
/// LagavulinMatriarch/<AfterAddedToRoom>d__39::MoveNext   (0x360ea8)
///   IL_0080: call LagavulinMatriarch::Sleep              // the whole spawn body
/// LagavulinMatriarch/<Sleep>d__40::MoveNext              (0x3613b8)
///   IL_002e: call LagavulinMatriarch::set_IsAwake(false)
///   IL_00ab: ldc.i4.s 12; newobj Decimal::.ctor
///   IL_00ba: call Apply<PlatingPower>                    // LAGAVULIN_MATRIARCH_PLATING
///   IL_0123: ldc.i4.3;    newobj Decimal::.ctor
///   IL_0131: call Apply<AsleepPower>                     // LAGAVULIN_MATRIARCH_ASLEEP
/// ```
///
/// Both are untiered `ldc` literals, so neither depends on the ascension.
pub fn build_lagavulin_matriarch_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![
        fixed(ctx, MonsterKind::LagavulinMatriarch)?
            .with_state("mplating", constants::LAGAVULIN_MATRIARCH_PLATING)
            .with_state("asleep", constants::LAGAVULIN_MATRIARCH_ASLEEP),
    ])
}

/// `ENCOUNTER.CEREMONIAL_BEAST_BOSS` — `boss.py::_ceremonial_beast`.
pub fn build_ceremonial_beast_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::CeremonialBeast)?])
}

/// The `MinionPower` a secondary kind applies to itself at spawn, as the
/// oracle's `Monster.secondary` flag.
///
/// `TorchHeadAmalgam/<AfterAddedToRoom>d__16::MoveNext` (`0x371e7c`) and
/// `KinFollower/<AfterAddedToRoom>d__31::MoveNext` (`0x35f954`) each apply
/// `MinionPower` with `Decimal.One` (`IL_008a`/`IL_0097` in both). Its only
/// roster-visible effect is `OwnerIsSecondaryEnemy`, which `combat_sim`
/// carries as `secondary: bool`. The amount is read only to prove the power
/// is applied at all; a kind that does not apply it refuses by name.
fn secondary(ctx: &dyn EncounterCtx, spec: MonsterSpec) -> Result<MonsterSpec, RosterRefusal> {
    initial_power(spec.kind, "MinionPower", ctx.ascension())?;
    Ok(spec.with_flag("secondary", true))
}

/// `ENCOUNTER.QUEEN_BOSS` — `boss.py::_queen`.
///
/// `QueenBoss::GenerateMonsters` (`0xd4a1c`) builds a two-element array:
/// `Monster<TorchHeadAmalgam>` into slot `'amalgam'` (`IL_0014`-`IL_0028`),
/// then `Monster<Queen>` into slot `'queen'` (`IL_002f`-`IL_0043`). The
/// amalgam is created first and is the sole secondary enemy. Both bands are
/// single values; each spends its `SetUniqueMonsterHpValue` draw with the
/// earlier sibling's value taken.
///
/// `Queen/<AfterAddedToRoom>d__39::MoveNext` (`0x36719c`) applies no power —
/// it caches the amalgam (`IL_00a3` `First<Creature>`, `IL_00a8`
/// `set_Amalgam`) and sets music — so the Queen carries no spawn state.
pub fn build_queen_boss(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (amalgam_lo, amalgam_hi) = hp_band(MonsterKind::TorchHeadAmalgam, ctx.ascension())?;
    let (queen_lo, queen_hi) = hp_band(MonsterKind::Queen, ctx.ascension())?;
    let amalgam_hp = ctx.unique_hp(amalgam_lo, amalgam_hi, &[]);
    let queen_hp = ctx.unique_hp(queen_lo, queen_hi, &[amalgam_hp]);
    Ok(vec![
        secondary(
            ctx,
            MonsterSpec::new(MonsterKind::TorchHeadAmalgam, amalgam_hp).slot(0),
        )?,
        MonsterSpec::new(MonsterKind::Queen, queen_hp).slot(1),
    ])
}

/// `ENCOUNTER.THE_KIN_BOSS` — `boss.py::_the_kin`.
///
/// `TheKinBoss::GenerateMonsters` (`0xd56c4`):
///
/// ```text
/// IL_000c: Monster<KinFollower>.ToMutable() -> loc0
/// IL_001e: loc0.set_StartsWithDance(true)
/// IL_002b: [0] = (loc0, 'slot1')
/// IL_003d: [1] = (Monster<KinFollower>.ToMutable(), 'slot2')
/// IL_0058: [2] = (Monster<KinPriest>.ToMutable(), 'leaderSlot')
/// ```
///
/// `KinFollower::GenerateMoveStateMachine` (`0xb7160`) builds
/// `QUICK_SLASH_MOVE` (loc1), `BOOMERANG_MOVE` (loc2) and `POWER_DANCE_MOVE`
/// (loc3), and picks the initial state at `IL_00b9`-`IL_00c3`:
/// `get_StartsWithDance ? loc3 : loc1`. So the first follower opens on
/// `POWER_DANCE_MOVE` and the second on `QUICK_SLASH_MOVE`; the position is
/// read out of the generated rotation by that name.
///
/// The followers' band is a real range at A8+ (`0xb707b`/`0xb7087`), so the
/// second follower's draw excludes the first's value; the priest's is a
/// single value drawn last with both taken.
pub fn build_the_kin_boss(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (follower_lo, follower_hi) = hp_band(MonsterKind::KinFollower, ctx.ascension())?;
    let (priest_lo, priest_hi) = hp_band(MonsterKind::KinPriest, ctx.ascension())?;
    let dance = loop_position(MonsterKind::KinFollower, "POWER_DANCE_MOVE")?;
    let slash = loop_position(MonsterKind::KinFollower, "QUICK_SLASH_MOVE")?;
    let first_hp = ctx.unique_hp(follower_lo, follower_hi, &[]);
    let second_hp = ctx.unique_hp(follower_lo, follower_hi, &[first_hp]);
    let priest_hp = ctx.unique_hp(priest_lo, priest_hi, &[first_hp, second_hp]);
    Ok(vec![
        secondary(
            ctx,
            MonsterSpec::new(MonsterKind::KinFollower, first_hp)
                .slot(0)
                .loop_pos(dance),
        )?,
        secondary(
            ctx,
            MonsterSpec::new(MonsterKind::KinFollower, second_hp)
                .slot(1)
                .loop_pos(slash),
        )?,
        MonsterSpec::new(MonsterKind::KinPriest, priest_hp).slot(2),
    ])
}

/// `ENCOUNTER.VANTOM_BOSS` — `boss.py::_vantom`.
///
/// `Vantom/<AfterAddedToRoom>d__35::MoveNext` (`0x374420`) applies
/// `SlipperyPower` with `Vantom::get_SlipperyAmt` (`0xc4547`,
/// `GetValueIfAscension(8, 9, 8)`) widened through `Decimal::op_Implicit`
/// (`IL_008b`-`IL_009d`).
pub fn build_vantom_boss(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let slippery = initial_power(MonsterKind::Vantom, "SlipperyPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, MonsterKind::Vantom)?.with_state("slippery", slippery),
    ])
}

/// `ENCOUNTER.WATERFALL_GIANT_BOSS` — `boss.py::_waterfall_giant`.
///
/// `WaterfallGiant/<AfterAddedToRoom>d__64::MoveNext` (`0x3755f4`) applies
/// no power; after the base hook it writes the move's starting damage into a
/// field:
///
/// ```text
/// IL_0077: call WaterfallGiant::get_BasePressureGunDamage   // 0xc4d55: GetValueIfAscension(9, 23, 20)
/// IL_007c: call WaterfallGiant::set_CurrentPressureGunDamage
/// ```
///
/// `combat_sim` carries `_currentPressureGunDamage` as
/// `Monster.pressure_gun_damage`. A field write is not a column of
/// `MONSTER_MODELS`, so the tier is the assembly's own triple as
/// `content_tables::move_constants::WATERFALL_GIANT_BASE_PRESSURE_GUN_DAMAGE`
/// (read by `dll_content.monster_move_constants`, routed to this builder by
/// `tools/move_constant_sites.py::ROSTER_CONSTANTS`), selected at the fight's
/// ascension like every other tier. The oracle's `steam_pressure=0` is the
/// dataclass default and so no roster state.
pub fn build_waterfall_giant_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let gun = tier(
        move_constants::WATERFALL_GIANT_BASE_PRESSURE_GUN_DAMAGE,
        ctx.ascension(),
    );
    Ok(vec![
        fixed(ctx, MonsterKind::WaterfallGiant)?.with_state("pressure_gun_damage", gun),
    ])
}

/// `ENCOUNTER.THE_INSATIABLE_BOSS` — `boss.py::_the_insatiable`.
pub fn build_the_insatiable_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::TheInsatiable)?])
}

/// `ENCOUNTER.TEST_SUBJECT_BOSS` — `boss.py::_test_subject`.
///
/// HP: `TestSubject::get_MinInitialHp` (`0xc0136`) is `ldarg.0; call
/// TestSubject::get_FirstFormHp; ret`, `get_FirstFormHp` (`0xc0146`) is
/// `GetValueIfAscension(8, 111, 100)`, and `get_MaxInitialHp` (`0xc013e`)
/// delegates to `get_MinInitialHp` — a fixed band two getter hops deep, which
/// the model reader follows since #2535.
///
/// Spawn state: `TestSubject/<AfterAddedToRoom>d__68::MoveNext` (`0x36d628`)
/// applies `AdaptablePower` with `Decimal.One` (`IL_0092`-`IL_009f`) and then
/// `EnragePower` with `get_EnrageAmount` (`0xc0176`, `GetValueIfAscension(9,
/// 3, 2)`, `IL_0106`-`IL_0118`). The oracle carries both as ints
/// (`adaptable`, `enrage`).
///
/// Opening move: `TestSubject::GenerateMoveStateMachine` (`0xc031c`) passes
/// loc1 — the `BITE_MOVE` state built at `IL_004b`-`IL_0075` — as the
/// machine's initial state (`IL_0222`), so form one opens on `BITE_MOVE`
/// rather than on `RESPAWN_MOVE`, which sits first in the oracle's rotation.
pub fn build_test_subject_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let adaptable = initial_power(MonsterKind::TestSubject, "AdaptablePower", ctx.ascension())?;
    let enrage = initial_power(MonsterKind::TestSubject, "EnragePower", ctx.ascension())?;
    let bite = loop_position(MonsterKind::TestSubject, "BITE_MOVE")?;
    Ok(vec![
        fixed(ctx, MonsterKind::TestSubject)?
            .loop_pos(bite)
            .with_state("adaptable", adaptable)
            .with_state("enrage", enrage),
    ])
}

/// `ENCOUNTER.KAISER_CRAB_BOSS` — `boss.py::_kaiser_crab`.
///
/// `KaiserCrabBoss::GenerateMonsters` (`0xd40e0`) creates `Monster<Crusher>`
/// into slot `'crusher'` (`IL_0014`-`IL_0028`) and then `Monster<Rocket>` into
/// slot `'rocket'` (`IL_002f`-`IL_0043`). Each spends its single-value
/// `SetUniqueMonsterHpValue` draw with the earlier value taken.
///
/// Both apply `CrabRagePower` with `Decimal.One` at spawn
/// (`Crusher/<AfterAddedToRoom>d__36` `0x3570b8` `IL_0105`-`IL_0112`,
/// `Rocket/<AfterAddedToRoom>d__29` `0x367c94` `IL_018a`-`IL_0197`). The
/// oracle carries it as the presence flag `crab_rage: bool`; the amount is
/// read only to prove the power is applied.
pub fn build_kaiser_crab_boss(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (crusher_lo, crusher_hi) = hp_band(MonsterKind::Crusher, ctx.ascension())?;
    let (rocket_lo, rocket_hi) = hp_band(MonsterKind::Rocket, ctx.ascension())?;
    initial_power(MonsterKind::Crusher, "CrabRagePower", ctx.ascension())?;
    initial_power(MonsterKind::Rocket, "CrabRagePower", ctx.ascension())?;
    let crusher_hp = ctx.unique_hp(crusher_lo, crusher_hi, &[]);
    let rocket_hp = ctx.unique_hp(rocket_lo, rocket_hi, &[crusher_hp]);
    Ok(vec![
        MonsterSpec::new(MonsterKind::Crusher, crusher_hp)
            .slot(0)
            .with_flag("crab_rage", true),
        MonsterSpec::new(MonsterKind::Rocket, rocket_hp)
            .slot(1)
            .with_flag("crab_rage", true),
    ])
}

#[cfg(test)]
mod tests {
    //! Witnesses for the branches the corpus pins cannot reach. Every pinned
    //! Test Subject, Queen, Kin and Kaiser Crab fight is at A8 or above, so
    //! the below-gate tiers those builders select are pinned here against the
    //! IL values cited on each builder.

    use super::*;
    use crate::rng::Xoshiro256StarStar;

    struct Ctx(Xoshiro256StarStar, u8);

    impl EncounterCtx for Ctx {
        fn ascension(&self) -> u8 {
            self.1
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

    fn state(spec: &MonsterSpec) -> Vec<(&'static str, i64)> {
        spec.initial_state
            .iter()
            .map(|(field, value)| (*field, value.as_i64().expect("an integer spawn value")))
            .collect()
    }

    /// `TestSubject::get_FirstFormHp` `(8, 111, 100)` and
    /// `get_EnrageAmount` `(9, 3, 2)`: the two gates differ, so A8 is the
    /// level that takes the high HP with the low Enrage.
    #[test]
    fn test_subject_selects_its_hp_and_enrage_gates_independently() {
        let mut got = Vec::new();
        for ascension in [7, 8, 9] {
            let mut ctx = Ctx(Xoshiro256StarStar::from_seed(1), ascension);
            let roster = build_test_subject_boss(&mut ctx).unwrap();
            assert_eq!(ctx.0.counter, 1, "one Niche draw");
            assert_eq!(roster[0].loop_pos, 1, "BITE_MOVE");
            got.push((roster[0].max_hp, state(&roster[0])));
        }
        assert_eq!(
            got,
            [
                (100, vec![("adaptable", 1), ("enrage", 2)]),
                (111, vec![("adaptable", 1), ("enrage", 2)]),
                (111, vec![("adaptable", 1), ("enrage", 3)]),
            ]
        );
    }

    /// `get_BasePressureGunDamage` `(9, 23, 20)` against the HP gate at 8.
    #[test]
    fn waterfall_giant_pressure_gun_is_gated_at_nine() {
        let got: Vec<_> = [7, 8, 9]
            .map(|ascension| {
                let mut ctx = Ctx(Xoshiro256StarStar::from_seed(1), ascension);
                let roster = build_waterfall_giant_boss(&mut ctx).unwrap();
                (roster[0].max_hp, state(&roster[0]))
            })
            .to_vec();
        assert_eq!(
            got,
            [
                (240, vec![("pressure_gun_damage", 20)]),
                (250, vec![("pressure_gun_damage", 20)]),
                (250, vec![("pressure_gun_damage", 23)]),
            ]
        );
    }

    /// Below A8 the Kin followers' band is 58..=59 and the priest 190; the
    /// two followers still take distinct values, the dancer opens on
    /// POWER_DANCE_MOVE, and three Niche draws are spent.
    #[test]
    fn the_kin_below_the_gate_takes_distinct_follower_hp() {
        for seed in 0..16 {
            let mut ctx = Ctx(Xoshiro256StarStar::from_seed(seed), 7);
            let roster = build_the_kin_boss(&mut ctx).unwrap();
            assert_eq!(ctx.0.counter, 3);
            let mut followers = [roster[0].max_hp, roster[1].max_hp];
            followers.sort_unstable();
            assert_eq!(followers, [58, 59], "seed {seed}");
            assert_eq!(roster[2].max_hp, 190);
            assert_eq!((roster[0].loop_pos, roster[1].loop_pos), (2, 0));
            assert_eq!(state(&roster[0]), [("secondary", 1)]);
            assert!(state(&roster[2]).is_empty());
        }
    }

    /// Queen and Kaiser Crab below A8: the single-value bands at that tier,
    /// one draw per creation.
    #[test]
    fn two_monster_bosses_take_their_below_gate_bands() {
        let mut ctx = Ctx(Xoshiro256StarStar::from_seed(5), 7);
        let queen = build_queen_boss(&mut ctx).unwrap();
        assert_eq!(ctx.0.counter, 2);
        assert_eq!((queen[0].max_hp, queen[1].max_hp), (199, 400));
        let mut ctx = Ctx(Xoshiro256StarStar::from_seed(5), 7);
        let crab = build_kaiser_crab_boss(&mut ctx).unwrap();
        assert_eq!(ctx.0.counter, 2);
        assert_eq!((crab[0].max_hp, crab[1].max_hp), (209, 199));
    }

    /// `secondary` is a projection of the native `MinionPower`: a kind whose
    /// spawn hook does not apply it refuses by name rather than being flagged.
    #[test]
    fn secondary_refuses_a_kind_without_minion_power() {
        let ctx = Ctx(Xoshiro256StarStar::from_seed(1), 10);
        assert_eq!(
            secondary(&ctx, MonsterSpec::new(MonsterKind::Queen, 1)),
            Err(RosterRefusal::MonsterPowerAbsent {
                kind: MonsterKind::Queen,
                power: "MinionPower",
            })
        );
    }
}

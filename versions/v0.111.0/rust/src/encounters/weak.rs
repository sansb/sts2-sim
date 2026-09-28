//! Weak-pool encounter rosters — E4b family F1 (#2531).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/weak.py`, builder
//! for builder. Thirteen registered keys, 156 corpus fights and 94 eval
//! fixtures — the wave's largest demand block.
//!
//! Every number below is read from a generated table. HP bands and spawn-time
//! power amounts come from `content_tables::MONSTER_MODELS`, extracted from
//! `MonsterModel::get_MinInitialHp` / `get_MaxInitialHp` and the
//! `Apply<XPower>` sites of each class's `<AfterAddedToRoom>d__N::MoveNext`.
//! The starting rotation positions come from `content_tables::LOOPS` — looked
//! up **by move name**, because that is what the oracle's own comment says
//! those indices are (`# SPIKEN, WHIRL`) and an index typed in by hand would
//! be a hand-transcribed number (PORT_PLAN §5). Move-state names themselves
//! are `&'static str` literals, the idiom `elite.rs` established.
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs (v0.111.0 `sts2.dll` sha256 `9cb4f1ad…`, read with
//! `versions/v0.111.0/solver/tools/dump_il.py`):
//!
//! | class | `GenerateMonsters` | note |
//! |---|---|---|
//! | `SeapunkWeak` | `0xd4da3` | one creation, null slot |
//! | `SludgeSpinnerWeak` | `0xd531e` | one creation, null slot |
//! | `ShrinkerBeetleWeak` | `0xd4e0a` | one creation, null slot |
//! | `FuzzyWurmCrawlerWeak` | `0xd3e0b` | one creation, null slot |
//! | `TunnelerWeak` | `0xd596a` | one creation, `MaxInitialHp` delegates |
//! | `DevotedSculptorWeak` | `0xd3877` | one creation, fixed band |
//! | `NibbitsWeak` | `0xd4590` | sets `Nibbit::IsAlone` |
//! | `ToadpolesWeak` | `0xd5854` | sets `Toadpole::IsFront` true then false |
//! | `ExoskeletonsWeak` | `0xd39dc` | three creations, slots `first/second/third` (`get_Slots` `0xd39a4`) |
//! | `TurretOperatorWeak` | `0xd59ac` | `LivingShield` then `TurretOperator` |
//! | `CorpseSlugsWeak` | `0xd3540` | -> `EnsureCorpseSlugsStartWithDifferentMoves` `0xb18a4` |
//! | `SlimesWeak` | `0xd5018` | three Encounter draws, `.cctor` `0xd50a6` |
//! | `BowlbugsWeak` | `0xd3210` | `get_Bugs` `0xd31e0` |
//!
//! The spawn-time powers this family carries as roster state are
//! `CorpseSlug`'s `RavenousPower` (`0x356820`), `Exoskeleton`'s
//! `HardToKillPower` (`0x359b84`) and `LivingShield`'s `RampartPower`
//! (`0x362430`). `BowlbugRock`'s `ImbalancedPower` is deliberately **not**
//! roster state, for the same reason `elite.rs` leaves three of its own out:
//! it is not roster state in the oracle either — it is the fully-blocked
//! Headbutt stun, modeled in the engine's `headbutt` move kind. Parity here
//! is with `make_monsters`' output.

use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position,
    rotation_len,
};
use crate::ids::MonsterKind;

/// `ENCOUNTER.SEAPUNK_WEAK` — `content/encounters/weak.py::_seapunk`.
///
/// One seapunk. The full id is deliberate in the oracle and inherited here:
/// `SEAPUNK_NORMAL` adds a Calcified Cultist and is a different roster.
pub fn build_seapunk_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::Seapunk, &[])?])
}

/// `ENCOUNTER.SHRINKER_BEETLE_WEAK` — `weak.py::_shrinker_beetle`.
pub fn build_shrinker_beetle_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::ShrinkerBeetle, &[])?])
}

/// `ENCOUNTER.FUZZY_WURM_CRAWLER_WEAK` — `weak.py::_fuzzy_wurm_crawler`.
pub fn build_fuzzy_wurm_crawler_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::FuzzyWurmCrawler, &[])?])
}

/// `ENCOUNTER.NIBBITS_WEAK` — `weak.py::_nibbit`.
///
/// One lone nibbit. `NibbitsWeak::GenerateMonsters` sets `Nibbit::IsAlone`,
/// which is the predicate the machine's INIT `ConditionalBranchState` reads,
/// so the loop starts at its first position. `NIBBITS_NORMAL`'s multi-nibbit
/// front/back split is a different roster and belongs to its own family.
pub fn build_nibbits_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::Nibbit, &[])?])
}

/// `ENCOUNTER.SLUDGE_SPINNER_WEAK` — `weak.py::_sludge`.
///
/// A random-AI kind whose machine's initial state is a concrete `MoveState`
/// rather than the RAND node, so turn 1 is used as-is with no `MonsterAi`
/// draw — and it is logged exactly like a rolled move.
pub fn build_sludge_spinner_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const OIL_SPRAY: &str = "OIL_SPRAY";
    Ok(vec![
        rolled(ctx, MonsterKind::SludgeSpinner, &[])?.opening_move(OIL_SPRAY, &[OIL_SPRAY]),
    ])
}

/// `ENCOUNTER.TUNNELER_WEAK` — `weak.py::_tunneler`.
///
/// `Tunneler::get_MaxInitialHp` delegates to `get_MinInitialHp`, so the band
/// is one value wide — which is not a special case in the roll: a one-element
/// candidate set still spends its `Niche` draw.
pub fn build_tunneler_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::Tunneler)?])
}

/// `ENCOUNTER.DEVOTED_SCULPTOR_WEAK` — `weak.py::_devoted_sculptor`.
pub fn build_devoted_sculptor_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::DevotedSculptor)?])
}

/// `ENCOUNTER.TOADPOLES_WEAK` — `weak.py::_toadpoles`.
///
/// The front toad is created first with `Toadpole::set_IsFront(true)` and the
/// back one second with `set_IsFront(false)`, so creation order is slot order
/// and `Niche` roll order. `IsFront` is what the INIT conditional reads to put
/// each toad on a different point of the same rotation: front on `SPIKEN`,
/// back on `WHIRL` — in-game validated on the oracle's side (ZPJHU3WSH2 fight
/// 1). Both share one band, so the second creation excludes the first's roll.
pub fn build_toadpoles_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTS: [&str; 2] = ["SPIKEN", "WHIRL"];
    let (lo, hi) = hp_band(MonsterKind::Toadpole, ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(STARTS.len());
    for (slot, start) in STARTS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(MonsterKind::Toadpole, hp)
                .slot(slot as i32)
                .loop_pos(loop_position(MonsterKind::Toadpole, start)?),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.EXOSKELETONS_WEAK` — `weak.py::_exoskeletons`.
///
/// Three exoskeletons in the named slots `first`, `second`, `third`. All three
/// share one band, so the uniqueness set accumulates across creations. Each
/// carries `HardToKillPower`, and each opens on the concrete MoveState its
/// INIT node resolves to for that slot — a hybrid random-AI kind, so the
/// opener is logged like a rolled move and consumes no `MonsterAi` draw.
pub fn build_exoskeletons_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STARTERS: [(&str, &[&str]); 3] = [
        ("SKITTER_MOVE", &["SKITTER_MOVE"]),
        ("MANDIBLES_MOVE", &["MANDIBLES_MOVE"]),
        ("ENRAGE_MOVE", &["ENRAGE_MOVE"]),
    ];
    let (lo, hi) = hp_band(MonsterKind::Exoskeleton, ctx.ascension())?;
    let hard_to_kill = initial_power(MonsterKind::Exoskeleton, "HardToKillPower", ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(STARTERS.len());
    for (slot, (opener, log)) in STARTERS.iter().enumerate() {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(MonsterKind::Exoskeleton, hp)
                .slot(slot as i32)
                .opening_move(opener, log)
                .with_state("hard_to_kill", hard_to_kill),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.TURRET_OPERATOR_WEAK` — `weak.py::_turret_operator`.
///
/// A Living Shield then the Operator behind it, both fixed-band and both still
/// spending a `Niche` draw in creation order. The shield carries
/// `RampartPower`. The operator's uniqueness set contains the shield's rolled
/// value, which cannot collide with its own single-value band — carried anyway
/// because that is the call the oracle makes, and the draw is what matters.
///
/// The Living Shield's mid-fight `SHIELD_SLAM` branch re-runs `GetAllyCount`;
/// that lifecycle is engine work under #2528, not roster content.
pub fn build_turret_operator_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let rampart = initial_power(MonsterKind::LivingShield, "RampartPower", ctx.ascension())?;
    let shield = rolled(ctx, MonsterKind::LivingShield, &[])?
        .slot(0)
        .with_state("rampart", rampart);
    let operator = rolled(ctx, MonsterKind::TurretOperator, &[shield.hp])?.slot(1);
    Ok(vec![shield, operator])
}

/// `ENCOUNTER.CORPSE_SLUGS_WEAK` — `weak.py::_corpse_slugs_weak`.
pub fn build_corpse_slugs_weak(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    corpse_slugs(ctx, 2)
}

/// The shared Corpse Slug roster: `count` slugs, staggered by one draw.
///
/// `weak.py::build_corpse_slugs`, which `normal._corpse_slugs_normal` also
/// calls — deliberately one primitive with two callers, because the
/// Encounter-stream draw lives here and duplicating it would duplicate the
/// stream accounting. `pub(crate)` so F4 can reach it without copying it.
///
/// `CorpseSlugsWeak::GenerateMonsters` creates the slugs with null slots and
/// hands them, with the encounter's own `Rng`, to
/// `CorpseSlug::EnsureCorpseSlugsStartWithDifferentMoves` (`0xb18a4`):
///
/// ```text
/// ldarg.1; ldc.i4.3; callvirt Rng::NextInt        // one draw, 0..3
/// stloc.1
/// loop over OfType<CorpseSlug>():
///     ldloc.3; ldloc.1; ldc.i4.3; rem
///     callvirt CorpseSlug::set_StarterMoveIdx      // idx % 3
///     ldloc.1; ldc.i4.1; add; stloc.1              // idx++
/// ```
///
/// The literal `3` in both the draw bound and the modulus is the slug's
/// rotation length, so it is read from the generated loop rather than typed
/// in: one draw picks a starter offset and creation order advances it, which
/// is what makes two slugs never open on the same move.
///
/// One shared uniqueness set — the slugs share a band, so each creation
/// excludes its predecessors' rolled max HP.
pub(crate) fn corpse_slugs(
    ctx: &mut dyn EncounterCtx,
    count: usize,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::CorpseSlug;
    let rotation = rotation_len(kind)?;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let ravenous = initial_power(kind, "RavenousPower", ctx.ascension())?;
    let starter = ctx.encounter_next_int("CorpseSlug shared starter offset", 0, rotation)?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(count);
    for slot in 0..count {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(kind, hp)
                .slot(slot as i32)
                .loop_pos((starter + slot as i32) % rotation)
                .with_state("ravenous", ravenous),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.BOWLBUGS_WEAK` — `weak.py::_bowlbugs`.
///
/// A Bowlbug Rock in slot `odd`, then `Rng.NextItem(get_Bugs())` in slot
/// `even`. `BowlbugsWeak::get_Bugs` (`0xd31e0`) is `[BowlbugEgg,
/// BowlbugNectar]` in that order, so the single draw selects the egg on 0.
/// That draw is the first on the per-fight `Encounter` stream, which
/// `GenerateMonstersWithSlots` constructs at counter 0.
///
/// Creation order is array order — Rock, then the bug — which is also slot
/// order and `Niche` roll order. The two bands are disjoint, so each creation
/// rolls against an empty uniqueness set: a value outside a band cannot be in
/// its candidate set to begin with, which is why the oracle passes two
/// separate sets here and one shared set for the slugs.
pub fn build_bowlbugs_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const BUGS: [MonsterKind; 2] = [MonsterKind::BowlbugEgg, MonsterKind::BowlbugNectar];
    let pick = ctx.encounter_next_int("Encounter-stream bug roll", 0, BUGS.len() as i32)?;
    Ok(vec![
        rolled(ctx, MonsterKind::BowlbugRock, &[])?.slot(0),
        rolled(ctx, BUGS[pick as usize], &[])?.slot(1),
    ])
}

/// `ENCOUNTER.SLIMES_WEAK` — `weak.py::_slimes_weak` and `build_slimes`.
///
/// `SlimesWeak::GenerateMonsters` (`0xd5018`), in body order:
///
/// ```text
/// ldsfld SlimesWeak::_smallSlimes; ToList          // [LeafSlimeS, TwigSlimeS]
/// get_Rng; NextItem                                 // draw 1 -> small1
/// Remove(small1)
/// get_Rng; NextItem                                 // draw 2 -> small2
/// Add(small1)                                       // creation 0
/// get_Rng; ldsfld _mediumSlimes; NextItem           // draw 3 -> medium
/// Add(medium)                                       // creation 1
/// Add(small2)                                       // creation 2
/// ```
///
/// Two facts here are easy to get wrong and both are measured rather than
/// assumed. **Draw order is not creation order**: both small slimes are chosen
/// before the medium one, but the medium is added between them. And **the
/// forced second pick still costs a draw** — `Rng::NextItem` (`0x5ee74`)
/// returns the element default without drawing only when the source is
/// *empty*, so a one-element remainder still spends its `NextInt(0, 1)`.
///
/// The pools are the `.cctor`'s (`0xd50a6`), in declaration order.
pub fn build_slimes_weak(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const SMALL: [MonsterKind; 2] = [MonsterKind::LeafSlimeS, MonsterKind::TwigSlimeS];
    const MEDIUM: [MonsterKind; 2] = [MonsterKind::LeafSlimeM, MonsterKind::TwigSlimeM];
    let roll = "Encounter-stream roster rolls";
    let first = ctx.encounter_next_int(roll, 0, SMALL.len() as i32)? as usize;
    // The remainder is a one-element list: the answer is forced, the draw is
    // not, and skipping it would desynchronise the stream for the medium pick.
    let remainder = ctx.encounter_next_int(roll, 0, SMALL.len() as i32 - 1)? as usize;
    let small2 = SMALL[1 - first + remainder];
    let medium = MEDIUM[ctx.encounter_next_int(roll, 0, MEDIUM.len() as i32)? as usize];
    slimes(ctx, &[SMALL[first], medium, small2])
}

/// The shared slime roster: one `Niche` roll per kind, in creation order.
///
/// `weak.py::build_slimes`, shared with `SLIMES_NORMAL` and the later
/// encounters that reuse the four slime kinds — `pub(crate)` for the same
/// reason [`corpse_slugs`] is.
///
/// **One shared uniqueness set**, because
/// `Creature::SetUniqueMonsterHpValue` excludes every other creature's
/// already-rolled max HP and the two small bands overlap: Leaf Slime S
/// (12-16) and Twig Slime S (8-12) share a value, so a roster carrying both
/// can actually collide.
///
/// Twig Slime M's machine opens on a concrete `MoveState`, used as-is on turn
/// 1 and logged like a rolled move; Leaf Slime S's INIT node *is* the RAND
/// node, so its opening intent is a turn-1 `MonsterAi` roll the engine owns
/// (#2528) and its `next_move` stays empty here.
pub(crate) fn slimes(
    ctx: &mut dyn EncounterCtx,
    kinds: &[MonsterKind],
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const STICKY_SHOT: &str = "STICKY_SHOT_MOVE";
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(kinds.len());
    for (slot, kind) in kinds.iter().enumerate() {
        let (lo, hi) = hp_band(*kind, ctx.ascension())?;
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        let mut spec = MonsterSpec::new(*kind, hp).slot(slot as i32);
        if *kind == MonsterKind::TwigSlimeM {
            spec = spec.opening_move(STICKY_SHOT, &[STICKY_SHOT]);
        }
        roster.push(spec);
    }
    Ok(roster)
}

/// One creation whose band is a real range: one `Niche` draw over the band
/// minus `taken`, then the spec.
fn rolled(
    ctx: &mut dyn EncounterCtx,
    kind: MonsterKind,
    taken: &[i32],
) -> Result<MonsterSpec, RosterRefusal> {
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let hp = ctx.unique_hp(lo, hi, taken);
    Ok(MonsterSpec::new(kind, hp))
}

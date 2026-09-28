//! Normal-pool encounter rosters, demand rank 16-30 — E4b family F4 (#2534).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/normal.py`, builder
//! for builder, under the same rules as [`super::weak`], [`super::elite`] and
//! [`super::normal_a`]: every number in a roster comes from a generated table
//! (`content_tables::MONSTER_MODELS` for HP bands and spawn-time power
//! amounts, `content_tables::LOOPS` for rotation positions, looked up **by
//! move name**), and the control flow around them is hand-ported with
//! current-build IL citations (PORT_PLAN §5).
//!
//! # All fifteen ids
//!
//! `SLUMBERING_BEETLE_NORMAL` landed after the other fourteen (#2848): its
//! oracle roster sets `override="SNORE"` on the beetle — a *string*
//! spawn-time field — which needed `SpawnValue::Text` and
//! [`MonsterSpec::with_text`], the opening's projection of it, and a pin shape
//! that carries a string, before [`build_slumbering_beetle_normal`] could be
//! written.
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs (v0.111.0 `sts2.dll` sha256 `9cb4f1ad…`, read with
//! `versions/v0.111.0/solver/tools/dump_il.py`):
//!
//! | class | `GenerateMonsters` | shape |
//! |---|---|---|
//! | `PunchConstructNormal` | `0xd48eb` | one creation, null slot |
//! | `SewerClamNormal` | `0xd4dd1` | one creation, null slot |
//! | `BowlbugsNormal` | `0xd3090` | Rock, then two `NextItem` workers (`.cctor` `0xd3174`) |
//! | `LouseProgenitorNormal` | `0xd437f` | one creation, null slot |
//! | `RubyRaidersNormal` | `0xd4a84` | three `NextItem` raiders (`.cctor` `0xd4b24`) |
//! | `HauntedShipNormal` | `0xd3f39` | one creation, null slot |
//! | `SeapunkNormal` | `0xd4d45` | `CalcifiedCultist` then `Seapunk` |
//! | `ChompersNormal` | `0xd3360` | two Chompers, the second `set_ScreamFirst(true)` |
//! | `FrogKnightNormal` | `0xd3dd2` | one creation, null slot |
//! | `SnappingJaxfruitNormal` | `0xd5433` | `SnappingJaxfruit` then `Flyconid` |
//! | `SpinyToadNormal` | `0xd550f` | one creation, null slot |
//! | `SlimesNormal` | `0xd4ed8` | one `NextBool`, then a fixed four-slot array |
//! | `OvergrowthCrawlers` | `0xd45fd` | `ShrinkerBeetle` then `FuzzyWurmCrawler` |
//! | `AxebotsNormal` | `0xd2eed` | one creation, slot `front`, no `StockAmount` set |
//! | `SlumberingBeetleNormal` | `0xd5370` | `BowlbugRock`, `BowlbugSilk`, `SlumberingBeetle` |
//!
//! Every one of them builds its array in creation order, which is also the
//! `Niche` roll order (`Creature::SetUniqueMonsterHpValue` runs at each
//! creation) and the order `_EncounterCtx.done` stamps uids in.
//!
//! The spawn-time powers this family carries as roster state are exactly the
//! ones the oracle's builders set a `combat_sim.Monster` field for:
//! `PunchConstruct`'s and `Chomper`'s `ArtifactPower`, `SewerClam`'s and
//! `FrogKnight`'s `PlatingPower` (`mplating`), `LouseProgenitor`'s
//! `CurlUpPower` (`curl_up`), `Axebot`'s `StockPower` (`stock`), and
//! `SlumberingBeetle`'s `PlatingPower`/`SlumberPower` (`mplating`/`slumber`)
//! plus its initial `SNORE_MOVE` state (`override`, the one text field).

use crate::encounters::{
    EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power, loop_position,
};
use crate::hot::MonsterOverride;
use crate::ids::MonsterKind;

/// `ENCOUNTER.PUNCH_CONSTRUCT_NORMAL` — `normal.py::_punch_construct`.
///
/// One Punch Construct (`PunchConstructNormal::GenerateMonsters` `0xd48eb`,
/// IL_0001-IL_0016: one `ToMutable`, a null slot). Its band is a single value
/// (`get_MinInitialHp` `0xbbef7` / `get_MaxInitialHp` `0xbbf03`), so this is
/// `_EncounterCtx.fixed` and still spends one `Niche` draw.
///
/// `PunchConstruct/<AfterAddedToRoom>d__24::MoveNext` (`0x366c04`) applies
/// `Apply<ArtifactPower>` at IL_0097, which the oracle carries as
/// `artifact` — it is what eats a turn-1 relic debuff (#148).
pub fn build_punch_construct_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::PunchConstruct;
    let artifact = initial_power(kind, "ArtifactPower", ctx.ascension())?;
    Ok(vec![fixed(ctx, kind)?.with_state("artifact", artifact)])
}

/// `ENCOUNTER.SEWER_CLAM_NORMAL` — `normal.py::_sewer_clam`.
///
/// One Sewer Clam (`SewerClamNormal::GenerateMonsters` `0xd4dd1`, one
/// creation, null slot). `SewerClam::get_MaxInitialHp` (`0xbd3bf`) delegates
/// to `get_MinInitialHp` (`0xbd3b3`), so the band is one value and this is
/// `_EncounterCtx.fixed`.
///
/// `SewerClam/<AfterAddedToRoom>d__9::MoveNext` (`0x368f9c`) computes the
/// Plating amount inline — `ldc.i4.8; ldc.i4.s 9; ldc.i4.8; call
/// AscensionHelper::GetValueIfAscension; stloc.2` at IL_007f-IL_0088 — and
/// widens that local through `Decimal::op_Implicit` (IL_0095) into
/// `Apply<PlatingPower>` (IL_00a2). The oracle carries it as `mplating`.
pub fn build_sewer_clam_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::SewerClam;
    let plating = initial_power(kind, "PlatingPower", ctx.ascension())?;
    Ok(vec![fixed(ctx, kind)?.with_state("mplating", plating)])
}

/// `ENCOUNTER.BOWLBUGS_NORMAL` — `normal.py::_bowlbugs_normal`.
///
/// A Bowlbug Rock, then two workers drawn without replacement.
/// `BowlbugsNormal::GenerateMonsters` (`0xd3090`), in body order:
///
/// ```text
/// IL_003f..IL_0055  Monster<BowlbugRock>.ToMutable, slot _slotNames[0]  // creation 0
/// loop i in 0..2 (IL_00d7: ldc.i4.2; blt):
///   IL_0060..IL_0095  _workerValidCounts.Keys.Where(b__0).ToList()
///   IL_0098: call     EncounterModel::get_Rng
///   IL_009f: callvirt Rng::NextItem                       // one draw
///   IL_00ae: currentWorkers.Add(pick)
///   IL_00b6..IL_00ca  pick.ToMutable, slot _slotNames[i + 1] // creation i + 1
/// ```
///
/// The filter `<>c__DisplayClass10_0::<GenerateMonsters>b__0` (`0x387fa4`) is
/// `currentWorkers.Count(w => w == r) < _workerValidCounts[r]`, and the
/// `.cctor` (`0xd3174`) gives every worker a valid count of one, in insertion
/// order `[BowlbugEgg, BowlbugSilk, BowlbugNectar]` — so each draw is over the
/// workers not yet chosen, in that order. `_slotNames` is `first`, `middle`,
/// `last`: one slot for the Rock and one per worker, which is where the loop's
/// two iterations come from.
///
/// The native creates the Rock *before* the two draws; the oracle draws first.
/// The draws come from the per-fight `Encounter` stream and the HP rolls from
/// `Niche`, so the observable is identical, and this mirrors the oracle
/// because parity with `make_monsters` is what E4b measures — the shape of
/// [`super::normal_a`]'s scrolls.
///
/// Every band here is disjoint from every other, so each creation rolls
/// against an empty uniqueness set, as the oracle does.
pub fn build_bowlbugs_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const WORKERS: [MonsterKind; 3] = [
        MonsterKind::BowlbugEgg,
        MonsterKind::BowlbugSilk,
        MonsterKind::BowlbugNectar,
    ];
    const SLOTS: [&str; 3] = ["first", "middle", "last"];
    let roll = "Encounter-stream worker roster rolls";
    let mut remaining: Vec<MonsterKind> = WORKERS.to_vec();
    let mut chosen = Vec::with_capacity(SLOTS.len() - 1);
    for _ in 1..SLOTS.len() {
        let pick = ctx.encounter_next_int(roll, 0, remaining.len() as i32)?;
        chosen.push(remaining.remove(pick as usize));
    }
    let mut roster = vec![rolled(ctx, MonsterKind::BowlbugRock, &[])?.slot(0)];
    for (slot, kind) in (1..).zip(chosen) {
        roster.push(rolled(ctx, kind, &[])?.slot(slot));
    }
    Ok(roster)
}

/// `ENCOUNTER.LOUSE_PROGENITOR_NORMAL` — `normal.py::_louse_progenitor`.
///
/// One Louse Progenitor (`LouseProgenitorNormal::GenerateMonsters`
/// `0xd437f`, one creation, null slot) over its real band
/// (`get_MinInitialHp` `0xb8e25` / `get_MaxInitialHp` `0xb8e37`).
/// `LouseProgenitor/<AfterAddedToRoom>d__29::MoveNext` (`0x362804`) applies
/// `Apply<CurlUpPower>` at IL_009d, tiered at A8, which the oracle carries as
/// `curl_up`; the per-card latch that power drives is engine behaviour.
pub fn build_louse_progenitor_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::LouseProgenitor;
    let curl_up = initial_power(kind, "CurlUpPower", ctx.ascension())?;
    Ok(vec![
        rolled(ctx, kind, &[])?
            .slot(0)
            .with_state("curl_up", curl_up),
    ])
}

/// `ENCOUNTER.RUBY_RAIDERS_NORMAL` — `normal.py::_ruby_raiders`.
///
/// Three raiders drawn without replacement, each created as it is drawn.
/// `RubyRaidersNormal::GenerateMonsters` (`0xd4a84`):
///
/// ```text
/// loop i in 0..3 (IL_0090: ldc.i4.3; blt):
///   IL_0027..IL_005c  _raiderValidCounts.Keys.Where(b__0).ToList()
///   IL_005e: call     EncounterModel::get_Rng
///   IL_0064: callvirt Rng::NextItem                 // one draw
///   IL_0073: currentRaiders.Add(pick)
///   IL_0079..IL_0086  pick.ToMutable, null slot     // creation i
/// ```
///
/// The filter (`<>c__DisplayClass5_0::<GenerateMonsters>b__0` `0x388060`) is
/// the Bowlbugs one — `currentRaiders.Count(w => w == r) <
/// _raiderValidCounts[r]` — and the `.cctor` (`0xd4b24`) gives each of the
/// five raiders a count of one, in insertion order `[AxeRubyRaider,
/// AssassinRubyRaider, BruteRubyRaider, CrossbowRubyRaider,
/// TrackerRubyRaider]`. The three `ToMutable` creations are what the loop's
/// bound counts.
///
/// Each raider's band overlaps others' (Assassin and Crossbow share most of
/// theirs), so the creations share one uniqueness set:
/// `Creature::SetUniqueMonsterHpValue` excludes every creature already rolled.
/// The native interleaves draw and creation; the oracle draws all three first.
/// Disjoint streams, so the observable is identical and this mirrors the
/// oracle.
pub fn build_ruby_raiders_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const RAIDERS: [MonsterKind; 5] = [
        MonsterKind::AxeRubyRaider,
        MonsterKind::AssassinRubyRaider,
        MonsterKind::BruteRubyRaider,
        MonsterKind::CrossbowRubyRaider,
        MonsterKind::TrackerRubyRaider,
    ];
    // The loop bound at IL_0090: how many `ToMutable` creations the body makes.
    const CREATIONS: usize = 3;
    let roll = "Encounter-stream Ruby Raider roster rolls";
    let mut remaining: Vec<MonsterKind> = RAIDERS.to_vec();
    let mut chosen = Vec::with_capacity(CREATIONS);
    for _ in 0..CREATIONS {
        let pick = ctx.encounter_next_int(roll, 0, remaining.len() as i32)?;
        chosen.push(remaining.remove(pick as usize));
    }
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(CREATIONS);
    for (slot, kind) in (0..).zip(chosen) {
        let spec = rolled(ctx, kind, &taken)?.slot(slot);
        taken.push(spec.hp);
        roster.push(spec);
    }
    Ok(roster)
}

/// `ENCOUNTER.HAUNTED_SHIP_NORMAL` — `normal.py::_haunted_ship`.
///
/// One Haunted Ship (`HauntedShipNormal::GenerateMonsters` `0xd3f39`, one
/// creation, null slot), single-value band (`get_MinInitialHp` `0xb630e` /
/// `get_MaxInitialHp` `0xb631a`): `_EncounterCtx.fixed`.
pub fn build_haunted_ship_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::HauntedShip)?])
}

/// `ENCOUNTER.SEAPUNK_NORMAL` — `normal.py::_seapunk_normal`.
///
/// `SeapunkNormal::GenerateMonsters` (`0xd4d45`) builds a two-element array:
/// a Calcified Cultist at index 0 (IL_0009) and the Seapunk at index 1
/// (IL_0020), both with null slots. The oracle pins `slot` to the array index.
/// The bands are disjoint, so each creation rolls against an empty set.
pub fn build_seapunk_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![
        rolled(ctx, MonsterKind::CalcifiedCultist, &[])?.slot(0),
        rolled(ctx, MonsterKind::Seapunk, &[])?.slot(1),
    ])
}

/// `ENCOUNTER.CHOMPERS_NORMAL` — `normal.py::_chompers`.
///
/// Two Chompers. `ChompersNormal::GenerateMonsters` (`0xd3360`) creates
/// `loc0` (IL_000c) then `loc1` (IL_001c), and calls
/// `Chomper::set_ScreamFirst(true)` on **`loc1` only** (IL_002c-IL_002e); the
/// array is `[(loc0, null), (loc1, null)]`. `ScreamFirst` is what
/// `Chomper::GenerateMoveStateMachine` (`0xb14f0`) reads to pick the initial
/// state, so the first Chomper opens on `CLAMP_MOVE` and the second on
/// `SCREECH_MOVE` — out of phase for the whole fight. Both positions are read
/// out of `content_tables::LOOPS` by name.
///
/// One band, so one shared uniqueness set. Each carries
/// `Chomper/<AfterAddedToRoom>d__15::MoveNext`'s (`0x356460`)
/// `Apply<ArtifactPower>` (IL_0098) as `artifact`.
pub fn build_chompers_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const OPENERS: [&str; 2] = ["CLAMP_MOVE", "SCREECH_MOVE"];
    let kind = MonsterKind::Chomper;
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    let artifact = initial_power(kind, "ArtifactPower", ctx.ascension())?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(OPENERS.len());
    for (slot, opener) in (0..).zip(OPENERS) {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(kind, hp)
                .loop_pos(loop_position(kind, opener)?)
                .slot(slot)
                .with_state("artifact", artifact),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.FROG_KNIGHT_NORMAL` — `normal.py::_frog_knight`.
///
/// One Frog Knight (`FrogKnightNormal::GenerateMonsters` `0xd3dd2`, one
/// creation, null slot), single-value band (`get_MinInitialHp` `0xb5311` /
/// `get_MaxInitialHp` `0xb5323`): `_EncounterCtx.fixed`.
/// `FrogKnight/<AfterAddedToRoom>d__24::MoveNext` (`0x35c6c0`) applies
/// `Apply<PlatingPower>` at IL_009d, which the oracle carries as `mplating`.
pub fn build_frog_knight_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::FrogKnight;
    let plating = initial_power(kind, "PlatingPower", ctx.ascension())?;
    Ok(vec![fixed(ctx, kind)?.with_state("mplating", plating)])
}

/// `ENCOUNTER.SNAPPING_JAXFRUIT_NORMAL` — `normal.py::_snapping_jaxfruit`.
///
/// `SnappingJaxfruitNormal::GenerateMonsters` (`0xd5433`): Snapping Jaxfruit
/// at index 0 (IL_0009), Flyconid at index 1 (IL_0020), null slots; the oracle
/// pins `slot` to the index. Disjoint bands, empty uniqueness sets. The
/// Flyconid's opening intent is a turn-1 `MonsterAi` roll over its INITIAL
/// `RandomBranchState`, which is engine work (#2528), so its `next_move`
/// stays empty here exactly as it does in the oracle.
pub fn build_snapping_jaxfruit_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![
        rolled(ctx, MonsterKind::SnappingJaxfruit, &[])?.slot(0),
        rolled(ctx, MonsterKind::Flyconid, &[])?.slot(1),
    ])
}

/// `ENCOUNTER.SPINY_TOAD_NORMAL` — `normal.py::_spiny_toad`.
///
/// One Spiny Toad (`SpinyToadNormal::GenerateMonsters` `0xd550f`, one
/// creation, null slot) over its real band (`get_MinInitialHp` `0xbf87b` /
/// `get_MaxInitialHp` `0xbf887`). No spawn-time power: Thorns starts at zero.
pub fn build_spiny_toad_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![rolled(ctx, MonsterKind::SpinyToad, &[])?])
}

/// `ENCOUNTER.SLIMES_NORMAL` — `normal.py::_slimes_normal`.
///
/// `SlimesNormal::GenerateMonsters` (`0xd4ed8`), in body order:
///
/// ```text
/// IL_000d: call     EncounterModel::get_Rng
/// IL_0012: callvirt Rng::NextBool                     // ONE draw -> b
/// IL_0018..IL_0027  small1 = b ? LeafSlimeS : TwigSlimeS
/// IL_0028..IL_0037  small2 = b ? TwigSlimeS : LeafSlimeS
/// IL_0038..IL_0092  [ TwigSlimeM, LeafSlimeM, small1, small2 ], null slots
/// ```
///
/// The mediums are **not** rolled — only which small slime comes first is.
/// (`get_AllPossibleMonsters` lists the four in a different order; it is the
/// possible-set, and `GenerateMonsters` is the roster.)
///
/// `Rng::NextBool` (`0x5eb07`) is `_counter += 1; return _random.Next(2) ==
/// 0` (IL_0001-IL_001c), and `Rng::NextInt(int, int)` (`0x5eb42`) spends the
/// same single `MegaRandom::Next` over the same two outcomes, so the bool is
/// [`encounter_next_bool`]'s `NextInt(0, 2) == 0` — one draw, one counter
/// step, identical value.
///
/// The roster itself is [`super::weak::slimes`], the shared slime primitive:
/// one shared uniqueness set across all four (the two small bands overlap),
/// and Twig Slime M's concrete `STICKY_SHOT_MOVE` opener.
pub fn build_slimes_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let leaf_first = encounter_next_bool(ctx, "Encounter-stream small-pair roll")?;
    let (small1, small2) = if leaf_first {
        (MonsterKind::LeafSlimeS, MonsterKind::TwigSlimeS)
    } else {
        (MonsterKind::TwigSlimeS, MonsterKind::LeafSlimeS)
    };
    super::weak::slimes(
        ctx,
        &[
            MonsterKind::TwigSlimeM,
            MonsterKind::LeafSlimeM,
            small1,
            small2,
        ],
    )
}

/// `ENCOUNTER.OVERGROWTH_CRAWLERS` — `normal.py::_overgrowth_crawlers`.
///
/// `OvergrowthCrawlers::GenerateMonsters` (`0xd45fd`) makes no `Rng` call at
/// all: Shrinker Beetle at index 0 (IL_0009), Fuzzy Wurm Crawler at index 1
/// (IL_0020), null slots; the oracle pins `slot` to the index. Disjoint bands.
pub fn build_overgrowth_crawlers(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![
        rolled(ctx, MonsterKind::ShrinkerBeetle, &[])?.slot(0),
        rolled(ctx, MonsterKind::FuzzyWurmCrawler, &[])?.slot(1),
    ])
}

/// `ENCOUNTER.AXEBOTS_NORMAL` — `normal.py::_axebots`.
///
/// One Axebot. `AxebotsNormal::GenerateMonsters` (`0xd2eed`) is
/// `Monster<Axebot>.ToMutable()` in slot `front` (IL_0001-IL_001a) and
/// nothing else — in particular it never calls `Axebot::set_StockAmount`
/// (`0xaede1`), so the creature is built with `_stockOverrideAmount` unset.
///
/// That is what makes the generated band exact for this encounter.
/// `Axebot::get_MinInitialHp` (`0xaed95`) and `get_MaxInitialHp` (`0xaeda8`)
/// are `GetValueIfAscension(8, …) + get_RespawnMaxHpBonus()`;
/// `get_RespawnMaxHpBonus` (`0xaedbb`) is `get_RespawnCount() * 10`,
/// `get_RespawnCount` (`0xaedc6`) is `2 - get_StockAmount()`, and
/// `get_StockAmount` (`0xaedd3`) is `_stockOverrideAmount.GetValueOrDefault(2)`
/// — so with the override unset the bonus is zero and the band is the tiered
/// constant. `content_tables::MONSTER_MODELS` evaluates exactly that chain
/// under exactly that premise (the unset override), and this builder is where
/// the premise is checked against the encounter's IL.
///
/// A real band, so `SetUniqueMonsterHpValue` rolls one `Niche` draw over it.
/// `Axebot/<AfterAddedToRoom>d__34::MoveNext` (`0x352dcc`) applies
/// `Apply<StockPower>(get_StockAmount())` behind `get_StockAmount() > 0`
/// (IL_001e-IL_003f) — taken, with the override unset — which the oracle
/// carries as `stock`.
pub fn build_axebots_normal(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let kind = MonsterKind::Axebot;
    let stock = initial_power(kind, "StockPower", ctx.ascension())?;
    Ok(vec![rolled(ctx, kind, &[])?.with_state("stock", stock)])
}

/// `ENCOUNTER.SLUMBERING_BEETLE_NORMAL` — `normal.py::_slumbering_beetle`
/// (#2848).
///
/// A fixed three-creation roster, `SlumberingBeetleNormal::GenerateMonsters`
/// (`0xd5370`), in body order:
///
/// ```text
/// IL_0014..IL_0028  Monster<BowlbugRock>.ToMutable, slot 'first'         // creation 0
/// IL_002f..IL_0043  Monster<BowlbugSilk>.ToMutable, slot 'second'        // creation 1
/// IL_004a..IL_005e  Monster<SlumberingBeetle>.ToMutable, slot 'third'    // creation 2
/// ```
///
/// Each creation is one `Niche` draw (`SetUniqueMonsterHpValue`); the three
/// bands are disjoint, so each rolls against an empty uniqueness set, and the
/// beetle's band is one value (`get_MinInitialHp` `0xbe4e0` /
/// `get_MaxInitialHp` `0xbe4ec`), which still spends its draw.
///
/// `SlumberingBeetle/<AfterAddedToRoom>d__22::MoveNext` (`0x36a9bc`) applies
/// `Apply<PlatingPower>(get_PlatingAmount())` (IL_0096-IL_00a8) and then
/// `Apply<SlumberPower>` (IL_011f), which the oracle carries as `mplating`
/// and `slumber`, both amounts from `content_tables::MONSTER_MODELS`.
///
/// `SlumberingBeetle::GenerateMoveStateMachine` (`0xbe5f4`) builds
/// `SNORE_MOVE` (IL_0012-IL_0036, a `SleepIntent`), `ROLL_OUT_MOVE` and the
/// `SNORE_NEXT` branch, and hands `SNORE_MOVE` (`ldloc.1`, IL_00bf) to the
/// `MonsterMoveStateMachine` constructor as the initial state. The oracle
/// encodes "asleep in `SNORE_MOVE`" as `override = "SNORE"`, and the engine's
/// vocabulary for it is [`MonsterOverride::BeetleSnore`], whose wire string is
/// read from there rather than retyped here.
pub fn build_slumbering_beetle_normal(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let beetle = MonsterKind::SlumberingBeetle;
    let plating = initial_power(beetle, "PlatingPower", ctx.ascension())?;
    let slumber = initial_power(beetle, "SlumberPower", ctx.ascension())?;
    let rock = rolled(ctx, MonsterKind::BowlbugRock, &[])?.slot(0);
    let silk = rolled(ctx, MonsterKind::BowlbugSilk, &[])?.slot(1);
    // In `combat_sim.Monster`'s field order (`override` :5609, `mplating`
    // :5667, `slumber` :5769), which is the order the roster pins record.
    let sleeper = fixed(ctx, beetle)?
        .slot(2)
        .with_text("override", MonsterOverride::BeetleSnore.as_str())
        .with_state("mplating", plating)
        .with_state("slumber", slumber);
    Ok(vec![rock, silk, sleeper])
}

/// One `Rng::NextBool` on the per-fight `Encounter` stream.
///
/// `Rng::NextBool` (`0x5eb07`): `_counter += 1` (IL_0001-IL_000a), then
/// `_random.Next(2) == 0` (IL_000f-IL_001c) — the same single draw
/// `Rng::NextInt(0, 2)` makes, compared with zero. The `2` is the method's own
/// operand (IL_0015 `ldc.i4.2`), not a content number.
fn encounter_next_bool(
    ctx: &mut dyn EncounterCtx,
    roll: &'static str,
) -> Result<bool, RosterRefusal> {
    Ok(ctx.encounter_next_int(roll, 0, 2)? == 0)
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

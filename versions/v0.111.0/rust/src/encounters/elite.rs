//! Elite-pool encounter rosters — E4b family F2 (#2532).
//!
//! Ported from `versions/v0.111.0/solver/content/encounters/elite.py`, builder
//! for builder. Twelve registered keys, ten of which `make_monsters` matches
//! as a proper **substring** of the wire id
//! (`ENCOUNTER.DECIMILLIPEDE_ELITE` is built by the builder registered under
//! `DECIMILLIPEDE`); the substring dispatch itself is the engine's (#2528).
//!
//! Every number below is read from a generated table. HP bands and
//! spawn-time power amounts come from `content_tables::MONSTER_MODELS`,
//! extracted from `MonsterModel::get_MinInitialHp` / `get_MaxInitialHp` and
//! the `Apply<XPower>` sites of each class's
//! `<AfterAddedToRoom>d__N::MoveNext` in the archived v0.111.0 assembly
//! (sha256 `9cb4f1ad…`). The two per-slot starter orders come from
//! `content_tables::encounter_pool_constants::elite`, because they live in
//! `GenerateMoveStateMachine` branch structure rather than in any table.
//!
//! Current-build IL, re-derived for this port rather than inherited from
//! Python's frozen RVAs:
//!
//! | class | `get_MinInitialHp` | spawn-time power |
//! |---|---|---|
//! | `TerrorEel` | `0xbfd88` | `ShriekPower` (`0x36cfe4`) |
//! | `SkulkingColony` | `0xbd889` | `HardenedShellPower` (`0x3697a4`) |
//! | `BygoneEffigy` | `0xb0354` | `SlowPower` (`0x3548ec`) |
//! | `Byrdonis` | `0xb065c` | `TerritorialPower` (`0x354f1c`) |
//! | `Entomancer` | `0xb32d9` | `PersonalHivePower` (`0x35959c`) |
//! | `InfestedPrism` | `0xb68a1` | `VitalSparkPower` (`0x35ee10`) |
//! | `PhantasmalGardener` | `0xbb743` | `SkittishPower` (`0x366124`) |
//! | `PhrogParasite` | `0xbbd42` | `InfestedPower` (`0x3667dc`) |
//! | `DecimillipedeSegment` | `0xb2ae8` | `ReattachPower` (`0x3586e0`) |
//! | `SoulNexus` | `0xbf26a` | — |
//! | `MechaKnight` | `0xb98c5` | `ArtifactPower` (`0x363a9c`) |
//! | `FlailKnight` / `SpectralKnight` / `MagiKnight` | `0xb478a` / `0xbf5bc` / `0xb9213` | — |
//!
//! Three of those spawn-time powers are deliberately **not** roster state
//! here, because they are not roster state in the oracle either:
//! `SkulkingColony`'s `HardenedShellPower` 20, `BygoneEffigy`'s `SlowPower` 1
//! (`Monster.slow` is a cards-played counter that starts at zero) and
//! `PhrogParasite`'s `InfestedPower` 4 are modeled in the engine, and
//! `DecimillipedeSegment`'s `ReattachPower` 25 is the reattach cycle ported
//! in #2516. Parity here is with `make_monsters`' output.

use crate::content_tables::encounter_pool_constants::elite as constants;
use crate::encounters::{EncounterCtx, MonsterSpec, RosterRefusal, fixed, hp_band, initial_power};
use crate::ids::MonsterKind;

/// `ENCOUNTER.TERROR_EEL` — `content/encounters/elite.py::_eel`.
///
/// One fixed-HP eel carrying `ShriekPower`. `Monster.shriek` is a presence
/// flag in the oracle, so the amount (`TerrorEel::get_ShriekAmount`,
/// `0xbfda2`) is read only to prove the power is applied at all.
///
/// **A flag, not an amount, natively** — re-read this session on the archived
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…`, the one `solver/dll-archive/index.json`
/// records). `TerrorEel::get_ShriekAmount` (RVA `0xbfda2`) is
/// `AscensionHelper.GetValueIfAscension(8, 75, 70)`, and
/// `TerrorEel/<AfterAddedToRoom>d__25::MoveNext` `IL_008b`-`IL_009d` applies
/// exactly one `ShriekPower` with it. That Amount is an **HP threshold**, not
/// a stack: `ShriekPower/<AfterDamageReceived>d__10::MoveNext` (RVA
/// `0x344348`) gates on `UnblockedDamage > 0` (`IL_0040`-`IL_0046`) and
/// `target.CurrentHp > PowerModel.Amount` (`IL_0053`-`IL_005e`), Stuns into
/// `TerrorEel.TerrorState` (`IL_0081`-`IL_008b`) and then **removes itself**
/// with `PowerCmd::Remove` (`IL_00e5`-`IL_00e6`). Nothing ever increments it,
/// so the only state the oracle carries is present-or-gone —
/// the frozen Python (deleted #2827) field `shriek: bool = False`, cleared by
/// `damage_monster` on that removal and type-asserted by
/// `_validated_tag_team_instances` (`if type(target.shriek) is not bool`). Python's own
/// builder spells it `ctx.fixed(EEL, 150, shriek=True)`
/// (`content/encounters/elite.py::_eel`), and `with_state("shriek", 1)` emitted
/// `int 1` where the oracle emits `bool True` (#2790).
pub fn build_terror_eel(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    initial_power(MonsterKind::TerrorEel, "ShriekPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, MonsterKind::TerrorEel)?.with_flag("shriek", true),
    ])
}

/// `ENCOUNTER.SKULKING_COLONY` — `content/encounters/elite.py::_colony`.
pub fn build_skulking_colony(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::SkulkingColony)?])
}

/// `ENCOUNTER.BYGONE_EFFIGY` — `content/encounters/elite.py::_effigy`.
pub fn build_bygone_effigy(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    Ok(vec![fixed(ctx, MonsterKind::BygoneEffigy)?])
}

/// `ENCOUNTER.BYRDONIS` — `content/encounters/elite.py::_byrdonis`.
///
/// `TerritorialPower` is applied with `Decimal.One`, so the Strength gained
/// per side end is 1 at every ascension.
///
/// Not [`fixed`]: `MONSTER_MODELS` records a fixed 90 at A8+ but a real
/// 81..=84 band below A8 (#2539), so the creation is the
/// `SetUniqueMonsterHpValue` draw over the tier's band — which over a
/// single-value band is exactly the one `Niche` draw `fixed` spends.
pub fn build_byrdonis(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let territorial = initial_power(MonsterKind::Byrdonis, "TerritorialPower", ctx.ascension())?;
    let (lo, hi) = hp_band(MonsterKind::Byrdonis, ctx.ascension())?;
    let hp = ctx.unique_hp(lo, hi, &[]);
    Ok(vec![
        MonsterSpec::new(MonsterKind::Byrdonis, hp).with_state("territorial", territorial),
    ])
}

/// `ENCOUNTER.ENTOMANCER` — `content/encounters/elite.py::_entomancer`.
pub fn build_entomancer(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let hive = initial_power(
        MonsterKind::Entomancer,
        "PersonalHivePower",
        ctx.ascension(),
    )?;
    Ok(vec![
        fixed(ctx, MonsterKind::Entomancer)?.with_state("hive", hive),
    ])
}

/// `ENCOUNTER.INFESTED_PRISMS` — `content/encounters/elite.py::_prisms`.
pub fn build_infested_prisms(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let vital = initial_power(
        MonsterKind::InfestedPrism,
        "VitalSparkPower",
        ctx.ascension(),
    )?;
    Ok(vec![
        fixed(ctx, MonsterKind::InfestedPrism)?.with_state("vital", vital),
    ])
}

/// `ENCOUNTER.PHANTASMAL_GARDENERS` — `content/encounters/elite.py::_gardeners`.
///
/// Four gardeners with pairwise-distinct max HP: each spends one
/// `SetUniqueMonsterHpValue` draw over the native band minus the values its
/// siblings already took, in creation order. The opening MoveState is per
/// slot (`GARDENER_STARTS`: FLAIL, BITE, LASH, ENLARGE), which
/// `PhantasmalGardener::GenerateMoveStateMachine` picks by branch rather than
/// from a table.
pub fn build_phantasmal_gardeners(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (lo, hi) = hp_band(MonsterKind::PhantasmalGardener, ctx.ascension())?;
    let skittish = initial_power(
        MonsterKind::PhantasmalGardener,
        "SkittishPower",
        ctx.ascension(),
    )?;
    let mut taken: Vec<i32> = Vec::new();
    let mut roster = Vec::with_capacity(constants::GARDENER_STARTS.len());
    for start in constants::GARDENER_STARTS {
        let hp = ctx.unique_hp(lo, hi, &taken);
        taken.push(hp);
        roster.push(
            MonsterSpec::new(MonsterKind::PhantasmalGardener, hp)
                .loop_pos(start as i32)
                .with_state("skittish", skittish),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.PHROG_PARASITE` — `content/encounters/elite.py::_phrog`.
///
/// A single phrog, but its native band is a range, so it still spends the
/// unique-HP draw over the whole band with nothing taken.
pub fn build_phrog_parasite(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (lo, hi) = hp_band(MonsterKind::PhrogParasite, ctx.ascension())?;
    let hp = ctx.unique_hp(lo, hi, &[]);
    Ok(vec![MonsterSpec::new(MonsterKind::PhrogParasite, hp)])
}

/// `ENCOUNTER.DECIMILLIPEDE` — `content/encounters/elite.py::_decimillipede`.
///
/// Three segments, front to back. One draw on this fight's `Encounter`
/// stream picks the starter offset; segment *n* opens on
/// `SEGMENT_START_TO_POS[(k + n) % 3]`, which is
/// `DecimillipedeSegment::GenerateMoveStateMachine` (`0xb2c04`) switching on
/// `StarterMoveIdx % 3` — `0` WRITHE, `1` BULK, `2` CONSTRICT, mapped onto
/// the oracle's rotation order.
///
/// Then `DecimillipedeSegment::<AfterAddedToRoom>d__46::MoveNext`
/// (`0x3586e0`) rounds each odd max HP UP to even, front to back, and adds 2
/// while the value collides with another segment's current max, wrapping to
/// `MinInitialHp` once it passes `MaxInitialHp`. `SetMaxAndCurrentHp` carries
/// the adjusted value into current HP too.
pub fn build_decimillipede(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let (lo, hi) = hp_band(MonsterKind::DecimillipedeSegment, ctx.ascension())?;
    let starters = constants::SEGMENT_START_TO_POS;
    let k = ctx.encounter_next_int("Encounter-stream starter roll", 0, starters.len() as i32)?;
    let mut roster: Vec<MonsterSpec> = Vec::with_capacity(starters.len());
    for slot in 0..starters.len() {
        let taken: Vec<i32> = roster.iter().map(|m| m.max_hp).collect();
        let hp = ctx.unique_hp(lo, hi, &taken);
        let start = starters[(k as usize + slot) % starters.len()];
        roster.push(MonsterSpec::new(MonsterKind::DecimillipedeSegment, hp).loop_pos(start as i32));
    }
    for index in 0..roster.len() {
        let mut value = roster[index].max_hp;
        if value % 2 == 1 {
            value += 1;
        }
        while roster
            .iter()
            .enumerate()
            .any(|(other, m)| other != index && m.max_hp == value)
        {
            value += 2;
            if value > hi {
                value = lo;
            }
        }
        roster[index].max_hp = value;
        roster[index].hp = value;
    }
    Ok(roster)
}

/// `ENCOUNTER.SOUL_NEXUS` — `content/encounters/elite.py::_soul_nexus`.
///
/// Random-AI, so no rotation: the machine's initial MoveState is `SOUL_BURN`,
/// used as-is with no `MonsterAi` draw, and logged like every rolled move.
pub fn build_soul_nexus(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const SOUL_BURN: &str = "SOUL_BURN";
    Ok(vec![
        fixed(ctx, MonsterKind::SoulNexus)?.opening_move(SOUL_BURN, &[SOUL_BURN]),
    ])
}

/// `ENCOUNTER.KNIGHTS_ELITE` — `content/encounters/elite.py::_knights`.
///
/// Fixed Flail / Spectral / Magi slot order. All three bands are single
/// values, so every one still spends its `SetUniqueMonsterHpValue` draw —
/// the oracle spends all three up front, before any monster is built, and
/// this reproduces that order. Flail and Spectral are hybrid random-AI kinds
/// and open on a concrete MoveState; Magi is deterministic and opens on its
/// rotation. The machine openers themselves consume no `MonsterAi` draws.
pub fn build_knights_elite(ctx: &mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    const KNIGHTS: [(MonsterKind, &str, &[&str]); 3] = [
        (MonsterKind::FlailKnight, "RAM_MOVE", &["RAM_MOVE"]),
        (MonsterKind::SpectralKnight, "HEX_MOVE", &["HEX_MOVE"]),
        (MonsterKind::MagiKnight, "", &[]),
    ];
    let mut bands = Vec::with_capacity(KNIGHTS.len());
    for (kind, _, _) in KNIGHTS {
        let (lo, hi) = hp_band(kind, ctx.ascension())?;
        if lo != hi {
            return Err(RosterRefusal::MonsterHpNotFixed { kind });
        }
        bands.push(lo);
    }
    for _ in 0..KNIGHTS.len() {
        ctx.niche_next_int(0, 1);
    }
    let mut roster = Vec::with_capacity(KNIGHTS.len());
    for (slot, ((kind, opener, log), hp)) in KNIGHTS.iter().zip(bands).enumerate() {
        roster.push(
            MonsterSpec::new(*kind, hp)
                .slot(slot as i32)
                .opening_move(opener, log),
        );
    }
    Ok(roster)
}

/// `ENCOUNTER.MECHA_KNIGHT_ELITE` — `content/encounters/elite.py::_mecha_knight`.
pub fn build_mecha_knight_elite(
    ctx: &mut dyn EncounterCtx,
) -> Result<Vec<MonsterSpec>, RosterRefusal> {
    let artifact = initial_power(MonsterKind::MechaKnight, "ArtifactPower", ctx.ascension())?;
    Ok(vec![
        fixed(ctx, MonsterKind::MechaKnight)?.with_state("artifact", artifact),
    ])
}

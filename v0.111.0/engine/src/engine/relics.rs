//! Table-driven relic-template execution.
//!
//! Python derives `TEMPLATE_RELICS` from `relic_templates.json`, compiles the
//! safe guard/effect vocabulary in `_relic_templates`, then interprets it in
//! `_fire_relic_templates` and `_relic_modifier_total`. The Rust content
//! generator already emits the same 18 programs and [`crate::hooks::HookTable`]
//! already preserves authenticated inventory order. This module is the small
//! missing interpreter between those two pieces.
//!
//! `BeforeCombatStart` and `AfterRoomEntered` joined the fire-point list at
//! #2528, and the two subscriber walks this module serves are still distinct.
//! They are fired **once per fight, by the opening**
//! ([`crate::entry::opening`]), never from a loaded canonical v2 document:
//! such a document enters after `start_combat`, so those effects are already
//! reflected in the projected state and replaying them would apply Data Disk's
//! Focus or Fake Anchor's Block a second time.
//! `diff_serve::tests::coverage_reports_the_kinds_the_played_line_actually_exercised`
//! pins that a played line records neither.

use crate::catalog::{Catalog, CompiledArg};
use crate::decimal::DotNetDecimal;
use crate::engine::{EngineRefusal, Event, Subject};
use crate::hooks::{HookEvent, HookSubject, Subscriber, TemplateCond, TemplateEffect};
use crate::hot::{
    CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_LEGACY, CardInstanceState, FrozenAutoBatchEntry,
    HotCard, HotPile, HotState, LEGACY_CARD_UID, LocalCostExpiration, LocalCostModifier,
    LocalCostModifierKind, MiseryToken, PendingSelection, PileId, RelicPendingKind,
    RelicPendingRecord, RngStream, RngStreamState,
};
use crate::ids::{CardId, PowerId, RelicId, StepWord};
use crate::powers::SlotWire;
use crate::rng::Xoshiro256StarStar;

fn integer_arg(args: &[CompiledArg], index: usize) -> Result<i64, EngineRefusal> {
    match args.get(index) {
        Some(CompiledArg::I(value)) => Ok(*value),
        _ => Err(EngineRefusal::MalformedArgs("relic template integer")),
    }
}

fn condition_matches(
    catalog: &Catalog,
    condition: &crate::hooks::CompiledCond,
    state: &HotState,
) -> Result<bool, EngineRefusal> {
    let args = catalog.hooks().args(condition.args);
    match condition.verb {
        // Python `_relic_cond_eval`: template fire points are player-owner
        // points by construction.
        TemplateCond::IsOwner | TemplateCond::OwnSide if args.is_empty() => Ok(true),
        // The only room represented by a combat entry is CombatRoom. This
        // condition belongs to the pre-entry AfterRoomEntered hook, but keep
        // the compiled interpreter total for tests and future entry synthesis.
        TemplateCond::RoomType => Ok(matches!(args, [CompiledArg::Word(StepWord::Combatroom)])),
        TemplateCond::Turn => match args {
            [CompiledArg::Word(comparison), CompiledArg::I(expected)] => {
                let actual = i64::from(state.turn);
                match comparison {
                    StepWord::Eq => Ok(actual == *expected),
                    StepWord::Le => Ok(actual <= *expected),
                    _ => Err(EngineRefusal::MalformedArgs(
                        "relic template turn comparison",
                    )),
                }
            }
            _ => Err(EngineRefusal::MalformedArgs("relic template turn guard")),
        },
        _ => Err(EngineRefusal::MalformedArgs("relic template guard")),
    }
}

fn rule_matches(
    catalog: &Catalog,
    rule: &crate::hooks::CompiledRule,
    state: &HotState,
) -> Result<bool, EngineRefusal> {
    for condition in catalog.hooks().conds(rule) {
        if !condition_matches(catalog, condition, state)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn add_player_power(
    state: &mut HotState,
    power: PowerId,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    match power {
        PowerId::Strength | PowerId::Dexterity => {
            super::damage::apply_signed_player_stat(state, power, amount, events)
        }
        PowerId::Focus | PowerId::Blur => {
            let updated = state
                .powers
                .value(power)
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("relic template power"))?;
            state.powers.set(power, SlotWire::Int, updated);
            super::damage::note_power(events, Subject::Player, power, updated);
            if power == PowerId::Blur
                && updated > 0
                && !state.fanouts.register_after_side_turn_start(PowerId::Blur)
            {
                return Err(EngineRefusal::CounterOverflow(
                    "AfterSideTurnStart power order",
                ));
            }
            Ok(())
        }
        _ => Err(EngineRefusal::MalformedArgs("relic template self power")),
    }
}

fn apply_effect(
    catalog: &Catalog,
    effect: &crate::hooks::CompiledEffect,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let args = catalog.hooks().args(effect.args);
    match effect.verb {
        TemplateEffect::Energy => {
            let amount: i16 = integer_arg(args, 0)?
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("relic template energy"))?;
            if args.len() != 1 {
                return Err(EngineRefusal::MalformedArgs("relic template energy"));
            }
            state.energy = state
                .energy
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("energy"))?;
            Ok(())
        }
        TemplateEffect::Block => {
            if args.len() != 1 {
                return Err(EngineRefusal::MalformedArgs("relic template block"));
            }
            super::orbs::gain_flat_block(state, catalog, integer_arg(args, 0)?, events)
        }
        TemplateEffect::Heal => {
            let amount: i32 = integer_arg(args, 0)?
                .try_into()
                .map_err(|_| EngineRefusal::CounterOverflow("relic template heal"))?;
            if args.len() != 1 || amount < 0 {
                return Err(EngineRefusal::MalformedArgs("relic template heal"));
            }
            if !state.history.over {
                let hp_before = state.hp;
                state.hp = state
                    .hp
                    .checked_add(amount)
                    .ok_or(EngineRefusal::CounterOverflow("player hp"))?
                    .min(state.max_hp);
                // A template heal is one `CreatureCmd.Heal` (`0x3eb4b0`):
                // Red Skull's `AfterCurrentHpChanged` (#3044).
                super::damage::red_skull_after_player_hp_changed(
                    state,
                    hp_before,
                    state.max_hp,
                    events,
                )?;
            }
            Ok(())
        }
        TemplateEffect::Stars => {
            if args.len() != 1 {
                return Err(EngineRefusal::MalformedArgs("relic template stars"));
            }
            super::play::gain_stars(state, catalog, integer_arg(args, 0)?, events)
        }
        TemplateEffect::PowerSelf => match args {
            [CompiledArg::Power(power), CompiledArg::I(amount)] => {
                let amount: i32 = (*amount)
                    .try_into()
                    .map_err(|_| EngineRefusal::CounterOverflow("relic template power"))?;
                add_player_power(state, *power, amount, events)
            }
            _ => Err(EngineRefusal::MalformedArgs("relic template self power")),
        },
        TemplateEffect::DamageAll => match args {
            [CompiledArg::I(amount), CompiledArg::I(properties)] => {
                let blockable = *properties == 4;
                if !blockable {
                    return Err(EngineRefusal::MalformedArgs(
                        "relic template damage properties",
                    ));
                }
                for target in super::damage::alive_targets(state) {
                    if state.history.over {
                        break;
                    }
                    super::damage::damage_monster_with_catalog(
                        state,
                        catalog,
                        target,
                        DotNetDecimal::from_i64(*amount),
                        false,
                        blockable,
                        events,
                    )?;
                }
                Ok(())
            }
            _ => Err(EngineRefusal::MalformedArgs("relic template damage")),
        },
        // These are consumed only by `modifier_total`, never applied.
        TemplateEffect::MaxEnergy | TemplateEffect::DrawBonus => Err(EngineRefusal::MalformedArgs(
            "relic template modifier at fire point",
        )),
        // Python can interpret this verb, but no effective v0.111.0 template
        // contains it. Keep the unclaimed future surface typed-refused.
        TemplateEffect::PowerAll => Err(EngineRefusal::MalformedArgs(
            "relic template all-enemy power",
        )),
    }
}

pub(crate) fn fire_template_subscriber(
    catalog: &Catalog,
    subscriber: &Subscriber,
    relic: RelicId,
    event: HookEvent,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !super::admission::IMPLEMENTED_RELICS.contains(&relic) {
        return Err(EngineRefusal::HookNotModeled { relic, event });
    }
    for rule in catalog.hooks().rules(subscriber) {
        if !rule_matches(catalog, rule, state)? {
            continue;
        }
        let effects = catalog.hooks().effects(rule);
        if !effects.is_empty() {
            crate::coverage::record_relic(relic);
        }
        for effect in effects {
            apply_effect(catalog, effect, state, events)?;
        }
    }
    Ok(())
}

pub(crate) fn modifier_total(
    catalog: &Catalog,
    event: HookEvent,
    state: &HotState,
) -> Result<i16, EngineRefusal> {
    let expected = match event {
        HookEvent::ModifyMaxEnergy => TemplateEffect::MaxEnergy,
        HookEvent::ModifyHandDraw => TemplateEffect::DrawBonus,
        _ => return Err(EngineRefusal::MalformedArgs("relic template modifier hook")),
    };
    let mut total = 0_i64;
    let subscribers = if catalog.hooks().has(event) {
        catalog.hooks().subscribers(event)
    } else {
        &[]
    };
    for subscriber in subscribers {
        match subscriber.subject {
            HookSubject::Relic(relic) => {
                if !super::admission::IMPLEMENTED_RELICS.contains(&relic) {
                    return Err(EngineRefusal::HookNotModeled { relic, event });
                }
                for rule in catalog.hooks().rules(subscriber) {
                    if !rule_matches(catalog, rule, state)? {
                        continue;
                    }
                    let mut matched = false;
                    for effect in catalog.hooks().effects(rule) {
                        if effect.verb != expected {
                            return Err(EngineRefusal::MalformedArgs(
                                "relic template modifier effect",
                            ));
                        }
                        let args = catalog.hooks().args(effect.args);
                        if args.len() != 1 {
                            return Err(EngineRefusal::MalformedArgs("relic template modifier"));
                        }
                        total = total
                            .checked_add(integer_arg(args, 0)?)
                            .ok_or(EngineRefusal::CounterOverflow("relic template modifier"))?;
                        matched = true;
                    }
                    if matched {
                        crate::coverage::record_relic(relic);
                    }
                }
            }
            HookSubject::Power(power) => {
                return Err(EngineRefusal::PowerHookNotModeled { power, event });
            }
        }
    }
    // Hand-authored scalar modifiers from Python `_player_max_energy` and
    // `_begin_player_turn_hand_draw`. Ownership is immutable catalog data, so
    // these add no per-node state and join the same commutative integer fold.
    match event {
        HookEvent::ModifyMaxEnergy => {
            if state.turn >= 3 && catalog.hooks().owns(RelicId::RelicPaelsFlesh) {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicPaelsFlesh);
            }
            if state.turn > 1 && catalog.hooks().owns(RelicId::RelicBread) {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicBread);
            }
            if catalog.hooks().owns(RelicId::RelicSpikedGauntlets) {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicSpikedGauntlets);
            }
            if catalog.hooks().owns(RelicId::RelicVelvetChoker) {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicVelvetChoker);
            }
            if catalog.hooks().owns(RelicId::RelicWhisperingEarring) {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicWhisperingEarring);
            }
            if state.fanouts.pumpkin_candle_kindle_count() > 0 {
                total = total
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                crate::coverage::record_relic(RelicId::RelicPumpkinCandle);
            }
            for relic in [RelicId::RelicBlessedAntler, RelicId::RelicPhilosophersStone] {
                if catalog.hooks().owns(relic) {
                    total = total
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                    crate::coverage::record_relic(relic);
                }
            }
        }
        HookEvent::ModifyHandDraw => {
            // ONE non-exclusive fold. Big Mushroom used to sit in a `match`
            // arm of its own guarded by `state.turn == 1`, which made it
            // *exclusive* with the rows below: a turn-1 hand draw holding Big
            // Mushroom skipped Fiddle, Pendulum, Ring of the Drake, Booming
            // Conch and Snecko Eye entirely. Python runs them as separate
            // statements over the same running total
            // (`_begin_player_turn_hand_draw`, frozen Python, deleted #2827), and native runs
            // them as separate listeners each returning `draw ± its own
            // CanonicalVar`, so they are additive and commute. Folding them
            // together is what keeps a turn-1 penalty from hiding a turn-1
            // bonus — and it is load-bearing for the two bag relics below,
            // whose whole domain is the turn Big Mushroom's row also fires on.
            for (owned, amount, relic) in [
                (
                    state.turn == 1 && catalog.hooks().owns(RelicId::RelicBigMushroom),
                    -2_i64,
                    RelicId::RelicBigMushroom,
                ),
                // The two bag relics: +2 on the first player turn only.
                // `BagOfPreparation::ModifyHandDraw` (v0.111.0 RVA `0x90434`)
                // and `RingOfTheSnake::ModifyHandDraw` (`0x9a749`) are the
                // same three-part body — owner check `IL_0001`-`IL_000b`,
                // `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
                // `IL_000c`-`IL_0020` (past turn one it returns the draw
                // unchanged), then `draw + DynamicVars.Cards.BaseValue` at
                // `IL_0021`-`IL_0037`. Both `get_CanonicalVars`
                // (`0x90427` / `0x9a73c`) are `ldc.i4.2; newobj
                // CardsVar::.ctor`, which is `combat_sim.BAG_DRAWS`. The
                // oracle folds the pair into one `State.bag_draws` field
                // (`start_combat`, frozen Python, deleted #2827) and adds it on turn one
                // (`_begin_player_turn_hand_draw`); `boundary.rs`'s
                // `"bag_draws"` mirror is that same sum, and a test pins this
                // fold against it so the two cannot drift.
                //
                // Native's `ble` and the oracle's `s.turn == 1` coincide
                // because `TurnNumber` is one-based, and the count is folded
                // *before* the single `fromHandDraw` Draw, so neither relic
                // interleaves with the deal or moves a card: the only thing
                // either changes is how many cards that one command takes, and
                // turn one's `Math.Max(innate count, modified draw)` clamp is
                // applied to the total afterwards.
                (
                    state.turn == 1 && catalog.hooks().owns(RelicId::RelicBagOfPreparation),
                    2,
                    RelicId::RelicBagOfPreparation,
                ),
                (
                    state.turn == 1 && catalog.hooks().owns(RelicId::RelicRingOfTheSnake),
                    2,
                    RelicId::RelicRingOfTheSnake,
                ),
                (
                    catalog.hooks().owns(RelicId::RelicFiddle),
                    2,
                    RelicId::RelicFiddle,
                ),
                (
                    state.turn <= 3 && catalog.hooks().owns(RelicId::RelicRingOfTheDrake),
                    2,
                    RelicId::RelicRingOfTheDrake,
                ),
                (
                    state.turn <= 1 && state.booming_conch_elite(),
                    2,
                    RelicId::RelicBoomingConch,
                ),
                // `Pendulum::ModifyHandDraw` (v0.111.0 RVA `0x99022`): owner
                // check `IL_0001`-`IL_000b`, then `brfalse.s` on
                // `get_TurnsSeen` at `IL_0012` — a NON-zero counter returns the
                // draw unchanged (`IL_0014`), and only a counter that has just
                // wrapped to zero reaches `draw +
                // DynamicVars.Cards.BaseValue` at `IL_0016`-`IL_002c`.
                // `get_CanonicalVars` (`0x98f3a`) builds `CardsVar(1)` and
                // `'Turns' = 3`, which are `combat_sim.PENDULUM_CARDS` and
                // `PENDULUM_TURNS`. There is no turn gate: the counter is
                // persistent across combats and fires every third player turn
                // it sees. Oracle: frozen Python `_begin_player_turn_hand_draw` (deleted #2827)
                // (`s.pendulum >= 0 and s.pendulum == 0`, where the first
                // conjunct is ownership — pinned by `admission.rs`'s
                // `(state.pendulum() >= 0) != hooks.owns(RelicPendulum)`).
                (
                    state.pendulum() == 0 && catalog.hooks().owns(RelicId::RelicPendulum),
                    1,
                    RelicId::RelicPendulum,
                ),
                (
                    state.turn > 1
                        && state.pocketwatch_last_plays() <= 3
                        && catalog.hooks().owns(RelicId::RelicPocketwatch),
                    3,
                    RelicId::RelicPocketwatch,
                ),
                (
                    catalog.hooks().owns(RelicId::RelicSneckoEye),
                    2,
                    RelicId::RelicSneckoEye,
                ),
            ] {
                if owned {
                    total = total
                        .checked_add(amount)
                        .ok_or(EngineRefusal::CounterOverflow("relic modifier"))?;
                    crate::coverage::record_relic(relic);
                }
            }
        }
        _ => {}
    }
    total
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("relic template modifier"))
}

/// The turn-1 all-enemy debuff relics, in the oracle's fixed order.
///
/// `combat_sim._TURN_ONE_ALL_ENEMY_DEBUFF_RELICS` (frozen Python, deleted #2827) names
/// three relics. Red Mask and Bag of Marbles arrived first (#2693);
/// `RELIC.TWISTED_FUNNEL` joined on 2026-10-10 (#3756), with its own section
/// below, because its Poison is the one application here that Snecko Skull
/// modifies and that allocates an instance identity.
///
/// # The native contract, re-read 2026-09-22
///
/// `sim/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`, sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`
/// (hash-verified in session). The two bodies are the same shape **offset for
/// offset**, which is what licenses one loop over a table rather than two
/// hand-written arms:
///
/// | | `RELIC.RED_MASK` | `RELIC.BAG_OF_MARBLES` |
/// |---|---|---|
/// | body | `RedMask/<BeforeSideTurnStart>d__6::MoveNext` `0x32f4b8` | `BagOfMarbles/<BeforeSideTurnStart>d__6::MoveNext` `0x31ee48` |
/// | participant check | `Contains<Creature>(participants, Owner.Creature)` `IL_0020`-`IL_0038` | same, `IL_0020`-`IL_0038` |
/// | turn guard | `get_TurnNumber; ldc.i4.1; ble.s` `IL_0043`-`IL_004e`, `leave` at `IL_0050` | same, `IL_0043`-`IL_004e`, `leave` at `IL_0050` |
/// | targets | `ICombatState::get_HittableEnemies` `IL_0067` | same, `IL_0067` |
/// | amount | `DynamicVars["WeakPower"].BaseValue` `IL_0072`-`IL_007c`; `get_CanonicalVars` `0x9a22e` is `Decimal::One` | `DynamicVarSet::get_Vulnerable` `IL_0072`-`IL_0077`; `get_CanonicalVars` `0x9038e` is `Decimal::One` |
/// | apply | `spec:Apply<WeakPower>` `IL_008e`, applier `Owner.Creature` `IL_0081`-`IL_0087`, card `ldnull` `IL_008c` | `spec:Apply<VulnerablePower>` `IL_0089`, applier `IL_007c`-`IL_0082`, card `ldnull` `IL_0087` |
///
/// The guard is `ble`, i.e. `TurnNumber <= 1`, so `state.turn <= 1` is the
/// literal port rather than the oracle's equivalent `== 1`; `IL_0056`'s
/// `RelicModel::Flash` is presentation and has no state.
///
/// Oracle: frozen Python `start_combat`, deleted #2827, which applies both inline right after
/// `make_monsters` rather than inside `begin_player_turn`.
///
/// # Artifact eats the debuff, and the order that decides *which* one
///
/// An innate enemy `ArtifactPower` is applied at `AfterAddedToRoom`, which
/// precedes the player's first `BeforeSideTurnStart`, so a monster entering
/// with Artifact consumes one stack per blocked **application** and the debuff
/// does not land at all (#148; the oracle's own comment (frozen Python `start_combat`, deleted #2827)). That gate is not re-implemented here — it is
/// `damage::apply_relic_monster_debuff`'s shared received-side command, the
/// same one every card and power source runs.
///
/// When a monster's Artifact is **neither** zero **nor** at least the number of
/// these relics the owner holds, *which* debuff is eaten depends on the relics'
/// `Player.Relics` dispatch order, which the run payload does not vouch for.
/// The oracle refuses that case (`partial_artifact`, frozen Python `start_combat`, deleted #2827)
/// and so does this port, one stage earlier and by its own name
/// (`OpeningRefusal::TurnOneDebuffPartialArtifact`) — so the fixed order below
/// is only ever observed where the outcome does not depend on it.
///
/// # `RELIC.TWISTED_FUNNEL` (#3756)
///
/// `TwistedFunnel/<BeforeSideTurnStart>d__6::MoveNext` (RVA `0x3336e8`), the
/// type's only hook (beside `get_Rarity` `0x9d2af`, `get_CanonicalVars`
/// `0x9d2b2`, `get_ExtraHoverTips` `0x9d2c4` and the constructor; no field
/// and no saved property):
///
/// * `Contains<Creature>(participants, Owner.Creature)` at `IL_0027`-`IL_003d`,
///   else `leave` at `IL_003f`;
/// * the turn guard `get_TurnNumber; ldc.i4.1; ble.s` at `IL_0044`-`IL_0055`,
///   else `leave` at `IL_0057`;
/// * presentation: `Flash` (`IL_005d`), one VFX node per hittable enemy
///   (`IL_0062`-`IL_00a8`) and `Cmd::CustomScaledWait` (`IL_00ba`-`IL_00cf`);
/// * a second, fresh `ICombatState::get_HittableEnemies` enumeration
///   (`IL_012d`-`IL_0147`), and per enemy
///   `PowerCmd.Apply<PoisonPower>(choiceContext, enemy,
///   DynamicVars["PoisonPower"].IntValue, Owner.Creature, null, false)` at
///   `IL_0163`-`IL_0192`, awaited one enemy at a time.
///
/// So it is the Red Mask shape with Poison: applier `Owner.Creature`
/// (`IL_0185`-`IL_018b`), a null card (`IL_0190`), amount
/// [`TWISTED_FUNNEL_POISON`]. Because the applier is the owner's Creature,
/// `SneckoSkull::ModifyPowerAmountGivenAdditive` (`0x9bb8d`: `PoisonPower`
/// test at `IL_0001`-`IL_0007`, `applier == Owner.Creature` at
/// `IL_000f`-`IL_001b`, `+ DynamicVars.Poison` at `IL_0023`-`IL_002e`) adds
/// its one, which is the shared seam's Snecko Skull term
/// (`damage::apply_relic_monster_debuff`); the same seam allocates the fresh
/// Poison instance identity and runs the Artifact gate. The frozen oracle's
/// opening applied this Poison without an identity (its call ran before a
/// `State` existed), which is why the relic stayed gated while Python was the
/// authority. Native is the authority now, and the `.mcr` census certifies
/// the seam's allocation on the fights that own the relic.
///
/// ## Order among the three
///
/// Native awaits the three bodies in `Player.Relics` order. Which debuff an
/// enemy acquires first is state: it is the Misery acquisition order
/// (`damage::record_misery_debuff_application`) and, with Artifact between
/// zero and the number of owned relics, which application is eaten (the
/// first recorded ones; the opening refuses that only on an unvouched
/// inventory, #3756). [`turn_one_all_enemy_debuff_rows`] therefore
/// follows a vouched inventory, and the opening refuses any two of the three
/// when the inventory is not vouched
/// (`entry::opening::all_enemy_debuff_order_is_recorded`). Until #3765 only a
/// Twisted Funnel owner followed the inventory, and Red Mask beside Bag of
/// Marbles ran in table order whatever the save recorded.
const TURN_ONE_ALL_ENEMY_DEBUFFS: [(RelicId, PowerId, MiseryToken, i32); 3] = [
    (RelicId::RelicRedMask, PowerId::Weak, MiseryToken::Weak, 1),
    (
        RelicId::RelicBagOfMarbles,
        PowerId::Vuln,
        MiseryToken::Vuln,
        1,
    ),
    (
        RelicId::RelicTwistedFunnel,
        PowerId::Poison,
        MiseryToken::Poison,
        TWISTED_FUNNEL_POISON,
    ),
];

/// Twisted Funnel's Poison amount.
///
/// `TwistedFunnel::get_CanonicalVars` (v0.111.0 RVA `0x9d2b2`) is
/// `ldc.i4.4; newobj Decimal; newobj PowerVar<PoisonPower>` at
/// `IL_0001`-`IL_0007`, which the body reads back as
/// `DynamicVars["PoisonPower"].IntValue` (`IL_016b`-`IL_017b`).
const TWISTED_FUNNEL_POISON: i32 = 4;

/// The owned rows of [`TURN_ONE_ALL_ENEMY_DEBUFFS`], in the order this engine
/// runs them.
///
/// Under a vouched inventory ([`crate::hooks::HookTable::dispatch_ordered`])
/// every owned row sits at its relic's first recorded position, which is
/// native's `Player.Relics` walk (`CombatState::IterateHookListeners`
/// `0x137409`). Without that provenance the rows keep table order; the
/// opening refuses two or more of them there, so table order is only ever
/// applied to a single row. See the table's "Order among the three".
fn turn_one_all_enemy_debuff_rows(catalog: &Catalog) -> Vec<(RelicId, PowerId, MiseryToken, i32)> {
    let hooks = catalog.hooks();
    let mut rows: Vec<_> = TURN_ONE_ALL_ENEMY_DEBUFFS
        .into_iter()
        .filter(|(relic, ..)| hooks.owns(*relic))
        .collect();
    if hooks.dispatch_ordered() {
        rows.sort_by_key(|(relic, ..)| hooks.relics().iter().position(|owned| owned == relic));
    }
    rows
}

/// The turn-1 all-enemy debuff relic bodies ([`TURN_ONE_ALL_ENEMY_DEBUFFS`]).
///
/// # Why it is here and not in the opening
///
/// The oracle applies these to the freshly created roster inside
/// `start_combat`, above the `State` it constructs; this port runs them at the
/// native fire point instead, so the engine and
/// `entry::opening` — which reaches `begin_player_turn` through
/// `engine::deal_opening_hand` — run **the same code**. A body written into
/// `pre_hook_document` would have made a Rust-opened document and a
/// post-opening document disagree, which is the defect class #2731 gated.
///
/// # Position inside the walk
///
/// Python's position is earlier than any `begin_player_turn` listener; this
/// runs at the top of the relic group, after the compiled template pass. The
/// reordering is order-equivalent over the represented surface: every other
/// `BeforeSideTurnStart` peer this crate runs is a private-state reset
/// (History Course, Music Box, Regalite, Beating Remnant, Mini Regent, Demon
/// Tongue, Emotion Chip, Pocketwatch, Kunai, Shuriken, Ornamental Fan, Rainbow
/// Ring), Aggression's Discard-uid freeze, or Power Cell's local-cost-ordered
/// Draw shuffle — none of which reads a monster's Weak or Vulnerable, and none
/// of which these two writes can reach. Twisted Funnel's Poison is the same:
/// no peer on this hook reads a monster's Poison, Cracked Core's Lightning
/// channel evokes nothing on turn one, and Poison first ticks on the enemy's
/// own side turn.
fn turn_one_all_enemy_debuffs(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 {
        return Ok(());
    }
    for (relic, power, token, amount) in turn_one_all_enemy_debuff_rows(catalog) {
        // `ICombatState::get_HittableEnemies` (`0x1373a5`) is enumerated fresh
        // per body, and no application here can change liveness.
        for target in super::damage::alive_targets(state) {
            if state.history.over {
                break;
            }
            super::damage::apply_relic_monster_debuff(state, target, power, token, amount, events)?;
        }
        crate::coverage::record_relic(relic);
    }
    Ok(())
}

/// Owner-relic `BeforeSideTurnStart` state that is observed by later hooks.
pub(crate) fn before_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    turn_one_all_enemy_debuffs(catalog, state, events)?;
    if catalog.hooks().owns(RelicId::RelicHistoryCourse) {
        state.fanouts.roll_history_course_attack_turn();
        crate::coverage::record_relic(RelicId::RelicHistoryCourse);
    }
    if catalog.hooks().owns(RelicId::RelicMusicBox) {
        state.fanouts.set_music_box_used_this_turn(false);
        state.fanouts.set_music_box_card_uid(None);
        crate::coverage::record_relic(RelicId::RelicMusicBox);
    }
    if catalog.hooks().owns(RelicId::RelicRegalite) {
        state.fanouts.set_regalite_used_this_turn(false);
        crate::coverage::record_relic(RelicId::RelicRegalite);
    }
    if state.fanouts.beating_remnant_owned() {
        let written = state.fanouts.set_beating_remnant_damage_received(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicBeatingRemnant);
    }
    if catalog.hooks().owns(RelicId::RelicMiniRegent) {
        state.fanouts.set_mini_regent_used(false);
        crate::coverage::record_relic(RelicId::RelicMiniRegent);
    }
    if state.demon_tongue_owned() {
        state.set_demon_tongue_triggered(false);
        crate::coverage::record_relic(RelicId::RelicDemonTongue);
    }
    if state.emotion_chip_owned() {
        state.set_emotion_damage_previous_turn(state.emotion_damage_current_turn());
        state.set_emotion_damage_current_turn(false);
        crate::coverage::record_relic(RelicId::RelicEmotionChip);
    }
    if catalog.hooks().owns(RelicId::RelicPocketwatch) {
        let current = state.history.owner_card_plays_finished_this_turn;
        let written = state.set_pocketwatch_last_plays(current);
        debug_assert!(written, "admission bounds the Pocketwatch counter");
        crate::coverage::record_relic(RelicId::RelicPocketwatch);
    }
    if catalog.hooks().owns(RelicId::RelicKunai) {
        let written = state.set_kunai(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicKunai);
    }
    if catalog.hooks().owns(RelicId::RelicShuriken) {
        let written = state.set_shuriken(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicShuriken);
    }
    if catalog.hooks().owns(RelicId::RelicOrnamentalFan) {
        let written = state.set_ornamental_fan(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicOrnamentalFan);
    }
    if catalog.hooks().owns(RelicId::RelicRainbowRing) {
        let written = state.set_rainbow_ring_types_this_turn(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicRainbowRing);
    }
    cracked_core_before_side_turn_start(catalog, state, events)?;
    power_cell_before_side_turn_start(catalog, state)?;
    Ok(())
}

/// The number of Lightning orbs `CrackedCore` channels on turn one.
///
/// `CrackedCore::get_CanonicalVars` (v0.111.0 RVA `0x9248a`) builds one
/// `DynamicVar("Lightning", Decimal::One)` at `IL_0001`-`IL_000b`, which the
/// body's loop bound reads. `combat_sim.CRACKED_CORE_CHANNELS`.
const CRACKED_CORE_CHANNELS: usize = 1;

/// Cracked Core's turn-one `Channel<LightningOrb>` (#2827).
///
/// `CrackedCore/<BeforeSideTurnStart>d__7::MoveNext` (v0.111.0 RVA
/// `0x3221d4`): `participants.Contains(Owner.Creature)` at `IL_0020`-`IL_0036`
/// (solo: the owner's side is the only one that reaches this walk), then
/// `PlayerCombatState.TurnNumber <= 1` at `IL_003d`-`IL_004e` (`ble.s`, else
/// `leave`), then a loop of `OrbCmd::Channel<LightningOrb>` at `IL_0069` over
/// [`CRACKED_CORE_CHANNELS`]. It writes nothing else.
///
/// The oracle is `begin_player_turn`'s `s.cracked_core and s.turn <= 1`
/// block (frozen Python, deleted #2827), and it channels through the same
/// `channel` this calls. The turn guard makes the body reachable only from the
/// opening's first turn-start walk ([`super::deal_opening_hand`]): every other
/// entry into the walk has already advanced `turn` past one, and a
/// Python-rooted document carries the Lightning the oracle's own opening
/// channeled. The same-hook peers whose order around this channel is
/// observable (Bread, Symbiotic Virus, Runic Capacitor, Infused Core, Fencing
/// Manual, Brimstone) are refused by name in `engine::admission`.
fn cracked_core_before_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || !catalog.hooks().owns(RelicId::RelicCrackedCore) {
        return Ok(());
    }
    for _ in 0..CRACKED_CORE_CHANNELS {
        if state.history.over {
            break;
        }
        super::orbs::channel(state, catalog, crate::hot::OrbKind::Lightning, events)?;
    }
    crate::coverage::record_relic(RelicId::RelicCrackedCore);
    Ok(())
}

/// Power Cell's turn-one StableShuffle over the exact zero-local-cost Draw
/// snapshot. The selected physical cards move to Hand/Bottom, redirecting to
/// Discard when the hand is full.
fn power_cell_before_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || !catalog.hooks().owns(RelicId::RelicPowerCell) {
        return Ok(());
    }
    let mut candidates = state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .copied()
        .filter(|card| {
            catalog.spec(card.atom).is_some_and(|spec| {
                !spec.x_cost && super::play::resolved_local_energy_cost(state, *card, spec) == 0
            })
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(());
    }
    candidates = crate::dotnet_sort::dotnet_list_sort_by_key(&candidates, |card| {
        let spec = catalog.spec(card.atom).expect("hot card atom is interned");
        (spec.identity.id as u16, spec.identity.upgrade)
    });
    let live = state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    rng.shuffle(&mut candidates)
        .map_err(|_| EngineRefusal::CounterOverflow("Power Cell shuffle"))?;
    state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    if state.history.over {
        return Ok(());
    }
    let selected = candidates.into_iter().take(2).collect::<Vec<_>>();
    let mut draw = state.piles.get(PileId::Draw).as_slice().to_vec();
    let mut hand = state.piles.get(PileId::Hand).as_slice().to_vec();
    let mut discard = state.piles.get(PileId::Discard).as_slice().to_vec();
    for selected in selected {
        let index = draw
            .iter()
            .position(|card| card.uid == selected.uid)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let card = draw.remove(index);
        if hand.len() < super::draw::MAX_CARDS_IN_HAND {
            hand.push(card);
        } else {
            discard.push(card);
        }
    }
    state.piles.set(PileId::Draw, HotPile::from_cards(draw));
    state.piles.set(PileId::Hand, HotPile::from_cards(hand));
    state
        .piles
        .set(PileId::Discard, HotPile::from_cards(discard));
    state.exact_piles = true;
    crate::coverage::record_relic(RelicId::RelicPowerCell);
    Ok(())
}

/// Art of War's `AfterEnergyReset` body. Python clears both latches even when
/// No Energy Gain suppresses the optional grant (`begin_player_turn`, frozen Python, deleted #2827).
///
/// #2669 ending behavior: there is no over early-return. Art of War flag
/// resets and both Tea Set charge clears run even during the enemy-victory
/// suffix; only the GainEnergy grants and the Late BoundPhylactery summon
/// skip while ending.
pub(crate) fn after_energy_reset(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    let ending = super::damage::damage_combat_is_ending(state);
    if state.turn == 1 && state.tea_set_charged() {
        if !ending && !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(2)
                .ok_or(EngineRefusal::CounterOverflow("Venerable Tea Set energy"))?;
        }
        // Clear charge after the attempted GainEnergy even during ending.
        state.set_tea_set_charged(false);
        crate::coverage::record_relic(RelicId::RelicVenerableTeaSet);
    }
    if state.fake_tea_set_charged() {
        if !ending && !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Fake Tea Set energy"))?;
        }
        state.set_fake_tea_set_charged(false);
        crate::coverage::record_relic(RelicId::RelicFakeVenerableTeaSet);
    }
    if state.turn > 1 && catalog.hooks().owns(RelicId::RelicArtOfWar) {
        if !ending && !state.art_of_war_last_attack() && !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Art of War energy"))?;
        }
        state.set_art_of_war_last_attack(false);
        state.set_art_of_war_current_attack(false);
        crate::coverage::record_relic(RelicId::RelicArtOfWar);
    }
    // Fresh Late listener snapshot: live ownership read, skipped while ending.
    if !ending && state.turn > 1 && catalog.hooks().owns(RelicId::RelicBoundPhylactery) {
        state
            .fanouts
            .mutate_pet(|pet| pet.summon(1))
            .map_err(|_| EngineRefusal::CounterOverflow("Bound Phylactery Osty"))?;
        crate::coverage::record_relic(RelicId::RelicBoundPhylactery);
    }
    Ok(())
}

/// Every relic that listens on `BeforeHandDraw`, in the fixed sequence an
/// unvouched inventory runs ([`relic_walk_order`]).
///
/// A v0.111.0 method-table scan of `sts2.dll` (DLL 9cb4f1ad) finds exactly
/// these eight relic types overriding `AbstractModel::BeforeHandDraw` (base
/// RVA `0x7a0a9`): BlessedAntler `0x90d60`, FuneraryMask `0x93df4`,
/// JeweledMask `0x95284`, NinjaScroll `0x97774`, Pendulum `0x98fa0`,
/// PollinousCore `0x9998c`, RadiantPearl `0x99fd0`, Toolbox `0x9c9f0`. No type
/// overrides `BeforeHandDrawLate` (base `0x7a0b0`), the hook's second pass
/// (`Hook/<BeforeHandDraw>d__36::MoveNext` RVA `0x3d2e3c`,
/// `IterateCombatHookListeners` at `IL_012b`, the call at `IL_017e`).
///
/// The other listeners of the first pass are eight player powers
/// (CallOfTheVoid `0xa02f8`, CreativeAi `0xa0e9c`, ForegoneConclusion
/// `0xa288c`, HelloWorld `0xa35d0`, InfiniteBlades `0xa4038`, Nightmare
/// `0xa4de0`, SentryMode `0xa734c`, SpectrumShift `0xa8318`), which the
/// listener list holds ahead of the relics and `turn::begin_player_turn_walk`
/// runs first, and two cards, Bolas `0xd9734` and Thrumming Hatchet
/// `0xeec6c`, which the list holds behind the relics
/// (`CombatState/<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720`: powers
/// `IL_0092`-`IL_0097`, relics `IL_00c9`-`IL_00ef`, pile cards
/// `IL_0163`-`IL_01c7`). This engine returns those two cards before the relic
/// pass (`cards::complete_self_return_before_hand_draw`). That order cannot
/// show: each card moves only when it finished a play on the previous owner
/// turn (`Bolas::<BeforeHandDraw>b__6_0` RVA `0xd9796` and
/// `ThrummingHatchet::<BeforeHandDraw>b__4_0` RVA `0xeeccf`, each
/// `CombatHistoryEntry::HappenedLastPlayerTurn` at `IL_0008`), which turn one
/// does not have, and after turn one the only relic bodies that still act are
/// Pendulum's and Pollinous Core's counter writes.
const BEFORE_HAND_DRAW_RELIC_DISPATCH: [RelicId; 8] = [
    RelicId::RelicPendulum,
    RelicId::RelicToolbox,
    RelicId::RelicBlessedAntler,
    RelicId::RelicFuneraryMask,
    RelicId::RelicNinjaScroll,
    RelicId::RelicPollinousCore,
    RelicId::RelicJeweledMask,
    RelicId::RelicRadiantPearl,
];

/// Whether a `BeforeHandDraw` relic body acts this turn, by its own entry
/// gates (the bodies of [`before_hand_draw_body`]).
fn before_hand_draw_relic_acts(catalog: &Catalog, state: &HotState, relic: RelicId) -> bool {
    let owns = catalog.hooks().owns(relic);
    match relic {
        // `TurnNumber == 1` (`beq`): Toolbox `0x332adc` `IL_004a`-`IL_004b`,
        // Blessed Antler `0x31fe64` `IL_004a`-`IL_004b`, Funerary Mask
        // `0x325000` `IL_0043`-`IL_0044`, Radiant Pearl `0x32f10c`
        // `IL_0043`-`IL_0044`.
        RelicId::RelicToolbox
        | RelicId::RelicBlessedAntler
        | RelicId::RelicFuneraryMask
        | RelicId::RelicRadiantPearl => owns && state.turn == 1,
        // `TurnNumber <= 1` (`ble`): Ninja Scroll `0x32b770` `IL_0043`-`IL_0044`,
        // Jeweled Mask `0x3272f4` `IL_0043`-`IL_0044`.
        RelicId::RelicNinjaScroll | RelicId::RelicJeweledMask => owns && state.turn <= 1,
        // Pendulum and Pollinous Core count every turn.
        _ => owns,
    }
}

/// Toolbox paused `SetupPlayerTurn` inside the `BeforeHandDraw` relic pass, so
/// native `StartTurn` runs the side-start tail before the rest of that pass
/// (#3050, `turn::SetupPlayerTurnWindow`). Native walks the pass in
/// `Player.Relics` order (`Hook/<BeforeHandDraw>d__36::MoveNext` RVA
/// `0x3d2e3c`, `IterateCombatHookListeners` at `IL_0024`, the call at
/// `IL_0076`), so exactly the relics recorded before Toolbox ran ahead of the
/// pause and the tail.
///
/// On a vouched inventory the walk is that order. On an unvouched one the
/// walk is the fixed sequence, and an unrecorded acquisition order cannot
/// show which relics the pause splits off: any other relic of the pass that
/// acts this turn refuses by name.
pub(crate) fn before_hand_draw_pause_order_is_native(
    catalog: &Catalog,
    state: &HotState,
) -> Result<(), EngineRefusal> {
    if catalog.hooks().dispatch_ordered() {
        return Ok(());
    }
    if BEFORE_HAND_DRAW_RELIC_DISPATCH.into_iter().any(|relic| {
        relic != RelicId::RelicToolbox && before_hand_draw_relic_acts(catalog, state, relic)
    }) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "BeforeHandDraw relic order around Toolbox's paused choice",
        ));
    }
    Ok(())
}

/// Whether the turn's `BeforeHandDraw` relic walk can refuse behind earlier
/// turn-start writes, so the turn rehearses on a clone
/// (`turn::begin_player_turn`): a turn-one body (a listener of its cards that
/// parks a choice, Toolbox's pool provenance), or a Pollinous Core counter
/// the walk cannot advance.
pub(crate) fn before_hand_draw_walk_can_refuse(catalog: &Catalog, state: &HotState) -> bool {
    let hooks = catalog.hooks();
    state.turn <= 1
        && BEFORE_HAND_DRAW_RELIC_DISPATCH
            .into_iter()
            .any(|relic| relic != RelicId::RelicPendulum && hooks.owns(relic))
        || state.pollinous_core() >= POLLINOUS_CORE_MAX_SEEN
}

/// The most the stored Pollinous Core counter holds
/// (`HotState::set_pollinous_core`): `Turns` plus two more passes on combats
/// that ended before the hand draw could reset it.
const POLLINOUS_CORE_MAX_SEEN: i16 = 6;

/// Pollinous Core's `Turns` var: `PollinousCore::get_CanonicalVars` (v0.111.0
/// RVA `0x998d9`) builds `DynamicVar("Turns", 4)` at `IL_0012`-`IL_001d`
/// beside `CardsVar(2)` at `IL_0009`-`IL_000a`.
const POLLINOUS_CORE_TURNS: i16 = 4;

/// Pollinous Core's `ModifyHandDraw` and `AfterModifyingHandDraw`.
///
/// `PollinousCore::ModifyHandDraw` (v0.111.0 RVA `0x999cc`) returns the count
/// unchanged while `TurnsSeen < Turns` (`IL_0017`-`IL_0035`) and otherwise
/// adds `Cards` (`IL_0036`-`IL_004c`). `AfterModifyingHandDraw` (`0x99a19`),
/// which `Hook::AfterModifyingHandDraw` calls on a relic that changed the
/// count (`CombatManager/<SetupPlayerTurn>d__102` `0x3f6c6c` `IL_020e`), sets
/// `TurnsSeen = 0` (`IL_0001`-`IL_0003`). `Hook::ModifyHandDraw` (`0x1055e4`)
/// takes its listeners from `IterateCombatHookListeners` (`IL_0015`), so on a
/// combat that is over it runs nobody and the counter keeps what
/// `BeforeHandDraw` left ([`before_hand_draw_body`]).
pub(crate) fn pollinous_core_modify_hand_draw(catalog: &Catalog, state: &mut HotState) -> i16 {
    let activated = !state.history.over
        && catalog.hooks().owns(RelicId::RelicPollinousCore)
        && state.pollinous_core() >= POLLINOUS_CORE_TURNS;
    if activated {
        let written = state.set_pollinous_core(0);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicPollinousCore);
    }
    pollinous_hand_draw_bonus(activated)
}

/// One relic's `BeforeHandDraw` body, at whichever place the walk reaches it.
///
/// Each arm also says what the body does when an earlier listener of the same
/// pass ended the combat: the pass checks the combat once, before its first
/// listener, so the body still runs ([`before_hand_draw`]).
fn before_hand_draw_body(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    relic: RelicId,
) -> Result<WalkStep, EngineRefusal> {
    match relic {
        // `Pendulum::BeforeHandDraw` (v0.111.0 RVA `0x98fa0`, synchronous):
        // owner check `IL_000c`-`IL_001a`, then
        // `set_TurnsSeen((TurnsSeen + 1) % Turns)` at `IL_001b`-`IL_003a` with
        // `Turns = 3` from `get_CanonicalVars` (`0x98f3a`). Everything after
        // that write is presentation: `set_Status(TurnsSeen == Turns - 1)` at
        // `IL_003f`-`IL_0063` and the `DoActivateVisuals` flash at
        // `IL_0068`-`IL_0076`. `AfterCombatEnd` (`0x99093`) clears only
        // `Status`, never `TurnsSeen`, which is why the counter is a saved
        // per-run value the opening seeds. No command and no combat test, so
        // the write is the same on an ended combat.
        RelicId::RelicPendulum => {
            let next = (state.pendulum() + 1) % 3;
            let written = state.set_pendulum(next);
            debug_assert!(written);
        }
        // `Toolbox/<BeforeHandDraw>d__4::MoveNext` (RVA `0x332adc`): owner
        // and `TurnNumber == 1` (`IL_0027`-`IL_004d`), a `GetDistinctForCombat`
        // draw of `Cards` (3, `get_CanonicalVars` `0x9c9e2`) from the Colorless
        // pool (`IL_005e`-`IL_00a8`), the pause on
        // `CardSelectCmd::FromChooseACardScreen` (`IL_00c1`), then
        // `AddGeneratedCardToCombat(pick, Hand, player, Bottom)`
        // (`IL_0126`-`IL_012f`) in [`resume_relic_selection`].
        //
        // On an ended combat the draw still happens and the pass stops at
        // this body. That is an observation, recorded from the game (a
        // Smokestack kill inside Blessed Antler's body ahead of Toolbox):
        // `CombatCardGeneration` moves by the draw, no option is offered (a
        // pushed selector's source is not dequeued), no card is added, a
        // Pendulum or Pollinous Core recorded behind Toolbox keeps its count
        // while one recorded ahead advances, and `StartTurn` goes on to
        // `CheckWinCondition`. Where exactly the body stops is not
        // established: `<FromChooseACardScreen>d__16::MoveNext` (RVA
        // `0x3e58c0`) has no combat test between its count tests
        // (`IL_0031`-`IL_0063`) and the selector (`IL_0073`-`IL_0087`), so
        // the stop is not a gate of that command read from IL.
        RelicId::RelicToolbox => {
            if state.turn == 1 {
                if state.history.over {
                    let pool =
                        generation_relic_draw_pool(catalog, state, RelicPendingKind::Toolbox)?;
                    super::cards::shuffle_generation_slice(state, &pool)?;
                    crate::coverage::record_relic(RelicId::RelicToolbox);
                    return Ok(WalkStep::Suspended);
                }
                begin_generation_relic_selection(state, catalog, RelicPendingKind::Toolbox, 3)?;
                crate::coverage::record_relic(RelicId::RelicToolbox);
                return Ok(WalkStep::Paused);
            }
        }
        // `BlessedAntler/<BeforeHandDraw>d__7::MoveNext` (RVA `0x31fe64`):
        // owner and `TurnNumber == 1` (`IL_0027`-`IL_004d`), `Cards` (3)
        // `CreateCard<Dazed>` into a list (`IL_005e`-`IL_0092`) and one
        // `AddGeneratedCardsToCombat(list, Draw, Owner, Random)` at
        // `IL_0094`-`IL_009d`.
        //
        // On an ended combat each card is still recorded as generated and
        // none enters a pile: `<AddGeneratedCardsToCombat>d__6::MoveNext` (RVA
        // `0x3e2f0c`) has no combat test of its own past `IsInProgress`
        // (`IL_0039`-`IL_004b`), and per card writes
        // `CombatHistory::CardGenerated` (`IL_0104`-`IL_0120`), then calls
        // `CardPileCmd::Add` (`IL_0155`), whose `<Add>d__10::MoveNext` (RVA
        // `0x3e1ba4`) returns before any move or position roll when the pile
        // is a combat pile and `CombatManager.IsEnding` (`IL_0041`-`IL_008a`).
        // That covers a listener of the first Dazed ending the combat inside
        // this body too (Smokestack): the first is in Draw, the other two are
        // recorded only. The same holds for Funerary Mask and Radiant Pearl.
        RelicId::RelicBlessedAntler => {
            if state.turn == 1 {
                turn_one_draw_inserts(catalog, state, events, CardId::Dazed, true)?;
                crate::coverage::record_relic(RelicId::RelicBlessedAntler);
            }
        }
        // `FuneraryMask/<BeforeHandDraw>d__6::MoveNext` (v0.111.0 RVA
        // `0x325000`, re-read for #3162): `player == Owner`
        // (`IL_0020`-`IL_002e`), `TurnNumber == 1` (`IL_0033`-`IL_0046`),
        // `Flash`, then while `i < DynamicVars.Cards.BaseValue` (`CardsVar(3)`,
        // `get_CanonicalVars` `0x93ddc`; loop test `IL_00fd`-`IL_011d`) one
        // `ICombatState::CreateCard(Card<Soul>, Owner)` and
        // `CardPileCmd::AddGeneratedCardToCombat(card, PileType.Draw, Owner,
        // CardPilePosition.Random)` (`IL_005d`-`IL_007d`), awaited per card;
        // `PreviewCardPileAdd` is presentation. No saved property.
        RelicId::RelicFuneraryMask => {
            if state.turn == 1 {
                turn_one_draw_inserts(catalog, state, events, CardId::Soul, false)?;
                crate::coverage::record_relic(RelicId::RelicFuneraryMask);
            }
        }
        RelicId::RelicNinjaScroll => ninja_scroll_before_hand_draw(catalog, state, events)?,
        // `PollinousCore::BeforeHandDraw` (RVA `0x9998c`, synchronous): owner
        // check `IL_000c`-`IL_001a`, then `TurnsSeen = TurnsSeen + 1`
        // (`IL_001b`-`IL_0026`). No turn test, no command and no combat test,
        // so the write is the same on an ended combat. The count is read and
        // reset by [`pollinous_core_modify_hand_draw`], after the whole pass.
        // A counter that enters the pass already at `Turns` (left there by a
        // combat that ended inside an earlier pass) counts on:
        // `ModifyHandDraw` tests `TurnsSeen >= Turns` (`0x999cc`
        // `IL_0017`-`IL_0032`), so the next live hand draw adds its two and
        // resets it whatever it reached. The stored counter holds up to
        // [`POLLINOUS_CORE_MAX_SEEN`]; past that the walk refuses by name.
        RelicId::RelicPollinousCore => {
            let seen = state.pollinous_core();
            if seen >= POLLINOUS_CORE_MAX_SEEN {
                return Err(EngineRefusal::CounterOverflow(
                    "Pollinous Core TurnsSeen past its stored range",
                ));
            }
            let written = state.set_pollinous_core(seen + 1);
            debug_assert!(written);
        }
        RelicId::RelicJeweledMask => jeweled_mask_before_hand_draw(catalog, state)?,
        RelicId::RelicRadiantPearl => radiant_pearl_before_hand_draw(catalog, state, events)?,
        _ => {}
    }
    Ok(WalkStep::Ran)
}

/// Blessed Antler's three Dazed and Funerary Mask's three Souls: one fresh
/// card at a time at a random Draw position ([`before_hand_draw_body`]
/// carries the IL). Each card re-reads the combat: once it is over, the card
/// is recorded as generated and not added.
///
/// `promote_exact_piles` is the solver's pile-identity promotion, not a native
/// field: each Dazed is a distinct physical card the later listeners must see
/// as one.
fn turn_one_draw_inserts(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    id: CardId,
    promote_exact_piles: bool,
) -> Result<(), EngineRefusal> {
    let identity = crate::catalog::CardIdentity {
        id,
        upgrade: 0,
        enchantment: None,
    };
    for _ in 0..3 {
        if state.history.over {
            super::cards::inject_generated_record_before_ending_draw_random(
                state, catalog, identity, events,
            )?;
            continue;
        }
        if promote_exact_piles {
            state.exact_piles = true;
        }
        super::cards::inject_generated_draw_random(state, catalog, identity, events)?;
    }
    Ok(())
}

/// The `BeforeHandDraw` relic walk (#3635), from the top or from the listener
/// after `resume_after`, whose parked choice just resolved.
///
/// `Hook/<BeforeHandDraw>d__36::MoveNext` (v0.111.0 RVA `0x3d2e3c`) takes the
/// listener list once (`IterateCombatHookListeners` at `IL_0024`) and awaits
/// each `AbstractModel::BeforeHandDraw` in turn (`IL_0076`, looping at
/// `IL_00f9`-`IL_00fe`). The relic segment of that list is
/// [`relic_walk_order`] over [`BEFORE_HAND_DRAW_RELIC_DISPATCH`]:
/// `Player.Relics` order on a vouched inventory.
///
/// * Toolbox parks a choice and stops the walk. `StartTurn`'s side-start tail
///   runs at that pause (#3050), and [`resume_relic_selection`] re-enters here
///   behind Toolbox. On a combat an earlier listener ended it stops the walk
///   for good ([`WalkStep::Suspended`]).
/// * A body with no choice of its own whose listener parked one (a
///   generated-card listener) cannot be resumed inside the body. With a later
///   relic still to act that refuses by name; with none it returns to the
///   caller parked, as before.
fn before_hand_draw_walk(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    resume_after: Option<RelicId>,
) -> Result<bool, EngineRefusal> {
    let mut waiting = resume_after;
    let mut order = relic_walk_order(catalog, &BEFORE_HAND_DRAW_RELIC_DISPATCH);
    while let Some(relic) = order.next() {
        if let Some(paused) = waiting {
            if relic == paused {
                waiting = None;
            }
            continue;
        }
        if before_hand_draw_body(catalog, state, events, relic)? != WalkStep::Ran {
            return Ok(false);
        }
        if state.pending.is_some() {
            if order
                .clone()
                .any(|later| before_hand_draw_relic_acts(catalog, state, later))
            {
                return Err(EngineRefusal::PowerOrderNotModeled(
                    "BeforeHandDraw relic behind a listener choice parked inside an earlier relic body",
                ));
            }
            return Ok(false);
        }
    }
    if waiting.is_some() {
        return Err(EngineRefusal::MalformedArgs(
            "BeforeHandDraw relic walk resume point",
        ));
    }
    Ok(true)
}

/// The owner's `BeforeHandDraw` relics, behind its power listeners: the
/// recorded-order walk of [`before_hand_draw_walk`].
///
/// Returns whether the pass ran to its end, so `SetupPlayerTurn` goes on to
/// the hand draw. `false` is Toolbox's pause (`state.pending` is set and the
/// answer resumes the walk), a listener's parked choice, or Toolbox
/// suspended on an ended combat, where the game never leaves the hook and
/// nothing behind it in `SetupPlayerTurn` runs.
///
/// `pass_started_live` is whether the combat was live when the pass reached
/// its first listener, which is one of the owner's powers when it has any.
/// `Hook/<IterateCombatHookListeners>d__0::MoveNext` (RVA `0x3d3bc0`) tests
/// `IsOverOrEnding && !IsStarting` once, before the first listener
/// (`IL_0028`-`IL_0042`), and never between listeners (`IL_0064`-`IL_0095`).
/// So a pass entered on an ended combat runs no relic, and a pass a listener
/// ended still runs every relic behind it: each body of
/// [`before_hand_draw_body`] says what it does there.
pub(crate) fn before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
    pass_started_live: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if !pass_started_live {
        return Ok(true);
    }
    before_hand_draw_walk(catalog, state, events, None)
}

/// The rest of the `BeforeHandDraw` relic walk once Toolbox's choice is
/// answered: every relic recorded behind Toolbox.
pub(crate) fn resume_before_hand_draw_after_toolbox(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    before_hand_draw_walk(catalog, state, events, Some(RelicId::RelicToolbox))
}

/// Ninja Scroll's Shiv count.
///
/// `NinjaScroll::get_CanonicalVars` (v0.111.0 RVA `0x97752`) builds
/// `DynamicVar("Shivs", 3)` at `IL_0001`-`IL_0006`, which the body reads back
/// as `DynamicVars["Shivs"].IntValue` (`IL_0058`-`IL_0067`).
const NINJA_SCROLL_SHIVS: usize = 3;

/// Ninja Scroll's turn-one Shivs into the Hand (#3619).
///
/// # The native contract
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…12b4`). The type declares one hook,
/// `BeforeHandDraw` (RVA `0x97774`, the async stub), no field and no saved
/// property. The body is `NinjaScroll/<BeforeHandDraw>d__7::MoveNext` (RVA
/// `0x32b770`):
///
/// * `player == Owner` at `IL_0027`-`IL_002c`, else `leave` at `IL_002e`;
/// * `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
///   `IL_0034`-`IL_0044`, else `leave` at `IL_0046`;
/// * `Flash`, then `Shiv::CreateInHand(Owner, DynamicVars["Shivs"].IntValue,
///   null)` at `IL_0052`-`IL_0073`, awaited.
///
/// `Shiv::CreateInHand`'s count overload is
/// `Shiv/<CreateInHand>d__12::MoveNext` (RVA `0x3bb20c`): it returns an empty
/// list while `CombatManager.IsOverOrEnding` (`IL_002c`-`IL_003e`), creates
/// the Shivs in a loop (`IL_004e`-`IL_0079`), and hands the whole list to one
/// `CardPileCmd::AddGeneratedCardsToCombat(list, PileType.Hand, …)` at
/// `IL_0081`-`IL_0093`. That is the plural generated-card transaction every
/// Shiv card already uses here (`steps::shared::generate_fixed_shivs`, Fan of
/// Knives and Blade of Ink): [`super::cards::inject_generated_bottom`] with a
/// level-zero Shiv, which allocates the uids in creation order, records each
/// generated card, runs the generated-card listeners and redirects past a
/// full hand. No RNG is drawn.
///
/// # Position inside the walk
///
/// Native runs relic `BeforeHandDraw` listeners in `Player.Relics` order, all
/// of them before `ModifyHandDraw` and the turn-one draw
/// (`CombatManager/<SetupPlayerTurn>d__102` `0x3f6c6c`, `IL_0177` then
/// `IL_01fd`). On a vouched inventory this body runs at its recorded place
/// ([`before_hand_draw_walk`], #3635), so the three Shivs sit in the Hand
/// among the other bodies' cards in that order, ahead of the deal. On an
/// unvouched one it runs at the fixed place of
/// [`BEFORE_HAND_DRAW_RELIC_DISPATCH`]. Pendulum and Pollinous Core write
/// only their counters and commute with it; every other same-hook body moves
/// a Draw or Hand card or allocates a uid, and the opening refuses each of
/// those co-ownerships by name on an unvouched inventory (`entry::opening`'s
/// `refuse_unordered_hand_draw_card_peers`, Blessed Antler's and Toolbox's
/// gates), as `engine::admission` does for a loaded document.
fn ninja_scroll_before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicNinjaScroll) {
        return Ok(());
    }
    super::cards::inject_generated_bottom(
        state,
        catalog,
        crate::catalog::CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        },
        NINJA_SCROLL_SHIVS,
        PileId::Hand,
        events,
    )?;
    crate::coverage::record_relic(RelicId::RelicNinjaScroll);
    Ok(())
}

/// Jeweled Mask's turn-one free Power card (#3162).
///
/// # The native contract
///
/// v0.111.0 `sts2.dll`, sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`
/// (hash-verified 2026-09-26). The type declares one hook, `BeforeHandDraw`
/// (RVA `0x95284`, the async stub), beside `get_Rarity` and the constructor:
/// no fields, no saved property, no `CanonicalVars`. The body is
/// `JeweledMask/<BeforeHandDraw>d__2::MoveNext` (RVA `0x3272f4`):
///
/// * `player == Owner`, else `leave` (`IL_0020`-`IL_002e`);
/// * `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s`, else `leave`
///   (`IL_0033`-`IL_0046`), so turn one only;
/// * `PileType.Draw` (`ldc.i4.1`, `IL_004b`) `.GetPile(player).Cards`, filtered
///   by `<>c::<BeforeHandDraw>b__2_0` (RVA `0x3272d6`: `Type == 3`, Power) into
///   a list; an empty list `leave`s (`IL_0088`-`IL_0090`);
/// * that list filtered again by `b__2_1` (RVA `0x3272e1`: `!Keywords.Contains(3)`,
///   `CardKeyword.Innate`), and the narrower list **replaces** the first only
///   when it is non-empty (`IL_00c1`-`IL_00cd`): a non-Innate Power is
///   preferred, and an all-Innate Draw pile still yields one;
/// * `RunState.Rng.CombatCardSelection.NextItem(list)` (`IL_00ce`-`IL_00e4`;
///   `Rng::NextItem` `0x5ee74` is `list[NextInt(0, Count)]`, one draw);
/// * `Flash` (presentation), `CardModel::SetToFreeThisTurn` (`IL_00f3`), then
///   `CardPileCmd::Add(card, PileType.Hand, CardPilePosition.Bottom, null,
///   false)` (`IL_00f8`-`IL_00fe`; `CardPilePosition` declares `None, Bottom,
///   Top, Random`), awaited.
///
/// `CardModel::SetToFreeThisTurn` (RVA `0x7d3c9`, re-read for #3176) is
/// `EnergyCost.SetThisTurnOrUntilPlayed(0, false)` (`IL_0001`-`IL_0009`), then
/// `SetStarCostThisTurn(0)` (`IL_000e`-`IL_0010`).
/// `CardEnergyCost::SetThisTurnOrUntilPlayed` (`0x11e1d6`) returns without
/// writing only for a zero amount on a negative canonical cost
/// (`IL_0001`-`IL_000d`) and otherwise appends one local modifier with
/// expiration `6` (`IL_0014`-`IL_001d`), `LocalCostExpiration::ThisTurnOrPlayed`.
/// Those rows go at `CardModel::EndOfTurnCleanup` (`0x7dbc4`: the Energy rows
/// at `IL_0027`, the temporary Star rows at `IL_0064`) or on play
/// (`CardEnergyCost::AfterCardPlayedCleanup` `0x11e2e1`), so a turn-one root
/// still carries them. That is why the moved card gets the slot-7 bit: the
/// boundary projects the rows only for a card that carries it (#3176).
///
/// There is no ending gate in the body. Reached after an earlier listener of
/// the pass ended the combat, it still draws the stream and frees the card
/// (the three steps up to `IL_00f3` are not commands), and only the move is
/// dropped: `CardPileCmd/<Add>d__10::MoveNext` (RVA `0x3e1ba4`) returns before
/// any move when the pile is a combat pile and `CombatManager.IsEnding`
/// (`IL_0041`-`IL_008a`). `CardModel.Keywords` is the effective set, and the only combat-local
/// keywords this crate represents are Ethereal, Retain and Sly
/// (`hot::CardStates`), none of them Innate, so the spec's `innate` flag is the
/// whole answer for a Power (`Imbued`, the one Innate-sensitive enchantment in
/// the opening's fixup, enchants Skills only: `Imbued::CanEnchantCardType`
/// `0xd60a9` is `Type == 2`).
///
/// # Position inside the walk
///
/// Native runs relic `BeforeHandDraw` listeners in `Player.Relics` order, and
/// so does [`before_hand_draw_walk`] on a vouched inventory (#3635): the
/// candidates are the Powers in Draw as the bodies recorded ahead left it (a
/// Funerary Mask or Blessed Antler insert moves the candidates' positions but
/// not their order), and the lifted card joins the Hand behind what those
/// bodies put there. On an unvouched inventory it runs at the fixed place of
/// [`BEFORE_HAND_DRAW_RELIC_DISPATCH`]. Pendulum's and Pollinous Core's bodies
/// write only their counters, so they commute with it; every other same-hook
/// body moves a Draw or Hand card, and the opening refuses those
/// co-ownerships by name on an unvouched inventory (`entry::opening`'s
/// `refuse_unordered_hand_draw_card_peers`, Blessed Antler's and Toolbox's
/// gates).
fn jeweled_mask_before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn != 1 || !catalog.hooks().owns(RelicId::RelicJeweledMask) {
        return Ok(());
    }
    let ending = state.history.over;
    let power = |card: &HotCard| catalog.spec(card.atom).is_some_and(|spec| spec.is_power);
    let not_innate = |card: &HotCard| catalog.spec(card.atom).is_some_and(|spec| !spec.innate);
    let powers = state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .copied()
        .filter(power)
        .collect::<Vec<_>>();
    if powers.is_empty() {
        crate::coverage::record_relic(RelicId::RelicJeweledMask);
        return Ok(());
    }
    let preferred = powers
        .iter()
        .copied()
        .filter(not_innate)
        .collect::<Vec<_>>();
    let candidates = if preferred.is_empty() {
        powers
    } else {
        preferred
    };
    let live = state.rng.get(RngStream::Sel);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound = i32::try_from(candidates.len())
        .map_err(|_| EngineRefusal::CounterOverflow("Jeweled Mask selection"))?;
    let selected = usize::try_from(
        rng.next_bounded(bound)
            .map_err(|_| EngineRefusal::CounterOverflow("Jeweled Mask selection"))?,
    )
    .map_err(|_| EngineRefusal::CounterOverflow("Jeweled Mask selection"))?;
    state.rng.set(
        RngStream::Sel,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let chosen = candidates[selected];
    let mut draw = state.piles.get(PileId::Draw).as_slice().to_vec();
    let index = draw
        .iter()
        .position(|card| card.uid == chosen.uid)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let spec = catalog
        .spec(chosen.atom)
        .ok_or(EngineRefusal::UnknownAtom(chosen.atom))?;
    state
        .card_states
        .set_to_free_this_turn(chosen.uid, spec.cost)
        .ok_or(EngineRefusal::MalformedArgs("Jeweled Mask free card"))?;
    if ending {
        // The freed card stays where it is in Draw: `Add` returned on the
        // ending combat. It carries the slot-7 bit all the same.
        draw[index].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.set(PileId::Draw, HotPile::from_cards(draw));
        state.exact_piles = true;
        crate::coverage::record_relic(RelicId::RelicJeweledMask);
        return Ok(());
    }
    let mut chosen = draw.remove(index);
    // The rows `SetToFreeThisTurn` just appended live in the card's slot-7
    // payload, which the boundary emits only for a card carrying this bit
    // (`HotBoundary::card_to_canonical_with_instance`). Without it the rows
    // were live in the engine but dropped from the canonical root, so a
    // reloaded root charged full cost for the lifted Power (#3176). Bullet
    // Time's live-Hand write sets the same bit (`steps::silent_rare`).
    chosen.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
    let destination = if state.piles.get(PileId::Hand).len() < super::draw::MAX_CARDS_IN_HAND {
        PileId::Hand
    } else {
        PileId::Discard
    };
    state.piles.set(PileId::Draw, HotPile::from_cards(draw));
    state.piles.get_mut(destination).make_mut().push(chosen);
    state.exact_piles = true;
    crate::coverage::record_relic(RelicId::RelicJeweledMask);
    Ok(())
}

/// Radiant Pearl's turn-one generated Luminesce is appended before the
/// ordinary hand-draw command and therefore precedes every drawn card.
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…fbf12b4`, re-read for #3162). The
/// one hook is `BeforeHandDraw` (RVA `0x99fd0`, the async stub);
/// `get_CanonicalVars` (`0x99f9e`) is `ldc.i4.1; newobj CardsVar::.ctor`. The
/// body is `RadiantPearl/<BeforeHandDraw>d__6::MoveNext` (RVA `0x32f10c`):
/// `player == Owner` (`IL_0020`-`IL_002e`), `TurnNumber == 1` (`ldc.i4.1;
/// beq.s`, `IL_0033`-`IL_0046`), then `Cards` iterations of
/// `CombatState.CreateCard<Luminesce>(Owner)` into a list
/// (`IL_004b`-`IL_008b`) and one `CardPileCmd::AddGeneratedCardsToCombat(list,
/// PileType.Hand, Owner, …)` at `IL_008d`-`IL_0096`, awaited. No saved
/// property and no RNG draw. It runs at its place in
/// [`before_hand_draw_walk`] (#3635): the recorded one on a vouched inventory,
/// the last of [`BEFORE_HAND_DRAW_RELIC_DISPATCH`] otherwise, where its
/// same-hook Hand/Draw peers are refused in the opening (`entry::opening`'s
/// `refuse_unordered_hand_draw_card_peers`). On an ended combat the card is
/// recorded as generated and not added ([`before_hand_draw_body`], Blessed
/// Antler's arm).
fn radiant_pearl_before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn == 1 && catalog.hooks().owns(RelicId::RelicRadiantPearl) {
        let identity = crate::catalog::CardIdentity {
            id: CardId::Luminesce,
            upgrade: 0,
            enchantment: None,
        };
        if state.history.over {
            super::cards::inject_generated_record_before_ending_bottom(
                state,
                catalog,
                identity,
                1,
                PileId::Hand,
                events,
            )?;
        } else {
            super::cards::inject_generated_bottom(
                state,
                catalog,
                identity,
                1,
                PileId::Hand,
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicRadiantPearl);
    }
    Ok(())
}

fn generation_relic_pool(kind: RelicPendingKind) -> Option<&'static [CardId]> {
    let name = match kind {
        RelicPendingKind::ChoicesParadox => "CHOICES_PARADOX",
        RelicPendingKind::Toolbox => "TOOLBOX",
        RelicPendingKind::GamblingChip | RelicPendingKind::ToastyMittens => return None,
    };
    crate::content_tables::GENERATION_RELIC_POOLS
        .iter()
        .find_map(|(candidate, pool)| (*candidate == name).then_some(*pool))
}

/// The pool a generation relic's grid draws for this fight's owner.
///
/// Choices Paradox draws its **owner's** unlocked pool (#3321):
/// `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext` (v0.111.0 RVA
/// `0x321b0c`) reads `Owner.Character.CardPool` at `IL_0064`-`IL_0069`,
/// `.GetUnlockedCards(Owner.UnlockState, RunState.CardMultiplayerConstraint)`
/// at `IL_0089`, and hands it with `DynamicVars.Cards.IntValue` (5,
/// `IL_008f`-`IL_0099`) to `CardFactory::GetDistinctForCombat(..,
/// RunRngSet.CombatCardGeneration)` at `IL_009e`-`IL_00b3`: the same read as
/// Vexing Puzzlebox's (`IL_005d`-`IL_009d`, [`vexing_puzzlebox_generated_card`]).
/// The frozen `GENERATION_RELIC_POOLS` row is the Ironclad pool, which every
/// certified Paradox fight holds, so an Ironclad (or unrecorded) owner keeps
/// it unchanged. Any other owner takes the Puzzlebox's owner branch under the
/// same gates: a solo run whose profile reveals the owner's card-pool epochs,
/// refusing by name otherwise.
///
/// Toolbox does NOT read its owner's character pool (#3325): its body
/// (`0x332adc` IL_005e) loads the static `CardPool<ColorlessCardPool>` and
/// filters it by the owner's `UnlockState`, so it draws
/// [`colorless_generation_relic_pool`] — its frozen row at the fully-unlocked
/// profile, the recorded profile's Colorless pool otherwise.
fn generation_relic_draw_pool(
    catalog: &Catalog,
    state: &HotState,
    kind: RelicPendingKind,
) -> Result<std::borrow::Cow<'static, [CardId]>, EngineRefusal> {
    const PARTIAL_PROFILE: &str =
        "Choices Paradox owner pool under a profile hiding the owner's card-pool epochs";
    const PROVENANCE: &str = "Choices Paradox owner pool provenance";
    let frozen =
        generation_relic_pool(kind).ok_or(EngineRefusal::MalformedArgs("generation relic pool"))?;
    match (kind, state.reward_card_pool) {
        (RelicPendingKind::ChoicesParadox, Some(owner))
            if owner != crate::catalog::RewardPool::Ironclad =>
        {
            if state.multiplayer_ally_key != 0
                || !(state.fully_unlocked_card_pool_epochs
                    || catalog.splash_unlock_epochs().is_some())
            {
                return Err(EngineRefusal::MalformedArgs(PROVENANCE));
            }
            let pool = catalog.owner_generation_pool(owner);
            if pool.is_empty() {
                return Err(EngineRefusal::MalformedArgs(PROVENANCE));
            }
            if pool != crate::steps::neutral::owner_generation_pool(owner, None) {
                return Err(EngineRefusal::MalformedArgs(PARTIAL_PROFILE));
            }
            Ok(std::borrow::Cow::Owned(pool))
        }
        (RelicPendingKind::Toolbox, _) => Ok(std::borrow::Cow::Owned(
            colorless_generation_relic_pool(catalog, state, "Toolbox Colorless pool provenance")?,
        )),
        _ => Ok(std::borrow::Cow::Borrowed(frozen)),
    }
}

fn begin_generation_relic_selection(
    state: &mut HotState,
    catalog: &Catalog,
    kind: RelicPendingKind,
    count: usize,
) -> Result<(), EngineRefusal> {
    if state.pending.is_some() || state.fanouts.batch_nine_relic_pending().is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let pool = generation_relic_draw_pool(catalog, state, kind)?;
    let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
    let mut entries = Vec::with_capacity(count);
    for id in shuffled.into_iter().take(count) {
        let identity = crate::catalog::CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        let atom = catalog
            .atom(&identity)
            .ok_or(EngineRefusal::UnknownMintIdentity(identity))?;
        let mut instance = CardInstanceState::default();
        if kind == RelicPendingKind::ChoicesParadox {
            instance.local_retain = true;
        }
        entries.push(FrozenAutoBatchEntry {
            card: HotCard {
                uid: LEGACY_CARD_UID,
                atom,
                flags: CARD_FLAG_LEGACY,
            },
            state: instance,
        });
    }
    if entries.len() != count {
        return Err(EngineRefusal::MalformedArgs(
            "generation relic option count",
        ));
    }
    state
        .fanouts
        .set_batch_nine_relic_pending(Some(RelicPendingRecord { kind, entries }));
    state.pending = Some(std::sync::Arc::new(PendingSelection::relic_selection()));
    Ok(())
}

fn hand_relic_selection_entries(state: &HotState) -> Vec<FrozenAutoBatchEntry> {
    state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .copied()
        .map(|card| FrozenAutoBatchEntry {
            card,
            state: state.card_states.get(card.uid),
        })
        .collect()
}

fn begin_hand_relic_selection(
    state: &mut HotState,
    kind: RelicPendingKind,
) -> Result<(), EngineRefusal> {
    if state.pending.is_some() || state.fanouts.batch_nine_relic_pending().is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let entries = hand_relic_selection_entries(state);
    state
        .fanouts
        .set_batch_nine_relic_pending(Some(RelicPendingRecord { kind, entries }));
    state.pending = Some(std::sync::Arc::new(PendingSelection::relic_selection()));
    Ok(())
}

/// Re-take a parked Gambling Chip / Toasty Mittens Hand snapshot after the
/// side-start tail ran at its pause (#3050).
///
/// Both select through `CardSelectCmd::FromHand`
/// (`<FromHand>d__28::MoveNext` RVA `0x3e7568`; Gambling Chip via
/// `FromHandForDiscard`, `<FromHandForDiscard>d__29` RVA `0x3e7a64`, which
/// forwards at IL_005e). `FromHand` signals the pause first
/// (`PlayerChoiceContext::SignalPlayerChoiceBegun`, IL_00b3) and reads
/// `PileTypeExtensions::GetPile(Hand).Cards` only after that await resumes
/// (IL_010d-0119), i.e. once the paused hook action runs, after
/// `StartTurn`'s side-start tail. Its options are therefore the Hand as that
/// tail left it (an Orange Dough or Crossbow card included).
pub(crate) fn refresh_hand_relic_selection_after_side_start(
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    let kind = state
        .fanouts
        .batch_nine_relic_pending()
        .map(|pending| pending.kind)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    if !matches!(
        kind,
        RelicPendingKind::GamblingChip | RelicPendingKind::ToastyMittens
    ) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let entries = hand_relic_selection_entries(state);
    state
        .fanouts
        .set_batch_nine_relic_pending(Some(RelicPendingRecord { kind, entries }));
    Ok(())
}

/// `FestivePopper::get_CanonicalVars` (v0.111.0 RVA `0x93902`):
/// `ldc.i4.s 9; newobj Decimal::.ctor; ldc.i4.4; newobj DamageVar::.ctor` at
/// `IL_0001`-`IL_0009`, i.e. 9 damage with `ValueProp` 4 (unpowered, still
/// blockable). `combat_sim.POPPER_DAMAGE` (frozen Python, deleted #2827).
const FESTIVE_POPPER_DAMAGE: i64 = 9;

/// Bellows' turn-one Hand upgrade (#2827).
///
/// `Bellows::AfterPlayerTurnStart` (v0.111.0 RVA `0x906dc`) is synchronous, so
/// there is no state machine: `player == Owner` at `IL_000c`-`IL_0013` (else
/// `CompletedTask`), `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
/// `IL_001b`-`IL_002c` (else `CompletedTask`), then `Flash`, and
/// `CardCmd::Upgrade(PileTypeExtensions::GetPile(2 /* Hand */, Owner).Cards,
/// 1)` at `IL_003a`-`IL_004c`. `CardCmd.Upgrade` walks the captured Hand
/// enumeration once and skips what cannot upgrade; nothing else is written.
///
/// The frozen Hand snapshot and the atomic transaction are
/// [`super::cards::upgrade_live_cards_once`], the same command Bone Tea's pass
/// uses. The body runs at its recorded place in the walk
/// ([`after_player_turn_start_walk`], #3635), so it upgrades exactly the cards
/// the earlier listeners left in the Hand: a Vexing Puzzlebox or Choices
/// Paradox card only when that relic is recorded first.
fn bellows_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicBellows) {
        return Ok(());
    }
    let uids = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    super::cards::upgrade_live_cards_once(state, catalog, &uids)?;
    crate::coverage::record_relic(RelicId::RelicBellows);
    Ok(())
}

/// Festive Popper's turn-one 9 damage to every hittable enemy (#2827).
///
/// `FestivePopper/<AfterPlayerTurnStart>d__4::MoveNext` (v0.111.0 RVA
/// `0x324b94`): `player == Owner` at `IL_0020`-`IL_002c` (else `leave`), the
/// guard `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; beq.s` at
/// `IL_0044`-`IL_0055` — **`== 1`**, not Bellows' `<= 1` — then `Flash`, and one
/// awaited `CreatureCmd::Damage(choiceContext, CombatState.HittableEnemies,
/// DynamicVars.Damage, Owner.Creature)` at `IL_0072`-`IL_0094`. It writes
/// nothing else.
///
/// The targets are one `alive_targets` snapshot taken before the first hit,
/// each hit the catalog-authenticated unpowered, blockable
/// `damage_monster_with_catalog` call, as for Mr Struggles. The body runs at
/// its recorded place in the walk ([`after_player_turn_start_walk`], #3635):
/// ahead of or behind each other enemy hit, each Hand choice and each
/// generated card, as the inventory has it. On an unvouched inventory the
/// peers whose order around it shows are refused by name at the opening
/// (`entry::opening::after_player_turn_start_peers_are_ordered`) and in
/// admission.
fn festive_popper_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn != 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicFestivePopper) {
        return Ok(());
    }
    let targets = super::damage::alive_targets(state);
    for target in targets {
        super::damage::damage_monster_with_catalog(
            state,
            catalog,
            target,
            DotNetDecimal::from_i64(FESTIVE_POPPER_DAMAGE),
            false,
            true,
            events,
        )?;
    }
    crate::coverage::record_relic(RelicId::RelicFestivePopper);
    Ok(())
}

/// Bone Tea's charged turn-one Hand upgrade.
///
/// `BoneTea::AfterPlayerTurnStart` (v0.111.0 RVA `0x90ffc`, synchronous):
/// `IsUsedUp` returns at `IL_000d`-`IL_0014`, `player == Owner` at
/// `IL_001c`-`IL_0023`, `TurnNumber <= 1` at `IL_002a`-`IL_003c`, then one
/// `CardCmd::Upgrade` per Hand card (`IL_0042`-`IL_006f`) and
/// `CombatsLeft - 1` written back (`IL_007e`-`IL_0088`). The Hand snapshot and
/// the atomic transaction are [`super::cards::upgrade_live_cards_once`], the
/// command Bellows uses.
fn bone_tea_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn > 1
        || state.history.over
        || !catalog.hooks().owns(RelicId::RelicBoneTea)
        || state.fanouts.bone_tea_combats_left() == 0
    {
        return Ok(());
    }
    let uids = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect::<Vec<_>>();
    super::cards::upgrade_live_cards_once(state, catalog, &uids)?;
    let written = state.fanouts.set_bone_tea_combats_left(0);
    debug_assert!(written);
    crate::coverage::record_relic(RelicId::RelicBoneTea);
    Ok(())
}

/// Mr Struggles' hit of `TurnNumber` on every hittable enemy, every turn.
///
/// `MrStruggles/<AfterPlayerTurnStart>d__2::MoveNext` (v0.111.0 RVA
/// `0x32aca4`): `player == Owner` at `IL_0027`-`IL_002e`, no turn gate, then
/// one awaited `CreatureCmd::Damage(choiceContext, CombatState.HittableEnemies,
/// (decimal)TurnNumber, ValueProp 4, Owner.Creature)` at `IL_0044`-`IL_0077`.
fn mr_struggles_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || !catalog.hooks().owns(RelicId::RelicMrStruggles) {
        return Ok(());
    }
    let targets = super::damage::alive_targets(state);
    for target in targets {
        super::damage::damage_monster_with_catalog(
            state,
            catalog,
            target,
            DotNetDecimal::from_i64(i64::from(state.turn)),
            false,
            true,
            events,
        )?;
    }
    crate::coverage::record_relic(RelicId::RelicMrStruggles);
    Ok(())
}

/// Royal Poison's turn-one hit on its owner.
///
/// `RoyalPoison/<AfterPlayerTurnStart>d__4::MoveNext` (v0.111.0 RVA
/// `0x32fcf4`): `player == Owner` at `IL_0027`-`IL_002e`, `TurnNumber <= 1` at
/// `IL_0034`-`IL_0046`, then one awaited `CreatureCmd::Damage` of
/// `DynamicVars.Damage` on `Owner.Creature` (`IL_0058`-`IL_0070`).
fn royal_poison_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicRoyalPoison) {
        return Ok(());
    }
    super::damage::damage_player_from_power_with_catalog(state, catalog, 4, false, events)?;
    crate::coverage::record_relic(RelicId::RelicRoyalPoison);
    Ok(())
}

/// Emotion Chip's passive pass over the orb queue after a turn in which its
/// owner lost HP.
///
/// `EmotionChip/<AfterPlayerTurnStart>d__5::MoveNext` (v0.111.0 RVA
/// `0x323ae8`): `player == Owner` at `IL_0025`-`IL_002c`,
/// `LostHpInPreviousTurn` at `IL_0039`-`IL_0040`, then one awaited
/// `OrbCmd::Passive` per orb of `OrbQueue.Orbs` (`IL_004d`-`IL_0090`,
/// looping at `IL_0153`-`IL_0158`). The body reads the private owner carrier,
/// which the boundary ties to the inventory.
fn emotion_chip_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.emotion_chip_owned() || !state.emotion_damage_previous_turn() || state.history.over {
        return Ok(());
    }
    let orb_count = state.orbs.as_slice().len();
    for index in 0..orb_count {
        super::orbs::passive_at_affected_by_hooks(state, catalog, index, events)?;
    }
    crate::coverage::record_relic(RelicId::RelicEmotionChip);
    Ok(())
}

/// Vexing Puzzlebox's turn-one card: the first of one full
/// `GetDistinctForCombat` shuffle of the **owner's** unlocked pool.
///
/// `VexingPuzzlebox/<AfterPlayerTurnStart>d__2::MoveNext` (v0.111.0 RVA
/// `0x333de0`): `player == Owner` at `IL_0020`-`IL_002c`, `TurnNumber == 1` at
/// `IL_0033`-`IL_0044`, then `Owner.Character.CardPool` (`IL_005d`/`IL_0062`)
/// `.GetUnlockedCards(Owner.UnlockState, RunState.CardMultiplayerConstraint)`
/// (`IL_0082`) handed to `CardFactory::GetDistinctForCombat(.., 1,
/// RunRngSet.CombatCardGeneration)` (`IL_0087`-`IL_009d`), `First` at
/// `IL_00a2`, `SetToFreeThisTurn` at `IL_00a9`, and one awaited
/// `CardPileCmd::AddGeneratedCardToCombat(card, Hand, player, true)` at
/// `IL_00ae`-`IL_00b7`. The pool is the owner's character pool, never a
/// fixed Ironclad one.
///
/// The fully unlocked Ironclad branch is the frozen `STOKE_CARD_POOL_V109`
/// (which is `owner_generation_pool(Ironclad, None)`, pinned by
/// `character_pools_reproduce_the_frozen_constants`), unchanged. Every other
/// solo case (#2827) — another owner, or an Ironclad whose recorded profile
/// lacks a card-pool gating epoch — derives its pool from the generated
/// `CHARACTER_CARD_POOL_ROWS_V1101` through [`Catalog::owner_generation_pool`],
/// filtered by the recorded unlock profile, which must therefore be known:
/// either every gating epoch is revealed (`fully_unlocked_card_pool_epochs`)
/// or the partial profile is in the catalog.
///
/// The oracle is `_apply_generation_relic_batch(s, "VEXING_PUZZLEBOX")`
/// (frozen Python `_continue_after_gambling_chip`, deleted #2827). Its `_generation_relic_pool` derives the owner pool at the FULLY UNLOCKED epochs
/// (`_FULLY_UNLOCKED_GENERATION_EPOCHS_V1101`) whatever the profile, after an
/// entry gate (`start_combat`) that checks the Ironclad epochs only. On a
/// profile that reveals the owner's own gating epochs the two are the same
/// list, and that is the only case admitted here. On one that does not, the
/// oracle over-draws the fully unlocked pool where native reads
/// `GetUnlockedCards(Owner.UnlockState, ..)`, so the two answers differ and
/// neither side has a captured witness: this refuses by name
/// (`PARTIAL_PROFILE`) rather than choose, pinned by
/// `a_vexing_puzzlebox_opens_for_its_owner_like_the_oracle`. That is a
/// narrowing, which can only refuse a fight the oracle roots.
fn vexing_puzzlebox_generated_card(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<CardId, EngineRefusal> {
    const PROVENANCE: &str = "Vexing Puzzlebox generation provenance";
    const PARTIAL_PROFILE: &str =
        "Vexing Puzzlebox owner pool under a profile hiding the owner's card-pool epochs";
    if state.rng.is_vacant(RngStream::Generation) {
        return Err(EngineRefusal::MalformedArgs(PROVENANCE));
    }
    match state.reward_card_pool {
        Some(crate::catalog::RewardPool::Ironclad) if state.fully_unlocked_card_pool_epochs => {
            let shuffled = super::cards::shuffle_generation_pool(
                state,
                &crate::content_tables::STOKE_CARD_POOL_V109,
            )?;
            Ok(shuffled[0])
        }
        Some(owner)
            if state.multiplayer_ally_key == 0
                && (state.fully_unlocked_card_pool_epochs
                    || catalog.splash_unlock_epochs().is_some()) =>
        {
            let pool = catalog.owner_generation_pool(owner);
            if pool.is_empty() {
                return Err(EngineRefusal::MalformedArgs(PROVENANCE));
            }
            if pool != crate::steps::neutral::owner_generation_pool(owner, None) {
                return Err(EngineRefusal::MalformedArgs(PARTIAL_PROFILE));
            }
            let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
            Ok(shuffled[0])
        }
        _ => Err(EngineRefusal::MalformedArgs(PROVENANCE)),
    }
}

/// Vexing Puzzlebox's turn-one body (`VexingPuzzlebox/<AfterPlayerTurnStart>d__2`
/// RVA `0x333de0`, see [`vexing_puzzlebox_generated_card`]), at whichever
/// place the dispatch runs it.
fn vexing_puzzlebox_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn == 1 && catalog.hooks().owns(RelicId::RelicVexingPuzzlebox) {
        let generated = vexing_puzzlebox_generated_card(catalog, state)?;
        super::cards::inject_generated_free_this_turn_bottom(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id: generated,
                upgrade: 0,
                enchantment: None,
            },
            PileId::Hand,
            events,
        )?;
        crate::coverage::record_relic(RelicId::RelicVexingPuzzlebox);
    }
    Ok(())
}

/// Toasty Mittens' place in the walk: the side-start tail it owes at its
/// pause, then the Hand choice ([`toasty_mittens_select`]).
///
/// `ToastyMittens/<AfterPlayerTurnStart>d__6` (RVA `0x33285c`) gates only
/// `player == Owner` (`IL_0033`-`IL_0042`) and has no Hand gate before
/// `CardSelectCmd::FromHand` (`IL_0068`), which pauses before it reads the
/// Hand (#3050): the side-start tail runs at that pause and can still hand it
/// a card. It runs here, ahead of the Hand read, whatever the Hand holds,
/// because the candidate count decides whether native prompts at all (#3273).
fn toasty_mittens_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<WalkStep, EngineRefusal> {
    if state.history.over || !catalog.hooks().owns(RelicId::RelicToastyMittens) {
        return Ok(WalkStep::Ran);
    }
    super::turn::run_side_start_ahead_of_setup_remainder(
        state,
        catalog,
        RelicId::RelicToastyMittens,
        events,
    )?;
    if toasty_mittens_select(catalog, state, events)? {
        Ok(WalkStep::Ran)
    } else {
        Ok(WalkStep::Paused)
    }
}

/// Toasty Mittens' `FromHand` over the post-pause Hand (#3273). Returns
/// `false` when it parked a choice.
///
/// v0.111.0 `ToastyMittens/<AfterPlayerTurnStart>d__6::MoveNext` (RVA
/// `0x33285c`) builds `new CardSelectorPrefs(ExhaustSelectionPrompt, 1)` at
/// `IL_005b`-`IL_0061` and awaits `CardSelectCmd::FromHand(choiceContext,
/// player, prefs, null, this)` at `IL_0068`. The two-argument ctor (RVA
/// `0x1397d4`) forwards `MinSelect = MaxSelect = 1` into ctor RVA `0x1397f8`,
/// which sets `RequireManualConfirmation = MinSelect >= 0 && MinSelect !=
/// MaxSelect` (`IL_006a`-`IL_0088`) — false. `<FromHand>d__28::MoveNext` RVA
/// `0x3e7568` reads the Hand after its pause (`IL_010d`-`IL_014c`, the null
/// filter defaulting to `<FromHand>b__28_0` RVA `0x3e54dd`, `ldc.i4.1; ret`),
/// returns empty for zero candidates (`IL_0152`-`IL_0161`), and returns the
/// whole list without a selector or a synchronized choice when
/// `!RequireManualConfirmation && Count <= MinSelect` (`IL_0167`-`IL_018d`);
/// only `Count > MinSelect` falls through `IL_0184 bgt.s` to the selector. So:
///
/// * an empty Hand exhausts nothing;
/// * a one-card Hand exhausts that card with no prompt;
/// * two or more park the choice.
///
/// Mittens then walks the result (`IL_00ca`-`IL_015b`), awaiting
/// `CardCmd::Exhaust(choiceContext, card, false, false)` per card (`IL_00f2`),
/// and applies `StrengthPower` of its `Strength` var (1) to the owner
/// (`IL_0181`-`IL_01af`) whether or not anything was exhausted.
fn toasty_mittens_select(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    match state.piles.get(PileId::Hand).len() {
        0 => {}
        1 => toasty_mittens_exhaust(catalog, state, 0, events)?,
        _ => {
            begin_hand_relic_selection(state, RelicPendingKind::ToastyMittens)?;
            return Ok(false);
        }
    }
    super::damage::apply_owner_strength(state, 1, events)?;
    crate::coverage::record_relic(RelicId::RelicToastyMittens);
    Ok(true)
}

/// Toasty Mittens' per-card `CardCmd::Exhaust` (`<AfterPlayerTurnStart>d__6`
/// RVA `0x33285c` `IL_00f2`), whose IsOverOrEnding gate (RVA `0x3e06c8`,
/// `IL_0020`-`IL_0034`) leaves the card in Hand (#3041). An exhaust listener
/// that parks a choice is not modeled inside this relic body.
fn toasty_mittens_exhaust(
    catalog: &Catalog,
    state: &mut HotState,
    index: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if let Some(card) = super::draw::detach_card_for_exhaust(state, PileId::Hand, index)? {
        super::draw::card_exhausted(state, catalog, card, events)?;
    }
    if state.pending.is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    Ok(())
}

/// Gambling Chip's place in the walk: its turn-one discard choice.
///
/// `GamblingChip/<AfterPlayerTurnStart>d__2` (RVA `0x325788`) gates only
/// `player == Owner` (`IL_002e`-`IL_0033`) and `TurnNumber <= 1`
/// (`IL_003b`-`IL_004b`) before `CardSelectCmd::FromHandForDiscard`
/// (`IL_0071`), whose `FromHand` pauses before it reads the Hand (#3050): a
/// side-start tail that runs at that pause can still hand it a card. The
/// chosen cards go through `CardCmd::DiscardAndDraw` (`IL_00f0`) on resume
/// ([`resume_relic_selection`]).
fn gambling_chip_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<WalkStep, EngineRefusal> {
    if state.turn > 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicGamblingChip) {
        return Ok(WalkStep::Ran);
    }
    if state.piles.get(PileId::Hand).is_empty() {
        super::turn::run_side_start_ahead_of_setup_remainder(
            state,
            catalog,
            RelicId::RelicGamblingChip,
            events,
        )?;
    }
    if state.piles.get(PileId::Hand).is_empty() {
        return Ok(WalkStep::Ran);
    }
    begin_hand_relic_selection(state, RelicPendingKind::GamblingChip)?;
    crate::coverage::record_relic(RelicId::RelicGamblingChip);
    Ok(WalkStep::Paused)
}

fn partial_permutations(n: usize, k: usize) -> Option<u32> {
    (0..k).try_fold(1_u32, |value, offset| {
        value.checked_mul(u32::try_from(n.checked_sub(offset)?).ok()?)
    })
}

fn gambling_action_count(entry_count: usize) -> Option<u32> {
    (0..=entry_count).try_fold(0_u32, |total, k| {
        total.checked_add(partial_permutations(entry_count, k)?)
    })
}

pub(crate) fn gambling_selection(
    entries: &[FrozenAutoBatchEntry],
    mut ordinal: u32,
) -> Option<Vec<HotCard>> {
    let n = entries.len();
    let mut k = 0;
    loop {
        let count = partial_permutations(n, k)?;
        if ordinal < count {
            break;
        }
        ordinal -= count;
        k += 1;
        if k > n {
            return None;
        }
    }
    let mut available = entries.iter().map(|entry| entry.card).collect::<Vec<_>>();
    let mut selected = Vec::with_capacity(k);
    for position in 0..k {
        let suffix = partial_permutations(n - position - 1, k - position - 1)?;
        let index = usize::try_from(ordinal / suffix).ok()?;
        ordinal %= suffix;
        selected.push(available.remove(index));
    }
    Some(selected)
}

pub(crate) fn relic_selection_action_count(
    state: &HotState,
    catalog: &Catalog,
) -> Result<u32, EngineRefusal> {
    if !relic_pending_is_exact(state, catalog) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let pending = state
        .fanouts
        .batch_nine_relic_pending()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    match pending.kind {
        RelicPendingKind::ChoicesParadox | RelicPendingKind::Toolbox => pending
            .entries
            .len()
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("generation relic choices")),
        RelicPendingKind::ToastyMittens => pending
            .entries
            .len()
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("Toasty Mittens choices")),
        RelicPendingKind::GamblingChip => gambling_action_count(pending.entries.len())
            .ok_or(EngineRefusal::CounterOverflow("Gambling Chip choices")),
    }
}

pub(crate) fn resume_relic_selection(
    state: &mut HotState,
    catalog: &Catalog,
    ordinal: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !relic_pending_is_exact(state, catalog) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let pending = state
        .fanouts
        .batch_nine_relic_pending()
        .cloned()
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    state.pending = None;
    state.fanouts.set_batch_nine_relic_pending(None);
    match pending.kind {
        RelicPendingKind::ChoicesParadox | RelicPendingKind::Toolbox => {
            let entry = pending
                .entries
                .get(usize::try_from(ordinal).map_err(|_| {
                    EngineRefusal::MalformedArgs("generation relic selection ordinal")
                })?)
                .ok_or(EngineRefusal::MalformedArgs(
                    "generation relic selection ordinal",
                ))?;
            let identity = catalog
                .spec(entry.card.atom)
                .ok_or(EngineRefusal::UnknownAtom(entry.card.atom))?
                .identity;
            let uid = state.next_card_uid;
            super::cards::inject_generated_bottom(
                state,
                catalog,
                identity,
                1,
                PileId::Hand,
                events,
            )?;
            if pending.kind == RelicPendingKind::ChoicesParadox {
                state.card_states.set_local_retain(uid);
                // `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext`
                // (`0x321b0c`) gives every offered card the Retain keyword
                // through `CardCmd.ApplyKeyword` (IL_00f2-IL_0105) before the
                // grid opens, so the chosen copy carries a keyword the same
                // card from the deck does not. Beside such a twin the two
                // are different physical cards with one sort key: promote
                // exact pile identity, as every other payload writer does,
                // or `legal_actions` collapses them into one play and never
                // offers the plain copy (#3756, site witness `0KPA7UA3BF5V`
                // node 34: a Paradox Headbutt beside the deck's Headbutt).
                if !state.exact_piles && super::cards::live_cards_need_exact_piles(state, catalog)?
                {
                    state.exact_piles = true;
                }
                after_player_turn_start_walk(
                    catalog,
                    state,
                    events,
                    Some(RelicId::RelicChoicesParadox),
                    true,
                )?;
            } else {
                super::turn::resume_player_turn_start_after_toolbox(state, catalog, events)?;
            }
        }
        RelicPendingKind::GamblingChip => {
            let selected = gambling_selection(&pending.entries, ordinal).ok_or(
                EngineRefusal::MalformedArgs("Gambling Chip selection ordinal"),
            )?;
            if !selected.is_empty() {
                let count = selected.len();
                let hand = state.piles.get(PileId::Hand).as_slice();
                if selected.iter().any(|selected_card| {
                    hand.iter().filter(|live| **live == *selected_card).count() != 1
                }) {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
                // DiscardAndDraw detaches each pick itself, in pick order,
                // behind its entry and per-card ending gates (#3075).
                super::draw::discard_and_draw(
                    state,
                    catalog,
                    PileId::Hand,
                    &selected,
                    count,
                    events,
                )?;
            }
            after_player_turn_start_walk(
                catalog,
                state,
                events,
                Some(RelicId::RelicGamblingChip),
                true,
            )?;
        }
        RelicPendingKind::ToastyMittens => {
            let entry = pending
                .entries
                .get(usize::try_from(ordinal).map_err(|_| {
                    EngineRefusal::MalformedArgs("Toasty Mittens selection ordinal")
                })?)
                .ok_or(EngineRefusal::MalformedArgs(
                    "Toasty Mittens selection ordinal",
                ))?;
            let index = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .position(|card| card.uid == entry.card.uid)
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            toasty_mittens_exhaust(catalog, state, index, events)?;
            super::damage::apply_owner_strength(state, 1, events)?;
            crate::coverage::record_relic(RelicId::RelicToastyMittens);
            after_player_turn_start_walk(
                catalog,
                state,
                events,
                Some(RelicId::RelicToastyMittens),
                true,
            )?;
        }
    }
    if pending.kind != RelicPendingKind::Toolbox && state.pending.is_none() {
        super::turn::resume_player_turn_start_after_relics(state, catalog, events)?;
    }
    Ok(())
}

pub(crate) fn relic_pending_is_exact(state: &HotState, catalog: &Catalog) -> bool {
    let marker = state
        .pending
        .as_deref()
        .is_some_and(PendingSelection::is_relic_selection);
    let Some(pending) = state.fanouts.batch_nine_relic_pending() else {
        return !marker;
    };
    if !marker || !state.frames.is_empty() {
        return false;
    }
    let owns = |relic| catalog.hooks().owns(relic);
    match pending.kind {
        RelicPendingKind::ChoicesParadox | RelicPendingKind::Toolbox => {
            let (relic, count) = if pending.kind == RelicPendingKind::ChoicesParadox {
                (RelicId::RelicChoicesParadox, 5)
            } else {
                (RelicId::RelicToolbox, 3)
            };
            let Ok(pool) = generation_relic_draw_pool(catalog, state, pending.kind) else {
                return false;
            };
            owns(relic)
                && pending.entries.len() == count
                && pending.entries.iter().all(|entry| {
                    entry.card.uid == LEGACY_CARD_UID
                        && entry.card.flags == CARD_FLAG_LEGACY
                        && catalog.spec(entry.card.atom).is_some_and(|spec| {
                            spec.identity.upgrade == 0
                                && spec.identity.enchantment.is_none()
                                && pool.contains(&spec.identity.id)
                        })
                        && {
                            let expected = CardInstanceState {
                                local_retain: pending.kind == RelicPendingKind::ChoicesParadox,
                                ..CardInstanceState::default()
                            };
                            entry.state == expected
                        }
                })
        }
        RelicPendingKind::GamblingChip | RelicPendingKind::ToastyMittens => {
            let relic = if pending.kind == RelicPendingKind::GamblingChip {
                RelicId::RelicGamblingChip
            } else {
                RelicId::RelicToastyMittens
            };
            owns(relic)
                && !pending.entries.is_empty()
                && pending.entries.len() == state.piles.get(PileId::Hand).len()
                && pending
                    .entries
                    .iter()
                    .zip(state.piles.get(PileId::Hand).as_slice().iter().copied())
                    .all(|(entry, card)| {
                        entry.card == card && entry.state == state.card_states.get(card.uid)
                    })
        }
    }
}

/// What one listener body did in a relic walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkStep {
    /// The body finished (or its own gates left at once).
    Ran,
    /// The body parked a player choice. The walk stops here, and resuming the
    /// choice re-enters it at the next listener.
    Paused,
    /// The body stopped the pass without offering its choice (Toolbox on a
    /// combat an earlier listener ended). The walk stops here for good.
    Suspended,
}

/// The order this engine runs one hook's relic bodies in (#3635).
///
/// `listeners` is every relic whose type overrides the hook, in the fixed
/// sequence this engine ran before the inventory order was read.
///
/// Native builds one listener list per dispatch and awaits it in order
/// (`CombatState/<IterateHookListeners>d__69::MoveNext`, v0.111.0 RVA
/// `0x3f9720`): for each ally and then each enemy, the creature's powers
/// (`IL_0092`-`IL_0097`), and for a player its relics by ascending
/// `Player.Relics` index, skipping `IsMelted` (`IL_00c9`-`IL_00ef`), then its
/// potions (`IL_0107`-`IL_0128`), orbs (`IL_014d`-`IL_015c`) and pile cards
/// (`IL_0163`-`IL_01c7`). `Player::ToSerializable` (RVA `0x116dec`) writes
/// that relic list in order, so a save- or capture-rooted inventory records
/// the dispatch order ([`crate::hooks::HookTable::dispatch_ordered`]).
///
/// So a vouched inventory yields its owned listeners in recorded order. An
/// unvouched one (a `.run`-only root) yields the fixed sequence; the pairs
/// whose order shows there are refused by name at the opening, in admission
/// and at a pause. The boundary refuses a duplicate relic, so each listener
/// appears once and "the listener after relic R" is a function of the
/// catalog.
pub(crate) fn relic_walk_order<'a>(
    catalog: &'a Catalog,
    listeners: &'static [RelicId],
) -> impl Iterator<Item = RelicId> + Clone + 'a {
    let hooks = catalog.hooks();
    let vouched = hooks.dispatch_ordered();
    let recorded = vouched
        .then(|| {
            hooks
                .relics()
                .iter()
                .copied()
                .filter(move |relic| listeners.contains(relic))
        })
        .into_iter()
        .flatten();
    let fixed = (!vouched)
        .then(|| {
            listeners
                .iter()
                .copied()
                .filter(move |relic| hooks.owns(*relic))
        })
        .into_iter()
        .flatten();
    recorded.chain(fixed)
}

/// Every relic that listens on `AfterPlayerTurnStart`, in the fixed sequence
/// an unvouched inventory runs ([`relic_walk_order`]).
///
/// A v0.111.0 method-table scan of `sts2.dll` (DLL 9cb4f1ad) finds exactly
/// these eleven relic types overriding `AbstractModel::AfterPlayerTurnStart`
/// (base RVA `0x7a1f2`): Bellows `0x906dc`, BoneTea `0x90ffc`, ChoicesParadox
/// `0x92214`, EmotionChip `0x9306c`, FestivePopper `0x93918`, GamblingChip
/// `0x94424`, MercuryHourglass `0x96c60`, MrStruggles `0x970e8`, RoyalPoison
/// `0x9a8dc`, ToastyMittens `0x9c984`, VexingPuzzlebox `0x9db70`. No type
/// overrides `AfterPlayerTurnStartEarly` (base `0x7a1eb`), and only Blood Vial
/// and Fake Blood Vial override `AfterPlayerTurnStartLate`, whose pass
/// (`Hook/<AfterPlayerTurnStart>d__56` IL_022f/IL_027c) is a separate walk
/// behind the whole ordinary one and so always lies in a paused remainder.
const AFTER_PLAYER_TURN_START_RELIC_DISPATCH: [RelicId; 11] = [
    RelicId::RelicChoicesParadox,
    RelicId::RelicGamblingChip,
    RelicId::RelicVexingPuzzlebox,
    RelicId::RelicToastyMittens,
    RelicId::RelicBellows,
    RelicId::RelicBoneTea,
    RelicId::RelicFestivePopper,
    RelicId::RelicMrStruggles,
    RelicId::RelicRoyalPoison,
    RelicId::RelicEmotionChip,
    RelicId::RelicMercuryHourglass,
];

/// Whether `relic`'s `AfterPlayerTurnStart` body is still ahead once the walk
/// is parked on `paused`'s choice: it sits behind `paused` in
/// [`relic_walk_order`]. False when either is not in the walk.
pub(crate) fn after_player_turn_start_relic_follows(
    catalog: &Catalog,
    relic: RelicId,
    paused: RelicId,
) -> bool {
    let mut passed = false;
    for listener in relic_walk_order(catalog, &AFTER_PLAYER_TURN_START_RELIC_DISPATCH) {
        if listener == paused {
            passed = true;
        } else if listener == relic {
            return passed;
        }
    }
    false
}

/// The `AfterPlayerTurnStart` template fire point. The walk
/// ([`after_player_turn_start`]) fires Mercury Hourglass, this hook's one
/// template relic at v0.111.0, at its own place, so this only records the
/// fire point. Any other compiled subscriber refuses by name rather than run
/// out of place.
pub(crate) fn after_player_turn_start_template_pass(
    catalog: &Catalog,
) -> Result<(), EngineRefusal> {
    let event = HookEvent::AfterPlayerTurnStart;
    crate::coverage::record_hook(event);
    let hooks = catalog.hooks();
    if hooks.has(event)
        && hooks.subscribers(event).iter().any(|subscriber| {
            subscriber.subject != HookSubject::Relic(RelicId::RelicMercuryHourglass)
        })
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "AfterPlayerTurnStart template subscriber outside the relic walk",
        ));
    }
    Ok(())
}

/// Mercury Hourglass's body: the compiled template rule, fired at the relic's
/// place in the walk.
///
/// `MercuryHourglass/<AfterPlayerTurnStart>d__4::MoveNext` (v0.111.0 RVA
/// `0x32a854`) gates only `player == Owner` (`IL_001d`-`IL_002b`), then
/// awaits one `CreatureCmd::Damage(HittableEnemies, DynamicVars.Damage,
/// Owner.Creature)` at `IL_0036`-`IL_0067`: no turn gate and no RNG draw.
fn mercury_hourglass_after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let event = HookEvent::AfterPlayerTurnStart;
    let subscriber = catalog
        .hooks()
        .subscribers(event)
        .iter()
        .find(|subscriber| subscriber.subject == HookSubject::Relic(RelicId::RelicMercuryHourglass))
        .ok_or(EngineRefusal::HookNotModeled {
            relic: RelicId::RelicMercuryHourglass,
            event,
        })?;
    fire_template_subscriber(
        catalog,
        subscriber,
        RelicId::RelicMercuryHourglass,
        event,
        state,
        events,
    )
}

/// Whether an `AfterPlayerTurnStart` relic body acts this turn, by its own
/// entry gates (the bodies of [`after_player_turn_start_body`]).
fn after_player_turn_start_relic_acts(catalog: &Catalog, state: &HotState, relic: RelicId) -> bool {
    let owns = catalog.hooks().owns(relic);
    match relic {
        RelicId::RelicChoicesParadox
        | RelicId::RelicVexingPuzzlebox
        | RelicId::RelicFestivePopper => owns && state.turn == 1,
        RelicId::RelicGamblingChip | RelicId::RelicBellows | RelicId::RelicRoyalPoison => {
            owns && state.turn <= 1
        }
        RelicId::RelicBoneTea => {
            owns && state.turn <= 1 && state.fanouts.bone_tea_combats_left() > 0
        }
        // The body reads the private owner carrier, not the hook table.
        RelicId::RelicEmotionChip => {
            state.emotion_chip_owned() && state.emotion_damage_previous_turn()
        }
        _ => owns,
    }
}

/// A relic choice paused `SetupPlayerTurn`, so native `StartTurn` runs the
/// side-start tail before the rest of the `AfterPlayerTurnStart` pass (#3050,
/// `turn::SetupPlayerTurnWindow`). Native walks that pass in `Player.Relics`
/// order (`Hook/<AfterPlayerTurnStart>d__56::MoveNext` RVA `0x3cff0c`,
/// `IterateCombatHookListeners` IL_012d then `AfterPlayerTurnStart` IL_017a),
/// so exactly the relics before the paused one ran ahead of the tail.
///
/// On a vouched inventory the walk is that order, so the prefix this engine
/// ran is native's. On an unvouched one the walk is the fixed sequence, and
/// an unrecorded acquisition order cannot show that its prefix is native's:
/// any other relic that acts this turn refuses by name.
pub(crate) fn after_player_turn_start_pause_order_is_native(
    catalog: &Catalog,
    state: &HotState,
    paused: RelicId,
) -> Result<(), EngineRefusal> {
    const REFUSAL: EngineRefusal = EngineRefusal::PowerOrderNotModeled(
        "AfterPlayerTurnStart relic order around a paused SetupPlayerTurn choice",
    );
    if !AFTER_PLAYER_TURN_START_RELIC_DISPATCH.contains(&paused) {
        return Err(REFUSAL);
    }
    if catalog.hooks().dispatch_ordered() {
        return Ok(());
    }
    if AFTER_PLAYER_TURN_START_RELIC_DISPATCH
        .into_iter()
        .any(|relic| relic != paused && after_player_turn_start_relic_acts(catalog, state, relic))
    {
        return Err(REFUSAL);
    }
    Ok(())
}

/// Vexing Puzzlebox beside Toasty Mittens on an unvouched inventory: the
/// free card is among Toasty's candidates exactly when the Puzzlebox is
/// recorded first (`VexingPuzzlebox/<AfterPlayerTurnStart>d__2` RVA `0x333de0`
/// `IL_00b7`; `ToastyMittens/<AfterPlayerTurnStart>d__6` RVA `0x33285c`
/// `IL_0068`), and that order is unknown. Refused before either body writes.
fn unvouched_puzzlebox_beside_toasty_mittens(catalog: &Catalog) -> Result<(), EngineRefusal> {
    let hooks = catalog.hooks();
    if !hooks.dispatch_ordered()
        && hooks.owns(RelicId::RelicVexingPuzzlebox)
        && hooks.owns(RelicId::RelicToastyMittens)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Toasty Mittens + Vexing Puzzlebox recorded order",
        ));
    }
    Ok(())
}

/// One relic's `AfterPlayerTurnStart` body, at whichever place the walk
/// reaches it. Each body carries its own IL and entry gates.
fn after_player_turn_start_body(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    relic: RelicId,
) -> Result<WalkStep, EngineRefusal> {
    match relic {
        // `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext` (RVA
        // `0x321b0c`): owner and `TurnNumber == 1` (`IL_0027`-`IL_004b`), a
        // `GetDistinctForCombat` draw of five (`IL_009e`-`IL_00b3`), then the
        // pause on `CardSelectCmd::FromSimpleGrid` (`IL_0144`).
        RelicId::RelicChoicesParadox => {
            if state.turn == 1 && !state.history.over {
                begin_generation_relic_selection(
                    state,
                    catalog,
                    RelicPendingKind::ChoicesParadox,
                    5,
                )?;
                crate::coverage::record_relic(RelicId::RelicChoicesParadox);
                return Ok(WalkStep::Paused);
            }
        }
        RelicId::RelicGamblingChip => {
            return gambling_chip_after_player_turn_start(catalog, state, events);
        }
        RelicId::RelicVexingPuzzlebox => {
            unvouched_puzzlebox_beside_toasty_mittens(catalog)?;
            if !state.history.over {
                vexing_puzzlebox_after_player_turn_start(catalog, state, events)?;
            }
        }
        RelicId::RelicToastyMittens => {
            unvouched_puzzlebox_beside_toasty_mittens(catalog)?;
            return toasty_mittens_after_player_turn_start(catalog, state, events);
        }
        RelicId::RelicBellows => bellows_after_player_turn_start(catalog, state)?,
        RelicId::RelicBoneTea => bone_tea_after_player_turn_start(catalog, state)?,
        RelicId::RelicFestivePopper => {
            festive_popper_after_player_turn_start(catalog, state, events)?;
        }
        RelicId::RelicMrStruggles => mr_struggles_after_player_turn_start(catalog, state, events)?,
        RelicId::RelicRoyalPoison => royal_poison_after_player_turn_start(catalog, state, events)?,
        RelicId::RelicEmotionChip => emotion_chip_after_player_turn_start(catalog, state, events)?,
        RelicId::RelicMercuryHourglass => {
            mercury_hourglass_after_player_turn_start(catalog, state, events)?;
        }
        _ => {}
    }
    Ok(WalkStep::Ran)
}

/// A relic of the walk reached after an earlier relic of the same walk ended
/// the combat.
///
/// Native checks the combat once, before the first listener
/// (`Hook/<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`,
/// `IsOverOrEnding && !IsStarting` at `IL_0028`-`IL_0042`), and then yields
/// every listener with no further check (`IL_0064`-`IL_0095`). So a later
/// body still runs on the ended combat.
///
/// What each body does there, from its commands' own gates (v0.111.0 DLL
/// 9cb4f1ad) and from the game run headless on the archived DLL (#3889; the
/// runs are listed in `solver/invariant-walks/2026-10-12-issue3889-…`):
///
/// * **Festive Popper, Mr Struggles, Mercury Hourglass**: nothing.
///   `CreatureCmd/<Damage>d__12::MoveNext` (RVA `0x3e96c8`) returns its empty
///   result list when the target list is empty (`IL_00c6`-`IL_00e4`), and
///   `CombatState::get_HittableEnemies` (RVA `0x1373a5`) keeps only
///   `Creature.IsHittable` enemies (`b__67_0` RVA `0x3f91ba`). A combat that
///   is over with an enemy still standing is not modeled for these hits and
///   refuses by the relic's name.
/// * **Toasty Mittens**: nothing. `CardSelectCmd/<FromHand>d__28` (RVA
///   `0x3e7568`) returns no card on `IsOverOrEnding` (`IL_0036`) before it
///   signals a choice, so nothing is exhausted and no side-start tail runs at
///   a pause; the Strength goes through ``PowerCmd/<Apply>d__1`1`` (RVA
///   `0x3ef988`, the `Apply<StrengthPower>` of `IL_01af`), which returns on
///   `IsEnding` (`IL_0025`-`IL_0034`).
/// * **Gambling Chip**: nothing. The same `FromHand` gate, then
///   `CardCmd/<DiscardAndDraw>d__4` (RVA `0x3e0274`) over no card.
/// * **Bellows**: nothing. `CardCmd::Upgrade` (RVA `0x12f660`) returns on
///   `IsEnding` (`IL_0011`).
/// * **Bone Tea**: the same gated `Upgrade`, so the Hand keeps its levels,
///   but the charge is still written back (`BoneTea::AfterPlayerTurnStart`
///   RVA `0x90ffc` `IL_007e`-`IL_0088`).
/// * **Royal Poison**: the hit lands. `CreatureCmd/<Damage>d__12` has no
///   ending gate ahead of the HP write.
/// * **Vexing Puzzlebox**: the `GetDistinctForCombat` shuffle still draws
///   `CombatCardGeneration` (`<AfterPlayerTurnStart>d__2` RVA `0x333de0`
///   `IL_0087`-`IL_009d`); the card is not added
///   (`CardPileCmd/<AddGeneratedCardsToCombat>d__6` RVA `0x3e2f0c`,
///   `IsInProgress` at `IL_003e`).
/// * **Choices Paradox**: the five-card `GetDistinctForCombat` draw runs
///   (`<AfterPlayerTurnStart>d__6` RVA `0x321b0c` `IL_00b3`), then
///   `CardSelectCmd/<FromSimpleGrid>d__18` (RVA `0x3e8044`) returns no card on
///   `IsEnding` (`IL_0041`): no choice opens and no card is added.
/// * **Emotion Chip**: nothing. `OrbCmd/<Passive>d__7` (RVA `0x3ede90`)
///   returns on `IsOverOrEnding` (`IL_0022`) for every orb.
///
/// `history.over` implies both `IsEnding` and `IsOverOrEnding`
/// ([`super::damage::damage_combat_is_ending`]).
///
/// These bodies commute with one another: each writes its own counter, the
/// owner's HP or the generation stream, and two full shuffles advance that
/// stream by the same count in either order. So a pass a player POWER ended
/// runs them in any relic order. A pass a RELIC ended needs the killer's
/// place among them, which an unvouched inventory does not record: that
/// keeps the refusal by the later relic's name
/// ([`after_player_turn_start_unvouched_ended_refusal`]).
fn after_player_turn_start_on_ended_combat(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    relic: RelicId,
) -> Result<(), EngineRefusal> {
    if !after_player_turn_start_relic_acts(catalog, state, relic) {
        return Ok(());
    }
    match relic {
        RelicId::RelicFestivePopper
        | RelicId::RelicMrStruggles
        | RelicId::RelicMercuryHourglass => {
            if !super::damage::alive_targets(state).is_empty() {
                return Err(after_player_turn_start_unvouched_ended_refusal(relic));
            }
        }
        RelicId::RelicChoicesParadox => {
            let pool =
                generation_relic_draw_pool(catalog, state, RelicPendingKind::ChoicesParadox)?;
            super::cards::shuffle_generation_slice(state, &pool)?;
            crate::coverage::record_relic(RelicId::RelicChoicesParadox);
        }
        RelicId::RelicVexingPuzzlebox => {
            vexing_puzzlebox_generated_card(catalog, state)?;
            crate::coverage::record_relic(RelicId::RelicVexingPuzzlebox);
        }
        RelicId::RelicBoneTea => {
            let written = state.fanouts.set_bone_tea_combats_left(0);
            debug_assert!(written);
            crate::coverage::record_relic(RelicId::RelicBoneTea);
        }
        RelicId::RelicRoyalPoison => {
            // The owner-damage command without its step-level ending gate,
            // as Corrupted's hit behind a lethal body takes it. Its listeners
            // carry their own gates: Rupture's Strength is an ending-gated
            // `PowerCmd.Apply` and is not applied; a player `CreatureCmd.Heal`
            // (Demon Tongue) is not gated (`<Heal>d__20` RVA `0x3eb4b0`
            // returns on `IsEnding` only for a non-player, `IL_0046`-`IL_005f`).
            //
            // A hit that takes the owner to 0 on the combat it already won
            // kills it: the run is over, a loss (#3889). The game, with
            // Festive Popper's kill ahead of Royal Poison and the owner at 3
            // or 4 HP: `Creature.IsDead` and `RunState.IsGameOver`; at 5 HP
            // the owner is left at 1. The death preventers answer as on a
            // live combat: Fairy in a Bottle is consumed and heals to 30% of
            // max HP (24 of 80) ahead of Lizard Tail, which is then unused;
            // Lizard Tail alone heals to half (40 of 80) and is used up, on
            // either side of the two relics in the inventory. That is the
            // owner-damage command's own lethal path
            // (`damage::resolve_player_lethal_with_catalog`,
            // `KillWithoutCheckingWinCondition` RVA `0x3ebe90`), which reads
            // no ending flag.
            //
            // What stays unmodeled is a choice parked inside the hit (a
            // listener of it, or Fairy's wrapper suspending): rehearsed on a
            // clone and refused by name.
            let mut probe = state.clone();
            let rehearsed = super::damage::damage_player_from_active_card_even_if_ending(
                &mut probe,
                catalog,
                0,
                4,
                false,
                &mut Vec::new(),
            );
            if rehearsed.is_err() || probe.pending.is_some() {
                return Err(EngineRefusal::PowerOrderNotModeled(
                    "Royal Poison parked after an AfterPlayerTurnStart listener ended combat",
                ));
            }
            super::damage::damage_player_from_active_card_even_if_ending(
                state, catalog, 0, 4, false, events,
            )?;
            crate::coverage::record_relic(RelicId::RelicRoyalPoison);
        }
        _ => {}
    }
    Ok(())
}

/// The refusal for a relic of the walk that would act behind a relic that
/// ended the combat, where the engine cannot run it: an unvouched inventory
/// (the killer's place among the relics is unknown), or an enemy hit on a
/// combat that is over with an enemy still standing.
fn after_player_turn_start_unvouched_ended_refusal(relic: RelicId) -> EngineRefusal {
    let name = match relic {
        RelicId::RelicFestivePopper => {
            "Festive Popper after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicMrStruggles => {
            "Mr Struggles after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicMercuryHourglass => {
            "Mercury Hourglass after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicChoicesParadox => {
            "Choices Paradox after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicGamblingChip => {
            "Gambling Chip after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicVexingPuzzlebox => {
            "Vexing Puzzlebox after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicToastyMittens => {
            "Toasty Mittens after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicBellows => "Bellows after an earlier AfterPlayerTurnStart relic ended combat",
        RelicId::RelicBoneTea => {
            "Bone Tea after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicRoyalPoison => {
            "Royal Poison after an earlier AfterPlayerTurnStart relic ended combat"
        }
        RelicId::RelicEmotionChip => {
            "Emotion Chip after an earlier AfterPlayerTurnStart relic ended combat"
        }
        _ => "AfterPlayerTurnStart relic after an earlier relic ended combat",
    };
    EngineRefusal::PowerOrderNotModeled(name)
}

/// The `AfterPlayerTurnStart` relic walk (#3635), from the top or from the
/// listener after `resume_after`, whose parked choice just resolved.
///
/// `Hook/<AfterPlayerTurnStart>d__56::MoveNext` (v0.111.0 RVA `0x3cff0c`)
/// awaits each listener of the ordinary pass in turn
/// (`IterateCombatHookListeners` at `IL_012d`, `AbstractModel::AfterPlayerTurnStart`
/// at `IL_017a`, looping at `IL_01fd`-`IL_0202`). The relic segment of that
/// list is [`relic_walk_order`]; the owner's powers sit ahead of it and run
/// before this function, as the list has them.
///
/// * A pass entered on an ended combat runs no listener
///   (`<IterateCombatHookListeners>d__0` `IL_0028`-`IL_0042`). The pass
///   begins at the owner's first POWER listener, not at the first relic, so
///   the caller says whether it began live (`pass_started_live`): a power
///   that ends the combat leaves the relics still to run (#3889).
/// * A body that parks a choice stops the walk. `StartTurn`'s side-start tail
///   runs at that pause (#3050), and [`resume_relic_selection`] re-enters
///   here behind that relic.
/// * A listener that ends the combat does not stop it:
///   [`after_player_turn_start_on_ended_combat`] runs each later body as the
///   game does. A dead owner is the lost fight, where nothing later can show.
/// * A body with no choice of its own whose listener parked one (an exhaust
///   or damage listener) cannot be resumed inside the body. With a later
///   relic still to act that refuses by name; with none it returns to the
///   caller parked, as before.
fn after_player_turn_start_walk(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    resume_after: Option<RelicId>,
    pass_started_live: bool,
) -> Result<(), EngineRefusal> {
    let hooks = catalog.hooks();
    if resume_after.is_none() && !pass_started_live {
        return Ok(());
    }
    // Ended before any relic of this entry ran: by a power of the pass, or
    // (on a resume) inside the side-start tail or the resumed body. Otherwise
    // a relic this entry ran ended it.
    let entered_over = state.history.over;
    let mut waiting = resume_after;
    if AFTER_PLAYER_TURN_START_RELIC_DISPATCH
        .iter()
        .any(|relic| hooks.owns(*relic))
    {
        let mut order = relic_walk_order(catalog, &AFTER_PLAYER_TURN_START_RELIC_DISPATCH);
        while let Some(relic) = order.next() {
            if let Some(paused) = waiting {
                if relic == paused {
                    waiting = None;
                }
                continue;
            }
            if state.history.over {
                if state.hp <= 0 {
                    return Ok(());
                }
                // A relic of this walk ended the combat and the inventory
                // does not record its place among the others.
                let killer_place_unknown = !entered_over && !hooks.dispatch_ordered();
                let hit_over_no_enemy = matches!(
                    relic,
                    RelicId::RelicFestivePopper
                        | RelicId::RelicMrStruggles
                        | RelicId::RelicMercuryHourglass
                ) && super::damage::alive_targets(state).is_empty();
                if killer_place_unknown
                    && !hit_over_no_enemy
                    && after_player_turn_start_relic_acts(catalog, state, relic)
                {
                    return Err(after_player_turn_start_unvouched_ended_refusal(relic));
                }
                after_player_turn_start_on_ended_combat(catalog, state, events, relic)?;
                continue;
            }
            if after_player_turn_start_body(catalog, state, events, relic)? == WalkStep::Paused {
                return Ok(());
            }
            if state.pending.is_some() {
                if order
                    .clone()
                    .any(|later| after_player_turn_start_relic_acts(catalog, state, later))
                {
                    return Err(EngineRefusal::PowerOrderNotModeled(
                        "AfterPlayerTurnStart relic behind a listener choice parked inside an earlier relic body",
                    ));
                }
                return Ok(());
            }
        }
    }
    if waiting.is_some() {
        return Err(EngineRefusal::MalformedArgs(
            "AfterPlayerTurnStart relic walk resume point",
        ));
    }
    // The carrier is tied to the inventory by the boundary; a state whose
    // carrier is set without the relic keeps the fixed tail place.
    if !hooks.owns(RelicId::RelicEmotionChip) && !state.history.over {
        emotion_chip_after_player_turn_start(catalog, state, events)?;
    }
    Ok(())
}

/// The owner's `AfterPlayerTurnStart` relics, behind its power listeners:
/// the recorded-order walk of [`after_player_turn_start_walk`].
/// `pass_started_live` is whether the combat was live when the pass reached
/// its first listener, the owner's first power.
pub(crate) fn after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    pass_started_live: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    after_player_turn_start_walk(catalog, state, events, None, pass_started_live)
}

/// Whether the owner holds a relic of the `AfterPlayerTurnStart` walk, whose
/// bodies can refuse by name after earlier turn-start writes
/// (`turn::begin_player_turn` rehearses such a turn on a clone, #3889).
pub(crate) fn after_player_turn_start_walk_is_reachable(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    AFTER_PLAYER_TURN_START_RELIC_DISPATCH
        .iter()
        .any(|relic| hooks.owns(*relic))
}

/// `BloodVial::get_CanonicalVars` (v0.111.0 RVA `0x90e65`) is
/// `ldc.i4.2; newobj HealVar::.ctor` — `combat_sim.BLOOD_VIAL_HEAL` (frozen Python `_relic_templates`, deleted #2827).
const BLOOD_VIAL_HEAL: i32 = 2;

/// `RELIC.BLOOD_VIAL` — heal 2 at the player's first `AfterPlayerTurnStartLate`
/// (#2756).
///
/// `BloodVial/<AfterPlayerTurnStartLate>d__4::MoveNext` (RVA `0x3200f4`): the
/// owner check `ldfld player; get_Owner; beq.s` at `IL_0021`-`IL_002c` with
/// `leave 220` at `IL_002e`; the turn guard
/// `get_Owner; get_PlayerCombatState; get_TurnNumber; ldc.i4.1; ble.s` at
/// `IL_0033`-`IL_0044`, whose fall-through `leave`s at `IL_0046`; then
/// `get_Owner; get_Creature` (`IL_004b`-`IL_0051`),
/// `DynamicVars.get_Heal().get_IntValue()` (`IL_0057`-`IL_0061`) and
/// `CreatureCmd::Heal` at `IL_006c`, awaited. The guard is `ble`, i.e.
/// `TurnNumber <= 1`, so `state.turn <= 1` is the literal port rather than the
/// oracle's equivalent `== 1`. Oracle: frozen Python `_run_after_side_turn_start_power_order` (deleted #2827).
///
/// # Why this is a body and not a document write
///
/// It is the one member of #2756's six whose gap is not a flag: `blood_vial`
/// itself is already a catalog mirror (`boundary.rs`, `owns(RelicBloodVial)`),
/// and what the opening was missing is `hp`. Applying it here rather than in
/// `entry::opening::pre_hook_document` is #2755/#2769's rule — `deal_opening_hand`
/// reaches this walk, so the opening and a post-opening document execute the
/// same code, and a copy in the document builder would let a Rust-opened root
/// and a capture-rooted one disagree.
///
/// **This hook is a ninth opening-window hook** neither gate table enumerates,
/// which is #2757's subject; the body lands here because the per-relic sweep
/// measured it, not because the hook set was re-derived. Nothing about the
/// window's *derivation* changes in this slice.
///
/// # Position, and why it is order-equivalent
///
/// Python runs this statement before `_fire_relic_templates(s,
/// "AfterPlayerTurnStartLate")`, so it is placed between the two template
/// fires in [`super::turn::resume_player_turn_start_after_relics`]. Native
/// instead walks one listener list in `Player.Relics` index order, and the
/// represented listener set on this hook is exactly **two** relics —
/// `RELIC.BLOOD_VIAL` here and `RELIC.FAKE_BLOOD_VIAL`, whose compiled template
/// is the same `heal` effect at amount 1 behind the same `turn <= 1` guard.
/// Two owner heals commute: `min(max_hp, hp + a)` then `min(max_hp, hp + b)`
/// equals `min(max_hp, hp + a + b)` for non-negative `a`, `b` either way round.
/// So the position cannot be observed, and a third listener arriving on this
/// hook re-opens the question rather than inheriting this argument.
///
/// The clamp and the ending gate are spelled exactly as
/// [`TemplateEffect::Heal`] spells them in [`apply_effect`], which is the same
/// arithmetic as the oracle's `healed = min(max_hp, hp + 2) - hp; hp += healed`.
pub(crate) fn after_player_turn_start_late(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn <= 1 && catalog.hooks().owns(RelicId::RelicBloodVial) && !state.history.over {
        let hp_before = state.hp;
        state.hp = state
            .hp
            .checked_add(BLOOD_VIAL_HEAL)
            .ok_or(EngineRefusal::CounterOverflow("Blood Vial heal"))?
            .min(state.max_hp);
        crate::coverage::record_relic(RelicId::RelicBloodVial);
        // The heal's own `AfterCurrentHpChanged` reaches Red Skull (#3044).
        // Red Skull's transition rides inside whichever heal crosses, so the
        // two heals still commute.
        super::damage::red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    }
    Ok(())
}

pub(crate) fn pollinous_hand_draw_bonus(activated: bool) -> i16 {
    if activated { 2 } else { 0 }
}

pub(crate) fn paels_eye_extra_turn_gate(state: &HotState, catalog: &Catalog) -> bool {
    catalog.hooks().owns(RelicId::RelicPaelsEye)
        && !state.fanouts.paels_eye_used()
        && state.history.manual_card_plays_finished_this_turn == 0
        && state.fanouts.paels_eye_was_owner_part_last_player_turn()
        && !(state.turn == 1 && catalog.hooks().owns(RelicId::RelicWhisperingEarring))
}

/// The early group of [`AFTER_SIDE_TURN_START_RELIC_DISPATCH`]: the bodies
/// an unvouched inventory runs ahead of the compiled templates.
const AFTER_SIDE_TURN_START_EARLY: [RelicId; 10] = [
    RelicId::RelicLantern,
    RelicId::RelicPaelsEye,
    RelicId::RelicBoomingConch,
    RelicId::RelicCandelabra,
    RelicId::RelicChandelier,
    RelicId::RelicHappyFlower,
    RelicId::RelicFakeHappyFlower,
    RelicId::RelicBigHat,
    RelicId::RelicCrossbow,
    RelicId::RelicOrangeDough,
];

/// The four compiled-template listeners of the hook
/// (`content_tables::TEMPLATE_RELIC_STEPS`). An unvouched inventory runs
/// them between the early and the late group, in the order it lists them.
const AFTER_SIDE_TURN_START_TEMPLATES: [RelicId; 4] = [
    RelicId::RelicDiamondDiadem,
    RelicId::RelicDivineDestiny,
    RelicId::RelicSai,
    RelicId::RelicVeryHotCocoa,
];

/// The late group: the bodies an unvouched inventory runs behind the
/// compiled templates.
const AFTER_SIDE_TURN_START_LATE: [RelicId; 10] = [
    RelicId::RelicPhylacteryUnbound,
    RelicId::RelicPaelsLegion,
    RelicId::RelicBread,
    RelicId::RelicPaelsTears,
    RelicId::RelicFencingManual,
    RelicId::RelicRunicCapacitor,
    RelicId::RelicInfusedCore,
    RelicId::RelicSymbioticVirus,
    RelicId::RelicBrimstone,
    RelicId::RelicSealOfGold,
];

/// Every relic that listens on `AfterSideTurnStart` (#3635).
///
/// A v0.111.0 method-table scan of `sts2.dll` (sha256 `9cb4f1ad…12b4`) finds
/// exactly these twenty-nine relic types overriding
/// `AbstractModel::AfterSideTurnStart` (base RVA `0x7a1dd`): Akabeko
/// `0x8fc44`, BigHat `0x909d8`, BoomingConch `0x91410`, Bread `0x91674`,
/// Brimstone `0x91930`, Candelabra `0x91ddc`, Chandelier `0x9208c`, Crossbow
/// `0x9251c`, DiamondDiadem `0x92840`, DivineDestiny `0x929a0`,
/// FakeHappyFlower `0x93364`, FencingManual `0x938ac`, HappyFlower `0x94c08`,
/// InfusedCore `0x94f6c`, Lantern `0x95afc`, LetterOpener `0x963b8`,
/// OrangeDough `0x97b88`, PaelsEye `0x980e4`, PaelsFlesh `0x98384`,
/// PaelsLegion `0x9875c`, PaelsTears `0x98898`, PhylacteryUnbound `0x99628`,
/// Pocketwatch `0x99824`, RunicCapacitor `0x9aa9c`, Sai `0x9ab20`, SealOfGold
/// `0x9b188`, StoneCalendar `0x9be34`, SymbioticVirus `0x9c3fc`, VeryHotCocoa
/// `0x9db18`.
///
/// The first twenty-four are [`AFTER_SIDE_TURN_START_EARLY`],
/// [`AFTER_SIDE_TURN_START_TEMPLATES`] and [`AFTER_SIDE_TURN_START_LATE`]. The
/// last five write nothing at this hook that the engine reads at its place
/// ([`after_side_turn_start_body`] carries each one's IL).
const AFTER_SIDE_TURN_START_RELIC_DISPATCH: [RelicId; 29] = [
    RelicId::RelicLantern,
    RelicId::RelicPaelsEye,
    RelicId::RelicBoomingConch,
    RelicId::RelicCandelabra,
    RelicId::RelicChandelier,
    RelicId::RelicHappyFlower,
    RelicId::RelicFakeHappyFlower,
    RelicId::RelicBigHat,
    RelicId::RelicCrossbow,
    RelicId::RelicOrangeDough,
    RelicId::RelicDiamondDiadem,
    RelicId::RelicDivineDestiny,
    RelicId::RelicSai,
    RelicId::RelicVeryHotCocoa,
    RelicId::RelicPhylacteryUnbound,
    RelicId::RelicPaelsLegion,
    RelicId::RelicBread,
    RelicId::RelicPaelsTears,
    RelicId::RelicFencingManual,
    RelicId::RelicRunicCapacitor,
    RelicId::RelicInfusedCore,
    RelicId::RelicSymbioticVirus,
    RelicId::RelicBrimstone,
    RelicId::RelicSealOfGold,
    RelicId::RelicAkabeko,
    RelicId::RelicLetterOpener,
    RelicId::RelicPaelsFlesh,
    RelicId::RelicPocketwatch,
    RelicId::RelicStoneCalendar,
];

/// The order this engine runs the `AfterSideTurnStart` relic bodies in.
///
/// A vouched inventory: [`relic_walk_order`] over the whole table, which is
/// the recorded `Player.Relics` order. An unvouched one: the early group,
/// then the template listeners in the order the inventory lists them (the
/// order [`super::fire_hook`] ran them in), then the late group. That is the
/// sequence this engine ran before #3635, and the pairs whose order shows
/// there are refused by name in `engine::admission` and `entry::opening`.
fn after_side_turn_start_order(catalog: &Catalog) -> impl Iterator<Item = RelicId> + Clone + '_ {
    let hooks = catalog.hooks();
    let vouched = hooks.dispatch_ordered();
    let recorded = vouched
        .then(|| relic_walk_order(catalog, &AFTER_SIDE_TURN_START_RELIC_DISPATCH))
        .into_iter()
        .flatten();
    let fixed = (!vouched)
        .then(|| {
            relic_walk_order(catalog, &AFTER_SIDE_TURN_START_EARLY)
                .chain(
                    hooks
                        .relics()
                        .iter()
                        .copied()
                        .filter(|relic| AFTER_SIDE_TURN_START_TEMPLATES.contains(relic)),
                )
                .chain(relic_walk_order(catalog, &AFTER_SIDE_TURN_START_LATE))
        })
        .into_iter()
        .flatten();
    recorded.chain(fixed)
}

/// The `AfterSideTurnStart` template fire point. The walk
/// ([`after_side_turn_start`]) fires the hook's four template relics at their
/// own places, so this only records the fire point. Any other compiled
/// subscriber refuses by name rather than run out of place.
pub(crate) fn after_side_turn_start_template_pass(catalog: &Catalog) -> Result<(), EngineRefusal> {
    let event = HookEvent::AfterSideTurnStart;
    crate::coverage::record_hook(event);
    let hooks = catalog.hooks();
    if hooks.has(event)
        && hooks
            .subscribers(event)
            .iter()
            .any(|subscriber| match subscriber.subject {
                HookSubject::Relic(relic) => !AFTER_SIDE_TURN_START_TEMPLATES.contains(&relic),
                HookSubject::Power(_) => true,
            })
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "AfterSideTurnStart template subscriber outside the relic walk",
        ));
    }
    Ok(())
}

/// One of the hook's compiled template rules, fired at the relic's place in
/// the walk.
///
/// The four bodies gate the owner's side, three of them `TurnNumber <= 1`,
/// and then await one or two commands (v0.111.0):
///
/// * `Sai/<AfterSideTurnStart>d__4::MoveNext` (RVA `0x3300c4`):
///   `CreatureCmd::GainBlock(Owner.Creature, DynamicVars.Block, ..)` at
///   `IL_0041`-`IL_0058`, every turn;
/// * `DiamondDiadem/<AfterSideTurnStart>d__6` (RVA `0x322c50`):
///   `TurnNumber <= 1` (`IL_004f`-`IL_0055`), `GainBlock` (`IL_007a`), then
///   `Apply<BlurPower>` (`IL_00f7`);
/// * `DivineDestiny/<AfterSideTurnStart>d__4` (RVA `0x3231b4`):
///   `TurnNumber <= 1` (`IL_0045`-`IL_004b`), `PlayerCmd::GainStars`
///   (`IL_0068`);
/// * `VeryHotCocoa/<AfterSideTurnStart>d__6` (RVA `0x333cc8`):
///   `TurnNumber <= 1` (`IL_0048`-`IL_004e`), `PlayerCmd::GainEnergy`
///   (`IL_0071`).
///
/// Each of those commands returns at once on an ending combat:
/// `CreatureCmd/<GainBlock>d__18` (RVA `0x3eaec0`) `IsOverOrEnding` at
/// `IL_0032`, `PowerCmd/<Apply>d__2` (RVA `0x3efbac`) `IsEnding` at `IL_003e`,
/// `PlayerCmd/<GainStars>d__6` (RVA `0x3eec94`) `IsEnding` at `IL_001e`,
/// `PlayerCmd/<GainEnergy>d__3` (RVA `0x3ee8a0`) `IsEnding` at `IL_0035`. So a
/// template reached after an earlier listener ended the combat writes
/// nothing, which is the `over` test here.
fn template_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    relic: RelicId,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let event = HookEvent::AfterSideTurnStart;
    let subscriber = catalog
        .hooks()
        .subscribers(event)
        .iter()
        .find(|subscriber| subscriber.subject == HookSubject::Relic(relic))
        .ok_or(EngineRefusal::HookNotModeled { relic, event })?;
    fire_template_subscriber(catalog, subscriber, relic, event, state, events)
}

/// Whether an `AfterSideTurnStart` relic body writes combat state this turn,
/// by its own entry gates (the bodies of [`after_side_turn_start_body`]).
fn after_side_turn_start_relic_acts(catalog: &Catalog, state: &HotState, relic: RelicId) -> bool {
    let owns = catalog.hooks().owns(relic);
    match relic {
        RelicId::RelicLantern
        | RelicId::RelicBigHat
        | RelicId::RelicOrangeDough
        | RelicId::RelicDiamondDiadem
        | RelicId::RelicDivineDestiny
        | RelicId::RelicVeryHotCocoa
        | RelicId::RelicFencingManual
        | RelicId::RelicRunicCapacitor
        | RelicId::RelicInfusedCore
        | RelicId::RelicSymbioticVirus => owns && state.turn <= 1,
        // The body reads the opening's room-type carrier, not the hook table.
        RelicId::RelicBoomingConch => state.turn <= 1 && state.booming_conch_elite(),
        RelicId::RelicBread => owns && state.turn == 1,
        RelicId::RelicCandelabra => owns && state.turn == 2,
        RelicId::RelicChandelier => owns && state.turn == 3,
        RelicId::RelicPaelsTears => owns && state.paels_tears_had_leftover_energy(),
        RelicId::RelicSealOfGold => owns && state.gold >= 3,
        RelicId::RelicAkabeko
        | RelicId::RelicLetterOpener
        | RelicId::RelicPaelsFlesh
        | RelicId::RelicPocketwatch
        | RelicId::RelicStoneCalendar => false,
        _ => owns,
    }
}

/// Booming Conch's body: one Energy on the first side turn of an elite room.
///
/// `BoomingConch::AfterSideTurnStart` (v0.111.0 stub RVA `0x91410`) ends in
/// `PlayerCmd::GainEnergy`, like Lantern's; the room-type test is the
/// opening's carrier (`entry::opening::booming_conch_elite`).
fn booming_conch_after_side_turn_start(state: &mut HotState) -> Result<(), EngineRefusal> {
    if state.turn <= 1 && state.booming_conch_elite() && !state.history.over {
        if !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Booming Conch energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicBoomingConch);
    }
    Ok(())
}

/// One relic's `AfterSideTurnStart` body, at whichever place the walk
/// reaches it. Each body carries its own IL and entry gates, including what
/// it does on a combat an earlier listener ended.
fn after_side_turn_start_body(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
    relic: RelicId,
) -> Result<(), EngineRefusal> {
    match relic {
        // `RELIC.LANTERN`: one Energy on the owner's first side turn.
        //
        // `Lantern/<AfterSideTurnStart>d__6::MoveNext` RVA `0x328268`:
        // `Contains<Creature>(participants, Owner.Creature)` at `IL_0020`-`IL_0038`,
        // the turn guard `get_TurnNumber; ldc.i4.1; ble.s` at `IL_0043`-`IL_004e`
        // whose fall-through `leave`s at `IL_0050`, `RelicModel::Flash` at
        // `IL_0056` (presentation), and `PlayerCmd::GainEnergy` at `IL_0071` with
        // `DynamicVars.Energy.BaseValue` (`IL_0061`-`IL_0066`) and `Owner`
        // (`IL_006c`). `Lantern::get_CanonicalVars` (`0x95adf`) is
        // `ldc.i4.1; newobj EnergyVar::.ctor`, so the amount is 1.
        //
        // Additive, never exclusive: it is one row beside Booming Conch,
        // Candelabra / Chandelier, the flowers, Venerable Tea Set's
        // `AfterEnergyReset` grant and Very Hot Cocoa's compiled template rule,
        // because `PlayerCmd/<GainEnergy>d__3::MoveNext` (`0x3ee8a0`) is a
        // per-listener command. The `NoEnergyGainPower::ModifyEnergyGain`
        // (`0xa4f9d`) suppression and the ending gate (`IsEnding` at
        // `IL_0035`) are that command's, so they are spelled here exactly as
        // the energy peers below spell them.
        RelicId::RelicLantern => {
            if state.turn <= 1 && !state.history.over {
                if !state.fanouts.no_energy_gain() {
                    state.energy = state
                        .energy
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("Lantern energy"))?;
                }
                crate::coverage::record_relic(RelicId::RelicLantern);
            }
        }
        RelicId::RelicPaelsEye => {
            state
                .fanouts
                .set_paels_eye_was_owner_part_last_player_turn(true);
            crate::coverage::record_relic(RelicId::RelicPaelsEye);
        }
        RelicId::RelicBoomingConch => booming_conch_after_side_turn_start(state)?,
        RelicId::RelicCandelabra | RelicId::RelicChandelier => {
            let (gated, amount) = if relic == RelicId::RelicCandelabra {
                (RelicId::RelicCandelabra, 2_i16)
            } else {
                (RelicId::RelicChandelier, 3_i16)
            };
            // Candelabra on turn two, Chandelier on turn three: each one's
            // Energy var is also its turn.
            if state.turn == amount && !state.history.over {
                state.energy = state
                    .energy
                    .checked_add(amount)
                    .ok_or(EngineRefusal::CounterOverflow("turn-gated relic energy"))?;
                crate::coverage::record_relic(gated);
            }
        }
        RelicId::RelicHappyFlower | RelicId::RelicFakeHappyFlower => {
            for (flower, current, period, amount) in [
                (RelicId::RelicHappyFlower, state.flower(), 3_i16, 1_i16),
                (
                    RelicId::RelicFakeHappyFlower,
                    state.fake_flower(),
                    5_i16,
                    1_i16,
                ),
            ] {
                if flower != relic {
                    continue;
                }
                let next = (current + 1) % period;
                let written = match flower {
                    RelicId::RelicHappyFlower => state.set_flower(next),
                    _ => state.set_fake_flower(next),
                };
                debug_assert!(written);
                if next == 0 && !state.history.over && !state.fanouts.no_energy_gain() {
                    state.energy = state
                        .energy
                        .checked_add(amount)
                        .ok_or(EngineRefusal::CounterOverflow("flower energy"))?;
                }
                crate::coverage::record_relic(flower);
            }
        }
        RelicId::RelicBigHat => {
            if state.turn <= 1 {
                // The fully-unlocked solo-Ironclad Ethereal generation pool is
                // empty, so native returns before consuming Generation RNG
                // (`BigHat/<AfterSideTurnStart>d__6::MoveNext` `0x31f64c`
                // IL_00af-IL_00b8), before `Flash` (IL_00be) and before the
                // `CombatCardGeneration` getter (IL_00ea): the body writes
                // nothing, so its place in the walk cannot show. Another
                // owner's Ethereal pool is not empty, and this body draws
                // nothing, so it refuses rather than open that fight without
                // its two cards (#3264: the owner-pool draw is read from IL,
                // but its oracle digest is not yet reproduced).
                if state.reward_card_pool != Some(crate::catalog::RewardPool::Ironclad) {
                    return Err(EngineRefusal::MalformedArgs(
                        "Big Hat owner Ethereal pool beyond Ironclad (#3264)",
                    ));
                }
                crate::coverage::record_relic(RelicId::RelicBigHat);
            }
        }
        RelicId::RelicCrossbow => crossbow_after_side_turn_start(catalog, state, events)?,
        RelicId::RelicOrangeDough => {
            if state.turn <= 1 {
                orange_dough_after_side_turn_start(catalog, state, events)?;
            }
        }
        RelicId::RelicDiamondDiadem
        | RelicId::RelicDivineDestiny
        | RelicId::RelicSai
        | RelicId::RelicVeryHotCocoa => {
            template_after_side_turn_start(catalog, state, events, relic)?;
        }
        // `PhylacteryUnbound/<AfterSideTurnStart>d__11::MoveNext` (v0.111.0 RVA
        // `0x32e134`): owner-participates test at `IL_002e`, then
        // `OstyCmd::Summon(Owner, DynamicVars["StartOfTurn"] = 2)` at `IL_005b`,
        // with no turn test, so turn one summons too. The opening reaches it
        // through `engine::deal_opening_hand` (#2827). The body reads no combat
        // state between the owner test and `Summon` (IL_003a-IL_005b), and
        // `<Summon>d__0` (`0x3ee040`) gates only its SFX on `IsInProgress`
        // (IL_00c6-IL_00d0), so a summon after an earlier turn-start peer ended
        // the combat runs under IsEnding: `summon_osty`'s projection (#3279).
        RelicId::RelicPhylacteryUnbound => {
            super::summon_osty(state, 2, "Phylactery Unbound Osty")?;
            crate::coverage::record_relic(RelicId::RelicPhylacteryUnbound);
        }
        RelicId::RelicPaelsLegion => {
            let cooldown = state.fanouts.paels_legion_cooldown();
            if cooldown > 0 {
                let written = state.fanouts.set_paels_legion_cooldown(cooldown - 1);
                debug_assert!(written);
                if cooldown == 1 {
                    state.fanouts.set_paels_legion_triggered_last_turn(false);
                }
            }
            crate::coverage::record_relic(RelicId::RelicPaelsLegion);
        }
        // `PlayerCmd::LoseEnergy` (RVA `0x13329b`) returns on `IsEnding`
        // (`IL_0019`), so an ended combat skips the loss.
        RelicId::RelicBread => {
            if state.turn == 1 && !state.history.over {
                state.energy = state.energy.saturating_sub(2).max(0);
                crate::coverage::record_relic(RelicId::RelicBread);
            }
        }
        RelicId::RelicPaelsTears => {
            if state.paels_tears_had_leftover_energy() && !state.history.over {
                if !state.fanouts.no_energy_gain() {
                    state.energy = state
                        .energy
                        .checked_add(2)
                        .ok_or(EngineRefusal::CounterOverflow("Pael's Tears energy"))?;
                }
                crate::coverage::record_relic(RelicId::RelicPaelsTears);
            }
        }
        RelicId::RelicFencingManual => {
            fencing_manual_after_side_turn_start(catalog, state, events)?
        }
        RelicId::RelicRunicCapacitor => runic_capacitor_after_side_turn_start(catalog, state)?,
        RelicId::RelicInfusedCore => infused_core_after_side_turn_start(catalog, state, events)?,
        RelicId::RelicSymbioticVirus => {
            symbiotic_virus_after_side_turn_start(catalog, state, events)?;
        }
        // `PowerCmd/<Apply>d__2::MoveNext` (RVA `0x3efbac`) returns on
        // `IsEnding` (`IL_003e`), so an ended combat applies nothing.
        RelicId::RelicBrimstone => {
            if !state.history.over {
                brimstone_after_side_turn_start(state, events)?;
            }
        }
        RelicId::RelicSealOfGold => {
            if state.gold >= 3 {
                if !state.history.over && !state.fanouts.no_energy_gain() {
                    state.energy = state
                        .energy
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("Seal of Gold energy"))?;
                }
                state.gold = state
                    .gold
                    .checked_sub(3)
                    .ok_or(EngineRefusal::CounterOverflow("Seal of Gold gold"))?;
                crate::coverage::record_relic(RelicId::RelicSealOfGold);
            }
        }
        // The five listeners that write nothing at this place:
        //
        // * `Akabeko/<AfterSideTurnStart>d__6::MoveNext` (RVA `0x31e418`):
        //   `TurnNumber <= 1` (`IL_003d`-`IL_004e`), then
        //   `Apply<VigorPower>` on the owner (`IL_005b`-`IL_0092`). The
        //   opening seeds that Vigor into its document
        //   (`entry::opening::pre_hook_document`), ahead of the whole walk,
        //   so this place writes nothing (#3891 tracks running it here).
        //   Nothing on this hook reads Vigor.
        // * `LetterOpener::AfterSideTurnStart` (RVA `0x963b8`, synchronous):
        //   on any turn but the first (`IL_0025`-`IL_0036`) it zeroes its own
        //   `SkillsPlayedThisTurn` (`IL_003e`-`IL_0040`) and its display
        //   status. The engine's counter is the per-turn Skill history the
        //   turn start already reset (`turn::letter_opener_count_is_shared`),
        //   and no listener of this hook plays a Skill.
        // * `PaelsFlesh::AfterSideTurnStart` (RVA `0x98384`, synchronous):
        //   from turn three (`IL_0025`-`IL_0036`) it sets its own display
        //   `Status` (`IL_004d`-`IL_004f`) and flashes. Its Energy is
        //   `modifier_total`'s `turn >= 3` row.
        // * `Pocketwatch::AfterSideTurnStart` (RVA `0x99824`, synchronous):
        //   `RefreshCounter` (`0x9984a`), a display status from its own
        //   counters. The counters move on `BeforeSideTurnStart` (`0x997f1`).
        // * `StoneCalendar::AfterSideTurnStart` (RVA `0x9be34`, synchronous):
        //   `TurnNumber == DamageTurn` (`IL_0025`-`IL_004a`), then its
        //   display status (`IL_004c`-`IL_0054`).
        RelicId::RelicAkabeko
        | RelicId::RelicLetterOpener
        | RelicId::RelicPaelsFlesh
        | RelicId::RelicPocketwatch
        | RelicId::RelicStoneCalendar => {}
        _ => {}
    }
    Ok(())
}

/// The owner's `AfterSideTurnStart` relics (#3635): one walk in recorded
/// order, behind the owner's power listeners and ahead of the enemies'.
///
/// `Hook/<AfterSideTurnStart>d__75::MoveNext` (v0.111.0 RVA `0x3d15ac`) takes
/// the listener list once (`IterateCombatHookListeners` at `IL_0024`) and
/// awaits each `AbstractModel::AfterSideTurnStart` in turn
/// (`IL_0065`-`IL_0092`). Its second pass, `AfterSideTurnStartLate`
/// (`IL_0109`, `IL_014b`), has no relic listener (only `SandpitPower`
/// overrides it). The relic segment of the first list is
/// [`after_side_turn_start_order`]: `Player.Relics` order on a vouched
/// inventory, where a compiled template and a hand-written body interleave as
/// recorded.
///
/// * `hook_started` false: the pass was entered on an ended combat and runs no
///   listener (`<IterateCombatHookListeners>d__0` `IL_0028`-`IL_0042`).
/// * A listener that ends the combat does not stop the walk, whether it is
///   an earlier relic or one of the owner's powers ahead of the relics (the
///   pass began on a live combat either way). Native checks the combat once,
///   before the first listener (`Hook/<IterateCombatHookListeners>d__0`
///   RVA `0x3d3bc0`, `IL_0028`-`IL_0042`), and yields every listener with no
///   further check (`IL_0064`-`IL_0095`). So each later body still runs, and
///   each says what it does on an ended combat: it ends in a command that
///   returns there, writes only its own counter, or (Crossbow, Orange Dough)
///   still draws the Generation stream and adds no card.
/// * No body of this hook parks a choice of its own. One whose listener
///   parked a choice (a generated-card or damage listener) cannot be resumed
///   inside the body. With a later relic still to act that refuses by name;
///   with none it returns to the caller parked, as before.
pub(crate) fn after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    hook_started: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !hook_started {
        return Ok(());
    }
    let hooks = catalog.hooks();
    // A `SetupPlayerTurn` choice can already be parked: the side-start tail
    // runs at that pause (`turn::run_side_start_at_setup_pause`, #3050).
    let entered_parked = state.pending.is_some();
    let mut order = after_side_turn_start_order(catalog);
    while let Some(relic) = order.next() {
        after_side_turn_start_body(catalog, state, events, relic)?;
        if !entered_parked && state.pending.is_some() {
            if order
                .clone()
                .any(|later| after_side_turn_start_relic_acts(catalog, state, later))
            {
                return Err(EngineRefusal::PowerOrderNotModeled(
                    "AfterSideTurnStart relic behind a listener choice parked inside an earlier relic body",
                ));
            }
            return Ok(());
        }
    }
    // Admission ties the room-type carrier to the relic; a state that sets it
    // without the relic keeps the body.
    if !hooks.owns(RelicId::RelicBoomingConch) {
        booming_conch_after_side_turn_start(state)?;
    }
    Ok(())
}

/// Orange Dough's turn-one body: two cards from the shared Colorless pool
/// into the Hand ([`colorless_generation_relic_pool`] carries the IL). On an
/// ended combat it draws and adds nothing, as Crossbow does
/// ([`crossbow_after_side_turn_start`], "On an ended combat").
fn orange_dough_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let pool =
        colorless_generation_relic_pool(catalog, state, "Orange Dough Colorless pool provenance")?;
    let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
    if state.history.over {
        crate::coverage::record_relic(RelicId::RelicOrangeDough);
        return Ok(());
    }
    for id in shuffled.into_iter().take(2) {
        super::cards::inject_generated_bottom(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            },
            1,
            PileId::Hand,
            events,
        )?;
    }
    crate::coverage::record_relic(RelicId::RelicOrangeDough);
    Ok(())
}

/// The pool Orange Dough and Toolbox draw: the shared Colorless pool under
/// the owner's recorded unlock profile, NOT the owner's character pool
/// (#3325, audited after the Choices Paradox owner-pool defect, #3321).
///
/// v0.111.0 `sts2.dll` (SHA-256 `9cb4f1ad…12b4`, re-hashed from the live
/// install for #3325):
///
/// * `OrangeDough/<AfterSideTurnStart>d__4::MoveNext` RVA `0x32bd7c`:
///   participant test IL_0020-IL_0036, `TurnNumber <= 1` IL_003d-IL_004e,
///   then `ModelDb::CardPool<ColorlessCardPool>` (MethodSpec `0x2b0007c2`,
///   the static `RVA 0x80e1b`, generic argument TypeDef `ColorlessCardPool`)
///   at IL_0061, `Owner.UnlockState` IL_0066-IL_006c and
///   `RunState.CardMultiplayerConstraint` IL_0071-IL_007c into
///   `CardPoolModel::GetUnlockedCards` IL_0081, `DynamicVars.Cards` (2)
///   IL_0086-IL_0091 and `CombatCardGeneration` IL_0096-IL_00a6 into
///   `CardFactory::GetDistinctForCombat` IL_00ab, then one
///   `AddGeneratedCardsToCombat(.., Hand, ..)` IL_00c5.
/// * `Toolbox/<BeforeHandDraw>d__4::MoveNext` RVA `0x332adc`: `player ==
///   Owner` IL_0027-IL_0033, `TurnNumber == 1` IL_003a-IL_004b, the same
///   `CardPool<ColorlessCardPool>` IL_005e, the hook's `player.UnlockState`
///   IL_0063-IL_0069 (the owner, by the IL_0033 test) and
///   `CardMultiplayerConstraint` IL_006e-IL_0079 into `GetUnlockedCards`
///   IL_007e, `Cards` (3) IL_0083-IL_008e, `GetDistinctForCombat` IL_00a8,
///   `FromChooseACardScreen` IL_00c1, then `AddGeneratedCardToCombat(..,
///   Hand, ..)` IL_012f for a non-null pick.
///
/// Neither reads `Owner.Character.CardPool`: the pool is character-blind, so
/// the frozen `GENERATION_RELIC_POOLS` rows were never an Ironclad-only
/// table. What both DO read is the owner's `UnlockState`
/// (`ColorlessCardPool::FilterThroughEpochs` RVA `0xf13f4`, the five
/// `COLORLESS<n>_EPOCH` tests) and the multiplayer constraint, which the
/// frozen rows ignored. So this is `Catalog::colorless_generation_pool`, the
/// Quasar/Bundle of Joy draw (`steps::neutral::colorless_generation_pool`),
/// which equals both frozen rows at the fully-unlocked profile
/// (`colorless_generation_relic_pool_is_the_frozen_row_when_fully_unlocked`).
/// It refuses by `refusal` when the profile is unrecorded or the run is a
/// party run (whose `GetUnlockedCards` keeps the `MultiplayerOnly` rows).
fn colorless_generation_relic_pool(
    catalog: &Catalog,
    state: &HotState,
    refusal: &'static str,
) -> Result<Vec<CardId>, EngineRefusal> {
    if state.multiplayer_ally_key != 0 || !super::cards::unlock_profile_is_recorded(state, catalog)
    {
        return Err(EngineRefusal::MalformedArgs(refusal));
    }
    Ok(catalog.colorless_generation_pool())
}

/// Crossbow's every-turn body: one card from the OWNER's unlocked Attack pool,
/// free this turn (#2970).
///
/// `Crossbow/<AfterSideTurnStart>d__2::MoveNext` (v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad…12b4`, RVA `0x322340`): the owner-participates test at
/// IL_0020-IL_0036 and no turn test; the owner pool
/// ([`crate::steps::neutral::crossbow_owner_attack_pool`] carries
/// IL_003d-IL_008c); an empty pre-filter list returns at IL_0097-IL_009f,
/// which no owner reaches (every character pool holds Attacks under every
/// profile); `Flash` at IL_00a5; `GetDistinctForCombat(Owner, list, 1,
/// CombatCardGeneration)` at IL_00aa-IL_00c7; `SetToFreeThisTurn` on each
/// result at IL_00e5-IL_00e7; then `CardPileCmd::AddGeneratedCardsToCombat
/// (cards, Hand = 2, Owner, 1)` at IL_0109-IL_0112.
///
/// Before #2970 this drew only the frozen `INFERNAL_BLADE_ATTACK_POOL_V109`
/// and refused every other owner or profile. Native reads `Owner.UnlockState`
/// and nothing about Entropy, so the gate is the shared owner-pool provenance
/// that Distraction and White Noise (the same one-type projection) use: a
/// `reward_card_pool` owner, a recorded profile, solo, and a live Generation
/// stream. The draw is that owner's pool under that profile, which is the
/// frozen table for a fully-unlocked Ironclad
/// (`crossbow_owner_pools_reproduce_the_frozen_table`).
///
/// # On an ended combat
///
/// A listener earlier in the pass can end the combat, and the game still
/// runs this body. `CardFactory::GetDistinctForCombat` (RVA `0x112878`) has
/// no ending gate, so the `CombatCardGeneration` draw happens;
/// `CardPileCmd/<AddGeneratedCardsToCombat>d__6::MoveNext` (RVA `0x3e2f0c`)
/// then leaves on `!IsInProgress` (`IL_003e`) and no card enters a pile. The
/// headless harness shows both: with every enemy at 9 HP and Infused Core
/// recorded first (its two overflow evokes end the fight), the stream ends
/// at the same counter as in a live fight (28 for Crossbow, 35 for Orange
/// Dough) and the Hand holds only the dealt five. That stream outlives the
/// fight, so the draw is kept.
fn crossbow_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let owner = state.reward_card_pool;
    if owner.is_none() || !super::cards::owner_pool_profile_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Crossbow generation provenance",
        ));
    }
    let pool = owner
        .map(|owner| catalog.crossbow_attack_pool(owner))
        .unwrap_or_default();
    let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
    if state.history.over {
        crate::coverage::record_relic(RelicId::RelicCrossbow);
        return Ok(());
    }
    if let Some(&id) = shuffled.first() {
        super::cards::inject_generated_free_this_turn_bottom(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            },
            PileId::Hand,
            events,
        )?;
    }
    crate::coverage::record_relic(RelicId::RelicCrossbow);
    Ok(())
}

/// Brimstone's every-turn body: 2 Strength to the owner, then 1 to each
/// living opponent.
///
/// `Brimstone/<AfterSideTurnStart>d__8::MoveNext` (v0.111.0 `sts2.dll`
/// SHA-256 `9cb4f1ad…12b4`, RVA `0x32098c`): the owner-participates test at
/// IL_0027-IL_003f, `Apply<StrengthPower>` of `DynamicVars["SelfStrength"]`
/// on the owner's Creature with the owner as applier at IL_004a-IL_007c,
/// then one `Apply<StrengthPower>` per `GetOpponentsOf(owner)` (IL_00d7-IL_00e8)
/// with a literal null applier at IL_012d-IL_0130, so the enemy rows record
/// `Applier::None`.
fn brimstone_after_side_turn_start(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    super::damage::apply_owner_strength(state, 2, events)?;
    let targets = super::damage::alive_targets(state);
    for target in targets {
        if state.history.over {
            break;
        }
        super::damage::apply_monster_strength_delta_after_type_two_gate(
            state,
            None,
            target,
            1,
            crate::hot::Applier::None,
            events,
        )?;
    }
    crate::coverage::record_relic(RelicId::RelicBrimstone);
    Ok(())
}

/// Fencing Manual's Forge amount.
///
/// `FencingManual::get_CanonicalVars` (v0.111.0 RVA `0x93896`) is
/// `ldc.i4.s 10; newobj ForgeVar::.ctor` at `IL_0001`-`IL_0003`, which the
/// body reads back as `DynamicVars.Forge.BaseValue` (`IL_0056`-`IL_0060`).
const FENCING_MANUAL_FORGE: i64 = 10;

/// Fencing Manual's turn-one `ForgeCmd::Forge(10)` (#3090).
///
/// # The native contract
///
/// `sim/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`, sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`
/// (hash-verified 2026-09-25). The type declares exactly one hook,
/// `AfterSideTurnStart` (RVA `0x938ac`, the async stub), beside
/// `get_Rarity` (`0x93893`), `get_CanonicalVars` (`0x93896`, see
/// [`FENCING_MANUAL_FORGE`]), `get_ExtraHoverTips` (`0x938a4`, a
/// `HoverTipFactory::FromForge` tooltip) and the constructor. It has no
/// fields, no saved property and no counter, so there is nothing for the
/// opening to seed. The body is
/// `FencingManual/<AfterSideTurnStart>d__6::MoveNext` (RVA `0x324a80`):
///
/// * `participants.Contains(Owner.Creature)` at `IL_0020`-`IL_0031`, else
///   `leave` at `IL_0038` (solo, the owner's side is the only one that
///   reaches this walk);
/// * the turn guard `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
///   `IL_003d`-`IL_004e`, else `leave` at `IL_0050` — so `state.turn <= 1` is
///   the literal port;
/// * `ForgeCmd::Forge(DynamicVars.Forge.BaseValue, Owner, this)` at
///   `IL_0055`-`IL_006c`, awaited (`IL_0071`-`IL_00c3`), its
///   `IEnumerable<SovereignBlade>` result popped at `IL_00c3`.
///
/// There is no `RelicModel::Flash`, and nothing else is written. The Forge
/// itself — its ending check, the level-zero Sovereign Blade it generates into
/// Hand when no live one exists, and the include-Exhausted damage growth — is
/// the shared command every Forge card already runs,
/// [`crate::steps::regent_forge::forge_exact`] (`ForgeCmd::Forge` RVA
/// `0x132bd0`, `IncreaseSovereignBladeDamage` `0x132c24`), so its ending gate
/// is that command's and is not repeated here.
///
/// # Position inside the walk
///
/// Native walks the relic `AfterSideTurnStart` listeners in `Player.Relics`
/// order, and so does [`after_side_turn_start`] on a vouched inventory
/// (#3635): the Blade lands on its recorded side of Crossbow's and Orange
/// Dough's cards, in the Hand and in the uid sequence. On an unvouched
/// inventory this sits in the late group, where the frozen oracle ran it.
/// There the order is unobservable against every same-hook peer except the
/// other turn-one Hand writers, Crossbow and Orange Dough, and Infused Core,
/// whose Lightning channel the oracle refused beside it; `engine::admission`
/// (`Fencing Manual + Infused Core turn-start order`, `Fencing Manual +
/// Orange Dough turn-one Forge order`, Crossbow's same-hook list) and
/// `entry::opening`'s `fencing_manual_turn_start_peers_are_ordered` refuse
/// those for an unvouched inventory only.
///
/// The turn guard makes the body reachable only from the opening's first
/// turn-start walk ([`super::deal_opening_hand`]) or a document parked inside
/// turn one's `SetupPlayerTurn`; a document rooted after that walk already
/// carries the Blade, so this cannot fire twice.
fn fencing_manual_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || !catalog.hooks().owns(RelicId::RelicFencingManual) {
        return Ok(());
    }
    crate::steps::regent_forge::forge_exact(state, catalog, FENCING_MANUAL_FORGE, events)?;
    crate::coverage::record_relic(RelicId::RelicFencingManual);
    Ok(())
}

/// Runic Capacitor's slot count.
///
/// `RunicCapacitor::get_CanonicalVars` (v0.111.0 RVA `0x9aa8f`) is
/// `ldc.i4.3; newobj RepeatVar::.ctor` at `IL_0001`-`IL_0002`, which the body
/// reads back as `DynamicVars.Repeat.IntValue` (`IL_0062`-`IL_006c`).
const RUNIC_CAPACITOR_SLOTS: i64 = 3;

/// Runic Capacitor's turn-one `OrbCmd::AddSlots(Owner, 3)` (#3756).
///
/// # The native contract
///
/// `sim/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`, sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`
/// (hash-verified 2026-10-10). The type declares exactly one hook,
/// `AfterSideTurnStart` (RVA `0x9aa9c`, the async stub), beside `get_Rarity`
/// (`0x9aa8c`), `get_CanonicalVars` (`0x9aa8f`, see
/// [`RUNIC_CAPACITOR_SLOTS`]) and the constructor (`0x9aae7`). It has no
/// fields, no saved property and no counter, so the opening has nothing to
/// seed. The body is `RunicCapacitor/<AfterSideTurnStart>d__4::MoveNext` (RVA
/// `0x32ffac`):
///
/// * `participants.Contains(Owner.Creature)` at `IL_0020`-`IL_0036`, else
///   `leave` at `IL_0038` (solo, the owner's side is the only one that reaches
///   this walk);
/// * the turn guard `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
///   `IL_003d`-`IL_004e`, else `leave` at `IL_0050`, so `state.turn <= 1` is
///   the literal port;
/// * `RelicModel::Flash` at `IL_0056` (presentation), then
///   `OrbCmd::AddSlots(Owner, DynamicVars.Repeat.IntValue)` at
///   `IL_005b`-`IL_0071`, awaited.
///
/// Nothing else is written. `OrbCmd::AddSlots` (RVA `0x132f14`) is the shared
/// command [`super::orbs::add_slots`] already ports: the `IsOverOrEnding`
/// return at `IL_000c`-`IL_001d`, `Math.Min(10 - Capacity, amount)` at
/// `IL_001e`-`IL_0037`, and `OrbQueue::AddCapacity` at `IL_0045` (`0x1184f2`,
/// `Capacity += amount`). The rest of the method is the slot animation. It
/// fires no hook and does not read `BaseOrbSlotCount`, so a character with no
/// base slots simply gains three.
///
/// # Position inside the walk
///
/// Native walks the relic `AfterSideTurnStart` listeners in `Player.Relics`
/// order, and so does [`after_side_turn_start`] on a vouched inventory
/// (#3635). On an unvouched one this call sits at a fixed place in the late
/// group. A whole-assembly call scan for `get_Capacity` finds these
/// readers and no others: the slot commands themselves, `OrbQueue`'s own
/// `AddCapacity`, `RemoveCapacity` and `Insert`, the `NOrbManager` layout
/// nodes, one potion's `OnUse`, and `OrbCmd/<Channel>d__3` (`0x3ed69c`) with
/// `OrbQueue/<TryEnqueue>d__13` (`0x3d96fc`). So the only same-hook peer that
/// can observe this body's position is one that **channels** on the same
/// walk, and only where the channel reads the capacity:
///
/// * at zero `BaseOrbSlotCount` with zero `Capacity`, where `Channel`
///   bootstraps one slot first (`IL_0066`-`IL_008c`), so channel-then-add
///   ends at four slots and add-then-channel at three;
/// * with a full queue, where `Channel` evokes the front orb
///   (`IL_0102`-`IL_012c`), which three more slots would have avoided.
///
/// The turn-one channelers of this hook are Symbiotic Virus (one Dark) and
/// Infused Core (three Lightning). On an unvouched inventory `entry::opening`
/// refuses Symbiotic Virus beside this relic at zero base slots
/// (`OpeningRefusal::SymbioticVirusTurnStartOrder`) and Infused Core where
/// its three do not fit, and `engine::admission` refuses the same pairs
/// (`Symbiotic Virus + Runic Capacitor zero-slot order`, `Runic
/// Capacitor + Infused Core turn-start order`). With base slots, the queue
/// entering this hook holds at most Cracked Core's one Lightning, channeled
/// on the earlier `BeforeSideTurnStart` hook, so a single Dark channel never
/// meets a full queue and the two bodies commute.
///
/// Cracked Core itself is on the earlier hook, so its order is fixed in
/// native too: at zero base slots its channel bootstraps one slot and this
/// body then makes four.
///
/// The turn guard makes the body reachable only from the opening's first
/// turn-start walk ([`super::deal_opening_hand`]) or a document parked inside
/// turn one's `SetupPlayerTurn`; a document rooted after that walk already
/// carries the three slots, so this cannot fire twice.
fn runic_capacitor_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || !catalog.hooks().owns(RelicId::RelicRunicCapacitor) {
        return Ok(());
    }
    super::orbs::add_slots(state, RUNIC_CAPACITOR_SLOTS)?;
    crate::coverage::record_relic(RelicId::RelicRunicCapacitor);
    Ok(())
}

/// The number of Lightning orbs `InfusedCore` channels on turn one.
///
/// `InfusedCore::get_CanonicalVars` (v0.111.0 RVA `0x94f14`) builds
/// `DynamicVar("Lightning", Decimal(3))` at `IL_0009`-`IL_0014`, which the
/// body's loop bound reads (`IL_00d3`-`IL_00f3`). Its second var,
/// `"ExtraDamage"` = `Decimal::One` (`IL_001c`-`IL_0026`), is the
/// `ModifyOrbValue` row in `engine::orbs::lightning_orb_value`.
const INFUSED_CORE_CHANNELS: usize = 3;

/// Infused Core's turn-one `Channel<LightningOrb>` x3 (#3756).
///
/// # The native contract
///
/// `sim/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`, sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`. The
/// type declares two hooks and no saved property: `ModifyOrbValue` (RVA
/// `0x94fb7`, +1 to the owner's Lightning orbs, already
/// `engine::orbs::lightning_orb_value`) and `AfterSideTurnStart` (`0x94f6c`,
/// the async stub). The body is
/// `InfusedCore/<AfterSideTurnStart>d__8::MoveNext` (RVA `0x326d00`):
///
/// * `participants.Contains(Owner.Creature)` at `IL_0020`-`IL_0036`, else
///   `leave` at `IL_0038`;
/// * the turn guard `Owner.PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
///   `IL_003d`-`IL_004e`, else `leave` at `IL_0050`;
/// * a loop from `i = 0` (`IL_0055`-`IL_0057`) of
///   `OrbCmd::Channel<LightningOrb>(BlockingPlayerChoiceContext, Owner)` at
///   `IL_005e`-`IL_0069`, awaited, while `i < DynamicVars["Lightning"]
///   .BaseValue` (`IL_00c3`-`IL_00f8`, [`INFUSED_CORE_CHANNELS`]).
///
/// There is no `Flash` and nothing else is written. Each channel is the
/// shared command ([`super::orbs::channel`]): its own `IsOverOrEnding` return,
/// the zero-base bootstrap, the full-queue evoke of the front orb, and
/// Metronome's and Voltaic's counters.
///
/// # Position inside the walk
///
/// Native walks the relic `AfterSideTurnStart` listeners in `Player.Relics`
/// order, and so does [`after_side_turn_start`] on a vouched inventory
/// (#3635). On an unvouched one this body has a fixed place in the late
/// group, after Runic Capacitor and before Symbiotic Virus, and the rest of
/// this section is about that inventory.
/// Three channels into an empty three-slot queue write only the queue
/// (and the channel counters), which no other listener of the hook reads, so
/// with a Defect's base slots and nothing queued the place is not
/// observable. It becomes observable when the queue matters to a peer, or
/// when a channel **evokes**, which happens when the three do not fit: at
/// zero base slots (one bootstrapped slot, two evokes), or behind Cracked
/// Core's Lightning from the earlier `BeforeSideTurnStart` hook (one evoke).
/// An evoke rolls `CombatTargets`, hits an enemy and can kill it. The
/// co-ownerships where that is observable refuse by name on an unvouched
/// inventory, in the opening
/// (`entry::opening::infused_core_turn_start_order_is_recorded`,
/// `symbiotic_virus_turn_start_order_is_recorded`,
/// `fencing_manual_turn_start_peers_are_ordered`) and in `engine::admission`:
///
/// * Symbiotic Virus, at any capacity (which orb overflows);
/// * Runic Capacitor, where the three do not fit (six slots or an evoke);
/// * Fencing Manual, at any capacity (the oracle's refusal, kept);
/// * Bread or Brimstone, where the three do not fit (the evoke against
///   Bread's turn-one loss, which an ended combat skips, and against
///   Brimstone's live-opponent snapshot).
///
/// The turn guard makes the body reachable only from the opening's first
/// turn-start walk ([`super::deal_opening_hand`]) or a document parked inside
/// turn one's `SetupPlayerTurn`; a document rooted after that walk already
/// carries the orbs, so this cannot fire twice.
fn infused_core_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || !catalog.hooks().owns(RelicId::RelicInfusedCore) {
        return Ok(());
    }
    for _ in 0..INFUSED_CORE_CHANNELS {
        super::orbs::channel(state, catalog, crate::hot::OrbKind::Lightning, events)?;
    }
    crate::coverage::record_relic(RelicId::RelicInfusedCore);
    Ok(())
}

/// The number of Dark orbs `SymbioticVirus` channels on turn one.
///
/// `SymbioticVirus::get_CanonicalVars` (v0.111.0 RVA `0x9c3c2`) builds one
/// `DynamicVar("Dark", Decimal::One)` at `IL_0001`-`IL_000b`, which the body's
/// loop bound reads (`IL_00df`-`IL_00f3`).
const SYMBIOTIC_VIRUS_CHANNELS: usize = 1;

/// Symbiotic Virus's turn-one `Channel<DarkOrb>` (#2827).
///
/// `SymbioticVirus/<AfterSideTurnStart>d__7::MoveNext` (v0.111.0 RVA
/// `0x332298`): `participants.Contains(Owner.Creature)` at `IL_0020`-`IL_0036`
/// (else `leave` at `IL_0038`; solo, the owner's side is the only one that
/// reaches this walk), then `PlayerCombatState.TurnNumber; ldc.i4.1; ble.s` at
/// `IL_003d`-`IL_004e` (else `leave` at `IL_0050`), then a loop of
/// `OrbCmd::Channel<DarkOrb>(BlockingPlayerChoiceContext, Owner)` at `IL_0069`
/// while `i < DynamicVars["Dark"].BaseValue` ([`SYMBIOTIC_VIRUS_CHANNELS`],
/// `IL_00c3`-`IL_00f8`). It writes nothing else; the ending gate is the
/// channel command's own (`OrbCmd.Channel` `IsOverOrEnding`, in
/// [`super::orbs::channel`]).
///
/// The oracle is `begin_player_turn`'s `s.symbiotic_virus and s.turn <= 1 and
/// not s.over` block (frozen Python `_turn_start_after_royal`, deleted #2827), one `channel(s, ORB_DARK)`.
/// It sits after Pael's Tears (`_turn_start_after_royal`) and before Brimstone
/// (`_turn_start_after_royal`) among the hand-authored `AfterSideTurnStart` listeners, and so
/// does this call in the late group of an unvouched inventory
/// ([`AFTER_SIDE_TURN_START_LATE`]). A vouched inventory runs it at its
/// recorded place ([`after_side_turn_start`], #3635).
///
/// The same-hook peers whose order around this channel is observable are the
/// oracle's `start_combat` refusals (frozen Python, deleted #2827: Infused Core; Runic
/// Capacitor at zero base slots; Cracked Core at zero base slots with Fencing
/// Manual or Brimstone). On an unvouched inventory `engine::admission`
/// refuses each by name, beside Crossbow's same-hook refusal that also lists
/// this relic, and because the opening builds its document without admission,
/// `entry::opening` refuses the same four itself
/// (`OpeningRefusal::SymbioticVirusTurnStartOrder`).
/// The turn guard makes the body reachable only from the opening's first
/// turn-start walk ([`super::deal_opening_hand`]); a Python-rooted document
/// already carries the Dark orb the oracle's own opening channeled.
fn symbiotic_virus_after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicSymbioticVirus) {
        return Ok(());
    }
    for _ in 0..SYMBIOTIC_VIRUS_CHANNELS {
        super::orbs::channel(state, catalog, crate::hot::OrbKind::Dark, events)?;
    }
    crate::coverage::record_relic(RelicId::RelicSymbioticVirus);
    Ok(())
}

/// Tea of Discourtesy's `DazedCount` (`get_CanonicalVars` `0x9c4bc`,
/// `IL_0039`-`IL_003f`: `DynamicVar("DazedCount", 2)`).
pub(crate) const TEA_OF_DISCOURTESY_DAZED: usize = 2;

/// `RELIC.TEA_OF_DISCOURTESY`'s charged `BeforeCombatStart` (#3756).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…12b4`). The type's one hook is
/// `BeforeCombatStart` (stub `0x9c56c`), body
/// `TeaOfDiscourtesy/<BeforeCombatStart>d__16::MoveNext` (RVA `0x3324dc`):
///
/// * `CombatsLeft; ldc.i4.0; bgt.s`, else `leave` (`IL_001e`-`IL_0026`): a
///   spent Tea writes nothing;
/// * `CardPileCmd::AddToCombatAndPreview<Dazed>(Owner.Creature, PileType 1 =
///   Draw, DynamicVars["DazedCount"].IntValue, Owner, CardPilePosition 3 =
///   Random)` (`IL_002c`-`IL_0053`), awaited;
/// * `set_CombatsLeft(CombatsLeft - 1)` and `Flash` (`IL_00ab`-`IL_00bb`).
///
/// The command is `CardPileCmd/<AddToCombatAndPreview>d__27::MoveNext` (RVA
/// `0x3e334c`): per card, one `CreateCard<T>` (`IL_0094`-`IL_0099`) and one
/// singular `AddGeneratedCardToCombat(card, pileType, creator, position)`
/// (`IL_00c4`-`IL_00d5`), awaited before the next (`IL_014d`-`IL_0165`); what
/// follows is `Cmd::Wait` and `PreviewCardPileAdd`, presentation. The creator
/// is the owner, so each Dazed is the owner-generated transaction
/// [`super::cards::inject_generated_draw_random`] runs (Funerary Mask's row
/// uses it the same way): one index drawn against the growing pile, a fresh
/// uid, the generated-card listeners.
///
/// The saved counter's domain is `0..=1` (constructor `0x9c5af` writes 1; the
/// decrement is the only other writer), which is the boundary's
/// `tea_discourtesy_combats_left` and this state's `tea_of_discourtesy_active`.
///
/// Native runs this at combat start, before turn one's `SetupPlayerTurn`
/// moves the Innate and Imbued cards, so the pile it indexes is the shuffled
/// one; `entry::opening::defers_turn_one_fixup` hands it that pile. Its place
/// among the other `BeforeCombatStart` bodies is not observable: none of them
/// reads the Draw pile, the `Rng` stream or the card uid counter. The
/// co-ownerships that would observe it (a generated-card listener, the
/// turn-one Draw and Hand movers) refuse by name in the opening
/// (`entry::opening::tea_of_discourtesy_combats_left`).
pub(crate) fn tea_of_discourtesy_before_combat_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !catalog.hooks().owns(RelicId::RelicTeaOfDiscourtesy)
        || !state.tea_of_discourtesy_active()
        || state.history.over
    {
        return Ok(());
    }
    for _ in 0..TEA_OF_DISCOURTESY_DAZED {
        super::cards::inject_generated_draw_random(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            },
            events,
        )?;
    }
    state.set_tea_of_discourtesy_active(false);
    crate::coverage::record_relic(RelicId::RelicTeaOfDiscourtesy);
    Ok(())
}

/// The player-relic tail of `AfterShuffle`, after player powers.
pub(crate) fn after_shuffle(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.history.over && catalog.hooks().owns(RelicId::RelicBiiigHug) {
        super::cards::inject_generated_draw_random(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id: CardId::Soot,
                upgrade: 0,
                enchantment: None,
            },
            events,
        )?;
        crate::coverage::record_relic(RelicId::RelicBiiigHug);
    }
    if !state.history.over && catalog.hooks().owns(RelicId::RelicTheAbacus) {
        super::orbs::gain_flat_block(state, catalog, 6, events)?;
        crate::coverage::record_relic(RelicId::RelicTheAbacus);
    }
    Ok(())
}

/// Player-owned discard relics in Python order: Tingsha, then Tough
/// Bandages. Each card gets a separate callback after its history row.
///
/// Both bodies answer only on their owner's side (#3650). v0.111.0 DLL
/// 9cb4f1ad: `Tingsha/<AfterCardDiscarded>d__4::MoveNext` RVA `0x3326e0` and
/// `ToughBandages/<AfterCardDiscarded>d__6::MoveNext` RVA `0x332e2c` test
/// `card.Owner == Owner` (IL_0020-IL_0031), then `Owner.Creature.Side ==
/// Owner.Creature.CombatState.CurrentSide` (IL_0038-IL_005d), and leave at
/// IL_005f when they differ. Tingsha's test precedes its `CombatTargets`
/// roll (IL_006a-IL_0094), so an enemy-side discard consumes no RNG.
/// [`HotState::player_side_active`] is `CurrentSide`.
///
/// No admitted state reaches this on the enemy side. `Hook.AfterCardDiscarded`
/// is fired only by `CardCmd/<DiscardAndDraw>d__4::MoveNext` `0x3e0274`
/// (IL_01a5), whose callers are nine card bodies, Gambling Chip, Tools of
/// the Trade and Gambler's Brew. The only card played on the enemy side is
/// one Hellraiser AutoPlays (`0x33c1a8` IL_0043-IL_004e, `CardTag.Strike`),
/// and no Strike-tagged card discards. The gate is native's own condition,
/// pinned by a unit test.
pub(crate) fn after_card_discarded(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || !state.player_side_active {
        return Ok(());
    }
    if catalog.hooks().owns(RelicId::RelicTingsha) {
        if let Some(target) = super::damage::roll_target(state)? {
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                target,
                DotNetDecimal::from_i64(3),
                false,
                true,
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicTingsha);
    }
    if !state.history.over && catalog.hooks().owns(RelicId::RelicToughBandages) {
        super::orbs::gain_flat_block(state, catalog, 3, events)?;
        crate::coverage::record_relic(RelicId::RelicToughBandages);
    }
    Ok(())
}

/// Bookmark's `AfterFlush`: one `AddUntilPlayed(-1)` row on a random flushed
/// card whose local Energy cost is positive.
///
/// The row lives in the chosen card's slot-7 payload, which the boundary
/// emits only for a card carrying `CARD_FLAG_DEFAULT_PHYSICAL_STATE`
/// (`HotBoundary::card_to_canonical_with_instance`). `flushed` is the caller's
/// snapshot, and on the retaining path the cards are not in Discard yet, so
/// the bit is set here on the snapshot and the caller commits it to the pile
/// ([`super::draw::flush_hand`]). Without it the row was live in the engine
/// and dropped from the canonical root, so a reloaded root charged the full
/// cost (#3180, the #3176 class).
pub(crate) fn after_hand_flushed(
    catalog: &Catalog,
    state: &mut HotState,
    flushed: &mut [HotCard],
) -> Result<(), EngineRefusal> {
    if state.history.over || !catalog.hooks().owns(RelicId::RelicBookmark) {
        return Ok(());
    }
    let candidates = flushed
        .iter()
        .copied()
        .filter(|card| {
            catalog.spec(card.atom).is_some_and(|spec| {
                !spec.x_cost && super::play::resolved_local_energy_cost(state, *card, spec) > 0
            })
        })
        .collect::<Vec<_>>();
    if let Some(chosen) = (!candidates.is_empty()).then(|| {
        let live = state.rng.get(RngStream::Sel);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let index = rng
            .next_bounded(candidates.len() as i32)
            .expect("nonempty Bookmark bound") as usize;
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: rng.words,
                counter: rng.counter,
            },
        );
        candidates[index]
    }) {
        state.card_states.append_local_cost_modifier(
            chosen.uid,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::UntilPlayed,
                reduce_only: false,
            },
        );
        let live = flushed
            .iter_mut()
            .find(|card| card.uid == chosen.uid)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        live.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.exact_piles = true;
    }
    crate::coverage::record_relic(RelicId::RelicBookmark);
    Ok(())
}

/// Charon's Ashes is the first player-relic `AfterCardExhausted` body.
pub(crate) fn after_card_exhausted(
    catalog: &Catalog,
    state: &mut HotState,
    card: crate::hot::HotCard,
    caused_by_ethereal: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    if catalog.hooks().owns(RelicId::RelicCharonsAshes) {
        let targets = super::damage::alive_targets(state);
        for target in targets {
            if state.history.over {
                break;
            }
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                target,
                DotNetDecimal::from_i64(3),
                false,
                true,
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicCharonsAshes);
    }
    if !state.history.over && catalog.hooks().owns(RelicId::RelicForgottenSoul) {
        if let Some(target) = super::damage::roll_target(state)? {
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                target,
                DotNetDecimal::from_i64(1),
                false,
                true,
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicForgottenSoul);
    }
    if !state.history.over
        && catalog.hooks().owns(RelicId::RelicBurningSticks)
        && !state.fanouts.burning_sticks_used()
        && catalog.spec(card.atom).is_some_and(|spec| spec.is_skill)
    {
        super::cards::inject_generated_clones_bottom(
            state,
            catalog,
            card,
            1,
            PileId::Hand,
            events,
        )?;
        state.fanouts.set_burning_sticks_used(true);
        crate::coverage::record_relic(RelicId::RelicBurningSticks);
    }
    if !state.history.over && state.fanouts.joss_paper_cards_exhausted() >= 0 {
        // JossPaper/<AfterCardExhausted>d__25 (v0.111.0, RVA 0x3275a0)
        // tests the command's causedByEthereal flag, not the card keyword.
        // Mittens/Pact exhausting Clumsy must advance the immediate counter.
        if caused_by_ethereal {
            let count = state
                .fanouts
                .joss_paper_ethereal_count()
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Joss Paper ethereal count"))?;
            let written = state.fanouts.set_joss_paper_ethereal_count(count);
            debug_assert!(written);
        } else {
            let total = state
                .fanouts
                .joss_paper_cards_exhausted()
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Joss Paper exhaust count"))?;
            let draws = total / 5;
            let written = state.fanouts.set_joss_paper_cards_exhausted(total);
            debug_assert!(written);
            if draws > 0 {
                // `<DrawIfThresholdMet>d__27` RVA `0x327888`: one awaited
                // Draw of `total / 5` (IL_0058-00eb), receipt-owned when it
                // can park (#3201), then `%= 5` on the live counter
                // (IL_00ec-0109).
                super::draw::joss_paper_draw(state, catalog, draws as usize, events)?;
                let live = state.fanouts.joss_paper_cards_exhausted();
                let written = state.fanouts.set_joss_paper_cards_exhausted(live % 5);
                debug_assert!(written);
            }
        }
        crate::coverage::record_relic(RelicId::RelicJossPaper);
    }
    Ok(())
}

pub(crate) fn after_stars_spent(
    catalog: &Catalog,
    state: &mut HotState,
    amount: i16,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount <= 0 {
        return Ok(());
    }
    if !state.history.over
        && catalog.hooks().owns(RelicId::RelicGalacticDust)
        && catalog.hooks().owns(RelicId::RelicMiniRegent)
        && !state.fanouts.mini_regent_used()
        && i32::from(state.fanouts.galactic_dust()) + i32::from(amount) >= 10
    {
        let juggernaut = state.powers.value(PowerId::Juggernaut);
        if juggernaut > 0
            && state
                .monsters
                .iter()
                .any(|monster| monster.hp > 0 && monster.hp <= (juggernaut - monster.block).max(0))
        {
            return Err(EngineRefusal::MalformedArgs(
                "Galactic Dust and Mini Regent listener order",
            ));
        }
    }
    if catalog.hooks().owns(RelicId::RelicGalacticDust) {
        let total = i32::from(state.fanouts.galactic_dust())
            .checked_add(i32::from(amount))
            .ok_or(EngineRefusal::CounterOverflow("Galactic Dust stars"))?;
        if total >= 10 {
            if !state.history.over {
                super::orbs::gain_flat_block(state, catalog, i64::from((total / 10) * 10), events)?;
            }
            let written = state.fanouts.set_galactic_dust((total % 10) as u8);
            debug_assert!(written);
        } else {
            let written = state.fanouts.set_galactic_dust(total as u8);
            debug_assert!(written);
        }
        crate::coverage::record_relic(RelicId::RelicGalacticDust);
    }
    if catalog.hooks().owns(RelicId::RelicMiniRegent) && !state.fanouts.mini_regent_used() {
        state.fanouts.set_mini_regent_used(true);
        if !state.history.over {
            super::damage::apply_owner_strength(state, 1, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicMiniRegent);
    }
    Ok(())
}

/// Bone Flute follows player-power `AfterAttack` listeners for an Osty
/// command and precedes card/enemy listeners.
///
/// Amount and props, v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…`):
/// `BoneFlute::get_CanonicalVars` (RVA `0x90ece`) is
/// `IL_0001 ldc.i4.2` -> `Decimal::.ctor`, `IL_0007 ldc.i4.4`,
/// `IL_0008 newobj BlockVar::.ctor(block, props)` (`0x06004B74`, RVA
/// `0x1010c4`): `BlockVar(2m, ValueProp.Unpowered)`. The `4` is the
/// `Unpowered` flag (`ValueProp`: Unblockable=2, Unpowered=4, Move=8,
/// SkipHurtAnim=16), not the amount. `BoneFlute::AfterAttack` (RVA
/// `0x90ee4`) `IL_0067`-`IL_0073` passes `DynamicVars.Block` to `GainBlock`
/// once per Osty attack, so the gain is a flat 2 that Dexterity and Frail
/// never touch (#2830, correcting the W206 read that took the props value
/// as the amount).
pub(crate) fn after_pet_attack(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.history.over && catalog.hooks().owns(RelicId::RelicBoneFlute) {
        super::orbs::gain_flat_block(state, catalog, 2, events)?;
        crate::coverage::record_relic(RelicId::RelicBoneFlute);
    }
    Ok(())
}

/// Horn Cleat's owner-turn-two flat block, after the complete block-clear
/// power walk and before template listeners.
pub(crate) fn after_block_cleared(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn == 2 && !state.history.over && catalog.hooks().owns(RelicId::RelicHornCleat) {
        super::orbs::gain_flat_block(state, catalog, 14, events)?;
        crate::coverage::record_relic(RelicId::RelicHornCleat);
    }
    Ok(())
}

/// Capture the VeryEarly zero-block latches before Plating/Regen or ordinary
/// player-power listeners mutate Block.
pub(crate) fn orichalcum_latches(catalog: &Catalog, state: &HotState) -> (bool, bool) {
    let empty = state.block <= 0;
    (
        empty && catalog.hooks().owns(RelicId::RelicOrichalcum),
        empty && catalog.hooks().owns(RelicId::RelicFakeOrichalcum),
    )
}

/// Whether a vouched inventory records every `first` before every `second`
/// (#2909). Relic listeners dispatch in `Player.Relics` order
/// (`CombatState.<IterateHookListeners>d__69` RVA `0x3f9720`), and the entry
/// document carries that order as `relics_entering` only when
/// `relics_entering_dispatch_ordered` vouches for it
/// ([`crate::hooks::HookTable::dispatch_ordered`]). False when either relic is
/// unowned or the inventory is unvouched, so callers keep the fixed order
/// there and admission refuses what that order cannot prove.
pub(crate) fn relic_recorded_before(catalog: &Catalog, first: RelicId, second: RelicId) -> bool {
    let hooks = catalog.hooks();
    if !hooks.owns(first) || !hooks.owns(second) || !hooks.dispatch_ordered() {
        return false;
    }
    let relics = hooks.relics();
    let last_first = relics.iter().rposition(|relic| *relic == first);
    let first_second = relics.iter().position(|relic| *relic == second);
    last_first
        .zip(first_second)
        .is_some_and(|(first, second)| first < second)
}

/// Whether a turn-end block relic runs after the turn-end damage relic
/// (#2909): true only when a vouched inventory records Stone Calendar or
/// Screaming Flagon before it. See [`before_side_turn_end_hand`].
fn turn_end_block_relic_follows_damage(catalog: &Catalog, block_relic: RelicId) -> bool {
    relic_recorded_before(catalog, RelicId::RelicStoneCalendar, block_relic)
        || relic_recorded_before(catalog, RelicId::RelicScreamingFlagon, block_relic)
}

/// The ordinary `BeforeSideTurnEnd` relic group: Orichalcum, Fake
/// Orichalcum, Cloak Clasp, Ripple Basin (block) and Stone Calendar,
/// Screaming Flagon (AoE damage). Every AoE captures its target list once,
/// and every block command completes its synchronous Juggernaut fan-out
/// before the next relic starts. Orichalcum's bodies consume the latches
/// taken in the VeryEarly pass ([`orichalcum_latches`]).
///
/// Listener order (#2909), v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…`):
/// `Hook/<BeforeSideTurnEnd>d__80::MoveNext` (RVA `0x3d34f4`) walks
/// `IterateCombatHookListeners` three times (VeryEarly IL_0058-IL_0145,
/// Early IL_016d-IL_0263, ordinary IL_028b-IL_0381), and in the ordinary
/// pass awaits each listener's `BeforeSideTurnEnd` through
/// `AssignTaskAndWaitForPauseOrCompletion` (IL_02e7-IL_02f6) before
/// `MoveNext` (IL_0377), with no ending check between listeners. Relics come
/// in `Player.Relics` order. The bodies:
///
/// - `Orichalcum/<BeforeSideTurnEnd>d__11::MoveNext` (RVA `0x32bef4`):
///   `ShouldTrigger` at IL_001e, reset at IL_002b, `CreatureCmd::GainBlock`
///   at IL_004f. `FakeOrichalcum/<BeforeSideTurnEnd>d__13` (RVA `0x324608`)
///   is the same shape.
/// - `CloakClasp/<BeforeSideTurnEnd>d__6::MoveNext` (RVA `0x32208c`): Hand
///   count at IL_003d-IL_0055, `GainBlock(count * Block)` after IL_0077.
/// - `RippleBasin/<BeforeSideTurnEnd>d__6::MoveNext` (RVA `0x32fbc0`): no
///   Attack in `CardPlaysFinished` this turn (IL_003d-IL_005f), then a block
///   gain.
/// - `StoneCalendar/<BeforeSideTurnEnd>d__14::MoveNext` (RVA `0x3318b4`):
///   `TurnNumber == DamageTurn` at IL_0053-IL_006d, then
///   `CreatureCmd::Damage(HittableEnemies, Damage)` at IL_007e-IL_00af.
/// - `ScreamingFlagon/<BeforeSideTurnEnd>d__4::MoveNext` (RVA `0x3301e4`):
///   empty Hand at IL_003d-IL_004e, then `Damage(HittableEnemies, Damage)`
///   from IL_005c.
///
/// A block gain fans out to `JuggernautPower/<AfterBlockGained>d__4`
/// (RVA `0x33dab4`), which draws its target with
/// `CombatTargets.NextItem(HittableEnemies)` at IL_004a-IL_007e and deals
/// its Amount at IL_00a2. So a damage relic and a block relic do not commute
/// under Juggernaut: the AoE changes the hittable list the draw sees, and a
/// Juggernaut hit changes what the AoE kills.
///
/// The block relics commute with each other: each Juggernaut hit draws from
/// the same stream for the same Amount, and none of the four reads what
/// another writes. Stone Calendar with Screaming Flagon is refused by
/// admission. So one split is exact: the block relics recorded before the
/// damage relic run first (in this engine's fixed order), then the damage
/// relic, then the block relics recorded after it. An unvouched inventory
/// keeps every block relic first, and admission refuses a damage/block pair
/// there when Juggernaut is reachable
/// (`turn-end relic acquisition order with Juggernaut`).
pub(crate) fn before_side_turn_end_hand(
    catalog: &Catalog,
    state: &mut HotState,
    real_latched: bool,
    fake_latched: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hooks = catalog.hooks();
    let damage_relic_owned =
        hooks.owns(RelicId::RelicStoneCalendar) || hooks.owns(RelicId::RelicScreamingFlagon);
    let follows_damage =
        |relic| damage_relic_owned && turn_end_block_relic_follows_damage(catalog, relic);
    before_side_turn_end_block_relics(
        catalog,
        state,
        real_latched,
        fake_latched,
        |relic| !follows_damage(relic),
        events,
    )?;
    before_side_turn_end_damage_relics(catalog, state, events)?;
    if damage_relic_owned {
        before_side_turn_end_block_relics(
            catalog,
            state,
            real_latched,
            fake_latched,
            follows_damage,
            events,
        )?;
    }
    Ok(())
}

/// The four turn-end block relics in this engine's fixed order, restricted
/// to those `runs` selects (see [`before_side_turn_end_hand`]).
fn before_side_turn_end_block_relics(
    catalog: &Catalog,
    state: &mut HotState,
    real_latched: bool,
    fake_latched: bool,
    runs: impl Fn(RelicId) -> bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    for (latched, relic, amount) in [
        (real_latched, RelicId::RelicOrichalcum, 6_i64),
        (fake_latched, RelicId::RelicFakeOrichalcum, 3_i64),
    ] {
        if latched && !state.history.over && runs(relic) {
            super::orbs::gain_flat_block(state, catalog, amount, events)?;
            crate::coverage::record_relic(relic);
        }
    }
    if catalog.hooks().owns(RelicId::RelicCloakClasp)
        && !state.history.over
        && !state.piles.get(crate::hot::PileId::Hand).is_empty()
        && runs(RelicId::RelicCloakClasp)
    {
        let amount = i64::try_from(state.piles.get(crate::hot::PileId::Hand).len())
            .map_err(|_| EngineRefusal::CounterOverflow("Cloak Clasp hand size"))?;
        super::orbs::gain_flat_block(state, catalog, amount, events)?;
        crate::coverage::record_relic(RelicId::RelicCloakClasp);
    }
    if catalog.hooks().owns(RelicId::RelicRippleBasin)
        && !state.history.over
        && state.history.attack_plays_finished_this_turn == 0
        && runs(RelicId::RelicRippleBasin)
    {
        super::orbs::gain_flat_block(state, catalog, 4, events)?;
        crate::coverage::record_relic(RelicId::RelicRippleBasin);
    }
    Ok(())
}

/// Stone Calendar's turn-seven and Screaming Flagon's empty-Hand AoE (see
/// [`before_side_turn_end_hand`]).
fn before_side_turn_end_damage_relics(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if catalog.hooks().owns(RelicId::RelicStoneCalendar) && !state.history.over && state.turn == 7 {
        let targets = super::damage::alive_targets(state);
        for target in targets {
            if state.history.over {
                break;
            }
            if state
                .monsters
                .get(target)
                .is_some_and(|monster| monster.hp > 0)
            {
                super::damage::damage_monster_with_catalog(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(52),
                    false,
                    true,
                    events,
                )?;
            }
        }
        crate::coverage::record_relic(RelicId::RelicStoneCalendar);
    }
    if catalog.hooks().owns(RelicId::RelicScreamingFlagon)
        && !state.history.over
        && state.piles.get(crate::hot::PileId::Hand).is_empty()
    {
        let targets = super::damage::alive_targets(state);
        for target in targets {
            if state.history.over {
                break;
            }
            if state
                .monsters
                .get(target)
                .is_some_and(|monster| monster.hp > 0)
            {
                super::damage::damage_monster_with_catalog(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(20),
                    false,
                    true,
                    events,
                )?;
            }
        }
        crate::coverage::record_relic(RelicId::RelicScreamingFlagon);
    }
    Ok(())
}

/// Pael's Tears samples energy after ordinary `BeforeSideTurnEnd` relics and
/// before the orb passives/hand flush (`_end_player_turn_after_auto_post`, frozen Python, deleted #2827).
pub(crate) fn after_before_side_turn_end(catalog: &Catalog, state: &mut HotState) {
    if catalog.hooks().owns(RelicId::RelicPaelsTears) {
        state.set_paels_tears_had_leftover_energy(state.energy > 0);
        crate::coverage::record_relic(RelicId::RelicPaelsTears);
    }
}

/// Whether Joss Paper's side-end threshold Draw may park on the receipt
/// carrier (`None`), or the name it refuses under (#3386).
///
/// The Draw is awaited inside Joss Paper's own listener of the
/// deferred-choice `Hook/<AfterSideTurnEnd>d__81` walk (RVA `0x3d11c0`,
/// v0.111.0, SHA-256 `9cb4f1ad…`; see `puzzle::DeferredChoiceListener`), so
/// native starts every LATER ordinary listener before the choice resolves,
/// then `WhenAll`s (IL_016c) before `AfterSideTurnEndLate`. The receipt
/// carrier runs them after the answer instead. The two orders agree exactly
/// when every later listener neither reads nor writes what the choice and
/// its continuation (the rest of the Draw, then `CardsExhausted %= 5`,
/// `JossPaper/<DrawIfThresholdMet>d__27` RVA `0x327888` IL_00ec-0109) read
/// or write, and consumes no RNG.
///
/// `CombatState/<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720`
/// enumerates each ally creature's powers, then (for a player) relics,
/// potions, orbs and pile cards, then the enemies (IL_0059-019b). The
/// complete native override set of `AfterSideTurnEnd` that can act after a
/// relic at the player side end is:
///
/// * relics `ArtOfWar`, `Kusarigama`, `LunarPastry`, `ParryingShield` (no
///   potion, orb or card overrides it);
/// * an ally creature's own powers (a multiplayer ally, `multiplayer_ally_key`);
/// * enemy powers. Every one gates on its owner's side ending
///   (`participants.Contains(Owner)` or `side == 2`) except
///   `OblivionPower/<AfterSideTurnEnd>d__10` RVA `0x33f4a4`, which removes
///   itself at `side == 1` (IL_001e-0029).
///
/// Of those, three commute with a continuation that plays no card:
///
/// * `ArtOfWar::AfterSideTurnEnd` RVA `0x90289` only moves
///   `AnyAttacksPlayedThisTurn` into `AnyAttacksPlayedLastTurn` and clears it
///   (IL_001a-0028); the only other writer is `ArtOfWar::AfterCardPlayed`
///   RVA `0x9021c`.
/// * `Kusarigama::AfterSideTurnEnd` RVA `0x959f9` only zeroes
///   `AttacksPlayedThisTurn` and `Status` (IL_001a-0023), advanced only by
///   `Kusarigama::AfterCardPlayed` RVA `0x95a28`.
/// * Oblivion is read only by `OblivionPower::BeforeCardPlayed` RVA
///   `0xa51c0` and `AfterCardPlayed` RVA `0xa5228`.
///
/// None consumes RNG. The continuation plays a card only through
/// Hellraiser's `AfterCardDrawnEarly` AutoPlay (`draw.rs`; Stratagem's
/// selection moves cards to Hand and plays none), so with Hellraiser absent
/// the three commute and the park stays exact. Lunar Pastry (Stars, which
/// Black Hole answers with damage), Parrying Shield (a Block read, an RNG
/// target roll and monster damage) and an ally's powers are not proven, and
/// refuse. So does a Draw issued while another choice is already parked
/// below it (a Dark Embrace child), which the carrier cannot order.
///
/// "Later" is read from the vouched inventory (#3400): a relic recorded
/// before Joss Paper has already run when the Draw parks
/// ([`after_side_turn_end_relics`]) and is not a later listener. Without a
/// vouched order every owned peer counts as later, as #3386 had it.
pub(crate) fn joss_paper_side_end_draw_wall(
    catalog: &Catalog,
    state: &HotState,
) -> Option<&'static str> {
    let card_play_reachable = state.powers.value(PowerId::Hellraiser) > 0;
    let later = |relic| after_side_turn_end_relic_may_follow_joss_paper(catalog, relic);
    let blocked = state.pending.is_some()
        || state.multiplayer_ally_key != 0
        || later(RelicId::RelicLunarPastry)
        || later(RelicId::RelicParryingShield)
        || card_play_reachable
            && (later(RelicId::RelicArtOfWar)
                || later(RelicId::RelicKusarigama)
                || state
                    .monsters
                    .iter()
                    .any(|monster| monster.hp > 0 && monster.powers.value(PowerId::Oblivion) > 0));
    blocked.then_some(super::puzzle::JOSS_SIDE_END_WALL)
}

/// Whether the `AfterSideTurnEnd` relic listener `relic` can run after Joss
/// Paper's (#3400): owned, and not recorded before Joss Paper by a vouched
/// inventory ([`relic_recorded_before`]). On a vouched inventory a relic
/// recorded first has already run when Joss Paper's Draw parks
/// ([`after_side_turn_end_relics`]), so it is not a later listener of the
/// walk. On an unvouched one its side is unknown, and it counts as later.
pub(crate) fn after_side_turn_end_relic_may_follow_joss_paper(
    catalog: &Catalog,
    relic: RelicId,
) -> bool {
    catalog.hooks().owns(relic) && !relic_recorded_before(catalog, relic, RelicId::RelicJossPaper)
}

/// The hand-written `AfterSideTurnEnd` relic listeners, in the fixed order an
/// unvouched inventory runs them (after the template group, Lunar Pastry).
const AFTER_SIDE_TURN_END_HAND_RELICS: [RelicId; 4] = [
    RelicId::RelicJossPaper,
    RelicId::RelicArtOfWar,
    RelicId::RelicParryingShield,
    RelicId::RelicKusarigama,
];

/// The player side end's `AfterSideTurnEnd` relic listeners (#3400).
///
/// v0.111.0 (DLL `9cb4f1ad…`). The ordinary pass of
/// `Hook/<AfterSideTurnEnd>d__81::MoveNext` (RVA `0x3d11c0`) enumerates
/// `Hook::IterateCombatHookListeners` (IL_0058), calls each listener's
/// `AbstractModel::AfterSideTurnEnd` (IL_00b0) and awaits it through
/// `HookPlayerChoiceContext::AssignTaskAndWaitForPauseOrCompletion`
/// (IL_00bd) before `MoveNext` (IL_013b), with no ending test between
/// listeners. Relics come in `Player.Relics` order
/// (`CombatState/<IterateHookListeners>d__69` RVA `0x3f9720`). The
/// assembly's relic overrides of `AfterSideTurnEnd` are exactly these five
/// (a method-name sweep finds no other `Relics.*::AfterSideTurnEnd`):
///
/// * `LunarPastry/<AfterSideTurnEnd>d__4::MoveNext` RVA `0x32a08c`: owner
///   side test IL_001d-0035, then `PlayerCmd::GainStars(Stars)` awaited at
///   IL_0050. `PlayerCmd/<GainStars>d__6` RVA `0x3eec94` returns once
///   `CombatManager.IsEnding` (IL_0019-0025), so an ended fight gains
///   nothing, which is the template group's own ending rule. It is the one
///   template listener (`stars`).
/// * `JossPaper/<AfterSideTurnEnd>d__26::MoveNext` RVA `0x3276ac`: owner
///   side test IL_001d-0035, `CardsExhausted += EtherealCount;
///   EtherealCount = 0` (IL_003a-004f), then `DrawIfThresholdMet` awaited
///   (IL_0054-005b).
/// * `ArtOfWar::AfterSideTurnEnd` RVA `0x90289`: owner side test
///   IL_0001-0012, then `AnyAttacksPlayedLastTurn = AnyAttacksPlayedThisTurn;
///   AnyAttacksPlayedThisTurn = false` (IL_001a-0028). Synchronous, with no
///   ending test.
/// * `ParryingShield/<AfterSideTurnEnd>d__4::MoveNext` RVA `0x32d7b0`: owner
///   side test IL_0020-0038, a return while `Block < DynamicVars.Block`
///   (IL_003d-0069), then `CombatTargets.NextItem(HittableEnemies)`
///   (IL_006e-0098) and `CreatureCmd::Damage` awaited (IL_00cc).
/// * `Kusarigama::AfterSideTurnEnd` RVA `0x959f9`: owner side test
///   IL_0001-0012, then `AttacksPlayedThisTurn = 0; Status = 0`
///   (IL_001a-0023). Synchronous, with no ending test.
///
/// They do not all commute. Lunar Pastry's Stars feed Black Hole damage,
/// which can end the fight or change Parrying Shield's target pool. Joss
/// Paper's Draw can change Block and consume RNG, and it plays cards
/// through a live Hellraiser, which Art of War and Kusarigama count. So a
/// vouched inventory runs them in its recorded order
/// ([`super::fire_hook_in_inventory_order`]). An unvouched one runs the fixed
/// order (Lunar Pastry, Joss Paper, Art of War, Parrying Shield, Kusarigama)
/// and refuses by name wherever that order is not proven native
/// ([`after_side_turn_end_fixed_order_refusal`]).
pub(crate) fn after_side_turn_end_relics(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hooks = catalog.hooks();
    if hooks.dispatch_ordered() {
        // A hand-written body keys on ownership, so it runs once, at its
        // first inventory entry.
        let mut ran = [false; AFTER_SIDE_TURN_END_HAND_RELICS.len()];
        super::fire_hook_in_inventory_order(
            catalog,
            HookEvent::AfterSideTurnEnd,
            state,
            events,
            |relic, state, events| {
                if let Some(index) = AFTER_SIDE_TURN_END_HAND_RELICS
                    .iter()
                    .position(|hand| *hand == relic)
                    && !std::mem::replace(&mut ran[index], true)
                {
                    after_side_turn_end_hand_relic(catalog, state, relic, events)?;
                }
                Ok(())
            },
        )?;
    } else {
        let joss_pair_refusal = after_side_turn_end_fixed_order_refusal(catalog, state)?;
        super::fire_hook(catalog, HookEvent::AfterSideTurnEnd, state, events)?;
        for relic in AFTER_SIDE_TURN_END_HAND_RELICS {
            after_side_turn_end_hand_relic(catalog, state, relic, events)?;
            // A Joss Paper pair refuses once its Draw has run, so a park
            // inside that Draw still refuses under #3386's own name
            // (`joss_paper_side_end_draw_wall` counts every owned peer as
            // later on an unvouched inventory). Either way the action
            // refuses.
            if relic == RelicId::RelicJossPaper
                && let Some(refusal) = joss_pair_refusal
            {
                return Err(refusal);
            }
        }
    }
    if state.turn <= 1 && hooks.owns(RelicId::RelicRingingTriangle) {
        crate::coverage::record_relic(RelicId::RelicRingingTriangle);
    }
    Ok(())
}

/// The refusal name for an unvouched `AfterSideTurnEnd` relic interleaving
/// that the fixed order cannot prove (#3400). The pre-existing Black Hole
/// horizon shares it.
pub(crate) const AFTER_SIDE_TURN_END_RELIC_ORDER: &str = "AfterSideTurnEnd relic acquisition order";

/// Whether the fixed `AfterSideTurnEnd` relic order is native for this side
/// end on an unvouched inventory (#3400), or the refusal naming the pair.
///
/// Art of War and Kusarigama write only their own counters, which otherwise
/// only their `AfterCardPlayed` bodies touch, so they commute with every
/// peer whose body plays no card. Lunar Pastry and Parrying Shield play
/// none. The pairs whose order is observable, when both act:
///
/// * Joss Paper's Draw (a threshold met) against Lunar Pastry or Parrying
///   Shield. The Draw's `AfterCardDrawn` listeners and a live Hellraiser's
///   AutoPlay can gain Block, spend RNG or end the fight, and Lunar Pastry's
///   Stars can end it (Black Hole) before the Draw.
/// * Joss Paper's Draw under a live Hellraiser against Art of War or
///   Kusarigama. The AutoPlayed Attack is counted into whichever turn that
///   side of the rollover belongs to.
/// * Lunar Pastry and Parrying Shield under Black Hole, where either hit can
///   kill (the pre-#3400 horizon, kept unchanged). The kill ends the fight
///   or changes the other's target pool.
///
/// Every other co-owned pair commutes, and one relic alone has no order.
///
/// The Black Hole pair refuses here, before any listener runs. A Joss Paper
/// pair is returned as `Ok(Some(_))`, for the caller to raise once Joss
/// Paper's own body has run.
fn after_side_turn_end_fixed_order_refusal(
    catalog: &Catalog,
    state: &HotState,
) -> Result<Option<EngineRefusal>, EngineRefusal> {
    if state.history.over {
        return Ok(None);
    }
    let hooks = catalog.hooks();
    let refusal = EngineRefusal::PowerOrderNotModeled(AFTER_SIDE_TURN_END_RELIC_ORDER);
    let black_hole = state.powers.value(PowerId::BlackHole);
    if hooks.owns(RelicId::RelicLunarPastry)
        && hooks.owns(RelicId::RelicParryingShield)
        && black_hole > 0
        && state.block >= 10
        && super::damage::alive_targets(state)
            .into_iter()
            .any(|target| {
                let monster = &state.monsters[target];
                monster.hp <= (black_hole - monster.block).max(0)
                    || monster.hp <= (6 - monster.block).max(0)
            })
    {
        return Err(refusal);
    }
    let joss_draws = hooks.owns(RelicId::RelicJossPaper)
        && state
            .fanouts
            .joss_paper_cards_exhausted()
            .saturating_add(state.fanouts.joss_paper_ethereal_count())
            >= 5;
    let joss_pair = joss_draws
        && (hooks.owns(RelicId::RelicLunarPastry)
            || hooks.owns(RelicId::RelicParryingShield)
            || state.powers.value(PowerId::Hellraiser) > 0
                && (hooks.owns(RelicId::RelicArtOfWar) || hooks.owns(RelicId::RelicKusarigama)));
    Ok(joss_pair.then_some(refusal))
}

/// One hand-written `AfterSideTurnEnd` relic body (see
/// [`after_side_turn_end_relics`] for each one's IL).
fn after_side_turn_end_hand_relic(
    catalog: &Catalog,
    state: &mut HotState,
    relic: RelicId,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let owned = catalog.hooks().owns(relic);
    match relic {
        RelicId::RelicJossPaper => joss_paper_after_side_turn_end(catalog, state, events)?,
        RelicId::RelicArtOfWar if owned => {
            state.set_art_of_war_last_attack(state.art_of_war_current_attack());
            state.set_art_of_war_current_attack(false);
            crate::coverage::record_relic(relic);
        }
        RelicId::RelicParryingShield if owned && !state.history.over && state.block >= 10 => {
            if let Some(target) = super::damage::roll_target(state)? {
                super::damage::damage_monster_with_catalog(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(6),
                    false,
                    true,
                    events,
                )?;
                crate::coverage::record_relic(relic);
            }
        }
        // Kusarigama is the Kunai-family exception: its owner counter resets
        // in AfterSideTurnEnd rather than at the next BeforeSideTurnStart.
        // This remains observable when the enemy phase ends combat before
        // another player turn begins.
        RelicId::RelicKusarigama if owned => {
            let written = state.fanouts.set_kusarigama(0);
            debug_assert!(written);
            crate::coverage::record_relic(relic);
        }
        _ => {}
    }
    Ok(())
}

/// Joss Paper's `AfterSideTurnEnd` body (`<AfterSideTurnEnd>d__26` RVA
/// `0x3276ac`; see [`after_side_turn_end_relics`]).
fn joss_paper_after_side_turn_end(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.fanouts.joss_paper_cards_exhausted() < 0 {
        return Ok(());
    }
    let total = state
        .fanouts
        .joss_paper_cards_exhausted()
        .checked_add(state.fanouts.joss_paper_ethereal_count())
        .ok_or(EngineRefusal::CounterOverflow("Joss Paper side-end count"))?;
    let ethereal_written = state.fanouts.set_joss_paper_ethereal_count(0);
    debug_assert!(ethereal_written);
    let exhausted_written = state.fanouts.set_joss_paper_cards_exhausted(total);
    debug_assert!(exhausted_written);
    let draws = total / 5;
    if draws > 0 && !state.history.over {
        // The same awaited threshold Draw from `<AfterSideTurnEnd>d__26`
        // RVA `0x3276ac` IL_0054-00b2 (#3201). A park is published only
        // where every later ordinary listener commutes with it (#3386).
        // Under a named wall, a select that resolves without a prompt refuses
        // as well (#3485); under `None` it commutes like a parked one.
        let listener = super::puzzle::DeferredChoiceListener::enter(joss_paper_side_end_draw_wall(
            catalog, state,
        ));
        let result = super::draw::joss_paper_draw(state, catalog, draws as usize, events);
        listener.settle(result)?;
        let live = state.fanouts.joss_paper_cards_exhausted();
        let written = state.fanouts.set_joss_paper_cards_exhausted(live % 5);
        debug_assert!(written);
    } else if draws > 0 {
        let written = state.fanouts.set_joss_paper_cards_exhausted(total % 5);
        debug_assert!(written);
    }
    crate::coverage::record_relic(RelicId::RelicJossPaper);
    Ok(())
}

/// Apply the gold branch of a fatal attack reward after the card body has
/// authenticated its exact reward table.
///
/// `PlayerCmd::GainGold` walks `Hook::ModifyGoldGained` over a `System.Decimal`
/// and truncates to `Int32` **once**, after the complete modifier walk
/// (`gain_player_gold`, frozen Python, deleted #2827). Two modifiers are represented and they
/// commute:
///
/// * `Ectoplasm::ModifyGoldGained` sets the amount to zero.
/// * `BowlerHat::ModifyGoldGained` RVA `0x915a9` returns
///   `amount * DynamicVars["GoldIncrease"].BaseValue` for its own owner, and
///   `BowlerHat::get_CanonicalVars` RVA `0x9158d` constructs that var as
///   `Decimal(125, 0, 0, 0, scale 2)` — exactly `1.25`. Its
///   `AfterModifyingGoldGained` RVA `0x915d1` is a presentation-only `Flash`.
///
/// So the exact result is `floor(amount * 5 / 4)` on the non-negative reward
/// amounts this path carries, and the command raises `AfterGoldGained` only
/// for a positive result: `PlayerCmd/<GainGold>d__9::MoveNext` RVA
/// `0x3ee9f0` tests `amount > 0` at IL_00dd-IL_00ed, adds it to `Gold` at
/// IL_01b8-IL_01d0, then awaits `Hook::AfterGoldGained` at IL_01e1. Both
/// history arms (`wasStolenBack` IL_017f) reach the add and the hook. Dragon
/// Fruit is that hook's one listener ([`dragon_fruit_after_gold_gained`]).
///
/// It is the crate's one `GainGold` port, so the opening's Maw Bank room-entry
/// gain (#3328, `entry::opening`'s `maw_bank_after_room_entered`, a positive
/// literal 12) calls it too.
pub(crate) fn gain_fatal_reward_gold(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.fanouts.ectoplasm_owned() {
        crate::coverage::record_relic(RelicId::RelicEctoplasm);
        return Ok(());
    }
    let gained = if catalog.hooks().owns(RelicId::RelicBowlerHat) {
        crate::coverage::record_relic(RelicId::RelicBowlerHat);
        i32::try_from(i64::from(amount) * 5 / 4)
            .map_err(|_| EngineRefusal::CounterOverflow("gold"))?
    } else {
        amount
    };
    if gained <= 0 {
        return Ok(());
    }
    state.gold = state
        .gold
        .checked_add(gained)
        .ok_or(EngineRefusal::CounterOverflow("gold"))?;
    dragon_fruit_after_gold_gained(state, catalog, events)
}

/// Dragon Fruit's `AfterGoldGained`: +1 max HP for its owner (#3320).
///
/// v0.111.0 (DLL `9cb4f1ad…`). `Hook/<AfterGoldGained>d__29::MoveNext` RVA
/// `0x3ce38c` walks `IRunState::IterateHookListeners` (IL_0018-IL_001e) and
/// calls `AbstractModel::AfterGoldGained` on each (IL_0053). The only override
/// in the assembly is `DragonFruit::AfterGoldGained` (RVA `0x92b30`; the
/// method-name sweep finds just it, the `AbstractModel` default `0x7a0a2` and
/// the `Hook` dispatcher), so there is no peer to order against.
///
/// `DragonFruit/<AfterGoldGained>d__5::MoveNext` RVA `0x323638`: `player ==
/// Owner` at IL_001d-IL_0029 (else `leave`, IL_002b), `Flash` (IL_0031,
/// presentation), then `CreatureCmd::GainMaxHp(Owner.Creature,
/// DynamicVars.MaxHp.BaseValue)` at IL_0036-IL_0051, awaited.
/// `DragonFruit::get_CanonicalVars` (`0x92b1e`) builds that var from
/// `ldsfld Decimal::One` — one max HP. No combat or ending guard, so it runs
/// after a lethal Hand of Greed as Feed's reward does
/// ([`super::damage::gain_player_max_hp`]).
///
/// The relic declares no instance field, so there is no per-fight state to
/// seed: the boundary's `dragon_fruit` is an ownership mirror.
pub(crate) fn dragon_fruit_after_gold_gained(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !catalog.hooks().owns(RelicId::RelicDragonFruit) {
        return Ok(());
    }
    super::damage::gain_player_max_hp(state, 1, events)?;
    crate::coverage::record_relic(RelicId::RelicDragonFruit);
    Ok(())
}

/// Intimidating Helmet is the player-relic member of `BeforeCardPlayed`.
///
/// v0.111.0 (DLL 9cb4f1ad) `IntimidatingHelmet/<BeforeCardPlayed>d__6::MoveNext`
/// RVA `0x326e4c`: a foreign card returns (IL_0020-IL_0038); the gate is
/// `cardPlay.Resources.EnergyValue >= DynamicVars.Energy` (IL_003d-IL_0060,
/// Energy = 2 per `get_CanonicalVars` RVA `0x94ff9` IL_0018), then
/// `GainBlock(Owner.Creature, DynamicVars.Block, null, false)` (IL_0067-IL_0085,
/// Block = 4). `energy_value` is that `EnergyValue`, not `EnergySpent`: an
/// AutoPlay pays nothing but carries its resolved cost
/// (`play::card_play_energy_value`, #3124).
pub(crate) fn intimidating_helmet_before_card_played(
    catalog: &Catalog,
    state: &mut HotState,
    energy_value: i16,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if energy_value >= 2
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicIntimidatingHelmet)
    {
        super::orbs::gain_flat_block(state, catalog, 4, events)?;
        crate::coverage::record_relic(RelicId::RelicIntimidatingHelmet);
    }
    Ok(())
}

/// Music Box's `BeforeCardPlayed` for one owner Attack CardPlay (#3640).
///
/// v0.111.0 (DLL 9cb4f1ad) `MusicBox::BeforeCardPlayed` RVA `0x972ac`
/// returns while `CardBeingPlayed` is set (IL_000d-IL_0019), for a foreign
/// card (IL_001b-IL_0032), once `WasUsedThisTurn` (IL_0034-IL_0040) and for
/// a non-Attack (`Type == 1`, IL_0042-IL_004f); otherwise it stores this
/// play's card in `CardBeingPlayed` (IL_0055-IL_005c). It reads nothing
/// else of the `CardPlay`: no `IsAutoPlay` and no `PlayIndex`, so an
/// AutoPlayed Attack latches like a manual one, and a replay body after the
/// clone is stopped by `WasUsedThisTurn`. The caller has tested the Attack
/// type and ownership; this engine has one player, so every card is the
/// owner's.
///
/// `MusicBox/<AfterCardPlayed>d__13::MoveNext` RVA `0x32ae04` clones only
/// when `cardPlay.Card == CardBeingPlayed` (IL_001e-IL_002e), with no type,
/// owner or used test of its own, and sets `WasUsedThisTurn` and clears the
/// latch after the add (IL_00bf, IL_00c6). `CardModel/<OnPlayWrapper>d__339`
/// (`0x31b8d0`) awaits `Hook::BeforeCardPlayed` at IL_05a1, before
/// `CardPlayStarted` (IL_0626) and `OnPlay` (IL_0659), and
/// `Hook::AfterCardPlayed` at IL_0874. So the Attack cloned is the turn's
/// first to START. A nested Attack played inside it (a Hellraiser AutoPlay
/// under a draw in its `OnPlay`, or under an earlier relic's
/// AfterCardPlayed body such as Iron Club's) finds the latch taken and is
/// not cloned, though it finishes first.
///
/// The latch is otherwise cleared only by `BeforeSideTurnStart` (`0x9735f`,
/// the owner's side: IL_001c, IL_0023) and `AfterCombatEnd` (`0x9738d`).
/// Both fields are plain instance fields with no `SavedProperty`, so a
/// combat starts with them at their defaults. A play suspended inside the
/// latched Attack keeps the latch in the document's `music_box_card_uid`.
fn music_box_before_card_played(state: &mut HotState, played_uid: u32) {
    if state.fanouts.music_box_card_uid().is_none() && !state.fanouts.music_box_used_this_turn() {
        state.fanouts.set_music_box_card_uid(Some(played_uid));
    }
}

/// Pen Nib's `BeforeCardPlayed` for one generated CardPlay; returns whether
/// this body's Attack is the one Pen Nib doubles.
///
/// v0.111.0 (DLL 9cb4f1ad): `CardModel/<OnPlayWrapper>d__339::MoveNext`
/// RVA `0x31b8d0` builds a fresh `CardPlay` for every index of the frozen
/// play count (IL_052b-IL_058b, loop IL_08ec-IL_0909) and awaits
/// `Hook::BeforeCardPlayed` (IL_05a1) and `Hook::AfterCardPlayed` (IL_0874)
/// inside that loop; `Hook/<BeforeCardPlayed>d__15` (`0x3d2190`) calls every
/// listener with no play-index filter. `PenNib::BeforeCardPlayed` RVA
/// `0x99218` returns for a non-Attack (IL_0012-IL_0018) or a foreign owner
/// (IL_0021-IL_0031), else calls `NotifyAttackPlayed` (`0x99148`:
/// `AttacksPlayed + 1`, stored `% 10` by `set_AttacksPlayed` `0x990ea`
/// IL_0009-IL_000c) and, at zero, sets `AttackToDouble` to this card
/// (IL_0040-IL_004e). `ModifyDamageMultiplicative` `0x9917c` doubles
/// exactly that card (IL_0082-IL_008d), and `AfterCardPlayed` `0x99271`
/// clears the field for the same card before the next replay's
/// BeforeCardPlayed. So a replay series advances the counter once per body
/// and only the body that reaches zero is doubled.
///
/// #3100 (re-derived on the same DLL): every replay source reaches that one
/// loop through the frozen play count, never a separate replay command, so
/// none of them needs its own Pen Nib rule. `<GeneratePlayCount>d__340`
/// (`0x31b5e0`) is `GetEnchantedReplayCount() + 1` (IL_001f-IL_0026) folded
/// through `Hook::ModifyCardPlayCount` (IL_0041); `GetEnchantedReplayCount`
/// (`0x7ca38`) is `BaseReplayCount`, passed through
/// `EnchantmentModel::EnchantPlayCount` when enchanted (IL_000c-IL_0018).
/// The sources: Duplicator's `<OnUse>d__6` (`0x34d154`) applies
/// `DuplicationPower` (IL_003a), whose `ModifyCardPlayCount` (`0xa1dd9`)
/// adds 1 for the owner's card (IL_0016-IL_0018); One-Two Punch's
/// (`0xa538f`) adds 1 for an owner Attack (IL_0016-IL_0022); Echo Form's
/// (`0xa1f0c`) adds 1 while the owner's first-in-series plays started this
/// turn (predicate `0xa1fef`) are below `Amount` (IL_0021-IL_004f); Sword Sage's `TryAddReplays` (`0xa9180`) raises a
/// Sovereign Blade's `BaseReplayCount` (IL_002b-IL_0034); Glam's
/// `EnchantPlayCount` (`0xd5f94`) adds `Times` until `UsedThisCombat`
/// (IL_0001-IL_0021) and Spiral's (`0xd6586`) always adds `Times`
/// (IL_0001-IL_0017). Each generated body's `CardPlay` sets `IsAutoPlay`,
/// `PlayIndex` and `PlayCount` (`0x31b8d0` IL_0573-IL_058b), and this hook
/// reads none of them. The one exit between bodies is
/// `CombatManager.IsOverOrEnding` (IL_0428-IL_0432): there is no dead-target
/// test before BeforeCardPlayed, so a Sovereign Blade body against a dead
/// target still advances this counter (#3101).
///
/// Music Box's latch is the other relic body here
/// ([`music_box_before_card_played`]); `played_uid` is its `CardPlay.Card`.
pub(crate) fn before_card_played_hand(
    catalog: &Catalog,
    state: &mut HotState,
    spec: &crate::catalog::CardSpec,
    played_uid: u32,
) -> Result<bool, EngineRefusal> {
    if spec.is_attack && catalog.hooks().owns(RelicId::RelicMusicBox) {
        music_box_before_card_played(state, played_uid);
    }
    if spec.is_attack && catalog.hooks().owns(RelicId::RelicPenNib) {
        if state.powers.value(PowerId::SealedThrone) > 0
            && state.powers.value(PowerId::BlackHole) > 0
        {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Sealed Throne and Black Hole with Pen Nib",
            ));
        }
        let next = (state.fanouts.pen_nib() + 1) % 10;
        let written = state.fanouts.set_pen_nib(next);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicPenNib);
        return Ok(next == 0);
    }
    Ok(false)
}

/// Same-hook peers covered by the counter-relic order admission proof.
pub(crate) const COUNTER_AFTER_CARD_PLAYED_PEERS: [RelicId; 23] = [
    RelicId::RelicArtOfWar,
    RelicId::RelicBrilliantScarf,
    RelicId::RelicDaughterOfTheWind,
    RelicId::RelicGamePiece,
    RelicId::RelicHelicalDart,
    RelicId::RelicIvoryTile,
    RelicId::RelicKunai,
    RelicId::RelicKusarigama,
    RelicId::RelicLetterOpener,
    RelicId::RelicLostWisp,
    RelicId::RelicMummifiedHand,
    RelicId::RelicNunchaku,
    RelicId::RelicOrnamentalFan,
    RelicId::RelicPenNib,
    RelicId::RelicPermafrost,
    RelicId::RelicPocketwatch,
    RelicId::RelicRainbowRing,
    RelicId::RelicRazorTooth,
    RelicId::RelicRippleBasin,
    RelicId::RelicShuriken,
    RelicId::RelicUnsettlingLamp,
    RelicId::RelicVambrace,
    RelicId::RelicVelvetChoker,
];

/// The AfterCardPlayed peers whose native callback this engine does not run
/// inside [`after_card_played_hand`]'s inventory-ordered walk: their state
/// lives in the combat history (Brilliant Scarf, Pocketwatch, Velvet Choker),
/// in the BeforeCardPlayed body (Pen Nib), in a display status (Ripple Basin)
/// or in the play frame closed just before the walk (Unsettling Lamp). Their
/// place relative to a counter relic is therefore fixed by the engine, not by
/// the inventory, and admission refuses a counter recorded before one of them
/// unless [`counter_precedes_out_of_walk_peer_commutes`] proves the two orders
/// equal. Velvet Choker also appears in the walk, for coverage only.
pub(crate) const COUNTER_OUT_OF_WALK_PEERS: [RelicId; 6] = [
    RelicId::RelicBrilliantScarf,
    RelicId::RelicPenNib,
    RelicId::RelicPocketwatch,
    RelicId::RelicRippleBasin,
    RelicId::RelicUnsettlingLamp,
    RelicId::RelicVelvetChoker,
];

/// Whether a counter relic (Iron Club or Tuning Fork), or Game Piece, recorded
/// *before* an out-of-walk peer commutes with it, so the engine's fixed
/// placement of the peer is unobservable. v0.111.0, DLL 9cb4f1ad:
///
/// - Pocketwatch: `AfterCardPlayed` RVA `0x99724` only increments
///   `_cardsPlayedThisTurn` (IL_0037-IL_0040) and refreshes the counter
///   display. The field's one game-logic reader is `BeforeSideTurnStart`
///   `0x997f1` (IL_001c, copying it to `_cardsPlayedLastTurn`); the others are
///   `get_DisplayAmount` `0x996f2` and `RefreshCounter` `0x9984a`. Neither a
///   counter's draw nor its block starts a side turn, so nothing inside the
///   walk observes the increment's position.
/// - Ripple Basin: `AfterCardPlayed` RVA `0x9a7ff` only calls `set_Status(0)`
///   for an owner Attack (IL_0001-IL_0030), a display status; the type has no
///   field at all.
/// - Pen Nib with Tuning Fork only: `AfterCardPlayed` RVA `0x99271` clears
///   `AttackToDouble` only for `cardPlay.Card == AttackToDouble`
///   (IL_000f-IL_0025), which `BeforeCardPlayed` `0x99218` sets only for an
///   Attack (IL_0012-IL_0018); Tuning Fork's `<AfterCardPlayed>d__22`
///   (`0x3334b8`) acts only on the owner's Skill. For one CardPlay at most one
///   of the two has an effect. Iron Club's draw fires on every card type, so
///   Pen Nib after Iron Club is not covered. Game Piece
///   (`<AfterCardPlayed>d__6` `0x325930`) acts only on the owner's Power
///   (#2909), so it is covered as Tuning Fork is.
/// - Brilliant Scarf (`_cardsPlayedThisTurn`, read by `ShouldModifyCost`
///   `0x91828` IL_0035), Velvet Choker (`_cardsPlayedThisTurn`, read by
///   `get_ShouldPreventCardPlay` `0x9d945` IL_0002) and Unsettling Lamp
///   (`IsFinishedTriggering`, `0x9d606` IL_0024-IL_002b) all have in-turn
///   readers, so none is covered. Game Piece's draw is Iron Club's command
///   (`CardPileCmd::Draw`), and a Hellraiser AutoPlay nested in it is a play
///   those counts see, so it is held to the same table, narrowed for Velvet
///   Choker and Unsettling Lamp by
///   [`game_piece_precedes_out_of_walk_peer_commutes`] (#3756).
pub(crate) fn counter_precedes_out_of_walk_peer_commutes(counter: RelicId, peer: RelicId) -> bool {
    match peer {
        RelicId::RelicPocketwatch | RelicId::RelicRippleBasin => true,
        RelicId::RelicPenNib => counter != RelicId::RelicIronClub,
        _ => false,
    }
}

/// Whether Game Piece recorded *before* an out-of-walk peer commutes with it
/// (#3756). [`counter_precedes_out_of_walk_peer_commutes`] holds Game Piece to
/// the counters' table because its body is a draw. Two of that table's
/// refusals are narrower for Game Piece than for a counter, read from IL
/// (v0.111.0, DLL 9cb4f1ad):
///
/// `GamePiece/<AfterCardPlayed>d__6::MoveNext` (RVA `0x325930`) acts only on
/// the owner's card (IL_0020-IL_0036) of `Type == Power` (IL_004e-IL_005f),
/// and its whole body is one `CardPileCmd::Draw(choiceContext, Cards, Owner,
/// false)` (IL_0089). The hooks a draw raises are `ShouldDraw` and
/// `AfterPreventingDraw` (`CardPileCmd/<DrawInternal>d__21` `0x3e3a70`
/// IL_005e, IL_0076), `ModifyShuffleOrder` and `AfterShuffle`
/// (`<Shuffle>d__22` `0x3e4b74` IL_00e2, IL_04a7), `AfterCardEnteredCombat`
/// and `AfterCardChangedPiles` (`<Add>d__10` `0x3e1ba4` IL_0659, IL_0887) and
/// `AfterCardDrawn` (`<DrawInternal>d__21` IL_0330, whose hook runs the
/// `AfterCardDrawnEarly` pass and then the ordinary one).
///
/// - **Velvet Choker.** `AfterCardPlayed` RVA `0x9d9a0` increments
///   `_cardsPlayedThisTurn` for an owner card (IL_0001-IL_001d). The field's
///   one game-logic reader is `get_ShouldPreventCardPlay` `0x9d945` (IL_0002),
///   reached only through `VelvetChoker::ShouldPlay` `0x9d985` (IL_0012), and
///   `Hook::ShouldPlay` has two callers in the assembly: `CardModel::CanPlay`
///   `0x7dca8` (IL_00b6) and `CardCmd/<AutoPlay>d__0::MoveNext` `0x3df9d4`
///   (IL_015c). Of every listener of the draw hooks above, only
///   `HellraiserPower/<AfterCardDrawnEarly>d__7::MoveNext` (`0x33c1a8`) calls
///   either. So the increment's place relative to Game Piece's draw is
///   observable exactly when a Hellraiser AutoPlay can nest in that draw
///   (`nested_play`, admission's `hellraiser_can_be_live`). There a vouched
///   inventory is exact in either order: the nested play asks `ShouldPlay`
///   against a count that leaves out the Power while the Choker is still to
///   come in the walk (`play::velvet_choker_cards_played`; native traces
///   `choker_after_piece`, `choker_before_piece` and their `_fifth` twins).
/// - **Unsettling Lamp.** `AfterCardPlayed` RVA `0x9d606` sets
///   `IsFinishedTriggering` when the played card is `TriggeringCard`
///   (IL_0001-IL_002b). The latch has two readers.
///   `BeforePowerAmountChanged` `0x9d4fc` returns before it reads the latch
///   whenever `TriggeringCard` is set (IL_000c-IL_0019), and the latch is
///   never set while `TriggeringCard` is null, so that reader cannot see the
///   position. `ModifyPowerAmountGivenMultiplicative` `0x9d5a8` doubles only
///   for `cardSource == TriggeringCard` (IL_001a-IL_0022), which here is the
///   Power card just played. Among the draw hooks' listeners exactly one
///   applies a power, `CorrosiveWavePower/<AfterCardDrawn>d__6::MoveNext`
///   (`0x3378b8`), and it passes a null `cardSource` (IL_00d8); a nested
///   Hellraiser AutoPlay plays the drawn card, a different card. No power
///   change inside Game Piece's draw names the played Power card, so the
///   position is unobservable with or without a nested play.
///
/// Brilliant Scarf stays refused: its count is read by every cost query
/// (`ShouldModifyCost` `0x91828` IL_0035), and nothing here establishes that a
/// draw makes none.
pub(crate) fn game_piece_precedes_out_of_walk_peer_commutes(
    peer: RelicId,
    nested_play: bool,
    vouched: bool,
) -> bool {
    match peer {
        RelicId::RelicUnsettlingLamp => true,
        // With a nested play the place is observable, and a vouched
        // inventory runs the Choker's count at its recorded place
        // (`play::velvet_choker_cards_played`).
        RelicId::RelicVelvetChoker => !nested_play || vouched,
        _ => counter_precedes_out_of_walk_peer_commutes(RelicId::RelicGamePiece, peer),
    }
}

/// Which relic bodies one call of [`after_card_played_peer_segment`] runs.
///
/// v0.111.0 (DLL 9cb4f1ad): `Hook.<AfterCardPlayed>d__16::MoveNext` RVA
/// `0x3ccbf4` walks `CombatState.IterateHookListeners` (IL_0024), awaiting each
/// listener's `AfterCardPlayed` (IL_0070-IL_00c5) before advancing
/// (IL_00f3). `<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720` yields the
/// player's relics in `Player.Relics` list order (IL_00c9-IL_00ef), skipping a
/// melted relic (`get_IsMelted`, IL_00de). So natively every relic's
/// `AfterCardPlayed` runs at its inventory position, and each finishes before
/// the next starts.
///
/// This engine does that for 22 of the 27 relics that override the hook, the
/// ones in [`in_after_card_played_walk`]. The other five run at a fixed place
/// before the walk whatever the inventory says: Brilliant Scarf, Pen Nib,
/// Pocketwatch, Ripple Basin and Unsettling Lamp
/// ([`COUNTER_OUT_OF_WALK_PEERS`]). Velvet Choker is in both lists: its count
/// is written when the play finishes, before the walk, and its walk entry only
/// records coverage. Admission refuses a drawing or counter body recorded
/// before an out-of-walk peer it does not commute with
/// ([`counter_precedes_out_of_walk_peer_commutes`]).
///
/// With authenticated dispatch-order provenance
/// ([`crate::hooks::HookTable::dispatch_ordered`]) the walk visits the recorded
/// inventory and runs one body per walk relic ([`WalkSlot::Relic`]), then the
/// bodies of relics the inventory does not hold ([`WalkSlot::Unowned`]).
/// Without it the order is unknown: the legacy fixed order stands
/// ([`WalkSlot::Every`], then Iron Club, then Tuning Fork), and admission
/// refuses by name every co-owned pair whose bodies do not commute.
///
/// Music Box's body clones the card `MusicBox::BeforeCardPlayed` latched,
/// which is not always the played card of this walk: a nested Attack (a
/// Hellraiser AutoPlay inside Iron Club's or Game Piece's draw) runs its own
/// walk inside this one and is not the latched card
/// ([`music_box_before_card_played`], #3640).
///
/// One limit this walk does not lift: the owner-death check is made once,
/// before the walk, not between bodies (#3642).
#[derive(Clone, Copy)]
enum WalkSlot {
    /// Every body, in this engine's fixed peer order (unvouched inventory).
    Every,
    /// The one body of the relic the vouched walk has reached.
    Relic(RelicId),
    /// Bodies whose relic is not in the inventory. Three bodies are gated on
    /// combat state rather than on ownership (Vambrace, Pael's Legion,
    /// Permafrost), and this slot runs them after the recorded relics. No
    /// admitted root reaches one: Vambrace's latch is armed only for an
    /// owner, Pael's Legion's trigger needs a zero cooldown and an unowned
    /// relic's cooldown is negative (`boundary::batch_nine_relic_state_is_exact`),
    /// and an armed Permafrost without the relic refuses
    /// (`batch-6 relic inventory/state`). So the slot's place is unobservable.
    Unowned,
}

/// Whether `relic` has a body in the inventory-ordered AfterCardPlayed walk
/// ([`after_card_played_hand`]): the two counters and every relic
/// [`after_card_played_peer_segment`] names.
fn in_after_card_played_walk(relic: RelicId) -> bool {
    matches!(
        relic,
        RelicId::RelicArtOfWar
            | RelicId::RelicDaughterOfTheWind
            | RelicId::RelicGamePiece
            | RelicId::RelicHelicalDart
            | RelicId::RelicIronClub
            | RelicId::RelicIvoryTile
            | RelicId::RelicKunai
            | RelicId::RelicKusarigama
            | RelicId::RelicLetterOpener
            | RelicId::RelicLostWisp
            | RelicId::RelicMummifiedHand
            | RelicId::RelicMusicBox
            | RelicId::RelicNunchaku
            | RelicId::RelicOrnamentalFan
            | RelicId::RelicPaelsLegion
            | RelicId::RelicPermafrost
            | RelicId::RelicRainbowRing
            | RelicId::RelicRazorTooth
            | RelicId::RelicShuriken
            | RelicId::RelicTuningFork
            | RelicId::RelicVambrace
            | RelicId::RelicVelvetChoker
    )
}

/// TuningFork/<AfterCardPlayed>d__22 MoveNext RVA 0x3334b8 (v0.111.0,
/// DLL 9cb4f1ad) increments only for the owner's Skill; at ten, awaits raw
/// block 7 before subtracting ten. The native inventory-order Hook awaits
/// this callback before the next relic (including a lethal Letter Opener).
fn tuning_fork_after_card_played(
    catalog: &Catalog,
    state: &mut HotState,
    spec: &crate::catalog::CardSpec,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if spec.is_skill && catalog.hooks().owns(RelicId::RelicTuningFork) {
        let next = state
            .fanouts
            .tuning_fork_skills()
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("Tuning Fork skills"))?;
        let written = state.fanouts.set_tuning_fork_skills(next);
        debug_assert!(written);
        if next >= 10 {
            if !state.history.over {
                super::orbs::gain_flat_block(state, catalog, 7, events)?;
            }
            let written = state.fanouts.set_tuning_fork_skills(next - 10);
            debug_assert!(written);
        }
        crate::coverage::record_relic(RelicId::RelicTuningFork);
    }
    Ok(())
}

/// Iron Club (v0.111.0, DLL 9cb4f1ad; `IronClub::AfterCardPlayed` RVA
/// `0x95158` builds its state machine): counts every owner card and draws one
/// at each fourth. Run at its inventory position by [`after_card_played_hand`].
fn iron_club_after_card_played(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if catalog.hooks().owns(RelicId::RelicIronClub) {
        let next = (state.fanouts.iron_club_cards() + 1) % 4;
        let written = state.fanouts.set_iron_club_cards(next);
        debug_assert!(written);
        if next == 0 && !state.history.over {
            // The listener's tail Draw, receipt-owned when it can park
            // (#3822, `draw::iron_club_draw`).
            super::draw::iron_club_draw(state, catalog, 1, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicIronClub);
    }
    Ok(())
}

/// Whether Brilliant Scarf's count of the CardPlay `played_uid` is still to
/// come when Mummified Hand's body runs (#3756): a vouched inventory records
/// every Mummified Hand before every Brilliant Scarf, and the play is one the
/// Scarf counts, a manual one (`BrilliantScarf::AfterCardPlayed` RVA `0x917bc`
/// skips `CardPlay.IsAutoPlay` at IL_001e-IL_0024). False for any other
/// inventory, where this engine's count-before-the-walk order stands.
///
/// Whether the play is manual is read from the active CardPlay. Outside one,
/// nothing says whether the play was counted, and the body refuses by name.
fn brilliant_scarf_counts_after_mummified_hand(
    catalog: &Catalog,
    played_uid: u32,
) -> Result<bool, EngineRefusal> {
    if !relic_recorded_before(
        catalog,
        RelicId::RelicMummifiedHand,
        RelicId::RelicBrilliantScarf,
    ) {
        return Ok(false);
    }
    super::play::active_play_is_manual(played_uid).ok_or(EngineRefusal::MalformedArgs(
        "Mummified Hand recorded before Brilliant Scarf outside an active CardPlay",
    ))
}

/// Mummified Hand's `AfterCardPlayed` for an owner Power play: make one Hand
/// card free this turn. The caller has already checked the Power type and the
/// relic's place in the walk.
///
/// v0.111.0 DLL `9cb4f1ad…`, `MummifiedHand::AfterCardPlayed` RVA `0x97148`
/// (synchronous, no state machine):
///
/// * IL_000c-IL_0016 returns unless `CombatManager.IsInProgress`.
/// * IL_001e-IL_002f requires the owner's card, and IL_0037-IL_0043 requires
///   `CardType.Power`.
/// * IL_004b-IL_0060 reads `RunRngSet.CombatCardSelection`
///   ([`RngStream::Sel`]).
/// * IL_0061-IL_010e filters the Hand. Each `NextItem` over a filtered list
///   consumes a draw only when the list is nonempty.
/// * IL_0113 calls `SetToFreeThisTurn` on the chosen card.
///
/// **The gate is `IsInProgress`. It is not `history.over` and it is not the
/// ending projection** (#3247). `CombatManager::get_IsInProgress` RVA
/// `0x1356e9` reads `CombatTurnState.IsInProgress`. Only two writes clear it:
/// `ProcessPendingLoss` RVA `0x13646c` IL_002b, and
/// `<EndCombatInternal>d__122::MoveNext` RVA `0x3f3e60` IL_0096. Both are
/// reached only through `CombatManager.CheckWinCondition`.
///
/// On the card-play path, `CheckWinCondition`'s caller is
/// `ActionExecutor/<ExecuteActions>d__28::MoveNext` RVA `0x3d4310` IL_0334.
/// That call comes after `GameAction.Execute` (IL_0148) has finished the whole
/// play, AfterCardPlayed suffix included. The other callers are turn
/// transitions and console commands.
///
/// So consider the killing play. There, `IsCombatEnding` (RVA `0x135854`)
/// and `history.over` already hold, but `IsInProgress` is still true, and the
/// relic still draws. For example, a lethal Consuming Shadow into Soul Nexus
/// advances `CombatCardSelection` (F3C51JD0JVEK n39). During a play, the body
/// stops only on owner death. [`after_card_played_hand`]'s
/// `player_hooks_deactivated` return already covers that case.
///
/// # The two cost reads (#3756)
///
/// The filter's four tiers (IL_0061-IL_010e) are built from two predicates,
/// and they read the cost differently:
///
/// * `b__2_0` (RVA `0x32add6`): `EnergyCost.GetWithModifiers(0)` > 0
///   (IL_0002-IL_000e) or `BaseStarCost > 0` (IL_0011-IL_0017). The argument
///   is `CostModifiers` **none**: `CardEnergyCost::GetWithModifiers` RVA
///   `0x11e044` walks the card-local modifiers only under flag 2
///   (IL_0037-IL_0048) and calls `Hook::ModifyEnergyCostInCombat` only under
///   flag 4 (IL_0081-IL_00b8). So this is the base cost, `require_base`.
/// * `b__2_1` / `b__2_2` (RVA `0x32adf2`, `0x32adfb`):
///   `CardModel::CostsEnergyOrStars(true)` RVA `0x7db3c`, whose `true` arm is
///   `GetWithModifiers(-1)` > 0 for a non-X card (IL_000f-IL_0029), every
///   flag, or `GetStarCostWithModifiers` > 0 (IL_002d-IL_003c), which runs
///   `Hook::ModifyStarCost` (`0x7dad0` IL_0057). So `require_current` reads
///   the cost through the card-local modifiers **and** the global cost hooks:
///   [`super::play::resolved_energy_cost`], not the card-local value alone.
///   Before #3756 this body read the card-local value, which is the same
///   answer only while no global cost hook is live (Brilliant Scarf's window,
///   Void Form, Corruption and the Free powers, Tangled).
///
/// # Beside Brilliant Scarf (#3756)
///
/// Brilliant Scarf is one of those global hooks:
/// `TryModifyEnergyCostInCombatLate` RVA `0x9175d` and `TryModifyStarCost`
/// `0x91779` both ask `ShouldModifyCost` RVA `0x91828`, which zeroes an
/// owner's Hand or Play card while `CardsPlayedThisTurn == Cards - 1`
/// (IL_0034-IL_005e, pile test IL_0062-IL_00a2). The Scarf writes that count
/// in its own `AfterCardPlayed`
/// RVA `0x917bc`, for an owner play that is not an AutoPlay
/// (IL_001e-IL_0050). Both are relic listeners of
/// `Hook/<AfterCardPlayed>d__16::MoveNext` (RVA `0x3ccbf4`), awaited in
/// `Player.Relics` order, so:
///
/// * the Scarf recorded first has counted this play when the filter runs;
/// * Mummified Hand recorded first reads the count **before** this play.
///
/// This engine counts the play before the walk
/// (`play::record_card_play_finished`, the Scarf being an out-of-walk peer,
/// [`COUNTER_OUT_OF_WALK_PEERS`]), which is the first case. For the second,
/// on a vouched inventory, the filter runs against the count less this play
/// when this play was counted, that is, when it is manual
/// ([`brilliant_scarf_counts_after_mummified_hand`]). The two differ exactly
/// on the fourth and fifth manual plays of a turn, where the Scarf's free
/// window opens and closes. An unvouched inventory keeps the first case, and
/// admission refuses the pair there.
fn mummified_hand_after_card_played(
    catalog: &Catalog,
    state: &mut HotState,
    played_uid: u32,
) -> Result<(), EngineRefusal> {
    let counted = state.history.manual_card_plays_finished_this_turn;
    if brilliant_scarf_counts_after_mummified_hand(catalog, played_uid)? {
        state.history.manual_card_plays_finished_this_turn = counted
            .checked_sub(1)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
    }
    let candidates = {
        let hand = state.piles.get(PileId::Hand).as_slice();
        let candidates_for = |require_base: bool, require_current: bool| {
            hand.iter()
                .copied()
                .filter(|card| {
                    let Some(candidate) = catalog.spec(card.atom) else {
                        return false;
                    };
                    let base_positive = candidate.cost > 0 || candidate.row.star_cost > 0;
                    // `CostsEnergyOrStars(true)`: every modifier, card-local
                    // and global (see "The two cost reads" above).
                    let current_positive =
                        super::play::resolved_energy_cost(state, *card, candidate) > 0
                            || super::play::resolved_star_cost(state, *card, candidate)
                                .is_some_and(|cost| cost > 0);
                    (!require_base || base_positive) && (!require_current || current_positive)
                })
                .collect::<Vec<_>>()
        };
        [(true, true), (false, true), (true, false), (false, false)]
            .into_iter()
            .map(|(base, current)| candidates_for(base, current))
            .find(|candidates| !candidates.is_empty())
            .unwrap_or_default()
    };
    // The Scarf's own body counts the play after this one.
    state.history.manual_card_plays_finished_this_turn = counted;
    if !candidates.is_empty() {
        let live = state.rng.get(RngStream::Sel);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let index: usize = rng
            .next_bounded(candidates.len() as i32)
            .map_err(|_| EngineRefusal::CounterOverflow("Mummified Hand selection"))?
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("Mummified Hand selection"))?;
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: rng.words,
                counter: rng.counter,
            },
        );
        let chosen = candidates[index];
        let chosen_spec = catalog
            .spec(chosen.atom)
            .ok_or(EngineRefusal::UnknownAtom(chosen.atom))?;
        state
            .card_states
            .set_to_free_this_turn(chosen.uid, chosen_spec.cost)
            .ok_or(EngineRefusal::CounterOverflow("Mummified Hand free card"))?;
        // The rows just appended live in the card's slot-7 payload, which the
        // boundary emits only for a card carrying this bit
        // (`HotBoundary::card_to_canonical_with_instance`). Without it they
        // were live here and dropped from the canonical root, so a reloaded
        // root charged the chosen card's full cost (#3180, the #3176 class).
        // Bullet Time's live-Hand write sets the same bit and promotes exact
        // piles (`steps::silent_rare`).
        let live = state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .iter_mut()
            .find(|card| card.uid == chosen.uid)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        live.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.exact_piles = true;
    }
    crate::coverage::record_relic(RelicId::RelicMummifiedHand);
    Ok(())
}

/// Permafrost's once-per-combat Power-card block, at its recorded side of
/// Lost Wisp (#2909).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…`):
/// `Permafrost/<AfterCardPlayed>d__9::MoveNext` (RVA `0x32dbc4`) checks
/// `CombatManager.IsInProgress` at IL_0020-IL_002a, the card's owner at
/// IL_0032-IL_0047, `Type == Power` at IL_004f-IL_005f and
/// `ActivatedThisCombat` at IL_0067-IL_006c, then `CreatureCmd::GainBlock`
/// at IL_007a-IL_0091. `LostWisp/<AfterCardPlayed>d__4::MoveNext` (RVA
/// `0x329e14`) checks the owner at IL_0021-IL_0036, `IsInProgress` at
/// IL_003d-IL_0047 and `Type == Power` at IL_004f-IL_005f, then
/// `CreatureCmd::Damage(HittableEnemies, Damage)` at IL_006d-IL_00b2.
///
/// Both are relic listeners of `Hook/<AfterCardPlayed>d__16::MoveNext` (RVA
/// `0x3ccbf4`), which walks `IterateHookListeners` (IL_0024) and awaits each
/// `AfterCardPlayed` (IL_0070) before `MoveNext` (IL_00f3). Relics come in
/// `Player.Relics` order. Under Juggernaut the pair does not commute:
/// Permafrost's block draws a `CombatTargets` target from the hittable list
/// (`JuggernautPower/<AfterBlockGained>d__4` RVA `0x33dab4`, IL_004a-IL_007e)
/// that Lost Wisp's AoE changes. So a vouched inventory that records
/// Permafrost first runs it first ([`relic_recorded_before`]). Otherwise it
/// keeps its fixed place after Lost Wisp, and admission refuses the pair
/// with Juggernaut reachable on an unvouched inventory
/// (`Lost Wisp + Permafrost order with Juggernaut`).
fn permafrost_after_card_played(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.history.over && state.permafrost_armed() {
        state.set_permafrost_armed(false);
        super::orbs::gain_flat_block(state, catalog, 7, events)?;
        crate::coverage::record_relic(RelicId::RelicPermafrost);
    }
    Ok(())
}

/// The allocation-free AfterCardPlayed relic suffix that precedes enemy
/// listeners. `energy_value` is the action's native `CardPlay.Resources
/// .EnergyValue` (`play::card_play_energy_value`), shared by every replay
/// body: the manual payment, or an AutoPlay's resolved cost (#3168). `None`
/// is a resumed AutoPlay whose value no record carries; Ivory Tile, its one
/// reader here, refuses by name on it.
///
/// #3192, #2909: on a vouched inventory every relic body runs at its recorded
/// position, one relic at a time, as the native Hook awaits them (see
/// [`WalkSlot`] for the IL). #3192 placed only Iron Club and Tuning Fork that
/// way and kept this engine's fixed order between them, which ran Music Box's
/// clone ahead of a Razor Tooth recorded before it (and so on for every other
/// pair the fixed order has backwards). An unvouched inventory keeps the fixed
/// order, and `engine::admission` refuses its non-commuting pairs by name.
pub(crate) fn after_card_played_hand(
    catalog: &Catalog,
    state: &mut HotState,
    spec: &crate::catalog::CardSpec,
    played_uid: u32,
    energy_value: Option<i16>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // Native Contains0x137564 IL00db–00ee excludes the deactivated owner.
    // Power callbacks may have completed owner death before this suffix.
    if state.fanouts.player_hooks_deactivated() {
        return Ok(());
    }
    let played_is_dupe = if spec.is_attack && catalog.hooks().owns(RelicId::RelicHistoryCourse) {
        let (pile, index) = super::play::unique_live_card_location(state, played_uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        state.piles.get(pile).as_slice()[index].flags & crate::hot::CARD_FLAG_DUPE != 0
    } else {
        false
    };
    if spec.is_attack && !played_is_dupe && catalog.hooks().owns(RelicId::RelicHistoryCourse) {
        let (pile, index) = super::play::unique_live_card_location(state, played_uid)?
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let source = state.piles.get(pile).as_slice()[index];
        let retained = crate::hot::FrozenAutoBatchEntry::from_current(source, &state.card_states);
        state
            .fanouts
            .set_history_course_attack_current_turn(Some(retained));
        crate::coverage::record_relic(RelicId::RelicHistoryCourse);
    }
    if catalog.hooks().owns(RelicId::RelicIronClub)
        && catalog.hooks().owns(RelicId::RelicTuningFork)
        && spec.is_skill
        && (state.fanouts.iron_club_cards() + 1).is_multiple_of(4)
        && state
            .fanouts
            .tuning_fork_skills()
            .checked_add(1)
            .is_some_and(|value| value >= 10)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Iron Club and Tuning Fork simultaneous thresholds",
        ));
    }
    let hooks = catalog.hooks();
    if !hooks.dispatch_ordered() {
        after_card_played_peer_segment(
            catalog,
            state,
            spec,
            played_uid,
            energy_value,
            events,
            WalkSlot::Every,
        )?;
        iron_club_after_card_played(catalog, state, events)?;
        return tuning_fork_after_card_played(catalog, state, spec, events);
    }
    let relics = hooks.relics();
    for (index, relic) in relics.iter().enumerate() {
        if !in_after_card_played_walk(*relic) || relics[..index].contains(relic) {
            continue;
        }
        match relic {
            RelicId::RelicIronClub => iron_club_after_card_played(catalog, state, events)?,
            RelicId::RelicTuningFork => {
                tuning_fork_after_card_played(catalog, state, spec, events)?
            }
            _ => after_card_played_peer_segment(
                catalog,
                state,
                spec,
                played_uid,
                energy_value,
                events,
                WalkSlot::Relic(*relic),
            )?,
        }
    }
    after_card_played_peer_segment(
        catalog,
        state,
        spec,
        played_uid,
        energy_value,
        events,
        WalkSlot::Unowned,
    )
}

/// Whether Letter Opener's private `SkillsPlayedThisTurn` equals the shared
/// `history.skill_plays_finished_this_turn` quotient at this skill play
/// (#3466).
///
/// Native keeps two different counts. `LetterOpener/<AfterCardPlayed>d__19::
/// MoveNext` RVA `0x328e98` increments its own `SkillsPlayedThisTurn` on
/// every owner Skill (owner IL_0020-0038, `IsInProgress` IL_003d-0049,
/// `Type == Skill` IL_004e-0061, increment IL_0066-0071) with no side test,
/// then fires on `% Cards == 0` (IL_0087-008f). Only
/// `LetterOpener::AfterSideTurnStart` RVA `0x963b8` resets it: on the owner's
/// side start (IL_000c-001d), after turn one (IL_0025-0036), at
/// IL_003e-0040. That hook is `StartTurn`'s side-start tail, after
/// `SetupPlayerTurn`'s hand draw and AfterPlayerTurnStart (see
/// [`crate::engine::turn`]'s `SetupPlayerTurnWindow`).
///
/// The shared quotient is the `HappenedThisTurn` count instead, which rolls at
/// every `SwitchSides` (`turn::roll_happened_this_turn_counters`). The two
/// agree whenever this read happens on the player side after the side-start
/// tail, or on turn one, which neither resets. They part on the enemy side
/// (Letter Opener still holds the player turn's Skills) and between a later
/// turn's switch and its side-start tail (Letter Opener has not reset yet).
/// A Skill play there refuses by name.
fn letter_opener_count_is_shared(state: &HotState) -> bool {
    state.player_side_active
        && (state.turn <= 1 || !super::turn::player_side_start_tail_is_pending())
}

/// The in-walk AfterCardPlayed peer bodies `slot` selects (see [`WalkSlot`]),
/// in this engine's fixed peer order. A vouched walk calls this once per
/// recorded relic, so that order only decides anything for [`WalkSlot::Every`].
#[allow(clippy::too_many_arguments)]
fn after_card_played_peer_segment(
    catalog: &Catalog,
    state: &mut HotState,
    spec: &crate::catalog::CardSpec,
    played_uid: u32,
    energy_value: Option<i16>,
    events: &mut Vec<Event>,
    slot: WalkSlot,
) -> Result<(), EngineRefusal> {
    let runs = |relic: RelicId| match slot {
        WalkSlot::Every => true,
        WalkSlot::Relic(current) => current == relic,
        WalkSlot::Unowned => !catalog.hooks().owns(relic),
    };
    // The card latched at BeforeCardPlayed, and only that one (`0x32ae04`
    // IL_001e-IL_002e): see [`music_box_before_card_played`].
    if state.fanouts.music_box_card_uid() == Some(played_uid)
        && catalog.hooks().owns(RelicId::RelicMusicBox)
        && runs(RelicId::RelicMusicBox)
    {
        let source = state
            .piles
            .get(PileId::Play)
            .as_slice()
            .iter()
            .copied()
            .find(|card| card.uid == played_uid)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        let clone_uid = state.next_card_uid;
        super::cards::inject_generated_clones_bottom(
            state,
            catalog,
            source,
            1,
            PileId::Hand,
            events,
        )?;
        let mut clone_state = state.card_states.get(clone_uid);
        clone_state.set_local_ethereal(true);
        state.card_states.set(clone_uid, clone_state);
        state.fanouts.set_music_box_used_this_turn(true);
        state.fanouts.set_music_box_card_uid(None);
        crate::coverage::record_relic(RelicId::RelicMusicBox);
    }
    if spec.is_power
        && catalog.hooks().owns(RelicId::RelicMummifiedHand)
        && runs(RelicId::RelicMummifiedHand)
    {
        mummified_hand_after_card_played(catalog, state, played_uid)?;
    }
    if spec.is_attack
        && !state.art_of_war_last_attack()
        && catalog.hooks().owns(RelicId::RelicArtOfWar)
        && runs(RelicId::RelicArtOfWar)
    {
        state.set_art_of_war_current_attack(true);
        crate::coverage::record_relic(RelicId::RelicArtOfWar);
    }
    if spec.is_skill
        && catalog.hooks().owns(RelicId::RelicLetterOpener)
        && runs(RelicId::RelicLetterOpener)
    {
        if !letter_opener_count_is_shared(state) {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Letter Opener skill count between a side switch and its reset",
            ));
        }
        if state.history.skill_plays_finished_this_turn % 3 == 0 {
            for target in super::damage::alive_targets(state) {
                if state.monsters[target].hp > 0 {
                    super::damage::damage_monster_with_catalog(
                        state,
                        catalog,
                        target,
                        DotNetDecimal::from_i64(5),
                        false,
                        true,
                        events,
                    )?;
                }
            }
        }
        crate::coverage::record_relic(RelicId::RelicLetterOpener);
    }
    if !state.history.over
        && catalog.hooks().owns(RelicId::RelicRainbowRing)
        && runs(RelicId::RelicRainbowRing)
    {
        let bit = if spec.is_attack {
            1
        } else if spec.is_skill {
            2
        } else if spec.is_power {
            4
        } else {
            0
        };
        if bit != 0 {
            let old = state.rainbow_ring_types_this_turn();
            let new = old | bit;
            let written = state.set_rainbow_ring_types_this_turn(new);
            debug_assert!(written);
            if old != 7 && new == 7 {
                super::damage::apply_owner_strength(state, 1, events)?;
                super::damage::apply_signed_player_stat(state, PowerId::Dexterity, 1, events)?;
            }
        }
        crate::coverage::record_relic(RelicId::RelicRainbowRing);
    }
    if spec.is_attack
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicDaughterOfTheWind)
        && runs(RelicId::RelicDaughterOfTheWind)
    {
        super::orbs::gain_flat_block(state, catalog, 1, events)?;
        crate::coverage::record_relic(RelicId::RelicDaughterOfTheWind);
    }
    if spec.is_power
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicGamePiece)
        && runs(RelicId::RelicGamePiece)
    {
        // The listener's tail Draw, receipt-owned when it can park (#3822,
        // `draw::game_piece_draw`).
        super::draw::game_piece_draw(state, catalog, 1, events)?;
        crate::coverage::record_relic(RelicId::RelicGamePiece);
    }
    if spec.row.tags.contains(&"Shiv")
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicHelicalDart)
        && runs(RelicId::RelicHelicalDart)
    {
        let current = state.powers.value(PowerId::TempDexterity);
        let updated = current
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow("Helical Dart Dexterity"))?;
        super::turn::prepare_after_side_turn_end_scalar_write(
            state,
            PowerId::TempDexterity,
            current,
            updated,
        )?;
        state
            .powers
            .set(PowerId::TempDexterity, SlotWire::Int, updated);
        super::damage::note_power(events, Subject::Player, PowerId::TempDexterity, updated);
        crate::coverage::record_relic(RelicId::RelicHelicalDart);
    }
    // v0.111.0 (DLL 9cb4f1ad) `IvoryTile/<AfterCardPlayed>d__7::MoveNext` RVA
    // `0x327194`: a foreign card returns (IL_0020-IL_0038); the gate is
    // `cardPlay.Resources.EnergyValue >= DynamicVars["EnergyThreshold"]`
    // (IL_003d-IL_0065, threshold 3), then `GainEnergy(Energy.BaseValue = 1)`
    // (IL_006c-IL_0088). `EnergyValue`, not `EnergySpent`: an AutoPlay of a
    // 3-cost card pays nothing but grants the Energy (#3168).
    if !state.history.over
        && catalog.hooks().owns(RelicId::RelicIvoryTile)
        && runs(RelicId::RelicIvoryTile)
        && energy_value.ok_or(EngineRefusal::HookNotModeled {
            relic: RelicId::RelicIvoryTile,
            event: HookEvent::AfterCardPlayed,
        })? >= 3
    {
        if !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Ivory Tile energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicIvoryTile);
    }
    // #2909: Permafrost recorded before Lost Wisp runs first (see
    // `permafrost_after_card_played`). The two are adjacent in this fixed
    // order, so the swap moves neither past any other peer.
    let permafrost_first = spec.is_power
        && relic_recorded_before(catalog, RelicId::RelicPermafrost, RelicId::RelicLostWisp);
    if permafrost_first && runs(RelicId::RelicPermafrost) {
        permafrost_after_card_played(catalog, state, events)?;
    }
    if spec.is_power
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicLostWisp)
        && runs(RelicId::RelicLostWisp)
    {
        let targets = super::damage::alive_targets(state);
        for target in targets {
            if state
                .monsters
                .get(target)
                .is_some_and(|monster| monster.hp > 0)
            {
                super::damage::damage_monster_with_catalog(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(8),
                    false,
                    true,
                    events,
                )?;
            }
        }
        crate::coverage::record_relic(RelicId::RelicLostWisp);
    }
    if spec.is_power && !permafrost_first && runs(RelicId::RelicPermafrost) {
        permafrost_after_card_played(catalog, state, events)?;
    }
    if spec.is_attack
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicOrnamentalFan)
        && runs(RelicId::RelicOrnamentalFan)
    {
        let next = (state.ornamental_fan() + 1) % 3;
        let written = state.set_ornamental_fan(next);
        debug_assert!(written);
        if next == 0 {
            super::orbs::gain_flat_block(state, catalog, 4, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicOrnamentalFan);
    }
    if spec.is_attack
        && catalog.hooks().owns(RelicId::RelicNunchaku)
        && runs(RelicId::RelicNunchaku)
    {
        let next = (state.nunchaku() + 1) % 10;
        let written = state.set_nunchaku(next);
        debug_assert!(written);
        if next == 0 && !state.history.over {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Nunchaku energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicNunchaku);
    }
    if spec.is_attack && !state.history.over {
        if catalog.hooks().owns(RelicId::RelicKunai) && runs(RelicId::RelicKunai) {
            let next = (state.kunai() + 1) % 3;
            let written = state.set_kunai(next);
            debug_assert!(written);
            if next == 0 {
                super::damage::apply_signed_player_stat(state, PowerId::Dexterity, 1, events)?;
            }
            crate::coverage::record_relic(RelicId::RelicKunai);
        }
        if catalog.hooks().owns(RelicId::RelicShuriken) && runs(RelicId::RelicShuriken) {
            let next = (state.shuriken() + 1) % 3;
            let written = state.set_shuriken(next);
            debug_assert!(written);
            if next == 0 {
                super::damage::apply_owner_strength(state, 1, events)?;
            }
            crate::coverage::record_relic(RelicId::RelicShuriken);
        }
    }
    // `Kusarigama/<AfterCardPlayed>d__19::MoveNext` (v0.111.0 RVA `0x327fe0`):
    // owner check (IL_0020-IL_0036), `IsInProgress` (IL_003d-IL_0047), Attack
    // type (IL_004e-IL_005f), `AttacksPlayedThisTurn += 1` (IL_0066-IL_0071),
    // then `% Cards` (IL_0087-IL_008f) before the `CombatTargets` pick at
    // IL_00be and `CreatureCmd.Damage` at IL_00f6. No `history.over` gate
    // (#3261): the counter advances on the killing Attack too, and the pick
    // is [`super::roll_ungated_after_card_played_target`].
    if spec.is_attack
        && catalog.hooks().owns(RelicId::RelicKusarigama)
        && runs(RelicId::RelicKusarigama)
    {
        let next = (state.fanouts.kusarigama() + 1) % 3;
        let written = state.fanouts.set_kusarigama(next);
        debug_assert!(written);
        if next == 0
            && let Some(target) =
                super::roll_ungated_after_card_played_target(state, "Kusarigama damage")?
        {
            super::damage::damage_monster_with_catalog(
                state,
                catalog,
                target,
                DotNetDecimal::from_i64(6),
                false,
                true,
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicKusarigama);
    }
    if !state.history.over
        && (spec.is_attack || spec.is_skill)
        && catalog.hooks().owns(RelicId::RelicRazorTooth)
        && runs(RelicId::RelicRazorTooth)
    {
        super::cards::upgrade_live_cards_once(state, catalog, &[played_uid])?;
        crate::coverage::record_relic(RelicId::RelicRazorTooth);
    }
    if catalog.hooks().owns(RelicId::RelicVelvetChoker) && runs(RelicId::RelicVelvetChoker) {
        // The Choker's own count moves here (`play::velvet_choker_cards_played`).
        super::play::velvet_choker_counts_play(played_uid);
        crate::coverage::record_relic(RelicId::RelicVelvetChoker);
    }
    if state.fanouts.vambrace_available()
        && state.fanouts.vambrace_trigger_uid() == Some(played_uid)
        && runs(RelicId::RelicVambrace)
    {
        state.fanouts.set_vambrace_available(false);
        crate::coverage::record_relic(RelicId::RelicVambrace);
    }
    if state.fanouts.paels_legion_trigger_uid() == Some(played_uid)
        && runs(RelicId::RelicPaelsLegion)
    {
        state.fanouts.set_paels_legion_trigger_uid(None);
        let written = state.fanouts.set_paels_legion_cooldown(2);
        debug_assert!(written);
        state.fanouts.set_paels_legion_triggered_last_turn(true);
        crate::coverage::record_relic(RelicId::RelicPaelsLegion);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CardInstanceState, FrozenAutoBatchEntry, HotCard,
        HotMonster, PileId, RngStream, RngStreamState,
    };
    use crate::ids::{CardId, MonsterKind};

    fn catalog(relics: &[RelicId]) -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.set_relics(relics).unwrap();
        builder.build()
    }

    #[test]
    fn modifier_templates_sum_in_inventory_order() {
        let state = HotState::at_defaults();
        let catalog = catalog(&[
            RelicId::RelicBloodSoakedRose,
            RelicId::RelicPaelsBlood,
            RelicId::RelicPrismaticGem,
        ]);
        assert_eq!(
            crate::engine::modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(2)
        );
        assert_eq!(
            crate::engine::modifier_total(&catalog, HookEvent::ModifyHandDraw, &state),
            Ok(1)
        );
    }

    #[test]
    fn batch_three_scalar_modifiers_match_the_python_turn_gates() {
        let catalog = catalog(&[RelicId::RelicBigMushroom, RelicId::RelicPaelsFlesh]);
        let mut state = HotState::at_defaults();
        state.turn = 1;
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyHandDraw, &state),
            Ok(-2)
        );
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(0)
        );
        state.turn = 3;
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyHandDraw, &state),
            Ok(0)
        );
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(1)
        );
    }

    #[test]
    fn batch_four_energy_and_turn_start_relics_match_python_order() {
        let catalog = catalog(&[
            RelicId::RelicBread,
            RelicId::RelicBrimstone,
            RelicId::RelicSealOfGold,
            RelicId::RelicSpikedGauntlets,
        ]);
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.energy = 4;
        state.gold = 5;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(1)
        );
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();

        assert_eq!((state.energy, state.gold), (3, 2));
        assert_eq!(state.powers.value(PowerId::Strength), 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), 1);

        state.turn = 2;
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(2)
        );
    }

    /// `PlayerCmd::GainGold` truncates to `Int32` **once**, after the whole
    /// `ModifyGoldGained` walk. Bowler Hat's `0x915a9` multiplies the Decimal
    /// by `1.25`, Ectoplasm's listener zeroes it, and the two commute — so a
    /// reward of 15 is 18 (not 18.75), an odd reward still truncates down,
    /// and either ordering with Ectoplasm yields nothing at all.
    #[test]
    fn bowler_hat_scales_the_fatal_gold_reward_before_the_single_truncation() {
        let plain = catalog(&[]);
        let hat = catalog(&[RelicId::RelicBowlerHat]);
        let both = catalog(&[RelicId::RelicBowlerHat, RelicId::RelicEctoplasm]);

        for (catalog, amount, expected) in [
            (&plain, 15, 15),
            (&hat, 15, 18),
            // 3 * 1.25 = 3.75 -> 3: the multiplier alone never rounds up.
            (&hat, 3, 3),
            (&hat, 4, 5),
            // 1 * 1.25 = 1.25 -> 1, still positive, so the gain is published.
            (&hat, 1, 1),
        ] {
            let mut state = HotState::at_defaults();
            state.gold = 7;
            gain_fatal_reward_gold(&mut state, catalog, amount, &mut Vec::new()).unwrap();
            assert_eq!(state.gold, 7 + expected, "reward {amount}");
        }

        let mut state = HotState::at_defaults();
        state.gold = 7;
        state
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, true, false, false);
        gain_fatal_reward_gold(&mut state, &both, 15, &mut Vec::new()).unwrap();
        assert_eq!(state.gold, 7, "Ectoplasm zeroes the scaled amount");
    }

    /// Dragon Fruit (#3320): one max HP and one HP per positive gold gain,
    /// for its owner only, clamped by `GainMaxHp`'s heal at the new max, and
    /// never on a gain `ModifyGoldGained` zeroed (Ectoplasm), since
    /// `<GainGold>d__9` raises `AfterGoldGained` only behind `amount > 0`.
    #[test]
    fn dragon_fruit_gains_one_max_hp_per_positive_owner_gold_gain() {
        let fruit = catalog(&[RelicId::RelicDragonFruit]);
        let plain = catalog(&[]);
        let fruit_ecto = catalog(&[RelicId::RelicDragonFruit, RelicId::RelicEctoplasm]);
        let fruit_hat = catalog(&[RelicId::RelicDragonFruit, RelicId::RelicBowlerHat]);
        let at = |hp, max_hp| {
            let mut state = HotState::at_defaults();
            state.hp = hp;
            state.max_hp = max_hp;
            state.gold = 7;
            state
        };
        // (catalog, ectoplasm fanout, hp, max_hp, reward) -> (gold, hp, max_hp)
        for (label, catalog, ecto, hp, max_hp, reward, expected) in [
            ("owner, damaged", &fruit, false, 50, 80, 20, (27, 51, 81)),
            ("owner, full", &fruit, false, 80, 80, 25, (32, 81, 81)),
            (
                "owner, Bowler Hat",
                &fruit_hat,
                false,
                50,
                80,
                20,
                (32, 51, 81),
            ),
            ("not owned", &plain, false, 50, 80, 20, (27, 50, 80)),
            ("owner, zero gain", &fruit, false, 50, 80, 0, (7, 50, 80)),
            (
                "owner, Ectoplasm",
                &fruit_ecto,
                true,
                50,
                80,
                20,
                (7, 50, 80),
            ),
            (
                "owner, max-HP ceiling",
                &fruit,
                false,
                999_999_999,
                999_999_999,
                20,
                (27, 999_999_999, 999_999_999),
            ),
        ] {
            let mut state = at(hp, max_hp);
            if ecto {
                state
                    .fanouts
                    .set_batch_eight_deep_relic_ownership(false, false, true, false, false);
            }
            gain_fatal_reward_gold(&mut state, catalog, reward, &mut Vec::new()).unwrap();
            assert_eq!((state.gold, state.hp, state.max_hp), expected, "{label}");
        }
    }

    /// Red Skull hears Dragon Fruit's heal (`GainMaxHp`'s `Heal` raises
    /// `AfterCurrentHpChanged`, #3044): 40/80 is exactly half (Strength held),
    /// and the +1/+1 makes it 41/81, above half, so the +3 is released.
    #[test]
    fn dragon_fruit_max_hp_gain_reaches_red_skull() {
        let fruit = catalog(&[RelicId::RelicDragonFruit, RelicId::RelicRedSkull]);
        let mut state = HotState::at_defaults();
        state.hp = 40;
        state.max_hp = 80;
        state.fanouts.set_red_skull_owned(true);
        state.powers.set(PowerId::Strength, SlotWire::Int, 3);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        gain_fatal_reward_gold(&mut state, &fruit, 20, &mut Vec::new()).unwrap();
        assert_eq!((state.gold, state.hp, state.max_hp), (20, 41, 81));
        assert_eq!(state.powers.value(PowerId::Strength), 0);
    }

    /// Blood Vial heals only on the first player turn, only for an owner, and
    /// only while the combat is live — the three conjuncts of
    /// frozen Python `_run_after_side_turn_start_power_order` (deleted #2827) and of the native guard at `0x3200f4`
    /// `IL_0021`-`IL_0044`.
    ///
    /// Written as a table so the *negative* arms are pinned too: the opening
    /// digests can only witness the turn-1 owner case, and a body missing the
    /// turn guard would pass every one of them while healing on every later
    /// turn of the same fight.
    #[test]
    fn blood_vial_heals_two_only_on_a_live_first_owner_turn() {
        let owned = catalog(&[RelicId::RelicBloodVial]);
        let unowned = catalog(&[]);
        for (label, catalog, turn, over, hp, expected) in [
            ("turn 1, owned", &owned, 1, false, 68, 70),
            ("turn 2, owned", &owned, 2, false, 68, 68),
            ("turn 1, ended", &owned, 1, true, 68, 68),
            ("turn 1, unowned", &unowned, 1, false, 68, 68),
            // The clamp, and that it never *lowers* HP.
            ("turn 1, one below max", &owned, 1, false, 79, 80),
            ("turn 1, at max", &owned, 1, false, 80, 80),
        ] {
            let mut state = HotState::at_defaults();
            state.turn = turn;
            state.history.over = over;
            state.max_hp = 80;
            state.hp = hp;
            after_player_turn_start_late(catalog, &mut state, &mut Vec::new()).unwrap();
            assert_eq!(state.hp, expected, "{label}");
        }
    }

    /// #3044: Blood Vial's heal and a compiled template heal (Fake Blood
    /// Vial's, on the same hook) are each a `CreatureCmd.Heal`, whose
    /// `AfterCurrentHpChanged` reaches Red Skull. The first one past half HP
    /// removes the Strength; the other then finds nothing to do, so the two
    /// still commute.
    #[test]
    fn blood_vial_and_template_heals_are_heals_red_skull_hears() {
        let skull = |hp| {
            let mut state = HotState::at_defaults();
            state.turn = 1;
            state.max_hp = 80;
            state.hp = hp;
            state.fanouts.set_red_skull_owned(true);
            state.powers.set(PowerId::Strength, SlotWire::Int, 3);
            state
        };
        let vial = catalog(&[RelicId::RelicBloodVial, RelicId::RelicRedSkull]);
        let mut state = skull(39);
        after_player_turn_start_late(&vial, &mut state, &mut Vec::new()).unwrap();
        assert_eq!((state.hp, state.powers.value(PowerId::Strength)), (41, 0));
        let mut short = skull(37);
        after_player_turn_start_late(&vial, &mut short, &mut Vec::new()).unwrap();
        assert_eq!((short.hp, short.powers.value(PowerId::Strength)), (39, 3));

        let fake = catalog(&[RelicId::RelicFakeBloodVial, RelicId::RelicRedSkull]);
        let mut state = skull(40);
        super::super::fire_hook(
            &fake,
            HookEvent::AfterPlayerTurnStartLate,
            &mut state,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((state.hp, state.powers.value(PowerId::Strength)), (41, 0));
    }

    #[test]
    fn batch_four_shuffle_discard_exhaust_and_pet_hooks_match_python_constants() {
        let catalog = catalog(&[
            RelicId::RelicBoneFlute,
            RelicId::RelicCharonsAshes,
            RelicId::RelicTheAbacus,
            RelicId::RelicTingsha,
            RelicId::RelicToughBandages,
        ]);
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        let mut first = HotMonster::new(MonsterKind::Toadpole, 20);
        first.block = 1;
        state.monsters_mut().push(first);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        after_shuffle(&catalog, &mut state, &mut Vec::new()).unwrap();
        after_card_discarded(&catalog, &mut state, &mut Vec::new()).unwrap();
        after_card_exhausted(
            &catalog,
            &mut state,
            HotCard {
                uid: 0,
                atom: 0,
                flags: 0,
            },
            false,
            &mut Vec::new(),
        )
        .unwrap();
        after_pet_attack(&catalog, &mut state, &mut Vec::new()).unwrap();

        assert_eq!(state.block, 11); // Abacus 6 + Bandages 3 + Bone Flute 2.
        assert_eq!((state.monsters[0].hp, state.monsters[0].block), (15, 0));
        assert_eq!(state.monsters[1].hp, 17);
        assert_eq!(state.rng.get(crate::hot::RngStream::Targets).counter, 1);
    }

    /// #3650: Tingsha (`0x3326e0`) and Tough Bandages (`0x332e2c`) leave at
    /// IL_005f unless `CurrentSide` is their owner's side (IL_0038-IL_005d),
    /// before Tingsha rolls its target. No admitted state discards on the
    /// enemy side, so this sets the marker directly. The live engine does the
    /// same with `CurrentSide` forced to Enemy around a Storm of Steel: four
    /// discards, no damage and no Block.
    #[test]
    fn discard_relics_answer_only_on_the_owners_side() {
        let catalog = catalog(&[RelicId::RelicTingsha, RelicId::RelicToughBandages]);
        let fight = |owner_side: bool| {
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            state.player_side_active = owner_side;
            let mut events = Vec::new();
            for _ in 0..4 {
                after_card_discarded(&catalog, &mut state, &mut events).unwrap();
            }
            (
                state.block,
                state.monsters[0].hp,
                state.rng.get(crate::hot::RngStream::Targets).counter,
                events.len(),
            )
        };
        assert_eq!(fight(false), (0, 20, 0, 0), "enemy side: neither relic");
        let (block, hp, rolls, events) = fight(true);
        assert_eq!(
            (block, hp, rolls),
            (12, 8, 4),
            "owner side: 3 and 3, four times"
        );
        assert!(events > 0);
    }

    /// Bone Flute's `BlockVar(2m, ValueProp.Unpowered)` (`get_CanonicalVars`
    /// RVA `0x90ece`): exactly 2 Block per pet attack, with Dexterity and
    /// Frail both present and neither applied (#2830). A powered path would
    /// give `(2 + 3) * 3/4 = 3`; the pre-fix constant gave 4.
    #[test]
    fn bone_flute_grants_two_unpowered_block_despite_dexterity_and_frail() {
        use crate::powers::SlotWire;
        let owned = catalog(&[RelicId::RelicBoneFlute]);
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 3);
        state.powers.set(PowerId::PlayerFrail, SlotWire::Int, 2);
        state.block = 1;
        let mut events = Vec::new();
        after_pet_attack(&owned, &mut state, &mut events).unwrap();
        assert_eq!(state.block, 3, "1 existing + 2 unpowered");
        assert!(events.iter().any(|e| matches!(
            e,
            Event::PlayerBlockGained {
                amount: 2,
                block: 3
            }
        )));

        // Negative arms: no relic, or the combat already over, gain nothing.
        let mut unowned = HotState::at_defaults();
        after_pet_attack(&catalog(&[]), &mut unowned, &mut Vec::new()).unwrap();
        assert_eq!(unowned.block, 0);
        let mut over = HotState::at_defaults();
        over.history.over = true;
        after_pet_attack(&owned, &mut over, &mut Vec::new()).unwrap();
        assert_eq!(over.block, 0);
    }

    #[test]
    fn batch_three_turn_end_relics_use_the_pre_flush_hand_and_frozen_targets() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[
                RelicId::RelicCloakClasp,
                RelicId::RelicRippleBasin,
                RelicId::RelicStoneCalendar,
            ])
            .unwrap();
        let multi_catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 7;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));

        before_side_turn_end_hand(&multi_catalog, &mut state, false, false, &mut Vec::new())
            .unwrap();

        assert_eq!(state.block, 6);
        assert_eq!(state.monsters[0].hp, 48);

        let catalog = catalog(&[RelicId::RelicScreamingFlagon]);
        let mut empty = HotState::at_defaults();
        empty.hp = 80;
        empty.max_hp = 80;
        empty
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        before_side_turn_end_hand(&catalog, &mut empty, false, false, &mut Vec::new()).unwrap();
        assert_eq!(empty.monsters[0].hp, 80);
    }

    #[test]
    fn batch_three_card_play_relics_use_actual_spend_and_card_tags() {
        let mut builder = CatalogBuilder::new();
        let shiv_atom = builder
            .intern(CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[
                RelicId::RelicDaughterOfTheWind,
                RelicId::RelicHelicalDart,
                RelicId::RelicIntimidatingHelmet,
                RelicId::RelicIvoryTile,
            ])
            .unwrap();
        let catalog = builder.build();
        let shiv = *catalog.spec(shiv_atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 0;

        intimidating_helmet_before_card_played(&catalog, &mut state, 3, &mut Vec::new()).unwrap();
        after_card_played_hand(&catalog, &mut state, &shiv, 0, Some(3), &mut Vec::new()).unwrap();

        assert_eq!(state.block, 5);
        assert_eq!(state.energy, 1);
        assert_eq!(state.powers.value(PowerId::TempDexterity), 1);
        assert!(
            state
                .fanouts
                .after_side_turn_end_power_uid(
                    crate::hot::AfterSideTurnEndPowerToken::TemporaryDexterity
                )
                .is_some()
        );
    }

    #[test]
    fn lost_wisp_is_unpowered_blockable_aoe_after_a_power() {
        let mut builder = CatalogBuilder::new();
        let power_atom = builder
            .intern(CardIdentity {
                id: CardId::DemonForm,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicLostWisp]).unwrap();
        let catalog = builder.build();
        let power = *catalog.spec(power_atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        let mut first = HotMonster::new(MonsterKind::Toadpole, 20);
        first.block = 3;
        state.monsters_mut().push(first);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        after_card_played_hand(&catalog, &mut state, &power, 0, Some(3), &mut Vec::new()).unwrap();

        assert_eq!(state.monsters[0].hp, 15);
        assert_eq!(state.monsters[0].block, 0);
        assert_eq!(state.monsters[1].hp, 12);
    }

    #[test]
    fn turn_one_templates_match_the_compiled_python_programs() {
        let catalog = catalog(&[
            RelicId::RelicDiamondDiadem,
            RelicId::RelicDivineDestiny,
            RelicId::RelicSai,
            RelicId::RelicVeryHotCocoa,
        ]);
        let mut state = HotState::at_defaults();
        // Reset grants check IsEnding, which is trivially true for a
        // monsterless state; relic turns run in live combat.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.turn = 1;
        let mut events = Vec::new();
        crate::engine::fire_hook(
            &catalog,
            HookEvent::AfterSideTurnStart,
            &mut state,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.block, 27);
        assert_eq!(state.powers.value(PowerId::Blur), 1);
        assert_eq!(state.stars, 7);
        assert_eq!(state.energy, 7);
        assert_eq!(state.fanouts.after_side_turn_start_order().len(), 1);
    }

    #[test]
    fn turn_three_templates_apply_block_and_signed_stats() {
        let catalog = catalog(&[RelicId::RelicCaptainsWheel, RelicId::RelicSparklingRouge]);
        let mut state = HotState::at_defaults();
        state.turn = 3;
        let mut events = Vec::new();
        crate::engine::fire_hook(
            &catalog,
            HookEvent::AfterBlockCleared,
            &mut state,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.block, 18);
        assert_eq!(state.powers.value(PowerId::Strength), 1);
        assert_eq!(state.powers.value(PowerId::Dexterity), 1);
    }

    #[test]
    fn hand_authored_turn_gates_match_python_constants() {
        let catalog = catalog(&[
            RelicId::RelicCandelabra,
            RelicId::RelicChandelier,
            RelicId::RelicHornCleat,
        ]);
        let mut state = HotState::at_defaults();
        state.energy = 3;
        state.turn = 2;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        after_block_cleared(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!((state.energy, state.block), (5, 14));

        state.energy = 3;
        state.block = 0;
        state.turn = 3;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        after_block_cleared(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!((state.energy, state.block), (6, 0));
    }

    #[test]
    fn orichalcum_latches_before_plating_and_applies_real_then_fake() {
        let catalog = catalog(&[RelicId::RelicOrichalcum, RelicId::RelicFakeOrichalcum]);
        let mut state = HotState::at_defaults();
        let latches = orichalcum_latches(&catalog, &state);
        state.block = 4; // Plating's later Early-pass gain.
        before_side_turn_end_hand(&catalog, &mut state, latches.0, latches.1, &mut Vec::new())
            .unwrap();
        assert_eq!(state.block, 13);

        state.block = 1;
        assert_eq!(orichalcum_latches(&catalog, &state), (false, false));
    }

    #[test]
    fn batch_five_draw_counters_match_the_python_lifecycles() {
        let catalog = catalog(&[
            RelicId::RelicFiddle,
            RelicId::RelicPendulum,
            RelicId::RelicPocketwatch,
            RelicId::RelicPollinousCore,
            RelicId::RelicRingOfTheDrake,
        ]);
        let mut state = HotState::at_defaults();
        state.turn = 2;
        state.history.owner_card_plays_finished_this_turn = 3;
        assert!(state.set_pendulum(2));
        assert!(state.set_pollinous_core(3));

        before_side_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        state.history.owner_card_plays_finished_this_turn = 0;
        before_hand_draw(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        // `BeforeHandDraw` leaves the fourth turn counted; the fold reads and
        // resets it.
        assert_eq!(state.pollinous_core(), 4);
        let bonus = modifier_total(&catalog, HookEvent::ModifyHandDraw, &state).unwrap()
            + pollinous_core_modify_hand_draw(&catalog, &mut state);

        assert_eq!(state.pocketwatch_last_plays(), 3);
        assert_eq!(state.pendulum(), 0);
        assert_eq!(state.pollinous_core(), 0);
        assert_eq!(bonus, 10); // Fiddle 2 + Pendulum 1 + Pocketwatch 3 + Core 2 + Drake 2.
    }

    /// Every turn-1 `ModifyHandDraw` row reaches the total, penalties included.
    ///
    /// Big Mushroom's `-2` used to live in a `match` arm of its own guarded by
    /// `state.turn == 1`, which made it **exclusive** with the rows below it: a
    /// turn-1 hand draw holding Big Mushroom dropped Fiddle, Pendulum, Ring of
    /// the Drake, Booming Conch and Snecko Eye. Python runs them as separate
    /// statements over one running total (`_begin_player_turn_hand_draw`, frozen Python, deleted #2827) and native as separate listeners, so the correct answer
    /// here is the sum. The two bag relics are turn-1-only, so they would have
    /// been swallowed exactly when they apply.
    #[test]
    fn the_turn_one_hand_draw_rows_sum_instead_of_excluding_each_other() {
        let four = catalog(&[
            RelicId::RelicBagOfPreparation,
            RelicId::RelicBigMushroom,
            RelicId::RelicFiddle,
            RelicId::RelicPendulum,
        ]);
        let mut state = HotState::at_defaults();
        state.turn = 1;
        // Pendulum's granting position, so all four rows are live at once.
        assert!(state.set_pendulum(0));
        assert_eq!(
            modifier_total(&four, HookEvent::ModifyHandDraw, &state),
            // Big Mushroom -2, Bag of Preparation +2, Fiddle +2, Pendulum +1.
            Ok(3),
        );

        // Past the first player turn only the two turn-agnostic rows remain.
        state.turn = 2;
        assert_eq!(
            modifier_total(&four, HookEvent::ModifyHandDraw, &state),
            Ok(3),
        );

        // The bag pair is one native body twice over, and each contributes on
        // its own: `BagOfPreparation::ModifyHandDraw` `0x90434` and
        // `RingOfTheSnake::ModifyHandDraw` `0x9a749` are the same three-part
        // shape with the same `CardsVar(2)`, which is why the oracle sums them
        // into one `bag_draws` field.
        let mut state = HotState::at_defaults();
        state.turn = 1;
        for (relics, expected) in [
            (vec![RelicId::RelicBagOfPreparation], 2),
            (vec![RelicId::RelicRingOfTheSnake], 2),
            (
                vec![RelicId::RelicBagOfPreparation, RelicId::RelicRingOfTheSnake],
                4,
            ),
        ] {
            let bag_catalog = catalog(&relics);
            assert_eq!(
                modifier_total(&bag_catalog, HookEvent::ModifyHandDraw, &state),
                Ok(expected),
                "{relics:?}"
            );
        }
    }

    #[test]
    fn batch_five_energy_latches_advance_and_roll_over_exactly() {
        let catalog = catalog(&[
            RelicId::RelicArtOfWar,
            RelicId::RelicFakeHappyFlower,
            RelicId::RelicHappyFlower,
            RelicId::RelicPaelsTears,
        ]);
        let mut state = HotState::at_defaults();
        // Reset grants check IsEnding, which is trivially true for a
        // monsterless state; relic turns run in live combat.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.turn = 2;
        state.energy = 3;
        assert!(state.set_flower(2));
        assert!(state.set_fake_flower(4));

        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(
            (state.flower(), state.fake_flower(), state.energy),
            (0, 0, 5)
        );

        after_before_side_turn_end(&catalog, &mut state);
        assert!(state.paels_tears_had_leftover_energy());
        state.energy = 0;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.energy, 2);

        state.set_art_of_war_current_attack(true);
        after_side_turn_end_relics(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert!(state.art_of_war_last_attack());
        state.energy = 3;
        after_energy_reset(&catalog, &mut state).unwrap();
        assert_eq!(state.energy, 3);
        assert!(!state.art_of_war_last_attack());
        after_energy_reset(&catalog, &mut state).unwrap();
        assert_eq!(state.energy, 4);
    }

    #[test]
    fn batch_six_turn_one_resource_and_fixed_generation_relics_match_python() {
        let mut builder = CatalogBuilder::new();
        let anger = builder
            .intern_reachable(CardIdentity {
                id: CardId::Anger,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let strike = builder
            .intern_reachable(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        for id in [CardId::Soul, CardId::Luminesce] {
            builder
                .intern_reachable(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder
            .set_relics(&[
                RelicId::RelicBoomingConch,
                RelicId::RelicFakeVenerableTeaSet,
                RelicId::RelicFuneraryMask,
                RelicId::RelicPowerCell,
                RelicId::RelicRadiantPearl,
                RelicId::RelicVenerableTeaSet,
            ])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // Reset grants check IsEnding, which is trivially true for a
        // monsterless state; relic turns run in live combat.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.energy = 3;
        state.next_card_uid = 2;
        state.set_booming_conch_elite(true);
        state.set_fake_tea_set_charged(true);
        state.set_tea_set_charged(true);
        let seeded = Xoshiro256StarStar::from_seed(7);
        for stream in [RngStream::Rng, RngStream::Sel] {
            state.rng.set(
                stream,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
        }
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 0,
                atom: anger,
                flags: 0,
            },
            HotCard {
                uid: 1,
                atom: strike,
                flags: 0,
            },
        ]);

        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyHandDraw, &state),
            Ok(2)
        );
        before_side_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].atom, anger);
        after_energy_reset(&catalog, &mut state).unwrap();
        assert_eq!(state.energy, 6);
        assert!(!state.fake_tea_set_charged());
        before_hand_draw(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Draw).len(), 4);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        let pearl = catalog
            .spec(state.piles.get(PileId::Hand).as_slice()[1].atom)
            .unwrap();
        assert_eq!(pearl.identity.id, CardId::Luminesce);
    }

    #[test]
    fn batch_six_vexing_and_royal_poison_match_python_turn_one_bodies() {
        let mut vexing_builder = CatalogBuilder::new();
        for identity in crate::boundary::stoke_generation_closure([true, false]) {
            vexing_builder.intern_reachable(identity).unwrap();
        }
        vexing_builder
            .set_relics(&[RelicId::RelicVexingPuzzlebox])
            .unwrap();
        let vexing_catalog = vexing_builder.build();
        let mut vexing = HotState::at_defaults();
        vexing.hp = 80;
        vexing.max_hp = 80;
        vexing.turn = 1;
        vexing.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        vexing.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        vexing.fully_unlocked_card_pool_epochs = true;
        let seeded = Xoshiro256StarStar::from_seed(11);
        vexing.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        after_player_turn_start_from_entry(&vexing_catalog, &mut vexing).unwrap();
        assert_eq!(vexing.piles.get(PileId::Hand).len(), 1);

        let royal_catalog = catalog(&[RelicId::RelicRoyalPoison]);
        let mut royal = HotState::at_defaults();
        royal.hp = 80;
        royal.max_hp = 80;
        royal.turn = 1;
        after_player_turn_start_from_entry(&royal_catalog, &mut royal).unwrap();
        assert_eq!(royal.hp, 76);
    }

    /// #3381: Brimstone runs at its recorded side of Crossbow.
    ///
    /// With a live Arsenal (1) and Ruined Helmet, whichever Strength
    /// application comes first is doubled. Crossbow first: Arsenal's 1 is
    /// doubled to 2, then Brimstone adds 2, for 4. Brimstone first: its 2 is
    /// doubled to 4, then Arsenal adds 1, for 5. A vouched Brimstone-first
    /// inventory takes the second order: the walk runs the two as recorded
    /// (#3635). An unvouched inventory in either order keeps the fixed
    /// Crossbow-first order, which admission refuses. Each enemy gets its 1
    /// Strength exactly once in every order.
    #[test]
    fn brimstone_runs_on_its_recorded_side_of_crossbow() {
        use crate::catalog::RewardPool;
        let pool = crate::steps::neutral::crossbow_owner_attack_pool(RewardPool::Ironclad, None);
        let run = |order: [RelicId; 3], vouched: bool| {
            let mut builder = CatalogBuilder::new();
            for id in &pool {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.set_relics_ordered(&order, vouched).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 2;
            state.reward_card_pool = Some(RewardPool::Ironclad);
            state.fully_unlocked_card_pool_epochs = true;
            let rng = Xoshiro256StarStar::from_seed(29);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            state.powers.set(PowerId::Arsenal, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_local_generated_power_order(&[PowerId::Arsenal])
            );
            state
                .fanouts
                .set_batch_eight_deep_relic_ownership(false, false, false, false, true);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            assert_eq!(state.piles.get(PileId::Hand).len(), 1, "one Crossbow card");
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                1,
                "Brimstone's enemy Strength, once"
            );
            state.powers.value(PowerId::Strength)
        };
        use RelicId::{RelicBrimstone as B, RelicCrossbow as C, RelicRuinedHelmet as H};
        assert_eq!(run([B, C, H], true), 5);
        assert_eq!(run([C, B, H], true), 4);
        assert_eq!(run([B, C, H], false), 4);
        assert_eq!(run([C, B, H], false), 4);
    }

    /// #2970: every arm of the Crossbow turn-start body, and Big Hat's
    /// remaining Ironclad-only fence (#3264).
    ///
    /// Each refusal is by its own name and atomic; each draw is the owner's
    /// pool under the catalog's recorded profile, consumed as one full
    /// Generation shuffle, and lands at the Hand's bottom (Crossbow's free
    /// this turn).
    #[test]
    fn crossbow_draws_the_owner_pool_and_each_unmodeled_arm_refuses_by_name() {
        use crate::catalog::RewardPool;
        let seeded = |state: &mut HotState| {
            let rng = Xoshiro256StarStar::from_seed(29);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
        };
        // A catalog holding `relic` and the owner pool it draws from, under a
        // partial `profile` when one is given.
        let pooled = |relic: RelicId, pool: &[CardId], profile: Option<Vec<&'static str>>| {
            let mut builder = CatalogBuilder::new();
            if let Some(epochs) = profile {
                builder.set_splash_unlock_epochs(epochs);
            }
            for id in pool {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            builder.set_relics(&[relic]).unwrap();
            builder.build()
        };
        let base = |owner: Option<RewardPool>, full: bool| {
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 1;
            state.reward_card_pool = owner;
            state.fully_unlocked_card_pool_epochs = full;
            state
        };
        let refusal = |catalog: &Catalog, state: &HotState| {
            let mut probe = state.clone();
            let refused = after_side_turn_start(catalog, &mut probe, true, &mut Vec::new())
                .expect_err("refuses");
            assert_eq!(&probe, state, "a refusal publishes nothing");
            refused
        };
        let hand_ids = |catalog: &Catalog, state: &HotState| -> Vec<CardId> {
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect()
        };

        // Crossbow provenance: owner, recorded profile, solo, live stream.
        let crossbow = catalog(&[RelicId::RelicCrossbow]);
        let provenance = EngineRefusal::MalformedArgs("Crossbow generation provenance");
        let mut live = base(Some(RewardPool::Ironclad), true);
        seeded(&mut live);
        let mut no_owner = live.clone();
        no_owner.reward_card_pool = None;
        let mut unrecorded = live.clone();
        unrecorded.fully_unlocked_card_pool_epochs = false;
        let mut vacant = live.clone();
        vacant.rng = base(None, true).rng;
        assert!(vacant.rng.is_vacant(RngStream::Generation));
        let mut party = live.clone();
        party.multiplayer_ally_key = 1;
        for state in [&no_owner, &unrecorded, &vacant, &party] {
            assert_eq!(refusal(&crossbow, state), provenance);
        }

        // Crossbow draws: a non-Ironclad owner at the full profile, and an
        // Ironclad under a partial profile (a hidden foreign epoch and a
        // hidden Ironclad Attack epoch), each the full-shuffle head of the
        // owner's derived pool, free this turn.
        let ironclad_gate = crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101
            .iter()
            .find(|(name, _)| *name == "IRONCLAD")
            .unwrap()
            .1
            .iter()
            .find_map(|(id, epoch)| {
                epoch.filter(|_| {
                    crate::steps::neutral::crossbow_owner_attack_pool(RewardPool::Ironclad, None)
                        .contains(id)
                })
            })
            .expect("an epoch-gated Ironclad Attack");
        let mut partial: Vec<&'static str> = crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .copied()
            .filter(|epoch| *epoch != "DEFECT7_EPOCH" && *epoch != ironclad_gate)
            .collect();
        partial.sort_unstable();
        for (owner, profile) in [
            (RewardPool::Necrobinder, None),
            (RewardPool::Ironclad, Some(partial.clone())),
        ] {
            let epochs = profile.clone();
            let pool = crate::steps::neutral::crossbow_owner_attack_pool(owner, epochs.as_deref());
            let catalog = pooled(RelicId::RelicCrossbow, &pool, profile.clone());
            let mut state = base(Some(owner), profile.is_none());
            seeded(&mut state);
            let mut expected_rng = state.clone();
            let head = crate::engine::cards::shuffle_generation_slice(&mut expected_rng, &pool)
                .unwrap()[0];
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            assert_eq!(hand_ids(&catalog, &state), vec![head], "{owner:?}");
            assert_eq!(
                state.rng.get(RngStream::Generation),
                expected_rng.rng.get(RngStream::Generation),
                "{owner:?}: one full shuffle of the derived pool"
            );
            let card = &state.piles.get(PileId::Hand).as_slice()[0];
            assert_eq!(
                state
                    .card_states
                    .get(card.uid)
                    .free_star_cost_this_turn_or_played_rows,
                1,
                "{owner:?}: free this turn"
            );
            if owner == RewardPool::Ironclad {
                let full =
                    crate::steps::neutral::crossbow_owner_attack_pool(RewardPool::Ironclad, None);
                assert!(pool.len() < full.len(), "the partial profile gates a row");
            }
        }
        // A pass entered on an ended combat runs no listener.
        let mut over = live.clone();
        over.history.over = true;
        let before = over.clone();
        after_side_turn_start(&crossbow, &mut over, false, &mut Vec::new()).unwrap();
        assert_eq!(over, before);
        // A pass that began live and reached Crossbow ended (here a power of
        // the pass ended it) still runs the body in the game: it draws the
        // Generation stream and adds no card (#3635).
        after_side_turn_start(&crossbow, &mut over, true, &mut Vec::new()).unwrap();
        assert_ne!(
            over.rng.get(RngStream::Generation),
            before.rng.get(RngStream::Generation)
        );
        assert_eq!(over.piles, before.piles, "no card");

        // Big Hat stays Ironclad-only (#3264): a non-Ironclad owner refuses by
        // name, atomically; the Ironclad's empty pool draws nothing on any
        // profile and needs no stream.
        let big_hat = catalog(&[RelicId::RelicBigHat]);
        let necrobinder = base(Some(RewardPool::Necrobinder), true);
        assert_eq!(
            refusal(&big_hat, &necrobinder),
            EngineRefusal::MalformedArgs("Big Hat owner Ethereal pool beyond Ironclad (#3264)")
        );
        for full in [true, false] {
            let mut ironclad = base(Some(RewardPool::Ironclad), full);
            let before = ironclad.clone();
            after_side_turn_start(&big_hat, &mut ironclad, true, &mut Vec::new()).unwrap();
            assert_eq!(ironclad, before, "{full}");
        }
    }

    #[test]
    fn fiddle_blocks_only_owner_command_draws_on_the_player_side() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicFiddle]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.player_side_active = true;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: 0,
        });

        crate::engine::draw::draw_cards(
            &mut state,
            &catalog,
            1,
            crate::engine::draw::DrawSource::Command,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.piles.get(PileId::Hand).is_empty());

        crate::engine::draw::draw_cards(
            &mut state,
            &catalog,
            1,
            crate::engine::draw::DrawSource::HandDraw,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    /// The resumable Draw entries (a card body's owned tail, a result-bearing
    /// body, a potion) owe `Fiddle::ShouldDraw` (RVA `0x939ab`) like the plain
    /// command: Big Bang and Scrawl drew through Fiddle before this gate.
    #[test]
    fn fiddle_blocks_resumable_command_draws_on_the_player_side() {
        use crate::engine::draw::{
            CardDrawResult, PotionDrawResult, draw_cards_for_card_result, draw_cards_for_potion,
        };
        use crate::hot::DrawCaller;
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicFiddle]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.player_side_active = true;
        for uid in 1..=3 {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        let before = state.clone();

        for caller in [DrawCaller::CardPlay, DrawCaller::PotionEpilogue] {
            assert_eq!(
                draw_cards_for_potion(&mut state, &catalog, 1, caller, &mut Vec::new()).unwrap(),
                PotionDrawResult::Complete
            );
            assert_eq!(state, before, "{caller:?}");
        }
        assert_eq!(
            draw_cards_for_card_result(
                &mut state,
                &catalog,
                1,
                DrawCaller::CardPlay,
                &mut Vec::new()
            )
            .unwrap(),
            CardDrawResult::Complete(Vec::new())
        );
        assert_eq!(state, before);

        // The enemy side is not the owner's turn: the same command draws.
        let mut enemy_side = before.clone();
        enemy_side.player_side_active = false;
        draw_cards_for_potion(
            &mut enemy_side,
            &catalog,
            1,
            DrawCaller::CardPlay,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(enemy_side.piles.get(PileId::Hand).len(), 1);

        // The turn-start HandDraw is `fromHandDraw` and is never vetoed.
        draw_cards_for_potion(
            &mut state,
            &catalog,
            1,
            DrawCaller::TurnStart,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    #[test]
    fn batch_seven_energy_draw_generation_and_parry_match_python_constants() {
        let mut builder = CatalogBuilder::new();
        let dazed = builder
            .intern_reachable(CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109 {
            builder
                .intern_reachable(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder
            .set_relics(&[
                RelicId::RelicBlessedAntler,
                RelicId::RelicCrossbow,
                RelicId::RelicParryingShield,
                RelicId::RelicPhilosophersStone,
                RelicId::RelicSneckoEye,
            ])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.block = 10;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.monsters_mut().extend([
            HotMonster::new(MonsterKind::Toadpole, 20),
            HotMonster::new(MonsterKind::Toadpole, 20),
        ]);
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(11);
        for stream in [RngStream::Rng, RngStream::Generation, RngStream::Targets] {
            state.rng.set(
                stream,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
        }

        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(2)
        );
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyHandDraw, &state),
            Ok(2)
        );
        before_hand_draw(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Draw).len(), 3);
        assert!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .all(|card| card.atom == dazed)
        );

        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        let generated = state.piles.get(PileId::Hand).as_slice()[0];
        let generated_spec = catalog.spec(generated.atom).unwrap();
        assert!(
            crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109
                .contains(&generated_spec.identity.id)
        );
        assert_eq!(
            state
                .card_states
                .get(generated.uid)
                .local_cost_modifiers
                .as_slice(),
            &[crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Set,
                amount: 0,
                expiration: crate::hot::LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            }]
        );

        after_side_turn_end_relics(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(
            state.monsters.iter().map(|monster| monster.hp).sum::<i32>(),
            34
        );
        assert_eq!(state.rng.get(RngStream::Targets).counter, 1);
    }

    #[test]
    fn batch_seven_completed_card_relics_tick_and_upgrade_the_exact_play() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern_reachable(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let upgraded = builder
            .intern_reachable(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern_reachable(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .intern_reachable(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[
                RelicId::RelicKunai,
                RelicId::RelicLetterOpener,
                RelicId::RelicNunchaku,
                RelicId::RelicOrnamentalFan,
                RelicId::RelicRainbowRing,
                RelicId::RelicRazorTooth,
                RelicId::RelicShuriken,
            ])
            .unwrap();
        let catalog = builder.build();
        let attack = *catalog.spec(strike).unwrap();
        let skill = *catalog.spec(defend).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.energy = 0;
        state.monsters_mut().extend([
            HotMonster::new(MonsterKind::Toadpole, 20),
            HotMonster::new(MonsterKind::Toadpole, 20),
        ]);
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom: strike,
            flags: 0,
        });
        assert!(state.set_nunchaku(9));
        assert!(state.set_kunai(2));
        assert!(state.set_shuriken(2));
        assert!(state.set_ornamental_fan(2));
        assert!(state.set_rainbow_ring_types_this_turn(6));

        after_card_played_hand(&catalog, &mut state, &attack, 7, Some(0), &mut Vec::new()).unwrap();
        assert_eq!((state.energy, state.block), (1, 4));
        assert_eq!(
            (
                state.nunchaku(),
                state.kunai(),
                state.shuriken(),
                state.ornamental_fan()
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(state.powers.value(PowerId::Strength), 2);
        assert_eq!(state.powers.value(PowerId::Dexterity), 2);
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].atom, upgraded);

        state.piles.get_mut(PileId::Play).make_mut()[0] = HotCard {
            uid: 99,
            atom: defend,
            flags: 0,
        };
        state.history.skill_plays_finished_this_turn = 3;
        after_card_played_hand(&catalog, &mut state, &skill, 99, Some(0), &mut Vec::new()).unwrap();
        assert_eq!(
            state.monsters.iter().map(|monster| monster.hp).sum::<i32>(),
            30
        );
    }

    #[test]
    fn batch_eight_counter_hooks_and_order_refusals_are_exact() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern_reachable(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern_reachable(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[
                RelicId::RelicBoundPhylactery,
                RelicId::RelicGalacticDust,
                RelicId::RelicIronClub,
                RelicId::RelicKusarigama,
                RelicId::RelicMiniRegent,
                RelicId::RelicPenNib,
                RelicId::RelicPhylacteryUnbound,
                RelicId::RelicTuningFork,
            ])
            .unwrap();
        let catalog = builder.build();
        let attack = *catalog.spec(strike).unwrap();
        let skill = *catalog.spec(defend).unwrap();

        let mut pen = HotState::at_defaults();
        assert!(pen.fanouts.set_pen_nib(9));
        assert!(before_card_played_hand(&catalog, &mut pen, &attack, 7).unwrap());
        assert_eq!(pen.fanouts.pen_nib(), 0);

        let mut counters = HotState::at_defaults();
        assert!(counters.fanouts.set_iron_club_cards(3));
        assert!(counters.fanouts.set_tuning_fork_skills(9));
        let before = counters.clone();
        assert!(matches!(
            after_card_played_hand(&catalog, &mut counters, &skill, 7, Some(0), &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Iron Club and Tuning Fork simultaneous thresholds"
            ))
        ));
        assert_eq!(counters, before);

        let mut kusarigama = HotState::at_defaults();
        assert!(kusarigama.fanouts.set_kusarigama(2));
        after_side_turn_end_relics(&catalog, &mut kusarigama, &mut Vec::new()).unwrap();
        assert_eq!(kusarigama.fanouts.kusarigama(), 0);

        let mut stars = HotState::at_defaults();
        stars.powers.set(PowerId::Juggernaut, SlotWire::Int, 3);
        stars
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 3));
        assert!(stars.fanouts.set_galactic_dust(9));
        let before = stars.clone();
        assert!(matches!(
            after_stars_spent(&catalog, &mut stars, 1, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Galactic Dust and Mini Regent listener order"
            ))
        ));
        assert_eq!(stars, before);

        let mut pet = HotState::at_defaults();
        // The Late Bound Phylactery summon skips while ending, which is
        // trivially true for a monsterless state.
        pet.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        pet.turn = 2;
        after_energy_reset(&catalog, &mut pet).unwrap();
        assert_eq!(pet.fanouts.pet().osty().map(|osty| osty.hp()), Some(1));
        after_side_turn_start(&catalog, &mut pet, true, &mut Vec::new()).unwrap();
        assert_eq!(pet.fanouts.pet().osty().map(|osty| osty.hp()), Some(3));
    }

    #[test]
    fn batch_nine_centennial_puzzle_draws_three_once_after_surviving_damage() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[RelicId::RelicCentennialPuzzle])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.next_card_uid = 4;
        state.fanouts.set_puzzle_armed(true);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=3).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        super::super::damage::damage_player_from_card_with_catalog(
            &mut state,
            &catalog,
            1,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((state.hp, state.piles.get(PileId::Hand).len()), (19, 3));
        assert!(!state.fanouts.puzzle_armed());

        super::super::damage::damage_player_from_card_with_catalog(
            &mut state,
            &catalog,
            1,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((state.hp, state.piles.get(PileId::Hand).len()), (18, 3));
    }

    #[test]
    fn batch_nine_history_course_replays_its_fallback_after_the_source_leaves_all_piles() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicHistoryCourse]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 2;
        state.next_card_uid = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(7);
        state.rng.set(
            RngStream::Targets,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state
            .fanouts
            .set_history_course_attack_previous_turn(Some(FrozenAutoBatchEntry {
                card: HotCard {
                    uid: 1,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                state: CardInstanceState::default(),
            }));

        let document = crate::boundary::HotBoundary::to_canonical(&state, &catalog);
        let mut loaded = crate::boundary::HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert!(crate::boundary::batch_nine_relic_state_is_exact(
            &loaded, &catalog
        ));
        super::super::turn::finish_auto_pre_relic_tail(&mut loaded, &catalog, &mut Vec::new())
            .unwrap();

        assert_eq!(loaded.monsters[0].hp, 14);
        assert_eq!(loaded.next_card_uid, 3);
        assert!(
            PileId::ALL
                .into_iter()
                .flat_map(|pile| loaded.piles.get(pile).as_slice())
                .all(|card| card.uid != 1)
        );
    }

    #[test]
    fn batch_nine_whispering_earring_rechecks_the_live_hand_after_each_play() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[RelicId::RelicWhisperingEarring])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 20;
        state.max_hp = 20;
        state.energy = 2;
        state.next_card_uid = 3;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=2).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        assert!(
            super::super::play::autoplay_whispering_earring_first(
                &mut state,
                &catalog,
                &mut Vec::new(),
            )
            .unwrap()
        );
        assert!(
            super::super::play::autoplay_whispering_earring_first(
                &mut state,
                &catalog,
                &mut Vec::new(),
            )
            .unwrap()
        );
        assert!(
            !super::super::play::autoplay_whispering_earring_first(
                &mut state,
                &catalog,
                &mut Vec::new(),
            )
            .unwrap()
        );
        assert_eq!((state.energy, state.block), (0, 10));
        assert!(state.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn batch_nine_choices_paradox_parks_five_options_and_retains_the_pick() {
        let mut builder = CatalogBuilder::new();
        let pool = generation_relic_pool(RelicPendingKind::ChoicesParadox).unwrap();
        for id in pool {
            builder
                .intern_reachable(CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.set_relics(&[RelicId::RelicChoicesParadox]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 1;
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(11);
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );

        after_player_turn_start_from_entry(&catalog, &mut state).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(5));
        resume_relic_selection(&mut state, &catalog, 0, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        let selected = state.piles.get(PileId::Hand).as_slice()[0];
        assert!(state.card_states.get(selected.uid).local_retain);
    }

    /// #3756, site witness `0KPA7UA3BF5V` node 34. The Paradox's pick keeps
    /// the Retain it was offered with (`0x321b0c` IL_00f2-IL_0105). When the
    /// deck already holds the same card, the two copies differ only by that
    /// keyword, so the fight is promoted to exact pile identity and both are
    /// offered as plays. With no twin nothing is promoted.
    #[test]
    fn choices_paradox_pick_beside_its_twin_promotes_exact_piles() {
        for twin in [false, true] {
            let mut builder = CatalogBuilder::new();
            let pool = generation_relic_pool(RelicPendingKind::ChoicesParadox).unwrap();
            for id in pool {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            builder.set_relics(&[RelicId::RelicChoicesParadox]).unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 20;
            state.max_hp = 20;
            state.turn = 1;
            state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(11);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            // Learn the first option, then put its plain twin in Hand.
            let mut probe = state.clone();
            probe.next_card_uid = 1;
            after_player_turn_start_from_entry(&catalog, &mut probe).unwrap();
            resume_relic_selection(&mut probe, &catalog, 0, &mut Vec::new()).unwrap();
            let picked_atom = probe.piles.get(PileId::Hand).as_slice()[0].atom;

            state.next_card_uid = 1;
            if twin {
                state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                    uid: 1,
                    atom: picked_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
                state.next_card_uid = 2;
            }
            assert!(!state.exact_piles);
            after_player_turn_start_from_entry(&catalog, &mut state).unwrap();
            resume_relic_selection(&mut state, &catalog, 0, &mut Vec::new()).unwrap();

            assert_eq!(state.exact_piles, twin);
            let hand = state.piles.get(PileId::Hand).as_slice().to_vec();
            assert_eq!(hand.len(), 1 + usize::from(twin));
            let retained = hand
                .iter()
                .filter(|card| state.card_states.get(card.uid).local_retain)
                .count();
            assert_eq!(retained, 1);
            if twin {
                assert_eq!(hand[0].atom, hand[1].atom);
                let offered = crate::engine::legal_actions(&state, &catalog)
                    .into_iter()
                    .filter_map(|action| match action {
                        crate::engine::Action::Play { uid, .. } => Some(uid),
                        _ => None,
                    })
                    .collect::<std::collections::BTreeSet<_>>();
                for card in &hand {
                    assert!(offered.contains(&card.uid), "uid {} is offered", card.uid);
                }
            }
        }
    }

    #[test]
    fn batch_nine_paels_eye_exhausts_the_hand_and_starts_one_extra_player_turn() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicPaelsEye]).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 2;
        state.history.round_number = 2;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        super::super::turn::end_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert_eq!(state.turn, 3);
        assert_eq!(state.history.round_number, 2);
        assert!(state.fanouts.paels_eye_used());
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.piles.get(PileId::Exhaust).len(), 1);
    }

    #[test]
    fn batch_nine_bookmark_and_pumpkin_candle_apply_local_cost_and_draw_modifiers() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[RelicId::RelicBookmark, RelicId::RelicPumpkinCandle])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        assert!(state.fanouts.set_pumpkin_candle_kindle_count(1));
        let card = HotCard {
            uid: 1,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };

        after_hand_flushed(&catalog, &mut state, &mut [card]).unwrap();

        let spec = catalog.spec(strike).unwrap();
        assert_eq!(
            super::super::play::resolved_local_energy_cost(&state, card, spec),
            0
        );
        assert_eq!(
            modifier_total(&catalog, HookEvent::ModifyMaxEnergy, &state),
            Ok(1)
        );
    }

    #[test]
    fn batch_nine_gambling_chip_discards_an_ordered_subset_and_draws_the_same_count() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicGamblingChip]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 5;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=2).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((3..=4).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        continue_after_choices_paradox(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(5));
        resume_relic_selection(&mut state, &catalog, 3, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert_eq!(state.piles.get(PileId::Discard).len(), 2);
    }

    /// #3125: a recorded Gambling Chip discard resolves to the one ordinal
    /// whose ordered subset it names, over the full `Σ_k P(5, k)` = 326
    /// offered answers, and an unrecorded card names none.
    #[test]
    fn recorded_gambling_chip_discard_resolves_to_its_ordered_ordinal() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicGamblingChip]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 10;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=5).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((6..=9).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        continue_after_choices_paradox(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(326));

        let recorded = [4, 2, 5, 1];
        let Some(crate::engine::SelectionAnswer::OptionIndex(ordinal)) =
            crate::engine::recorded_selection_answer(&state, &catalog, &recorded).unwrap()
        else {
            panic!("the recorded discard names one offered ordinal");
        };
        let entries = &state.fanouts.batch_nine_relic_pending().unwrap().entries;
        assert_eq!(
            gambling_selection(entries, ordinal)
                .unwrap()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            recorded
        );
        // Order is part of the answer: the reverse is a different ordinal.
        let reversed = crate::engine::recorded_selection_answer(&state, &catalog, &[1, 5, 2, 4])
            .unwrap()
            .unwrap();
        assert_ne!(
            reversed,
            crate::engine::SelectionAnswer::OptionIndex(ordinal)
        );
        // The empty discard is ordinal 0; a card outside the hand names none.
        assert_eq!(
            crate::engine::recorded_selection_answer(&state, &catalog, &[]),
            Ok(Some(crate::engine::SelectionAnswer::OptionIndex(0)))
        );
        assert_eq!(
            crate::engine::recorded_selection_answer(&state, &catalog, &[7]),
            Ok(None)
        );
    }

    #[test]
    fn batch_nine_toasty_mittens_exhausts_the_pick_before_gaining_strength() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicToastyMittens]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 3;
        // Two cards: one would auto-resolve without a choice (#3273).
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend([1, 2].map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(2));
        resume_relic_selection(&mut state, &catalog, 0, &mut Vec::new()).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 2);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        assert_eq!(state.powers.value(PowerId::Strength), 1);
    }

    /// #3273 witness at the relic body, outside any `StartTurn` frame: a
    /// one-card Hand is at Toasty Mittens' non-manual `MinSelect` of 1, so
    /// `FromHand` (`<FromHand>d__28` RVA `0x3e7568` IL_0167-018d) returns it
    /// unprompted; it is exhausted before Strength applies.
    #[test]
    fn toasty_mittens_exhausts_a_sole_hand_card_without_a_choice() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicToastyMittens]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert!(state.fanouts.batch_nine_relic_pending().is_none());
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 1);
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
        assert_eq!(state.powers.value(PowerId::Strength), 1);
    }

    #[test]
    fn toasty_mittens_pick_stays_in_hand_once_combat_is_over() {
        // #3041: ToastyMittens IL_00f2 awaits CardCmd.Exhaust, which returns
        // on IsOverOrEnding (0x3e06c8 IL_0020–0034) before detaching the card.
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicToastyMittens]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 3;
        // Two cards so the choice parks (#3273).
        let cards = [1, 2].map(|uid| HotCard {
            uid,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Hand).make_mut().extend(cards);

        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();
        state.history.over = true;
        resume_relic_selection(&mut state, &catalog, 0, &mut Vec::new()).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &cards);
        assert!(state.piles.get(PileId::Exhaust).is_empty());
        assert_eq!(state.history.owner_cards_exhausted_combat, 0);
    }

    #[test]
    fn paels_eye_leaves_the_rest_of_the_hand_after_a_lethal_exhaust_listener() {
        // #3041: PaelsEye/<BeforeSideTurnEndEarly>d__16 (0x32c36c) exhausts a
        // Hand snapshot with no loop gate; Charon's Ashes kills on the first
        // Exhaust, and every later CardCmd.Exhaust returns on IsOverOrEnding
        // (0x3e06c8 IL_0020–0034) with its card still in Hand.
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[RelicId::RelicPaelsEye, RelicId::RelicCharonsAshes])
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 2;
        state.history.round_number = 2;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        let cards = [1, 2].map(|uid| HotCard {
            uid,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Hand).make_mut().extend(cards);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));

        super::super::turn::end_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice(), &cards[..1]);
        assert_eq!(state.piles.get(PileId::Hand).as_slice(), &cards[1..]);
        assert_eq!(state.history.owner_cards_exhausted_combat, 1);
    }

    #[test]
    fn recorded_puzzlebox_card_is_in_mittens_selection_and_is_generated_once() {
        let mut builder = CatalogBuilder::new();
        for identity in crate::boundary::stoke_generation_closure([true, false]) {
            builder.intern_reachable(identity).unwrap();
        }
        builder
            .set_relics_ordered(
                &[RelicId::RelicVexingPuzzlebox, RelicId::RelicToastyMittens],
                true,
            )
            .unwrap();
        // Y3NULJSNND7N floor 48: native pre-selection Hand and generation
        // stream. Only this pair's hook window is certified by this fixture.
        let hand = [
            (CardId::Bloodletting, 1),
            (CardId::FightMe, 1),
            (CardId::Bash, 0),
            (CardId::Stampede, 1),
            (CardId::Tremble, 0),
            (CardId::Pyre, 1),
            (CardId::HowlFromBeyond, 1),
        ]
        .into_iter()
        .enumerate()
        .map(|(uid, (id, upgrade))| HotCard {
            uid: uid as u32,
            atom: builder
                .intern(CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                })
                .unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        })
        .collect();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.next_card_uid = 41;
        state.piles.set(PileId::Hand, HotPile::from_cards(hand));
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [
                    1213625501689665630,
                    7582735556604303185,
                    1987035550902429042,
                    15673710837299237479,
                ],
                counter: 1351,
            },
        );
        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(8));
        let generated = state.piles.get(PileId::Hand).as_slice()[7];
        assert_eq!(generated.uid, 41);
        assert_eq!(
            catalog.spec(generated.atom).unwrap().identity.id,
            CardId::Inflame
        );
        let expected_rng = RngStreamState {
            words: [
                6656125309049840867,
                745304177760808129,
                1838059888127698674,
                8164727771799619048,
            ],
            counter: 1428,
        };
        assert_eq!(state.rng.get(RngStream::Generation), expected_rng);
        let mut choose_generated = state.clone();
        resume_relic_selection(&mut choose_generated, &catalog, 7, &mut Vec::new()).unwrap();
        assert_eq!(
            choose_generated.piles.get(PileId::Exhaust).as_slice()[0].uid,
            41
        );
        // The recorded player chose Howl From Beyond, leaving Inflame in Hand.
        resume_relic_selection(&mut state, &catalog, 6, &mut Vec::new()).unwrap();
        assert!(state.pending.is_none());
        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 6);
        assert_eq!(state.powers.value(PowerId::Strength), 1);
        assert_eq!(
            state.piles.get(PileId::Hand).as_slice().last().unwrap().uid,
            41
        );
        assert_eq!(state.rng.get(RngStream::Generation), expected_rng);
    }

    #[test]
    fn puzzlebox_mittens_rejects_wrong_or_unrecorded_order_before_mutation() {
        for (order, recorded) in [
            (
                [RelicId::RelicVexingPuzzlebox, RelicId::RelicToastyMittens],
                false,
            ),
            (
                [RelicId::RelicToastyMittens, RelicId::RelicVexingPuzzlebox],
                false,
            ),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(&order, recorded).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            let before = state.clone();
            assert!(continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).is_err());
            assert_eq!(state, before);
        }
    }

    /// #3756: `ToastyMittens/<AfterPlayerTurnStart>d__6` (RVA `0x33285c`,
    /// `FromHand` at IL_0068, `Exhaust` at IL_00f2) and
    /// `VexingPuzzlebox/<AfterPlayerTurnStart>d__2` (RVA `0x333de0`,
    /// `AddGeneratedCardToCombat` at IL_00b7) in `Player.Relics` order.
    /// Recorded Puzzlebox first, the free card is among Toasty's candidates.
    /// Recorded Toasty first, the choice is over the dealt Hand alone, the
    /// `CombatCardGeneration` stream is untouched at the pause, and the card
    /// arrives once the pick is exhausted. Either way one card is generated,
    /// one exhausted and one Strength gained.
    #[test]
    fn toasty_mittens_runs_on_its_recorded_side_of_vexing_puzzlebox() {
        use RelicId::{RelicToastyMittens as TOASTY, RelicVexingPuzzlebox as PUZZLEBOX};
        for (relics, toasty_first) in [([TOASTY, PUZZLEBOX], true), ([PUZZLEBOX, TOASTY], false)] {
            let (catalog, mut state) = popper_puzzlebox_fixture(&relics, 20);
            // Any two cards the fixture's catalog already holds.
            let dealt = catalog
                .specs()
                .take(2)
                .map(|spec| catalog.atom(&spec.identity).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(dealt.len(), 2);
            state.piles.set(
                PileId::Hand,
                HotPile::from_cards(
                    dealt
                        .iter()
                        .enumerate()
                        .map(|(uid, atom)| HotCard {
                            uid: uid as u32,
                            atom: *atom,
                            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                        })
                        .collect(),
                ),
            );
            let stream_before = state.rng.get(RngStream::Generation);
            let mut events = Vec::new();
            continue_after_gambling_chip(&catalog, &mut state, &mut events).unwrap();
            let offered = if toasty_first { 2 } else { 3 };
            assert_eq!(
                relic_selection_action_count(&state, &catalog),
                Ok(offered),
                "{relics:?}"
            );
            assert_eq!(
                state.rng.get(RngStream::Generation) == stream_before,
                toasty_first,
                "{relics:?}: the Puzzlebox has not drawn at a Toasty-first pause"
            );
            resume_relic_selection(&mut state, &catalog, 0, &mut events).unwrap();
            assert!(state.pending.is_none(), "{relics:?}");
            assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 0);
            assert_eq!(state.powers.value(PowerId::Strength), 1, "{relics:?}");
            assert_eq!(
                state
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [1, 41],
                "{relics:?}: the dealt card, then the generated one"
            );
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                1428,
                "{relics:?}: one GetDistinctForCombat shuffle"
            );
        }
    }

    #[test]
    fn batch_nine_music_box_clones_one_attack_as_locally_ethereal() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicMusicBox]).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 1,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Play).make_mut().push(source);

        // The Attack's BeforeCardPlayed latches it (#3640).
        let spec = catalog.spec(strike).unwrap();
        assert!(!before_card_played_hand(&catalog, &mut state, spec, source.uid).unwrap());
        assert_eq!(state.fanouts.music_box_card_uid(), Some(source.uid));

        after_card_played_hand(
            &catalog,
            &mut state,
            catalog.spec(strike).unwrap(),
            source.uid,
            Some(1),
            &mut Vec::new(),
        )
        .unwrap();

        let clone = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!((clone.uid, clone.atom), (2, strike));
        assert!(state.card_states.get(clone.uid).local_ethereal());
        assert!(state.fanouts.music_box_used_this_turn());
        assert_eq!(state.fanouts.music_box_card_uid(), None);

        // Used: neither hook does anything for a later Attack this turn
        // (`0x972ac` IL_0034-IL_0040).
        assert!(!before_card_played_hand(&catalog, &mut state, spec, source.uid).unwrap());
        assert_eq!(state.fanouts.music_box_card_uid(), None);
        after_card_played_hand(
            &catalog,
            &mut state,
            spec,
            source.uid,
            Some(1),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);

        // An unlatched Attack's AfterCardPlayed clones nothing, whatever
        // `WasUsedThisTurn` says (`0x32ae04` IL_001e-IL_002e), and a latch
        // naming another card is left for that card.
        for latch in [None, Some(9)] {
            let mut unlatched = HotState::at_defaults();
            unlatched.hp = 20;
            unlatched.max_hp = 20;
            unlatched.next_card_uid = 10;
            unlatched
                .piles
                .get_mut(PileId::Play)
                .make_mut()
                .push(source);
            unlatched.fanouts.set_music_box_card_uid(latch);
            after_card_played_hand(
                &catalog,
                &mut unlatched,
                spec,
                source.uid,
                Some(1),
                &mut Vec::new(),
            )
            .unwrap();
            assert!(unlatched.piles.get(PileId::Hand).is_empty(), "{latch:?}");
            assert!(!unlatched.fanouts.music_box_used_this_turn(), "{latch:?}");
            assert_eq!(unlatched.fanouts.music_box_card_uid(), latch);
            // A taken latch is not replaced (`0x972ac` IL_000d-IL_0019).
            before_card_played_hand(&catalog, &mut unlatched, spec, source.uid).unwrap();
            assert_eq!(
                unlatched.fanouts.music_box_card_uid(),
                latch.or(Some(source.uid))
            );
        }
        // A Skill never latches (IL_0042-IL_004f), and neither does an
        // Attack without the relic.
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let unowned = builder.build();
        let mut fresh = HotState::at_defaults();
        before_card_played_hand(&catalog, &mut fresh, unowned.spec(defend).unwrap(), 1).unwrap();
        before_card_played_hand(&unowned, &mut fresh, unowned.spec(strike).unwrap(), 1).unwrap();
        assert_eq!(fresh.fanouts.music_box_card_uid(), None);
    }

    #[test]
    fn batch_nine_mummified_hand_makes_one_live_hand_card_free() {
        let mut builder = CatalogBuilder::new();
        let power = builder
            .intern(CardIdentity {
                id: CardId::DemonForm,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicMummifiedHand]).unwrap();
        let catalog = builder.build();
        let played = HotCard {
            uid: 1,
            atom: power,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let candidate = HotCard {
            uid: 2,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Play).make_mut().push(played);
        state.piles.get_mut(PileId::Hand).make_mut().push(candidate);

        after_card_played_hand(
            &catalog,
            &mut state,
            catalog.spec(power).unwrap(),
            played.uid,
            Some(3),
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            super::super::play::resolved_local_energy_cost(
                &state,
                candidate,
                catalog.spec(defend).unwrap(),
            ),
            0
        );
    }

    /// The Energy cost of `uid` on a state reloaded through the canonical
    /// document (#3180).
    fn reloaded_energy_cost(state: &HotState, catalog: &Catalog, uid: u32) -> i64 {
        let document = crate::boundary::HotBoundary::try_to_canonical(state, catalog)
            .expect("the state projects");
        let catalog = crate::boundary::HotBoundary::catalog_from_canonical(&document)
            .expect("the document builds a catalog");
        let loaded = crate::boundary::HotBoundary::from_canonical(&document, &catalog)
            .expect("the document hydrates");
        let card = PileId::ALL
            .into_iter()
            .flat_map(|pile| loaded.piles.get(pile).as_slice())
            .copied()
            .find(|card| card.uid == uid)
            .expect("the card survives the reload");
        super::super::play::resolved_local_energy_cost(
            &loaded,
            card,
            catalog.spec(card.atom).unwrap(),
        )
    }

    /// A default state with a live `CombatCardSelection` stream, which both
    /// relics below draw from.
    fn state_with_seeded_selection() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(7);
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state
    }

    /// #3180: Mummified Hand's `SetToFreeThisTurn` rows (IL_0113) live in the
    /// chosen card's slot-7 payload. A plain deck Defend carries no slot-7
    /// bit, so before the fix the rows were live in the engine and absent
    /// from the canonical document, and a reloaded root charged the full
    /// cost (the #3176 class).
    #[test]
    fn mummified_hand_free_card_stays_free_through_the_canonical_root() {
        let mut builder = CatalogBuilder::new();
        let power = builder
            .intern(CardIdentity {
                id: CardId::DemonForm,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicMummifiedHand]).unwrap();
        let catalog = builder.build();
        let mut state = state_with_seeded_selection();
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 2,
            atom: defend,
            flags: 0,
        });
        assert_eq!(reloaded_energy_cost(&state, &catalog, 2), 1);

        after_card_played_hand(
            &catalog,
            &mut state,
            catalog.spec(power).unwrap(),
            1,
            Some(3),
            &mut Vec::new(),
        )
        .unwrap();

        let live = state.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(
            super::super::play::resolved_local_energy_cost(
                &state,
                live,
                catalog.spec(defend).unwrap()
            ),
            0
        );
        assert_eq!(reloaded_energy_cost(&state, &catalog, 2), 0);
        assert_ne!(live.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert!(state.exact_piles);
    }

    /// #3180 sweep: Bookmark's `UntilPlayed` `-1` row is the same shape, on
    /// whichever card it picks. A plain Strike carries no slot-7 bit.
    #[test]
    fn bookmark_discount_survives_the_canonical_root() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicBookmark]).unwrap();
        let catalog = builder.build();
        let mut state = state_with_seeded_selection();
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: strike,
            flags: 0,
        });
        assert_eq!(reloaded_energy_cost(&state, &catalog, 1), 1);

        super::super::draw::flush_hand(&mut state, &catalog, &mut Vec::new()).unwrap();

        let live = state.piles.get(PileId::Discard).as_slice()[0];
        assert_eq!(
            super::super::play::resolved_local_energy_cost(
                &state,
                live,
                catalog.spec(strike).unwrap()
            ),
            0
        );
        assert_eq!(reloaded_energy_cost(&state, &catalog, 1), 0);
        assert_ne!(live.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert!(state.exact_piles);
    }

    /// The retaining flush is a second path: the relic runs on the discarded
    /// partition before those cards reach Discard, so the bit has to travel
    /// on the snapshot. The retained Strike stays in Hand, untouched.
    #[test]
    fn bookmark_discount_survives_the_canonical_root_beside_a_retained_card() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicBookmark]).unwrap();
        let catalog = builder.build();
        let mut state = state_with_seeded_selection();
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: strike,
                flags: 0,
            },
        ]);
        state.card_states.set_local_retain(1);
        state.exact_piles = true;

        super::super::draw::flush_hand(&mut state, &catalog, &mut Vec::new()).unwrap();

        assert_eq!(
            state.piles.get(PileId::Hand).as_slice(),
            [HotCard {
                uid: 1,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]
        );
        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            [HotCard {
                uid: 2,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]
        );
        assert_eq!(reloaded_energy_cost(&state, &catalog, 1), 1);
        assert_eq!(reloaded_energy_cost(&state, &catalog, 2), 0);
    }

    /// #3247: `MummifiedHand::AfterCardPlayed` RVA `0x97148` gates only on
    /// `IsInProgress` (IL_000c-IL_0016), which `CheckWinCondition` clears
    /// only after the whole play. So a killing Power play still draws
    /// `CombatCardSelection` and frees a Hand card, exactly as a live one
    /// does (F3C51JD0JVEK n39: Consuming Shadow kills Soul Nexus).
    /// #3261: Kusarigama's counter and `CombatTargets` pick are not
    /// `history.over`-gated (`Kusarigama/<AfterCardPlayed>d__19` `0x327fe0`
    /// gates on `IsInProgress` only, IL_003d-IL_0047, and draws at IL_00be).
    /// Over with every enemy dead, the third Attack still advances the
    /// counter and the empty pick draws nothing; a live enemy while ending
    /// draws once and then refuses the ending Damage by name; a live primary
    /// enemy takes the hit.
    #[test]
    fn kusarigama_counter_and_pick_are_ungated_and_refuse_ending_damage() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.set_relics(&[RelicId::RelicKusarigama]).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(strike).unwrap();
        let fixture = |monster: Option<HotMonster>, over: bool| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.history.over = over;
            state.monsters_mut().extend(monster);
            assert!(state.fanouts.set_kusarigama(2));
            state
        };
        let draws = |state: &HotState| state.rng.get(RngStream::Targets).counter;
        let run = |state: &mut HotState| {
            after_card_played_hand(&catalog, state, spec, 1, Some(1), &mut Vec::new())
        };

        let mut dead = HotMonster::new(MonsterKind::Toadpole, 10);
        dead.hp = 0;
        let mut over = fixture(Some(dead), true);
        let before = draws(&over);
        run(&mut over).unwrap();
        assert_eq!(over.fanouts.kusarigama(), 0, "the killing Attack counts");
        assert_eq!(draws(&over), before, "the empty pick draws nothing");

        let mut minion = HotMonster::new(MonsterKind::Toadpole, 10);
        minion.powers.set(PowerId::Secondary, SlotWire::Int, 1);
        let mut ending = fixture(Some(minion), true);
        assert_eq!(
            run(&mut ending),
            Err(EngineRefusal::EndingDamageNotModeled("Kusarigama damage"))
        );
        assert_eq!(draws(&ending), before + 1, "the pick drew once");

        let mut live = fixture(Some(HotMonster::new(MonsterKind::Toadpole, 10)), false);
        run(&mut live).unwrap();
        assert_eq!(live.fanouts.kusarigama(), 0);
        assert_eq!(draws(&live), before + 1);
        assert_eq!(live.monsters[0].hp, 4, "the live hit deals 6");
    }

    /// #3279: Phylactery Unbound's turn-start summon
    /// (`PhylacteryUnbound/<AfterSideTurnStart>d__11` `0x32e134` IL_005b)
    /// reads the IsEnding projection through `summon_osty`: a live enemy
    /// heals, an ending combat grows MaxHp only, and an absent Osty while
    /// ending is created at 1 HP under the summoned MaxHp (#3635). Before
    /// #3279 every arm healed.
    #[test]
    fn phylactery_unbound_turn_start_summon_reads_the_ending_projection() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[RelicId::RelicPhylacteryUnbound])
            .unwrap();
        let catalog = builder.build();
        for (live_enemy, osty, expected) in [
            (true, Some((3, 5)), Ok::<_, EngineRefusal>(Some((5, 7)))),
            (false, Some((3, 5)), Ok(Some((3, 7)))),
            (false, None, Ok(Some((1, 2)))),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.turn = 2;
            if live_enemy {
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 10));
            }
            state.fanouts.set_osty(osty).unwrap();
            let result =
                after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).map(|()| {
                    state
                        .fanouts
                        .pet()
                        .osty()
                        .map(|osty| (osty.hp(), osty.max_hp()))
                });
            assert_eq!(result, expected, "live enemy {live_enemy}, Osty {osty:?}");
        }
    }

    #[test]
    fn mummified_hand_still_draws_on_the_killing_power_play() {
        let mut builder = CatalogBuilder::new();
        let power = builder
            .intern(CardIdentity {
                id: CardId::DemonForm,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicMummifiedHand]).unwrap();
        let catalog = builder.build();
        let played = HotCard {
            uid: 1,
            atom: power,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let candidates = [2, 3].map(|uid| HotCard {
            uid,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut live = HotState::at_defaults();
        live.hp = 20;
        live.max_hp = 20;
        live.next_card_uid = 4;
        live.piles.get_mut(PileId::Play).make_mut().push(played);
        for card in candidates {
            live.piles.get_mut(PileId::Hand).make_mut().push(card);
        }
        let mut ending = live.clone();
        ending.history.over = true;

        for state in [&mut live, &mut ending] {
            let before = state.rng.get(RngStream::Sel).counter;
            after_card_played_hand(
                &catalog,
                state,
                catalog.spec(power).unwrap(),
                played.uid,
                Some(3),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.rng.get(RngStream::Sel).counter, before + 1);
        }
        assert_eq!(live.rng.get(RngStream::Sel), ending.rng.get(RngStream::Sel));
        let free = |state: &HotState| {
            candidates
                .iter()
                .filter(|card| {
                    super::super::play::resolved_local_energy_cost(
                        state,
                        **card,
                        catalog.spec(defend).unwrap(),
                    ) == 0
                })
                .map(|card| card.uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(free(&ending).len(), 1);
        assert_eq!(free(&live), free(&ending));
    }

    #[test]
    fn batch_nine_joss_paper_draws_on_each_fifth_non_ethereal_exhaust() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicJossPaper]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.next_card_uid = 7;
        assert!(state.fanouts.set_joss_paper_cards_exhausted(0));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 6,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        for uid in 1..=5 {
            after_card_exhausted(
                &catalog,
                &mut state,
                HotCard {
                    uid,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                false,
                &mut Vec::new(),
            )
            .unwrap();
        }

        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.fanouts.joss_paper_cards_exhausted(), 0);
    }

    #[test]
    fn queen_blockers_joss_uses_exhaust_cause_instead_of_ethereal_keyword() {
        for caused_by_ethereal in [false, true] {
            let mut builder = CatalogBuilder::new();
            let clumsy = builder
                .intern(CardIdentity {
                    id: CardId::Clumsy,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let defend = builder
                .intern(CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.set_relics(&[RelicId::RelicJossPaper]).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.next_card_uid = 3;
            assert!(state.fanouts.set_joss_paper_cards_exhausted(4));
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2,
                atom: defend,
                flags: 0,
            });
            after_card_exhausted(
                &catalog,
                &mut state,
                HotCard {
                    uid: 1,
                    atom: clumsy,
                    flags: 0,
                },
                caused_by_ethereal,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                state.piles.get(PileId::Hand).len(),
                usize::from(!caused_by_ethereal)
            );
            assert_eq!(
                state.fanouts.joss_paper_ethereal_count(),
                i32::from(caused_by_ethereal)
            );
            after_side_turn_end_relics(&catalog, &mut state, &mut Vec::new()).unwrap();
            assert_eq!(state.piles.get(PileId::Hand).len(), 1);
            assert_eq!(state.fanouts.joss_paper_cards_exhausted(), 0);
            assert_eq!(state.fanouts.joss_paper_ethereal_count(), 0);
        }
    }

    #[test]
    fn batch_nine_orange_dough_generates_two_distinct_pool_cards() {
        let mut builder = CatalogBuilder::new();
        let pool = crate::content_tables::GENERATION_RELIC_POOLS
            .iter()
            .find_map(|(name, pool)| (*name == "ORANGE_DOUGH").then_some(*pool))
            .unwrap();
        for id in pool {
            builder
                .intern_reachable(CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.set_relics(&[RelicId::RelicOrangeDough]).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 20;
        state.max_hp = 20;
        state.turn = 1;
        state.next_card_uid = 1;
        state.fully_unlocked_card_pool_epochs = true;
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(13);
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );

        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();

        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 2);
        assert_ne!(hand[0].atom, hand[1].atom);
        assert!(hand.iter().all(|card| {
            catalog
                .spec(card.atom)
                .is_some_and(|spec| pool.contains(&spec.identity.id))
        }));
    }

    /// #3325: both frozen generation-relic rows ARE the character-blind
    /// Colorless pool at the fully-unlocked profile, so routing Orange Dough
    /// and Toolbox through [`colorless_generation_relic_pool`] moves no
    /// fully-unlocked draw.
    #[test]
    fn colorless_generation_relic_pool_is_the_frozen_row_when_fully_unlocked() {
        let full = crate::steps::neutral::colorless_generation_pool(None);
        for name in ["ORANGE_DOUGH", "TOOLBOX"] {
            let frozen = crate::content_tables::GENERATION_RELIC_POOLS
                .iter()
                .find_map(|(candidate, pool)| (*candidate == name).then_some(*pool))
                .unwrap();
            assert_eq!(frozen, &full[..], "{name}");
        }
        // Under the fully-unlocked profile the catalog derives the same list.
        let mut builder = CatalogBuilder::new();
        builder.set_splash_unlock_epochs(crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS.to_vec());
        assert_eq!(builder.build().colorless_generation_pool(), full);
    }

    /// Orange Dough's two cards under one Generation seed: the same two
    /// Colorless cards for every owner class (the pool is character-blind,
    /// `0x32bd7c` IL_0061), drawn from the RECORDED profile's pool (a profile
    /// hiding `COLORLESS5_EPOCH` drops its rows and reshapes the shuffle), and
    /// refused by name with no recorded profile or in a party run.
    #[test]
    fn orange_dough_draws_the_recorded_colorless_pool_for_every_owner() {
        use crate::catalog::RewardPool;
        let run = |owner: Option<RewardPool>,
                   epochs: Option<Vec<&'static str>>,
                   fully_unlocked: bool,
                   ally: u8|
         -> Result<Vec<CardId>, EngineRefusal> {
            let mut builder = CatalogBuilder::new();
            for id in crate::steps::neutral::colorless_generation_pool(None) {
                builder
                    .intern_reachable(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            if let Some(epochs) = epochs {
                builder.set_splash_unlock_epochs(epochs);
            }
            builder.set_relics(&[RelicId::RelicOrangeDough]).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 20;
            state.max_hp = 20;
            state.turn = 1;
            state.next_card_uid = 1;
            state.reward_card_pool = owner;
            state.fully_unlocked_card_pool_epochs = fully_unlocked;
            state.multiplayer_ally_key = ally;
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(13);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new())?;
            Ok(state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect())
        };
        let full = crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS.to_vec();
        let owners = [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Defect,
            RewardPool::Necrobinder,
            RewardPool::Regent,
        ];
        let reference = run(Some(RewardPool::Ironclad), Some(full.clone()), true, 0).unwrap();
        assert_eq!(reference.len(), 2);
        // Seed 13's full-profile shuffle through the frozen row directly.
        let frozen = {
            let mut state = HotState::at_defaults();
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(13);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            let row = crate::content_tables::GENERATION_RELIC_POOLS
                .iter()
                .find_map(|(name, pool)| (*name == "ORANGE_DOUGH").then_some(*pool))
                .unwrap();
            super::super::cards::shuffle_generation_slice(&mut state, row).unwrap()
        };
        assert_eq!(reference, frozen[..2], "fully unlocked = the frozen row");
        for owner in owners {
            assert_eq!(
                run(Some(owner), Some(full.clone()), true, 0).unwrap(),
                reference,
                "{owner:?}"
            );
        }
        assert_eq!(run(None, Some(full.clone()), true, 0).unwrap(), reference);

        // A recorded profile hiding COLORLESS5 (Anointed, Calamity, Splash).
        let partial: Vec<&'static str> = full
            .iter()
            .copied()
            .filter(|epoch| *epoch != "COLORLESS5_EPOCH")
            .collect();
        let partial_pool = crate::steps::neutral::colorless_generation_pool(Some(&partial));
        assert_eq!(partial_pool.len(), 47);
        let expected = {
            let mut state = HotState::at_defaults();
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(13);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            super::super::cards::shuffle_generation_slice(&mut state, &partial_pool).unwrap()
        };
        for owner in owners {
            let drawn = run(Some(owner), Some(partial.clone()), false, 0).unwrap();
            assert_eq!(drawn, expected[..2], "{owner:?}");
        }
        assert_ne!(
            expected[..2],
            frozen[..2],
            "seed 13 separates the partial pool from the frozen row"
        );

        let refused = Err(EngineRefusal::MalformedArgs(
            "Orange Dough Colorless pool provenance",
        ));
        assert_eq!(run(Some(RewardPool::Silent), None, false, 0), refused);
        assert_eq!(run(Some(RewardPool::Silent), Some(full), true, 1), refused);
    }

    /// Toolbox's engine arm of [`generation_relic_draw_pool`] (#3325): under a
    /// recorded profile hiding `COLORLESS5_EPOCH` its three options come from
    /// that profile's pool for every owner, and `relic_pending_is_exact`
    /// validates them against the same pool (an option the profile hides is
    /// not exact); with no recorded profile, or in a party run, the pause
    /// refuses by name before drawing.
    #[test]
    fn toolbox_draws_the_recorded_colorless_pool_and_refuses_without_one() {
        use crate::catalog::RewardPool;
        let full = crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS.to_vec();
        let partial: Vec<&'static str> = full
            .iter()
            .copied()
            .filter(|epoch| *epoch != "COLORLESS5_EPOCH")
            .collect();
        let partial_pool = crate::steps::neutral::colorless_generation_pool(Some(&partial));
        let setup = |owner: Option<RewardPool>,
                     epochs: Option<Vec<&'static str>>,
                     ally: u8|
         -> (Catalog, HotState) {
            let mut builder = CatalogBuilder::new();
            for id in crate::steps::neutral::colorless_generation_pool(None) {
                builder
                    .intern_reachable(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            if let Some(epochs) = epochs {
                builder.set_splash_unlock_epochs(epochs);
            }
            builder.set_relics(&[RelicId::RelicToolbox]).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 20;
            state.max_hp = 20;
            state.turn = 1;
            state.next_card_uid = 1;
            state.reward_card_pool = owner;
            state.multiplayer_ally_key = ally;
            let seeded = crate::rng::Xoshiro256StarStar::from_seed(7);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: seeded.words,
                    counter: seeded.counter,
                },
            );
            (catalog, state)
        };
        let options = |catalog: &Catalog, state: &HotState| -> Vec<CardId> {
            state
                .fanouts
                .batch_nine_relic_pending()
                .expect("Toolbox paused")
                .entries
                .iter()
                .map(|entry| catalog.spec(entry.card.atom).unwrap().identity.id)
                .collect()
        };
        let mut reference = None;
        for owner in [
            RewardPool::Ironclad,
            RewardPool::Silent,
            RewardPool::Defect,
            RewardPool::Necrobinder,
            RewardPool::Regent,
        ] {
            let (catalog, mut state) = setup(Some(owner), Some(partial.clone()), 0);
            assert_eq!(
                before_hand_draw(&catalog, &mut state, true, &mut Vec::new()),
                Ok(false)
            );
            let drawn = options(&catalog, &state);
            assert_eq!(drawn.len(), 3);
            assert!(
                drawn.iter().all(|id| partial_pool.contains(id)),
                "{owner:?}"
            );
            assert_eq!(*reference.get_or_insert_with(|| drawn.clone()), drawn);
            assert!(relic_pending_is_exact(&state, &catalog), "{owner:?}");
        }
        // A pending option the recorded profile hides is not exact.
        let (catalog, mut state) = setup(Some(RewardPool::Silent), Some(partial.clone()), 0);
        before_hand_draw(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        let mut record = state.fanouts.batch_nine_relic_pending().unwrap().clone();
        record.entries[0].card.atom = catalog
            .atom(&CardIdentity {
                id: CardId::Splash,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        state.fanouts.set_batch_nine_relic_pending(Some(record));
        assert!(!relic_pending_is_exact(&state, &catalog));

        let refused = Err(EngineRefusal::MalformedArgs(
            "Toolbox Colorless pool provenance",
        ));
        let (catalog, mut state) = setup(Some(RewardPool::Silent), None, 0);
        assert_eq!(
            before_hand_draw(&catalog, &mut state, true, &mut Vec::new()),
            refused
        );
        let (catalog, mut state) = setup(Some(RewardPool::Silent), Some(full), 1);
        assert_eq!(
            before_hand_draw(&catalog, &mut state, true, &mut Vec::new()),
            refused
        );
    }
    #[test]
    fn queen_blockers_tuning_fork_precedes_lethal_letter_opener_only_when_recorded() {
        for (order, expected_block) in [
            ([RelicId::RelicTuningFork, RelicId::RelicLetterOpener], 7),
            ([RelicId::RelicLetterOpener, RelicId::RelicTuningFork], 0),
        ] {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.set_relics_ordered(&order, true).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.history.skill_plays_finished_this_turn = 3;
            assert!(state.fanouts.set_tuning_fork_skills(9));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 5));
            after_card_played_hand(
                &catalog,
                &mut state,
                catalog.spec(atom).unwrap(),
                0,
                Some(1),
                &mut Vec::new(),
            )
            .unwrap();
            assert!(state.history.over);
            assert_eq!(state.block, expected_block);
            assert_eq!(state.fanouts.tuning_fork_skills(), 0);
        }
    }

    /// #3192: Iron Club and Tuning Fork run at their recorded inventory
    /// positions, with a lethal Letter Opener between them. The native Hook
    /// (`0x3ccbf4`) awaits each relic in `Player.Relics` order (`0x3f9720`),
    /// so a counter recorded before the kill acts and one recorded after it
    /// sees the ended combat. Without dispatch-order provenance the legacy
    /// order (peers, then Iron Club, then Tuning Fork) stands.
    #[test]
    fn counter_relics_dispatch_at_their_recorded_inventory_positions() {
        let club = RelicId::RelicIronClub;
        let fork = RelicId::RelicTuningFork;
        let opener = RelicId::RelicLetterOpener;
        // (inventory, ordered, iron club counter, tuning fork counter,
        //  expected hand size, expected block)
        for (order, ordered, club_at, fork_at, hand, block) in [
            (vec![club, opener], true, 3, 0, 1, 0),
            (vec![opener, club], true, 3, 0, 0, 0),
            (vec![club, opener, fork], true, 3, 0, 1, 0),
            (vec![fork, opener, club], true, 3, 0, 0, 0),
            (vec![club, opener, fork], true, 0, 9, 0, 0),
            (vec![fork, opener, club], true, 0, 9, 0, 7),
            (vec![fork, club, opener], true, 0, 9, 0, 7),
            (vec![club, fork, opener], true, 3, 0, 1, 0),
            // No provenance: every peer first, so the kill precedes both.
            (vec![club, opener], false, 3, 0, 0, 0),
            (vec![fork, opener], false, 0, 9, 0, 0),
            // No provenance, both counters, recorded fork-first: still every
            // peer, then Iron Club, then Tuning Fork. Which of the two runs
            // first is not observable (both acting on one play refuses as
            // `Iron Club and Tuning Fork simultaneous thresholds`), so the
            // witness is that neither acts ahead of the peer's kill.
            (vec![fork, club, opener], false, 3, 0, 0, 0),
            (vec![fork, club, opener], false, 0, 9, 0, 0),
            (vec![fork, opener, club], false, 0, 9, 0, 0),
        ] {
            let mut builder = CatalogBuilder::new();
            let defend = builder
                .intern(CardIdentity {
                    id: CardId::DefendIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.set_relics_ordered(&order, ordered).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.history.skill_plays_finished_this_turn = 3;
            assert!(state.fanouts.set_iron_club_cards(club_at));
            assert!(state.fanouts.set_tuning_fork_skills(fork_at));
            state.piles.set(
                PileId::Draw,
                HotPile::from_cards(vec![HotCard {
                    uid: 1,
                    atom: defend,
                    flags: 0,
                }]),
            );
            state.next_card_uid = 2;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 5));
            after_card_played_hand(
                &catalog,
                &mut state,
                catalog.spec(defend).unwrap(),
                0,
                Some(1),
                &mut Vec::new(),
            )
            .unwrap();
            assert!(state.history.over, "{order:?}");
            assert_eq!(
                state.piles.get(PileId::Hand).len(),
                hand,
                "{order:?} ordered={ordered}"
            );
            assert_eq!(state.block, block, "{order:?} ordered={ordered}");
            // Both counters advance whatever their position.
            if catalog.hooks().owns(club) {
                assert_eq!(
                    state.fanouts.iron_club_cards(),
                    (club_at + 1) % 4,
                    "{order:?}"
                );
            }
            if catalog.hooks().owns(fork) {
                assert_eq!(
                    state.fanouts.tuning_fork_skills(),
                    (fork_at + 1) % 10,
                    "{order:?}"
                );
            }
        }
    }

    /// One Strike in Play (uid 1) under `order`, facing `monsters`, with or
    /// without a live Juggernaut. Returns the catalog, the state and the
    /// atoms of Strike, Strike+ and Defend.
    fn walk_order_fixture(
        order: &[RelicId],
        vouched: bool,
        monsters: &[i32],
        juggernaut: bool,
    ) -> (Catalog, HotState, [crate::engine::CardAtom; 3]) {
        let mut builder = CatalogBuilder::new();
        let mut intern = |id, upgrade| {
            builder
                .intern(CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                })
                .unwrap()
        };
        let atoms = [
            intern(CardId::StrikeIronclad, 0),
            intern(CardId::StrikeIronclad, 1),
            intern(CardId::DefendIronclad, 0),
        ];
        builder.set_relics_ordered(order, vouched).unwrap();
        let catalog = builder.build();
        let mut state = juggernaut_state(monsters);
        if !juggernaut {
            state
                .powers
                .set(PowerId::Juggernaut, crate::powers::SlotWire::Int, 0);
            assert!(state.fanouts.set_after_block_gained_order(&[]));
        }
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 1,
            atom: atoms[0],
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.next_card_uid = 3;
        // The Strike's BeforeCardPlayed ran before this walk (#3640).
        before_card_played_hand(&catalog, &mut state, catalog.spec(atoms[0]).unwrap(), 1).unwrap();
        (catalog, state, atoms)
    }

    fn play_walk_strike(
        catalog: &Catalog,
        state: &mut HotState,
        strike: crate::engine::CardAtom,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        after_card_played_hand(
            catalog,
            state,
            catalog.spec(strike).unwrap(),
            1,
            Some(1),
            &mut events,
        )
        .unwrap();
        events
    }

    /// #2909: `MusicBox/<AfterCardPlayed>d__13` (`0x32ae04`) clones
    /// `cardPlay.Card` as it stands when the body runs (`CreateClone`,
    /// IL_0046), and `RazorTooth::AfterCardPlayed` (`0x9a1b8`) upgrades that
    /// same card (`CardCmd::Upgrade`, IL_0060). The Hook awaits the relics in
    /// `Player.Relics` order, so the clone is upgraded exactly when Razor
    /// Tooth is recorded first. An unvouched inventory keeps the fixed order
    /// (clone first), which admission refuses.
    #[test]
    fn music_box_clone_copies_the_upgrade_only_when_razor_tooth_is_recorded_first() {
        for razor_recorded_first in [true, false] {
            for vouched in [true, false] {
                let order = if razor_recorded_first {
                    [RelicId::RelicRazorTooth, RelicId::RelicMusicBox]
                } else {
                    [RelicId::RelicMusicBox, RelicId::RelicRazorTooth]
                };
                let (catalog, mut state, [strike, strike_plus, _]) =
                    walk_order_fixture(&order, vouched, &[50], false);
                play_walk_strike(&catalog, &mut state, strike);
                let played = state.piles.get(PileId::Play).as_slice()[0];
                assert_eq!(played.atom, strike_plus, "{order:?}: the source upgrades");
                let hand = state.piles.get(PileId::Hand).as_slice();
                assert_eq!(hand.len(), 1, "{order:?} vouched={vouched}");
                assert!(state.card_states.get(hand[0].uid).local_ethereal());
                let expected = if vouched && razor_recorded_first {
                    strike_plus
                } else {
                    strike
                };
                assert_eq!(hand[0].atom, expected, "{order:?} vouched={vouched}");
            }
        }
    }

    /// #2909: Music Box's clone (`AddGeneratedCardToCombat`, IL_0065) and Iron
    /// Club's fourth-card draw both add to the Hand, so the recorded order is
    /// the Hand order.
    #[test]
    fn music_box_and_iron_club_fill_the_hand_in_recorded_order() {
        let run = |order: [RelicId; 2]| {
            let (catalog, mut state, [strike, _, defend]) =
                walk_order_fixture(&order, true, &[50], false);
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2,
                atom: defend,
                flags: 0,
            });
            assert!(state.fanouts.set_iron_club_cards(3));
            play_walk_strike(&catalog, &mut state, strike);
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>()
        };
        let box_first = run([RelicId::RelicMusicBox, RelicId::RelicIronClub]);
        let club_first = run([RelicId::RelicIronClub, RelicId::RelicMusicBox]);
        assert_eq!(box_first.len(), 2);
        assert_eq!(
            club_first,
            box_first.iter().rev().copied().collect::<Vec<_>>()
        );
    }

    /// #2909: `Kusarigama/<AfterCardPlayed>d__19` (`0x327fe0`) deals its third
    /// Attack's damage at IL_00f6. Recorded before Music Box, a lethal hit
    /// ends the combat before the clone is added: the clone is generated and
    /// recorded, and `CardPileCmd/<Add>d__10` (`0x3e1ba4`) drops it at its
    /// `IsEnding` check (IL_0053). Recorded after, the clone is already in
    /// the Hand. The same holds for Daughter of the Wind's block and for
    /// Ornamental Fan's third-Attack block under Juggernaut, whose nested hit is
    /// the lethal one.
    #[test]
    fn music_box_clone_lands_only_ahead_of_a_recorded_lethal_peer() {
        for (peer, juggernaut) in [
            (RelicId::RelicKusarigama, false),
            (RelicId::RelicDaughterOfTheWind, true),
            (RelicId::RelicOrnamentalFan, true),
        ] {
            for box_recorded_first in [true, false] {
                let order = if box_recorded_first {
                    [RelicId::RelicMusicBox, peer]
                } else {
                    [peer, RelicId::RelicMusicBox]
                };
                let (catalog, mut state, [strike, ..]) =
                    walk_order_fixture(&order, true, &[5], juggernaut);
                assert!(state.fanouts.set_kusarigama(2));
                assert!(state.set_ornamental_fan(2));
                play_walk_strike(&catalog, &mut state, strike);
                assert!(state.history.over, "{order:?}");
                assert_eq!(
                    state.piles.get(PileId::Hand).len(),
                    usize::from(box_recorded_first),
                    "{order:?}"
                );
                assert!(state.fanouts.music_box_used_this_turn(), "{order:?}");
            }
        }
    }

    /// #2909: Ornamental Fan's third-Attack block feeds Juggernaut's
    /// `CombatTargets` hit (`JuggernautPower/<AfterBlockGained>d__4`
    /// `0x33dab4`), and Kusarigama's third-Attack hit rolls the same stream.
    /// The recorded order decides which hit takes the first roll.
    #[test]
    fn ornamental_fan_and_kusarigama_roll_targets_in_recorded_order_under_juggernaut() {
        let first_hit = |order: [RelicId; 2], vouched: bool| {
            let (catalog, mut state, [strike, ..]) =
                walk_order_fixture(&order, vouched, &[100, 100], true);
            assert!(state.fanouts.set_kusarigama(2));
            assert!(state.set_ornamental_fan(2));
            let events = play_walk_strike(&catalog, &mut state, strike);
            let hits = events
                .iter()
                .filter_map(|event| match event {
                    Event::MonsterDamaged { unblocked, .. } => Some(*unblocked),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(hits.len(), 2, "{order:?}");
            assert_eq!(state.block, 4, "{order:?}");
            hits[0]
        };
        let fan = RelicId::RelicOrnamentalFan;
        let kusarigama = RelicId::RelicKusarigama;
        assert_eq!(first_hit([fan, kusarigama], true), 5, "Juggernaut's hit");
        assert_eq!(first_hit([kusarigama, fan], true), 6, "Kusarigama's hit");
        // Unvouched: the fixed order, which admission refuses.
        assert_eq!(first_hit([kusarigama, fan], false), 5);
    }

    /// #2909: `MummifiedHand::AfterCardPlayed` (`0x97148`) picks from the Hand
    /// as it stands (IL_0061-IL_010e) and Game Piece draws a card into it.
    /// Game Piece recorded first puts the drawn card among the candidates and
    /// the pick consumes a `CombatCardSelection` draw; recorded second, the
    /// Hand is empty at the pick, nothing is rolled, and the drawn card keeps
    /// its cost.
    #[test]
    fn mummified_hand_picks_the_card_game_piece_drew_only_when_recorded_after_it() {
        for piece_recorded_first in [true, false] {
            for vouched in [true, false] {
                let order = if piece_recorded_first {
                    [RelicId::RelicGamePiece, RelicId::RelicMummifiedHand]
                } else {
                    [RelicId::RelicMummifiedHand, RelicId::RelicGamePiece]
                };
                let mut builder = CatalogBuilder::new();
                let power = builder
                    .intern(CardIdentity {
                        id: CardId::DemonForm,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
                let defend = builder
                    .intern(CardIdentity {
                        id: CardId::DefendIronclad,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
                builder.set_relics_ordered(&order, vouched).unwrap();
                let catalog = builder.build();
                let mut state = state_with_seeded_selection();
                state.next_card_uid = 3;
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid: 2,
                    atom: defend,
                    flags: 0,
                });
                let rolls_before = state.rng.get(RngStream::Sel).counter;
                after_card_played_hand(
                    &catalog,
                    &mut state,
                    catalog.spec(power).unwrap(),
                    1,
                    Some(3),
                    &mut Vec::new(),
                )
                .unwrap();
                let drawn = state.piles.get(PileId::Hand).as_slice()[0];
                let picked = vouched && piece_recorded_first;
                assert_eq!(
                    super::super::play::resolved_local_energy_cost(
                        &state,
                        drawn,
                        catalog.spec(defend).unwrap()
                    ),
                    if picked { 0 } else { 1 },
                    "{order:?} vouched={vouched}"
                );
                assert_eq!(
                    state.rng.get(RngStream::Sel).counter != rolls_before,
                    picked,
                    "{order:?} vouched={vouched}"
                );
            }
        }
    }

    /// #3756: Mummified Hand's Hand filter reads Brilliant Scarf's play count
    /// (`MummifiedHand/<>c::<AfterCardPlayed>b__2_0` `0x32add6`,
    /// `GetWithModifiers` at IL_0008; `BrilliantScarf::ShouldModifyCost`
    /// `0x91828`), and the Scarf counts the play in its own `AfterCardPlayed`
    /// (`0x917bc`). The witness is the turn's fifth manual play, a Power,
    /// with two base-cost-1 cards in the Hand: a Defend at full cost and a
    /// Strike already free this turn.
    ///
    /// * **Scarf recorded first:** it has counted five, its free window is
    ///   closed, the Defend is the one card that still costs, and Mummified
    ///   Hand frees it wherever it sits.
    /// * **Mummified Hand recorded first:** the Scarf still reads four, every
    ///   Hand card costs zero, the filter falls through to the base-cost
    ///   tier, and the pick is one of the two by `CombatCardSelection`. The
    ///   same roll lands on the Defend in one Hand order and on the Strike in
    ///   the other.
    ///
    /// Each root round-trips and is admitted. An unvouched inventory is
    /// refused by admission
    /// (`scarf_and_mummified_hand_refuse_only_an_unvouched_inventory`).
    #[test]
    fn mummified_hand_reads_brilliant_scarf_on_its_recorded_side_of_the_count() {
        let scarf = RelicId::RelicBrilliantScarf;
        let hand_relic = RelicId::RelicMummifiedHand;
        // Whether the Defend (uid 2) is free once the Power play is done.
        let defend_freed = |order: [RelicId; 2], defend_first: bool| {
            let (mut state, catalog, atoms) = latch_fixture(
                &[
                    CardId::DemonForm,
                    CardId::DefendIronclad,
                    CardId::StrikeIronclad,
                ],
                &order,
                true,
            );
            let (power, defend, strike) = (atoms[0], atoms[1], atoms[2]);
            state
                .fanouts
                .set_batch_eight_deep_relic_ownership(false, true, false, false, false);
            state.history.manual_card_plays_finished_this_turn = 4;
            state.history.owner_card_plays_finished_this_turn = 4;
            state.history.card_plays_finished_combat = 4;
            let rest = if defend_first {
                [(2, defend), (3, strike)]
            } else {
                [(3, strike), (2, defend)]
            };
            latch_push(&mut state, PileId::Hand, &[(1, power)]);
            latch_push(&mut state, PileId::Hand, &rest);
            state.card_states.set_to_free_this_turn(3, 1).unwrap();
            let (state, catalog) = latch_cold(&state, &catalog);
            assert!(state.fanouts.brilliant_scarf_owned(), "{order:?}");
            assert_eq!(
                relic_recorded_before(&catalog, hand_relic, scarf),
                order[0] == hand_relic,
                "{order:?}: the loaded root keeps its vouched order"
            );
            let play = crate::engine::legal_actions(&state, &catalog)
                .into_iter()
                .find(|action| matches!(action, crate::engine::Action::Play { uid: 1, .. }))
                .unwrap_or_else(|| panic!("{order:?}: the Power is playable"));
            let done = crate::engine::apply_action(&state, &catalog, &play)
                .unwrap_or_else(|refusal| panic!("{order:?}: {refusal:?}"))
                .state;
            assert_eq!(
                done.history.manual_card_plays_finished_this_turn, 5,
                "{order:?}: the count is restored and holds this play"
            );
            assert_eq!(
                done.rng.get(RngStream::Sel).counter,
                1,
                "{order:?}: one CombatCardSelection roll"
            );
            let defend_card = *done
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .find(|card| card.uid == 2)
                .expect("the Defend stays in the Hand");
            super::super::play::resolved_local_energy_cost(
                &done,
                defend_card,
                catalog.spec(defend_card.atom).unwrap(),
            ) == 0
        };
        for defend_first in [true, false] {
            assert!(
                defend_freed([scarf, hand_relic], defend_first),
                "Scarf first, Defend first {defend_first}: the one costly card is freed"
            );
        }
        assert_ne!(
            defend_freed([hand_relic, scarf], true),
            defend_freed([hand_relic, scarf], false),
            "Mummified Hand first: the roll picks between both base-cost cards"
        );
    }

    /// #2909: three walk relics in three recorded orders give three results.
    /// The fixed order is Music Box, Kusarigama, Razor Tooth, which is none of
    /// them.
    #[test]
    fn three_walk_relics_run_in_each_recorded_order() {
        let razor = RelicId::RelicRazorTooth;
        let music = RelicId::RelicMusicBox;
        let kusarigama = RelicId::RelicKusarigama;
        // (order, clone in Hand, clone upgraded, played card upgraded)
        for (order, cloned, clone_upgraded, played_upgraded) in [
            ([razor, music, kusarigama], true, true, true),
            ([music, razor, kusarigama], true, false, true),
            // The lethal hit comes first: Razor Tooth's upgrade and the
            // clone's insertion both see the ended combat.
            ([kusarigama, razor, music], false, false, false),
        ] {
            let (catalog, mut state, [strike, strike_plus, _]) =
                walk_order_fixture(&order, true, &[5], false);
            assert!(state.fanouts.set_kusarigama(2));
            play_walk_strike(&catalog, &mut state, strike);
            assert!(state.history.over, "{order:?}");
            let hand = state.piles.get(PileId::Hand).as_slice();
            assert_eq!(hand.len(), usize::from(cloned), "{order:?}");
            if cloned {
                let expected = if clone_upgraded { strike_plus } else { strike };
                assert_eq!(hand[0].atom, expected, "{order:?}");
            }
            let played = state.piles.get(PileId::Play).as_slice()[0];
            assert_eq!(played.atom == strike_plus, played_upgraded, "{order:?}");
        }
    }

    /// #2909: every body runs exactly once per play, in any inventory order.
    ///
    /// One inventory holds all 22 walk relics. An Attack, a Skill and a Power
    /// are played, and each body's effect is counted: a counter that advanced
    /// by one, one block gain, one hit, one draw, one Energy. A body that
    /// lost its `runs(..)` gate would run once per recorded relic and move
    /// its count. The bodies whose effect is a latch (Art of War, Rainbow
    /// Ring's type bits, Vambrace, Pael's Legion, Velvet Choker) cannot be
    /// counted that way, so the source scan below holds every body to a
    /// `runs(..)` gate as well.
    #[test]
    fn every_walk_body_runs_exactly_once_per_play() {
        let production = include_str!("relics.rs")
            .split("\nmod tests {")
            .next()
            .unwrap();
        let body = production
            .split("fn after_card_played_peer_segment(")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        let walked = production
            .split("fn in_after_card_played_walk(")
            .nth(1)
            .unwrap()
            .split("\n}\n")
            .next()
            .unwrap();
        let mut named = std::collections::BTreeSet::new();
        for chunk in body.split("runs(RelicId::").skip(1) {
            let name = chunk.split(')').next().unwrap();
            assert!(
                walked.contains(&format!("RelicId::{name}\n")),
                "{name} has a body and is not walked"
            );
            named.insert(name);
        }
        assert_eq!(named.len(), 20, "{named:?}");
        // Every top-level statement that can run a body is gated by `runs`.
        let mut statements = 0;
        let mut statement = String::new();
        for line in body.lines() {
            if line.starts_with("    if ") {
                statement.clear();
            }
            statement.push_str(line);
            statement.push('\n');
            if line == "    }" {
                assert!(statement.contains("runs("), "ungated body:\n{statement}");
                statements += 1;
            }
        }
        assert!(statements >= 18, "{statements} gated statements");
        for recorded in body.split("record_relic(RelicId::").skip(1) {
            let name = recorded.split(')').next().unwrap();
            assert!(named.contains(name), "{name} records coverage ungated");
        }

        let all = [
            RelicId::RelicArtOfWar,
            RelicId::RelicDaughterOfTheWind,
            RelicId::RelicGamePiece,
            RelicId::RelicHelicalDart,
            RelicId::RelicIronClub,
            RelicId::RelicIvoryTile,
            RelicId::RelicKunai,
            RelicId::RelicKusarigama,
            RelicId::RelicLetterOpener,
            RelicId::RelicLostWisp,
            RelicId::RelicMummifiedHand,
            RelicId::RelicMusicBox,
            RelicId::RelicNunchaku,
            RelicId::RelicOrnamentalFan,
            RelicId::RelicPaelsLegion,
            RelicId::RelicPermafrost,
            RelicId::RelicRainbowRing,
            RelicId::RelicRazorTooth,
            RelicId::RelicShuriken,
            RelicId::RelicTuningFork,
            RelicId::RelicVambrace,
            RelicId::RelicVelvetChoker,
        ];
        assert!(all.iter().all(|relic| in_after_card_played_walk(*relic)));
        let reversed = all.iter().rev().copied().collect::<Vec<_>>();
        let mut rotated = all.to_vec();
        rotated.rotate_left(9);
        for (order, vouched) in [
            (all.to_vec(), true),
            (reversed, true),
            (rotated, true),
            (all.to_vec(), false),
        ] {
            let mut builder = CatalogBuilder::new();
            let mut intern = |id, upgrade| {
                builder
                    .intern(CardIdentity {
                        id,
                        upgrade,
                        enchantment: None,
                    })
                    .unwrap()
            };
            let strike = intern(CardId::StrikeIronclad, 0);
            intern(CardId::StrikeIronclad, 1);
            let defend = intern(CardId::DefendIronclad, 0);
            intern(CardId::DefendIronclad, 1);
            let power = intern(CardId::DemonForm, 0);
            builder.set_relics_ordered(&order, vouched).unwrap();
            let catalog = builder.build();
            let mut state = state_with_seeded_selection();
            state.hp = 80;
            state.max_hp = 80;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            for (uid, atom) in [(1, strike), (2, defend), (3, power)] {
                state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                    uid,
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
            }
            for uid in [4, 5] {
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid,
                    atom: defend,
                    flags: 0,
                });
            }
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 6,
                atom: defend,
                flags: 0,
            });
            state.next_card_uid = 7;
            state.history.skill_plays_finished_this_turn = 3;
            state.set_permafrost_armed(true);
            let energy = state.energy;
            let selections = state.rng.get(RngStream::Sel).counter;
            let mut events = Vec::new();
            let label = format!("{order:?} vouched={vouched}");
            for (uid, atom) in [(1, strike), (2, defend), (3, power)] {
                let spec = catalog.spec(atom).unwrap();
                if catalog.hooks().owns(RelicId::RelicMusicBox) && spec.is_attack {
                    // Its BeforeCardPlayed latch (#3640); Pen Nib is not held.
                    state.fanouts.set_music_box_card_uid(Some(uid));
                }
                after_card_played_hand(&catalog, &mut state, spec, uid, Some(3), &mut events)
                    .unwrap();
            }
            // Attack bodies.
            assert_eq!(state.ornamental_fan(), 1, "{label}");
            assert_eq!(state.nunchaku(), 1, "{label}");
            assert_eq!(state.kunai(), 1, "{label}");
            assert_eq!(state.shuriken(), 1, "{label}");
            assert_eq!(state.fanouts.kusarigama(), 1, "{label}");
            assert!(state.fanouts.music_box_used_this_turn(), "{label}");
            // Skill bodies.
            assert_eq!(state.fanouts.tuning_fork_skills(), 1, "{label}");
            // Every-type bodies: three plays.
            assert_eq!(state.fanouts.iron_club_cards(), 3, "{label}");
            assert_eq!(state.energy, energy + 3, "{label}: Ivory Tile");
            // Daughter of the Wind's 1 and Permafrost's 7, once each.
            assert_eq!(state.block, 8, "{label}");
            let gains = events
                .iter()
                .filter(|event| matches!(event, Event::PlayerBlockGained { .. }))
                .count();
            assert_eq!(gains, 2, "{label}");
            // Letter Opener's 5 and Lost Wisp's 8, once each.
            assert_eq!(state.monsters[0].hp, 100 - 5 - 8, "{label}");
            // The Music Box clone and Game Piece's draw, beside the Defend.
            assert_eq!(state.piles.get(PileId::Hand).len(), 3, "{label}");
            assert_eq!(state.piles.get(PileId::Draw).len(), 1, "{label}");
            // One Mummified Hand pick.
            assert_eq!(
                state.rng.get(RngStream::Sel).counter,
                selections + 1,
                "{label}"
            );
            // Rainbow Ring completes once, on the third type.
            assert_eq!(state.powers.value(PowerId::Strength), 1, "{label}");
            assert_eq!(state.powers.value(PowerId::Dexterity), 1, "{label}");
            // Razor Tooth upgraded the Attack and the Skill, once each.
            let play = state.piles.get(PileId::Play).as_slice();
            assert!(play.iter().all(|card| card.atom != strike), "{label}");
            assert!(play.iter().all(|card| card.atom != defend), "{label}");
        }
    }

    /// #2909: a duplicated inventory entry runs its body once. A body gated
    /// on combat state rather than ownership still runs for an inventory that
    /// does not hold its relic; that state is fabricated here, since no
    /// admitted root carries it (see [`WalkSlot::Unowned`]).
    #[test]
    fn vouched_walk_handles_duplicate_and_unowned_entries() {
        let order = [
            RelicId::RelicDaughterOfTheWind,
            RelicId::RelicKunai,
            RelicId::RelicDaughterOfTheWind,
        ];
        let (catalog, mut state, [strike, ..]) = walk_order_fixture(&order, true, &[50], false);
        play_walk_strike(&catalog, &mut state, strike);
        assert_eq!(state.block, 1);
        let (catalog, mut state, [strike, ..]) =
            walk_order_fixture(&[RelicId::RelicKunai], true, &[50], false);
        state.fanouts.set_vambrace_available(true);
        state.fanouts.set_vambrace_trigger_uid(Some(1));
        state.fanouts.set_paels_legion_trigger_uid(Some(1));
        play_walk_strike(&catalog, &mut state, strike);
        assert!(!state.fanouts.vambrace_available());
        assert_eq!(state.fanouts.paels_legion_trigger_uid(), None);
        assert_eq!(state.fanouts.paels_legion_cooldown(), 2);
    }

    /// #3192 commutation table for counters recorded before an out-of-walk
    /// peer (see [`counter_precedes_out_of_walk_peer_commutes`]).
    #[test]
    fn out_of_walk_peer_commutation_table() {
        for (counter, peer, commutes) in [
            (RelicId::RelicIronClub, RelicId::RelicPocketwatch, true),
            (RelicId::RelicTuningFork, RelicId::RelicPocketwatch, true),
            (RelicId::RelicIronClub, RelicId::RelicRippleBasin, true),
            (RelicId::RelicTuningFork, RelicId::RelicRippleBasin, true),
            (RelicId::RelicTuningFork, RelicId::RelicPenNib, true),
            (RelicId::RelicIronClub, RelicId::RelicPenNib, false),
            (RelicId::RelicIronClub, RelicId::RelicBrilliantScarf, false),
            (RelicId::RelicTuningFork, RelicId::RelicVelvetChoker, false),
            (RelicId::RelicIronClub, RelicId::RelicUnsettlingLamp, false),
        ] {
            assert!(COUNTER_OUT_OF_WALK_PEERS.contains(&peer));
            assert_eq!(
                counter_precedes_out_of_walk_peer_commutes(counter, peer),
                commutes,
                "{counter:?} before {peer:?}"
            );
        }
    }

    /// `SymbioticVirus/<AfterSideTurnStart>d__7::MoveNext` (`0x332298`) leaves
    /// at `IL_004e`-`IL_0050` once `TurnNumber > 1`, and the oracle's block is
    /// `s.turn <= 1 and not s.over` (frozen Python `_turn_start_after_royal`, deleted #2827). Witnesses the
    /// turn guard and the ending guard of
    /// [`symbiotic_virus_after_side_turn_start`]; the turn-1 channel itself
    /// is witnessed end to end in `entry::opening::fixture_tests`.
    #[test]
    fn symbiotic_virus_channels_only_on_turn_one_of_a_live_combat() {
        let catalog = catalog(&[RelicId::RelicSymbioticVirus]);
        let live = || {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state
        };

        let mut state = live();
        state.turn = 2;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert!(state.orbs.as_slice().is_empty(), "turn 2 channels nothing");

        let mut state = live();
        state.turn = 1;
        state.history.over = true;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert!(
            state.orbs.as_slice().is_empty(),
            "an over combat channels nothing"
        );

        let mut state = live();
        state.turn = 1;
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.orbs.as_slice().len(), SYMBIOTIC_VIRUS_CHANNELS);
    }

    /// `RunicCapacitor/<AfterSideTurnStart>d__4::MoveNext` (`0x32ffac`) leaves
    /// at `IL_004e`-`IL_0050` once `TurnNumber > 1`, and `OrbCmd::AddSlots`
    /// (`0x132f14`) returns at `IsOverOrEnding` (`IL_000c`-`IL_001d`) and
    /// clamps to `10 - Capacity` (`IL_001e`-`IL_0037`). Witnesses the turn
    /// guard of [`runic_capacitor_after_side_turn_start`], the command's
    /// ending gate and clamp reached through it, the zero-base case (no
    /// bootstrap: three slots, not four), and that a non-owner gains nothing.
    #[test]
    fn runic_capacitor_adds_three_slots_only_on_turn_one_of_a_live_combat() {
        let owned = catalog(&[RelicId::RelicRunicCapacitor]);
        let live = |turn: i16, slots: u8| {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.turn = turn;
            state.orbs.set_base_slots(slots.min(3));
            state.orbs.set_slots(slots);
            state
        };
        let run = |catalog: &Catalog, mut state: HotState| {
            after_side_turn_start(catalog, &mut state, true, &mut Vec::new()).unwrap();
            (state.orbs.slots(), state.orbs.base_slots())
        };

        assert_eq!(run(&owned, live(1, 3)), (6, 3), "Defect: 3 + 3");
        assert_eq!(run(&owned, live(1, 0)), (3, 0), "no base slots: 0 + 3");
        assert_eq!(run(&owned, live(1, 9)), (10, 3), "clamped at ten");
        assert_eq!(run(&owned, live(2, 3)), (3, 3), "turn 2 adds nothing");
        assert_eq!(
            run(&catalog(&[]), live(1, 3)),
            (3, 3),
            "a non-owner gains nothing"
        );

        let mut over = live(1, 3);
        over.history.over = true;
        assert_eq!(run(&owned, over), (3, 3), "an over combat adds nothing");

        // The hook did not start (another side's walk): nothing runs.
        let mut state = live(1, 3);
        after_side_turn_start(&owned, &mut state, false, &mut Vec::new()).unwrap();
        assert_eq!(state.orbs.slots(), 3);
    }

    /// Beside Symbiotic Virus with base slots the two bodies commute: the Dark
    /// channel meets a queue that is not full at three slots or at six, so the
    /// fixed position of [`runic_capacitor_after_side_turn_start`] is not
    /// observable. A front Lightning (Cracked Core's, from the earlier hook)
    /// is still there afterwards, so nothing was evoked.
    #[test]
    fn runic_capacitor_beside_symbiotic_virus_keeps_every_orb_at_base_three() {
        let catalog = catalog(&[RelicId::RelicRunicCapacitor, RelicId::RelicSymbioticVirus]);
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.turn = 1;
        state.orbs.set_base_slots(3);
        state.orbs.set_slots(3);
        state
            .orbs
            .push(crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Lightning, None).unwrap());
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.orbs.slots(), 6);
        let kinds: Vec<_> = state.orbs.as_slice().iter().map(|orb| orb.kind()).collect();
        assert_eq!(
            kinds,
            [crate::hot::OrbKind::Lightning, crate::hot::OrbKind::Dark]
        );
        assert_eq!(state.monsters[0].hp, 100, "no evoke hit anything");
    }

    /// `TwistedFunnel/<BeforeSideTurnStart>d__6::MoveNext` (`0x3336e8`):
    /// the turn guard leaves at `IL_0055`-`IL_0057` once `TurnNumber > 1`, the
    /// loop is over `HittableEnemies` (`IL_012d`-`IL_0147`), and each
    /// `Apply<PoisonPower>` (`IL_0192`) is the shared command, so Artifact
    /// eats one application and an over combat applies nothing. Witnesses
    /// each arm of the Funnel's row in [`turn_one_all_enemy_debuffs`].
    #[test]
    fn twisted_funnel_poisons_each_living_enemy_only_on_turn_one() {
        let owned = catalog(&[RelicId::RelicTwistedFunnel]);
        let roster = |turn: i16| {
            let mut state = HotState::at_defaults();
            for (uid, hp, artifact) in [(0, 30, 0), (1, 0, 0), (2, 30, 1)] {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
                monster.uid = uid;
                if artifact != 0 {
                    monster
                        .powers
                        .set(PowerId::Artifact, SlotWire::Int, artifact);
                }
                state.monsters_mut().push(monster);
            }
            state.turn = turn;
            state
        };
        let poison = |state: &HotState| {
            state
                .monsters
                .iter()
                .map(|monster| (monster.powers.value(PowerId::Poison), monster.poison_uid))
                .collect::<Vec<_>>()
        };

        let mut state = roster(1);
        before_side_turn_start(&owned, &mut state, &mut Vec::new()).unwrap();
        // The living enemy takes 4 and the first identity; the dead one is
        // not hittable; the third's Artifact eats the application.
        assert_eq!(poison(&state), [(4, 0), (0, -1), (0, -1)]);
        assert_eq!(state.next_poison_uid, 1);
        assert_eq!(state.monsters[2].powers.value(PowerId::Artifact), 0);
        assert_eq!(
            state.monsters[0].misery_debuff_order,
            crate::hot::MiseryOrder::from_tokens(vec![MiseryToken::Poison])
        );

        let mut state = roster(2);
        before_side_turn_start(&owned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(poison(&state), [(0, -1); 3], "turn 2 applies nothing");

        let mut state = roster(1);
        state.history.over = true;
        before_side_turn_start(&owned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(
            poison(&state),
            [(0, -1); 3],
            "an over combat applies nothing"
        );

        let mut state = roster(1);
        before_side_turn_start(&catalog(&[]), &mut state, &mut Vec::new()).unwrap();
        assert_eq!(poison(&state), [(0, -1); 3], "a non-owner applies nothing");
    }

    /// [`turn_one_all_enemy_debuff_rows`]: a vouched inventory decides the
    /// order of any of the three, first recorded copy deciding; an unvouched
    /// one leaves table order (#3765: the Funnel is no longer a condition).
    #[test]
    fn the_all_enemy_debuff_rows_follow_a_vouched_inventory() {
        const MASK: RelicId = RelicId::RelicRedMask;
        const BAG: RelicId = RelicId::RelicBagOfMarbles;
        const FUNNEL: RelicId = RelicId::RelicTwistedFunnel;
        let order = |inventory: &[RelicId], vouched: bool| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(inventory, vouched).unwrap();
            turn_one_all_enemy_debuff_rows(&builder.build())
                .into_iter()
                .map(|(relic, ..)| relic)
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&[BAG, FUNNEL, MASK], true), [BAG, FUNNEL, MASK]);
        assert_eq!(order(&[FUNNEL, MASK, BAG], true), [FUNNEL, MASK, BAG]);
        assert_eq!(order(&[BAG, FUNNEL, MASK], false), [MASK, BAG, FUNNEL]);
        assert_eq!(order(&[FUNNEL, MASK], true), [FUNNEL, MASK]);
        // An unrelated relic between them does not move anything.
        assert_eq!(
            order(&[FUNNEL, RelicId::RelicVajra, BAG], true),
            [FUNNEL, BAG]
        );
        // No Funnel (#3765): the pair follows a vouched inventory too.
        assert_eq!(order(&[BAG, MASK], true), [BAG, MASK]);
        assert_eq!(order(&[MASK, BAG], true), [MASK, BAG]);
        assert_eq!(order(&[BAG, MASK], false), [MASK, BAG]);
        assert_eq!(order(&[FUNNEL], false), [FUNNEL]);
        assert!(order(&[RelicId::RelicVajra], true).is_empty());
    }

    /// The loop applies in the row order: the recorded order becomes each
    /// enemy's Misery acquisition order.
    #[test]
    fn the_all_enemy_debuffs_land_in_the_recorded_order_beside_the_funnel() {
        let inventory = [
            RelicId::RelicBagOfMarbles,
            RelicId::RelicTwistedFunnel,
            RelicId::RelicRedMask,
        ];
        // #3756: an Artifact below the number of owned relics eats the first
        // recorded applications, one stack each
        // (`ArtifactPower::TryModifyPowerAmountReceived` `0x9f844`, then one
        // `PowerCmd::Decrement` in `<AfterModifyingPowerAmountReceived>d__7`
        // `0x334fd0` IL_001e). Both orders of the pair, and the trio at one
        // and two stacks; an eaten debuff leaves no Misery token.
        let landed = |inventory: &[RelicId], artifact: i32| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(inventory, true).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster
                .powers
                .set(PowerId::Artifact, SlotWire::Int, artifact);
            state.monsters_mut().push(monster);
            state.turn = 1;
            before_side_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
            let monster = &state.monsters[0];
            (
                [
                    PowerId::Weak,
                    PowerId::Vuln,
                    PowerId::Poison,
                    PowerId::Artifact,
                ]
                .map(|power| monster.powers.value(power)),
                monster.misery_debuff_order.clone(),
            )
        };
        let order = |tokens: &[MiseryToken]| crate::hot::MiseryOrder::from_tokens(tokens.to_vec());
        const MASK: RelicId = RelicId::RelicRedMask;
        const BAG: RelicId = RelicId::RelicBagOfMarbles;
        const FUNNEL: RelicId = RelicId::RelicTwistedFunnel;
        assert_eq!(
            landed(&[MASK, BAG], 1),
            ([0, 1, 0, 0], order(&[MiseryToken::Vuln])),
            "Red Mask first: its Weak is eaten"
        );
        assert_eq!(
            landed(&[BAG, MASK], 1),
            ([1, 0, 0, 0], order(&[MiseryToken::Weak])),
            "Bag of Marbles first: its Vulnerable is eaten"
        );
        assert_eq!(
            landed(&inventory, 1),
            (
                [1, 0, 4, 0],
                order(&[MiseryToken::Poison, MiseryToken::Weak])
            ),
        );
        assert_eq!(
            landed(&inventory, 2),
            ([1, 0, 0, 0], order(&[MiseryToken::Weak])),
        );
        assert_eq!(
            landed(&[FUNNEL, MASK, BAG], 2),
            ([0, 1, 0, 0], order(&[MiseryToken::Vuln])),
        );
        assert_eq!(landed(&inventory, 3), ([0, 0, 0, 0], order(&[])));
        assert_eq!(landed(&inventory, 4), ([0, 0, 0, 1], order(&[])));
        let tokens = |vouched: bool| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(&inventory, vouched).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 30));
            state.turn = 1;
            before_side_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
            let monster = &state.monsters[0];
            assert_eq!(monster.powers.value(PowerId::Weak), 1);
            assert_eq!(monster.powers.value(PowerId::Vuln), 1);
            assert_eq!(monster.powers.value(PowerId::Poison), 4);
            monster.misery_debuff_order.clone()
        };
        assert_eq!(
            tokens(true),
            crate::hot::MiseryOrder::from_tokens(vec![
                MiseryToken::Vuln,
                MiseryToken::Poison,
                MiseryToken::Weak
            ])
        );
        assert_eq!(
            tokens(false),
            crate::hot::MiseryOrder::from_tokens(vec![
                MiseryToken::Weak,
                MiseryToken::Vuln,
                MiseryToken::Poison
            ])
        );
    }

    /// `TeaOfDiscourtesy/<BeforeCombatStart>d__16::MoveNext` (`0x3324dc`):
    /// a charged Tea inserts two fresh Dazed into Draw, one `Rng` draw each,
    /// and spends the charge; a spent one, a non-owner and an ended combat
    /// write nothing. Witnesses each return of
    /// [`tea_of_discourtesy_before_combat_start`] and the charged body.
    #[test]
    fn tea_of_discourtesy_inserts_two_dazed_once_and_spends_the_charge() {
        let build = |relics: &[RelicId]| {
            let mut builder = CatalogBuilder::new();
            let strike = builder
                .intern(CardIdentity {
                    id: CardId::StrikeIronclad,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let dazed = builder
                .intern(CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            builder.set_relics(relics).unwrap();
            (builder.build(), strike, dazed)
        };
        let (owned, strike, dazed) = build(&[RelicId::RelicTeaOfDiscourtesy]);
        let live = |charged: bool, over: bool| {
            let mut state = HotState::at_defaults();
            state.history.over = over;
            state.next_card_uid = 3;
            state.set_tea_of_discourtesy_active(charged);
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((0..3).map(|uid| HotCard {
                    uid,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            state
        };
        let draw = |state: &HotState| -> Vec<(u32, bool)> {
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| (card.uid, card.atom == dazed))
                .collect()
        };
        let untouched = vec![(0, false), (1, false), (2, false)];

        let mut state = live(true, false);
        let rng_before = state.rng.get(crate::hot::RngStream::Rng).counter;
        tea_of_discourtesy_before_combat_start(&owned, &mut state, &mut Vec::new()).unwrap();
        let after = draw(&state);
        assert_eq!(after.len(), 5);
        let mut fresh: Vec<u32> = after
            .iter()
            .filter(|(_, is_dazed)| *is_dazed)
            .map(|(uid, _)| *uid)
            .collect();
        fresh.sort_unstable();
        assert_eq!(fresh, [3, 4], "two Dazed on the next two uids");
        assert_eq!(
            after
                .iter()
                .filter(|(_, is_dazed)| !is_dazed)
                .copied()
                .collect::<Vec<_>>(),
            untouched,
            "the deck cards keep their order"
        );
        assert_eq!(
            state.rng.get(crate::hot::RngStream::Rng).counter,
            rng_before + 2
        );
        assert_eq!(state.history.owner_generated_cards_combat, 2);
        assert!(!state.tea_of_discourtesy_active(), "the charge is spent");

        // Spent now: a second walk inserts nothing.
        let spent = state.clone();
        tea_of_discourtesy_before_combat_start(&owned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state, spent);

        let mut state = live(false, false);
        tea_of_discourtesy_before_combat_start(&owned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(draw(&state), untouched, "a spent Tea");

        let mut state = live(true, true);
        tea_of_discourtesy_before_combat_start(&owned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(draw(&state), untouched, "an over combat");
        assert!(state.tea_of_discourtesy_active());

        let (unowned, _, _) = build(&[]);
        let mut state = live(true, false);
        tea_of_discourtesy_before_combat_start(&unowned, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(draw(&state), untouched, "non-owner");
    }

    /// `InfusedCore/<AfterSideTurnStart>d__8::MoveNext` (`0x326d00`) leaves at
    /// `IL_004e`-`IL_0050` once `TurnNumber > 1` and otherwise channels three
    /// Lightning through the shared command. Witnesses the turn guard of
    /// [`infused_core_after_side_turn_start`], the non-owner and
    /// hook-not-started returns, the command's ending gate, and the three
    /// queue shapes: three into a Defect's empty queue, an evoke of Cracked
    /// Core's front Lightning when four do not fit, and two evokes behind a
    /// zero-base bootstrap.
    #[test]
    fn infused_core_channels_three_lightning_only_on_turn_one_of_a_live_combat() {
        let owned = catalog(&[RelicId::RelicInfusedCore]);
        let lightning =
            || crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Lightning, None).unwrap();
        let live = |turn: i16, base: u8| {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.turn = turn;
            state.orbs.set_base_slots(base);
            state.orbs.set_slots(base);
            state
        };
        let run = |catalog: &Catalog, mut state: HotState| {
            after_side_turn_start(catalog, &mut state, true, &mut Vec::new()).unwrap();
            let kinds: Vec<_> = state.orbs.as_slice().iter().map(|orb| orb.kind()).collect();
            (kinds, state.orbs.slots(), state.monsters[0].hp)
        };
        let l = crate::hot::OrbKind::Lightning;

        assert_eq!(run(&owned, live(1, 3)), (vec![l, l, l], 3, 100));
        assert_eq!(run(&owned, live(2, 3)), (vec![], 3, 100), "turn 2");
        assert_eq!(
            run(&catalog(&[]), live(1, 3)),
            (vec![], 3, 100),
            "non-owner"
        );

        let mut over = live(1, 3);
        over.history.over = true;
        assert_eq!(run(&owned, over), (vec![], 3, 100), "an over combat");

        let mut state = live(1, 3);
        after_side_turn_start(&owned, &mut state, false, &mut Vec::new()).unwrap();
        assert!(state.orbs.as_slice().is_empty(), "the hook did not start");

        // Behind Cracked Core's Lightning the third channel meets a full
        // queue and evokes the front orb: 8 + Infused Core's 1.
        let mut queued = live(1, 3);
        queued.orbs.push(lightning());
        assert_eq!(run(&owned, queued), (vec![l, l, l], 3, 91));

        // No base slots: the first channel bootstraps one slot, and each of
        // the next two evokes the Lightning before it.
        assert_eq!(run(&owned, live(1, 0)), (vec![l], 1, 82));
    }

    /// `NinjaScroll/<BeforeHandDraw>d__7::MoveNext` (`0x32b770`) leaves at
    /// `IL_0044`-`IL_0046` once `TurnNumber > 1`, and `Shiv::CreateInHand`
    /// (`0x3bb20c`) returns nothing while the combat is over or ending
    /// (`IL_002c`-`IL_003e`). Witnesses each arm of
    /// [`ninja_scroll_before_hand_draw`]: three level-zero Shivs with fresh
    /// uids on turn one, and nothing on turn two, in an over combat or for a
    /// non-owner.
    #[test]
    fn ninja_scroll_generates_three_shivs_only_on_turn_one_of_a_live_combat() {
        let build = |owned: bool| {
            let mut builder = CatalogBuilder::new();
            let shiv = builder
                .intern_reachable(CardIdentity {
                    id: CardId::Shiv,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            if owned {
                builder.set_relics(&[RelicId::RelicNinjaScroll]).unwrap();
            }
            (builder.build(), shiv)
        };
        let run = |owned: bool, turn: i16, over: bool| {
            let (catalog, shiv) = build(owned);
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.turn = turn;
            state.history.over = over;
            state.next_card_uid = 7;
            ninja_scroll_before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
            let hand = state.piles.get(PileId::Hand).as_slice().to_vec();
            assert!(hand.iter().all(|card| card.atom == shiv));
            hand.iter().map(|card| card.uid).collect::<Vec<_>>()
        };
        assert_eq!(run(true, 1, false), [7, 8, 9]);
        assert!(run(true, 2, false).is_empty(), "turn 2");
        assert!(run(true, 1, true).is_empty(), "an over combat");
        assert!(run(false, 1, false).is_empty(), "a non-owner");
    }

    /// `Bellows::AfterPlayerTurnStart` (`0x906dc`) returns `CompletedTask`
    /// unless `TurnNumber <= 1` (`IL_001b`-`IL_002c`), and the oracle's block
    /// is `s.turn <= 1 and s.bellows and not s.over` (frozen Python `_continue_after_toasty_mittens`, deleted #2827).
    /// Witnesses the turn guard and the ending guard of
    /// [`bellows_after_player_turn_start`] against a live turn-1 control; the
    /// turn-1 upgrade itself is witnessed end to end in
    /// `entry::opening::fixture_tests`. The ending case reaches the body's
    /// `history.over` arm, but `cards::upgrade_live_cards_once` carries the
    /// same guard, so this pins the observable outcome, not that one arm alone
    /// (dropping the turn arm fails the turn-2 case).
    #[test]
    fn bellows_upgrades_the_hand_only_on_turn_one_of_a_live_combat() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let upgraded = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 1,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics(&[RelicId::RelicBellows]).unwrap();
        let catalog = builder.build();
        let live = |turn: i16, over: bool| {
            let mut state = HotState::at_defaults();
            state.turn = turn;
            state.history.over = over;
            state.next_card_uid = 2;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
        };
        let hand_atom = |state: &HotState| state.piles.get(PileId::Hand).as_slice()[0].atom;

        let mut state = live(2, false);
        bellows_after_player_turn_start(&catalog, &mut state).unwrap();
        assert_eq!(hand_atom(&state), strike, "turn 2 upgrades nothing");

        let mut state = live(1, true);
        bellows_after_player_turn_start(&catalog, &mut state).unwrap();
        assert_eq!(hand_atom(&state), strike, "an over combat upgrades nothing");

        let mut state = live(1, false);
        bellows_after_player_turn_start(&catalog, &mut state).unwrap();
        assert_eq!(hand_atom(&state), upgraded, "turn 1 upgrades the Hand");
    }

    /// `FestivePopper/<AfterPlayerTurnStart>d__4::MoveNext` (`0x324b94`)
    /// `leave`s unless `TurnNumber == 1` (`beq.s` at `IL_0055`), and the
    /// oracle's block is `s.turn == 1 and s.popper and not s.over`
    /// (frozen Python `_continue_after_toasty_mittens`, deleted #2827). Witnesses the turn guard and the ending guard
    /// of [`festive_popper_after_player_turn_start`] against a live turn-1
    /// control; the turn-1 hit itself is witnessed end to end in
    /// `entry::opening::fixture_tests`.
    #[test]
    fn festive_popper_hits_only_on_turn_one_of_a_live_combat() {
        let catalog = catalog(&[RelicId::RelicFestivePopper]);
        let live = |turn: i16, over: bool| {
            let mut state = HotState::at_defaults();
            state.turn = turn;
            state.history.over = over;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state
        };

        let mut state = live(2, false);
        festive_popper_after_player_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 100, "turn 2 deals nothing");

        let mut state = live(1, true);
        festive_popper_after_player_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 100, "an over combat deals nothing");

        let mut state = live(1, false);
        festive_popper_after_player_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(
            i64::from(state.monsters[0].hp),
            100 - FESTIVE_POPPER_DAMAGE,
            "turn 1 hits for nine"
        );
    }

    fn fencing_manual_fixture(owned: bool) -> (Catalog, crate::catalog::CardAtom, HotState) {
        let mut builder = CatalogBuilder::new();
        let relics: &[RelicId] = if owned {
            &[RelicId::RelicFencingManual]
        } else {
            &[]
        };
        builder.set_relics(relics).unwrap();
        let blade = builder
            .intern_reachable(CardIdentity {
                id: CardId::SovereignBlade,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.next_card_uid = 30;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        (catalog, blade, state)
    }

    /// `FencingManual/<AfterSideTurnStart>d__6::MoveNext` (`0x324a80`): on
    /// turn one with no live Blade, `ForgeCmd::Forge(10)` generates the L0
    /// Blade at the Hand's bottom and grows it to 20.
    #[test]
    fn fencing_manual_generates_a_twenty_damage_blade_on_turn_one() {
        let (catalog, _, mut state) = fencing_manual_fixture(true);
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        let hand = state.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), 1);
        let made = hand[0];
        assert_eq!(made.uid, 30);
        assert_ne!(made.flags & crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE, 0);
        assert_eq!(
            catalog.spec(made.atom).unwrap().identity.id,
            CardId::SovereignBlade
        );
        assert_eq!(state.card_states.get(made.uid).damage_growth, 20);
        assert_eq!(state.next_card_uid, 31);
        assert_eq!(state.history.owner_generated_cards_combat, 1);
    }

    /// With a live Blade the Forge generates nothing and grows every Blade,
    /// the Exhausted one included (`IncreaseSovereignBladeDamage` `0x132c24`).
    #[test]
    fn fencing_manual_grows_existing_blades_without_generating() {
        let (catalog, blade, mut state) = fencing_manual_fixture(true);
        for (uid, pile) in [(5, PileId::Draw), (6, PileId::Exhaust)] {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid,
                atom: blade,
                flags: crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE,
            });
            state.card_states.set(
                uid,
                CardInstanceState {
                    damage_growth: 10,
                    ..CardInstanceState::default()
                },
            );
        }
        after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert!(state.piles.get(PileId::Hand).as_slice().is_empty());
        assert_eq!(state.card_states.get(5).damage_growth, 20);
        assert_eq!(state.card_states.get(6).damage_growth, 20);
        assert_eq!(state.next_card_uid, 30);
        assert_eq!(state.history.owner_generated_cards_combat, 0);
    }

    /// The `TurnNumber <= 1` guard (`IL_003d`-`IL_004e`), the Forge's own
    /// ending check, ownership, and the walk's not-started gate each leave the
    /// state untouched.
    #[test]
    fn fencing_manual_is_inert_past_turn_one_when_ending_or_unowned() {
        let untouched = |catalog: &Catalog, state: &mut HotState, started: bool| {
            let before = state.clone();
            after_side_turn_start(catalog, state, started, &mut Vec::new()).unwrap();
            assert!(state.piles.get(PileId::Hand).as_slice().is_empty());
            assert_eq!(state.next_card_uid, before.next_card_uid);
            assert_eq!(
                state.history.owner_generated_cards_combat,
                before.history.owner_generated_cards_combat
            );
        };
        let (catalog, _, mut state) = fencing_manual_fixture(true);
        state.turn = 2;
        untouched(&catalog, &mut state, true);

        let (catalog, _, mut state) = fencing_manual_fixture(true);
        state.history.over = true;
        untouched(&catalog, &mut state, true);

        let (catalog, _, mut state) = fencing_manual_fixture(true);
        untouched(&catalog, &mut state, false);

        let (catalog, _, mut state) = fencing_manual_fixture(false);
        untouched(&catalog, &mut state, true);
    }

    /// A Jeweled Mask fixture: Draw holds `draw` in order, uid = index.
    fn jeweled_mask_fixture(
        owned: bool,
        draw: &[(CardId, u8)],
    ) -> (Catalog, Vec<crate::catalog::CardAtom>, HotState) {
        let mut builder = CatalogBuilder::new();
        let relics: &[RelicId] = if owned {
            &[RelicId::RelicJeweledMask]
        } else {
            &[]
        };
        builder.set_relics(relics).unwrap();
        let atoms = draw
            .iter()
            .map(|(id, upgrade)| {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: *upgrade,
                        enchantment: None,
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.next_card_uid = draw.len() as u32;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let seeded = Xoshiro256StarStar::from_seed(11);
        state.rng.set(
            RngStream::Sel,
            RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend(atoms.iter().enumerate().map(|(uid, atom)| HotCard {
                uid: uid as u32,
                atom: *atom,
                flags: 0,
            }));
        (catalog, atoms, state)
    }

    fn jeweled_mask_hand_uids(state: &HotState) -> Vec<u32> {
        state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect()
    }

    /// `JeweledMask/<BeforeHandDraw>d__2::MoveNext` (`0x3272f4`): with a
    /// non-Innate Power in Draw, the `b__2_1` list replaces the Power list
    /// (`IL_00c1`-`IL_00cd`), so the one non-Innate Power is the only
    /// candidate however many Innate ones sit beside it. It leaves Draw for
    /// the Hand's bottom, free this turn, after one `CombatCardSelection` draw.
    #[test]
    fn jeweled_mask_prefers_the_non_innate_power_and_makes_it_free() {
        let draw = [
            (CardId::StrikeIronclad, 0),
            (CardId::Aggression, 1),
            (CardId::Aggression, 1),
            (CardId::Inflame, 0),
            (CardId::Aggression, 1),
            (CardId::Aggression, 1),
        ];
        let (catalog, atoms, mut state) = jeweled_mask_fixture(true, &draw);
        assert!(
            catalog.spec(atoms[1]).unwrap().innate,
            "Aggression+ is Innate"
        );
        assert!(!catalog.spec(atoms[3]).unwrap().innate, "Inflame is not");
        let counter = state.rng.get(RngStream::Sel).counter;
        jeweled_mask_before_hand_draw(&catalog, &mut state).unwrap();
        assert_eq!(jeweled_mask_hand_uids(&state), [3]);
        assert_eq!(state.piles.get(PileId::Draw).len(), 5);
        assert_eq!(state.rng.get(RngStream::Sel).counter, counter + 1);
        assert_eq!(
            state
                .card_states
                .get(3)
                .free_star_cost_this_turn_or_played_rows,
            1
        );
        assert_eq!(
            state.piles.get(PileId::Hand).as_slice()[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            "the moved card carries the slot-7 bit its free rows project through (#3176)"
        );
        assert!(state.exact_piles);
    }

    /// An all-Innate Power list keeps the whole Power list (the replacement
    /// needs a non-empty narrower list), so an Innate Power is still chosen;
    /// a non-Power never is.
    #[test]
    fn jeweled_mask_falls_back_to_an_innate_power_when_every_power_is_innate() {
        let draw = [
            (CardId::StrikeIronclad, 0),
            (CardId::Aggression, 1),
            (CardId::StrikeIronclad, 0),
        ];
        let (catalog, _, mut state) = jeweled_mask_fixture(true, &draw);
        jeweled_mask_before_hand_draw(&catalog, &mut state).unwrap();
        assert_eq!(jeweled_mask_hand_uids(&state), [1]);
    }

    /// A full Hand sends the card to Discard (`CardPileCmd::Add`'s overflow),
    /// still out of Draw.
    #[test]
    fn jeweled_mask_overflows_a_full_hand_to_discard() {
        let (catalog, atoms, mut state) =
            jeweled_mask_fixture(true, &[(CardId::Inflame, 0), (CardId::StrikeIronclad, 0)]);
        let filler = (0..super::super::draw::MAX_CARDS_IN_HAND as u32).map(|index| HotCard {
            uid: 100 + index,
            atom: atoms[1],
            flags: 0,
        });
        state.piles.get_mut(PileId::Hand).make_mut().extend(filler);
        jeweled_mask_before_hand_draw(&catalog, &mut state).unwrap();
        let discard: Vec<u32> = state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect();
        assert_eq!(discard, [0]);
        assert_ne!(
            state.piles.get(PileId::Discard).as_slice()[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            0,
            "the overflowed card keeps its free rows projectable too (#3176)"
        );
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
    }

    /// No Power in Draw leaves before `NextItem` (`IL_0088`-`IL_0090`): no
    /// Selection draw and no pile change. Past turn one or unowned, the body
    /// is inert. Reached on a combat an earlier listener ended (#3635) it
    /// still draws the stream and frees the Power, which stays in Draw: only
    /// the move is gated (`CardPileCmd/<Add>d__10` `0x3e1ba4`
    /// `IL_0041`-`IL_008a`). The draw and the unmoved card are the game's
    /// (`before_hand_draw_relics_behind_a_listener_that_ended_combat_do_what_the_game_does`);
    /// the free cost is read from IL.
    #[test]
    fn jeweled_mask_is_inert_without_a_power_past_turn_one_ending_or_unowned() {
        let untouched = |catalog: &Catalog, mut state: HotState| {
            let before = state.clone();
            jeweled_mask_before_hand_draw(catalog, &mut state).unwrap();
            assert!(state.piles.get(PileId::Hand).as_slice().is_empty());
            assert_eq!(
                state.piles.get(PileId::Draw).as_slice(),
                before.piles.get(PileId::Draw).as_slice()
            );
            assert_eq!(
                state.rng.get(RngStream::Sel).counter,
                before.rng.get(RngStream::Sel).counter
            );
        };
        let (catalog, _, state) = jeweled_mask_fixture(true, &[(CardId::StrikeIronclad, 0)]);
        untouched(&catalog, state);
        let (catalog, _, mut state) = jeweled_mask_fixture(true, &[(CardId::Inflame, 0)]);
        state.turn = 2;
        untouched(&catalog, state);
        let (catalog, _, state) = jeweled_mask_fixture(false, &[(CardId::Inflame, 0)]);
        untouched(&catalog, state);
        let (catalog, _, mut state) = jeweled_mask_fixture(true, &[(CardId::Inflame, 0)]);
        state.history.over = true;
        let before = state.clone();
        jeweled_mask_before_hand_draw(&catalog, &mut state).unwrap();
        assert!(state.piles.get(PileId::Hand).as_slice().is_empty());
        let draw = state.piles.get(PileId::Draw).as_slice();
        let was = before.piles.get(PileId::Draw).as_slice();
        assert_eq!(draw.len(), was.len());
        assert_eq!((draw[0].uid, draw[0].atom), (was[0].uid, was[0].atom));
        assert_ne!(draw[0].flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_ne!(
            state.card_states.get(draw[0].uid),
            before.card_states.get(was[0].uid),
            "the Power in Draw is free this turn"
        );
        assert_eq!(
            state.rng.get(RngStream::Sel).counter,
            before.rng.get(RngStream::Sel).counter + 1
        );
    }

    fn juggernaut_state(monsters: &[i32]) -> HotState {
        use crate::powers::SlotWire;
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        for (uid, hp) in (0_u32..).zip(monsters) {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, *hp);
            monster.uid = uid;
            state.monsters_mut().push(monster);
        }
        state
    }

    /// Whether the first order-relevant event is a monster hit (the damage
    /// relic ran first) rather than the relic's block gain.
    fn damage_ran_first(events: &[Event]) -> bool {
        events
            .iter()
            .find_map(|event| match event {
                Event::MonsterDamaged { .. } => Some(true),
                Event::PlayerBlockGained { .. } => Some(false),
                _ => None,
            })
            .expect("a block gain or a monster hit")
    }

    /// #2909: every turn-end damage/block pair admission used to refuse with
    /// Juggernaut, in both recorded orders. A vouched inventory runs the pair
    /// in that order; an unvouched one keeps the fixed block-first order.
    #[test]
    fn turn_end_damage_relic_runs_at_its_recorded_side_of_each_block_relic() {
        let pairs = [
            (RelicId::RelicStoneCalendar, RelicId::RelicOrichalcum),
            (RelicId::RelicStoneCalendar, RelicId::RelicFakeOrichalcum),
            (RelicId::RelicStoneCalendar, RelicId::RelicCloakClasp),
            (RelicId::RelicStoneCalendar, RelicId::RelicRippleBasin),
            (RelicId::RelicScreamingFlagon, RelicId::RelicOrichalcum),
            (RelicId::RelicScreamingFlagon, RelicId::RelicFakeOrichalcum),
            (RelicId::RelicScreamingFlagon, RelicId::RelicRippleBasin),
        ];
        for (damage, block) in pairs {
            for damage_recorded_first in [true, false] {
                for vouched in [true, false] {
                    let order = if damage_recorded_first {
                        [damage, block]
                    } else {
                        [block, damage]
                    };
                    let mut builder = CatalogBuilder::new();
                    let atom = builder
                        .intern(CardIdentity {
                            id: CardId::DefendIronclad,
                            upgrade: 0,
                            enchantment: None,
                        })
                        .unwrap();
                    builder.set_relics_ordered(&order, vouched).unwrap();
                    let catalog = builder.build();
                    let mut state = juggernaut_state(&[200, 200]);
                    if damage == RelicId::RelicStoneCalendar {
                        state.turn = 7;
                        state.piles.get_mut(PileId::Hand).make_mut().extend([
                            HotCard {
                                uid: 1,
                                atom,
                                flags: 0,
                            },
                            HotCard {
                                uid: 2,
                                atom,
                                flags: 0,
                            },
                        ]);
                    }
                    let mut events = Vec::new();
                    before_side_turn_end_hand(
                        &catalog,
                        &mut state,
                        block == RelicId::RelicOrichalcum,
                        block == RelicId::RelicFakeOrichalcum,
                        &mut events,
                    )
                    .unwrap();
                    assert!(state.block > 0, "{block:?} gained");
                    assert_eq!(
                        damage_ran_first(&events),
                        vouched && damage_recorded_first,
                        "{order:?} vouched={vouched}"
                    );
                    // One AoE hit per monster plus one Juggernaut hit.
                    let hits = events
                        .iter()
                        .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                        .count();
                    assert_eq!(hits, 3, "{order:?} vouched={vouched}");
                }
            }
        }
    }

    /// #2909: the orders do not commute under Juggernaut. Flagon recorded
    /// first kills the 20-HP enemy before Fake Orichalcum's block, so the
    /// Juggernaut roll has only the survivor to hit. Recorded second (the
    /// fixed order), Juggernaut's hit lands before the AoE.
    #[test]
    fn flagon_before_fake_orichalcum_leaves_juggernaut_one_target() {
        let run = |order: [RelicId; 2]| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(&order, true).unwrap();
            let catalog = builder.build();
            let mut state = juggernaut_state(&[20, 100]);
            let mut events = Vec::new();
            before_side_turn_end_hand(&catalog, &mut state, false, true, &mut events).unwrap();
            (state, events)
        };
        let (flagon_first, events) =
            run([RelicId::RelicScreamingFlagon, RelicId::RelicFakeOrichalcum]);
        assert!(damage_ran_first(&events));
        assert_eq!(flagon_first.monsters[0].hp, 0);
        assert_eq!(flagon_first.monsters[1].hp, 75, "Flagon 20 + Juggernaut 5");
        assert_eq!(flagon_first.block, 3);

        let (fake_first, events) =
            run([RelicId::RelicFakeOrichalcum, RelicId::RelicScreamingFlagon]);
        assert!(!damage_ran_first(&events));
        assert_eq!(fake_first.block, 3);
        assert!(fake_first.monsters[0].hp <= 0);
        let first_hit = events
            .iter()
            .find_map(|event| match event {
                Event::MonsterDamaged { unblocked, .. } => Some(*unblocked),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            first_hit, 5,
            "the first hit is Juggernaut's, before the AoE"
        );
    }

    fn lost_wisp_permafrost_run(order: &[RelicId], vouched: bool) -> (HotState, Vec<Event>) {
        let mut builder = CatalogBuilder::new();
        let power_atom = builder
            .intern(CardIdentity {
                id: CardId::DemonForm,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.set_relics_ordered(order, vouched).unwrap();
        let catalog = builder.build();
        let power = *catalog.spec(power_atom).unwrap();
        let mut state = juggernaut_state(&[8, 50]);
        state.set_permafrost_armed(true);
        let mut events = Vec::new();
        after_card_played_hand(&catalog, &mut state, &power, 0, Some(3), &mut events).unwrap();
        (state, events)
    }

    /// #2909: Permafrost runs at its recorded side of Lost Wisp on a vouched
    /// inventory, and after it (the fixed order) on an unvouched one.
    #[test]
    fn permafrost_runs_at_its_recorded_side_of_lost_wisp() {
        for permafrost_recorded_first in [true, false] {
            for vouched in [true, false] {
                let order = if permafrost_recorded_first {
                    [RelicId::RelicPermafrost, RelicId::RelicLostWisp]
                } else {
                    [RelicId::RelicLostWisp, RelicId::RelicPermafrost]
                };
                let (state, events) = lost_wisp_permafrost_run(&order, vouched);
                assert_eq!(state.block, 7);
                assert!(!state.permafrost_armed());
                assert!(state.monsters[0].hp <= 0, "Lost Wisp kills the first");
                assert_eq!(
                    damage_ran_first(&events),
                    !(vouched && permafrost_recorded_first),
                    "{order:?} vouched={vouched}"
                );
            }
        }
        // Lost Wisp first: the Juggernaut roll sees only the survivor.
        let (wisp_first, _) =
            lost_wisp_permafrost_run(&[RelicId::RelicLostWisp, RelicId::RelicPermafrost], true);
        assert_eq!(wisp_first.monsters[1].hp, 50 - 8 - 5);
    }

    /// #2909: a counter relic recorded between the pair changes nothing: each
    /// still runs at its recorded position, and Permafrost runs once.
    #[test]
    fn permafrost_and_lost_wisp_split_by_a_counter_keep_their_recorded_order() {
        for order in [
            [
                RelicId::RelicPermafrost,
                RelicId::RelicTuningFork,
                RelicId::RelicLostWisp,
            ],
            [
                RelicId::RelicLostWisp,
                RelicId::RelicTuningFork,
                RelicId::RelicPermafrost,
            ],
        ] {
            let (_, events) = lost_wisp_permafrost_run(&order, true);
            let gains = events
                .iter()
                .filter(|event| matches!(event, Event::PlayerBlockGained { .. }))
                .count();
            assert_eq!(gains, 1, "{order:?}: Permafrost runs exactly once");
            assert_eq!(
                damage_ran_first(&events),
                order[0] == RelicId::RelicLostWisp,
                "{order:?}"
            );
        }
    }

    // --- #3321: the vouched AfterPlayerTurnStart moves for Mercury Hourglass
    // and Vexing Puzzlebox.

    /// The relic walk as the turn start enters it when no power listener ran:
    /// the pass began live exactly when the combat is live now.
    fn after_player_turn_start_from_entry(
        catalog: &Catalog,
        state: &mut HotState,
    ) -> Result<(), EngineRefusal> {
        let live = !state.history.over;
        after_player_turn_start(catalog, state, live, &mut Vec::new())
    }

    fn ordered_catalog(relics: &[RelicId], ordered: bool) -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.set_relics_ordered(relics, ordered).unwrap();
        builder.build()
    }

    /// The walk behind Choices Paradox's grid (or from the top without one).
    fn continue_after_choices_paradox(
        catalog: &Catalog,
        state: &mut HotState,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let after = catalog
            .hooks()
            .owns(RelicId::RelicChoicesParadox)
            .then_some(RelicId::RelicChoicesParadox);
        after_player_turn_start_walk(catalog, state, events, after, true)
    }

    /// The walk behind Gambling Chip's choice (or behind the Paradox grid, or
    /// from the top, whichever the inventory has).
    fn continue_after_gambling_chip(
        catalog: &Catalog,
        state: &mut HotState,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let hooks = catalog.hooks();
        let after = [RelicId::RelicGamblingChip, RelicId::RelicChoicesParadox]
            .into_iter()
            .find(|relic| hooks.owns(*relic));
        after_player_turn_start_walk(catalog, state, events, after, true)
    }

    /// #3635: the walk is the recorded order of a vouched inventory, whatever
    /// that order is and whatever else the inventory holds, and the fixed
    /// sequence for an unvouched one. `after_player_turn_start_relic_follows`
    /// reads the same order.
    #[test]
    fn the_walk_runs_a_vouched_inventory_as_recorded_and_an_unvouched_one_as_fixed() {
        let fixed = AFTER_PLAYER_TURN_START_RELIC_DISPATCH.to_vec();
        let reversed = fixed.iter().rev().copied().collect::<Vec<_>>();
        let mut rotated = fixed.clone();
        rotated.rotate_left(4);
        for recorded in [&fixed, &reversed, &rotated] {
            // Relics of other hooks sit between the listeners and do not move
            // them.
            let mut inventory = vec![RelicId::RelicAkabeko];
            for relic in recorded {
                inventory.push(*relic);
                if *relic == RelicId::RelicBellows {
                    inventory.push(RelicId::RelicWhetstone);
                }
            }
            let walk = |vouched| {
                let catalog = ordered_catalog(&inventory, vouched);
                relic_walk_order(&catalog, &AFTER_PLAYER_TURN_START_RELIC_DISPATCH)
                    .collect::<Vec<_>>()
            };
            assert_eq!(&walk(true), recorded);
            assert_eq!(walk(false), fixed);
            let catalog = ordered_catalog(&inventory, true);
            for (place, relic) in recorded.iter().enumerate() {
                for (other_place, other) in recorded.iter().enumerate() {
                    assert_eq!(
                        after_player_turn_start_relic_follows(&catalog, *relic, *other),
                        place > other_place,
                        "{relic:?} behind {other:?} in {recorded:?}"
                    );
                }
            }
        }
        // A subset walks in its own recorded order; an absent relic follows
        // nothing and nothing follows it.
        let catalog = ordered_catalog(
            &[RelicId::RelicMercuryHourglass, RelicId::RelicChoicesParadox],
            true,
        );
        assert_eq!(
            relic_walk_order(&catalog, &AFTER_PLAYER_TURN_START_RELIC_DISPATCH).collect::<Vec<_>>(),
            [RelicId::RelicMercuryHourglass, RelicId::RelicChoicesParadox]
        );
        assert!(!after_player_turn_start_relic_follows(
            &catalog,
            RelicId::RelicVexingPuzzlebox,
            RelicId::RelicChoicesParadox
        ));
        assert!(!after_player_turn_start_relic_follows(
            &catalog,
            RelicId::RelicChoicesParadox,
            RelicId::RelicVexingPuzzlebox
        ));
    }

    /// #3635: Bellows and a charged Bone Tea upgrade the Hand as it stands
    /// when the walk reaches them (`Bellows::AfterPlayerTurnStart` RVA
    /// `0x906dc` `IL_003a`-`IL_004c`; `BoneTea::AfterPlayerTurnStart` RVA
    /// `0x90ffc` `IL_0042`-`IL_006f`), so Vexing Puzzlebox's free card
    /// (`VexingPuzzlebox/<AfterPlayerTurnStart>d__2` RVA `0x333de0`
    /// `IL_00b7`) is upgraded exactly when the Puzzlebox is recorded first.
    #[test]
    fn a_hand_upgrade_relic_upgrades_the_puzzlebox_card_only_when_recorded_behind_it() {
        use RelicId::{
            RelicBellows as BELLOWS, RelicBoneTea as BONE_TEA, RelicVexingPuzzlebox as PUZZLEBOX,
        };
        for upgrader in [BELLOWS, BONE_TEA] {
            for (relics, upgraded) in [
                ([PUZZLEBOX, upgrader], true),
                ([upgrader, PUZZLEBOX], false),
            ] {
                let mut builder = CatalogBuilder::new();
                for identity in crate::boundary::stoke_generation_closure([true, false]) {
                    builder.intern_reachable(identity).unwrap();
                }
                builder.intern_all_card_upgrade_closure().unwrap();
                builder.intern_monster(MonsterKind::Toadpole).unwrap();
                builder.set_relics_ordered(&relics, true).unwrap();
                let catalog = builder.build();
                let (_, mut state) = popper_puzzlebox_fixture(&relics, 20);
                assert!(state.fanouts.set_bone_tea_combats_left(1));
                after_player_turn_start_from_entry(&catalog, &mut state).unwrap();
                let hand = state.piles.get(PileId::Hand).as_slice();
                assert_eq!(hand.len(), 1, "{relics:?}: the free card");
                let identity = catalog.spec(hand[0].atom).unwrap().identity;
                assert_eq!(
                    identity.upgrade,
                    u8::from(upgraded),
                    "{relics:?}: {identity:?}"
                );
                assert_eq!(
                    state.fanouts.bone_tea_combats_left(),
                    u8::from(upgrader != BONE_TEA),
                    "{relics:?}: Bone Tea spends its charge either way"
                );
            }
        }
    }

    // --- #3756: Festive Popper on its recorded side of Vexing Puzzlebox and
    // of Mercury Hourglass.

    /// One Toadpole at `hp`, an Ironclad on turn one with a fully unlocked
    /// profile and a recorded `CombatCardGeneration` stream, holding `relics`
    /// in a vouched inventory.
    fn popper_puzzlebox_fixture(relics: &[RelicId], hp: i32) -> (Catalog, HotState) {
        let mut builder = CatalogBuilder::new();
        for identity in crate::boundary::stoke_generation_closure([true, false]) {
            builder.intern_reachable(identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.set_relics_ordered(relics, true).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.next_card_uid = 41;
        state.rng.set(
            RngStream::Generation,
            RngStreamState {
                words: [
                    1213625501689665630,
                    7582735556604303185,
                    1987035550902429042,
                    15673710837299237479,
                ],
                counter: 1351,
            },
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, hp));
        (catalog, state)
    }

    /// `FestivePopper/<AfterPlayerTurnStart>d__4` (RVA `0x324b94`,
    /// `CreatureCmd::Damage` at `IL_0094`) and
    /// `VexingPuzzlebox/<AfterPlayerTurnStart>d__2` (RVA `0x333de0`,
    /// `AddGeneratedCardToCombat` at `IL_00b7`) run in `Player.Relics` order.
    /// Against a live enemy the two orders reach one state, with the hit and
    /// the generated card each landing once. They part when Popper is lethal:
    /// recorded second it ends a combat whose Hand already holds the card;
    /// recorded first it leaves the Puzzlebox generating into an ended
    /// combat, where the game still draws the shuffle and adds no card
    /// (#3889; native run `k_pop_puzzlebox`: `CombatCardGeneration` at 68,
    /// the count the live run reaches, and the Hand the dealt five).
    #[test]
    fn festive_popper_runs_on_its_recorded_side_of_vexing_puzzlebox() {
        use RelicId::{RelicFestivePopper as POPPER, RelicVexingPuzzlebox as PUZZLEBOX};
        let mut reached = Vec::new();
        for relics in [[POPPER, PUZZLEBOX], [PUZZLEBOX, POPPER]] {
            let (catalog, mut state) = popper_puzzlebox_fixture(&relics, 20);
            let mut events = Vec::new();
            continue_after_gambling_chip(&catalog, &mut state, &mut events).unwrap();
            assert_eq!(state.monsters[0].hp, 11, "{relics:?}: Popper hit once");
            let hand = state.piles.get(PileId::Hand).as_slice();
            assert_eq!(hand.len(), 1, "{relics:?}: one generated card");
            assert_eq!(hand[0].uid, 41, "{relics:?}");
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                1428,
                "{relics:?}: one GetDistinctForCombat shuffle"
            );
            reached.push(state);
        }
        assert_eq!(reached[0], reached[1], "nothing listens to the hit here");

        let (catalog, mut state) = popper_puzzlebox_fixture(&[PUZZLEBOX, POPPER], 9);
        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert!(state.history.over, "Popper recorded second ends the combat");
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);

        let (catalog, mut state) = popper_puzzlebox_fixture(&[POPPER, PUZZLEBOX], 9);
        let before = state.clone();
        continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert!(state.history.over, "Popper recorded first already killed");
        assert_eq!(
            state.piles.get(PileId::Hand).as_slice(),
            before.piles.get(PileId::Hand).as_slice(),
            "no card was added"
        );
        assert_eq!(state.next_card_uid, before.next_card_uid);
        assert_eq!(
            state.rng.get(RngStream::Generation).counter,
            1428,
            "the shuffle still drew, as on the live combat"
        );
    }

    /// #3889: a lethal Popper ahead of Gambling Chip. `FromHandForDiscard`
    /// returns no card on an ended combat
    /// (`CardSelectCmd/<FromHand>d__28` RVA `0x3e7568`, `IsOverOrEnding` at
    /// `IL_0036`, ahead of the choice signal), so no choice opens (native run
    /// `k_pop_chip`: the selector's pending sources all still queued, Hand
    /// and Discard untouched) and the walk goes on to the Puzzlebox, which
    /// draws its shuffle and adds nothing.
    #[test]
    fn a_lethal_popper_ahead_of_gambling_chip_opens_no_choice() {
        use RelicId::{
            RelicFestivePopper as POPPER, RelicGamblingChip as CHIP,
            RelicVexingPuzzlebox as PUZZLEBOX,
        };
        let (catalog, mut state) = popper_puzzlebox_fixture(&[POPPER, CHIP, PUZZLEBOX], 9);
        let identity = crate::boundary::stoke_generation_closure([true, false])
            .into_iter()
            .next()
            .unwrap();
        let atom = catalog.atom(&identity).unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        let hand = state.piles.get(PileId::Hand).clone();
        continue_after_choices_paradox(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert!(state.history.over);
        assert!(state.pending.is_none(), "no choice on the ended combat");
        assert!(state.fanouts.batch_nine_relic_pending().is_none());
        assert_eq!(state.piles.get(PileId::Hand), &hand);
        assert!(state.piles.get(PileId::Discard).is_empty());
        assert_eq!(state.rng.get(RngStream::Generation).counter, 1428);

        let (catalog, mut state) = popper_puzzlebox_fixture(&[POPPER, CHIP, PUZZLEBOX], 20);
        continue_after_choices_paradox(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 11);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    /// #3889: Emotion Chip behind a Mr Struggles kill on turn two.
    /// `OrbCmd/<Passive>d__7` (RVA `0x3ede90`) returns on `IsOverOrEnding`
    /// (`IL_0022`) for each orb, so the chip does nothing. Native runs
    /// (Defect, Royal Poison for the turn-one HP loss, a Coolheaded Frost
    /// beside the starter Lightning): `k_struggles_emotion` ends with Block 0
    /// and `CombatTargets` at 1, the count the end of turn one left;
    /// `c_struggles_emotion`, the same order against live enemies, has Block
    /// 2 and `CombatTargets` at 2.
    #[test]
    fn emotion_chip_behind_a_lethal_mr_struggles_triggers_no_orb() {
        use RelicId::{RelicEmotionChip as CHIP, RelicMrStruggles as STRUGGLES};
        for (hp, block, over) in [(2, 0, true), (60, 2, false)] {
            let mut builder = CatalogBuilder::new();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder
                .set_relics_ordered(&[STRUGGLES, CHIP], true)
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 71;
            state.max_hp = 75;
            state.turn = 2;
            state.set_batch_six_deep_relic_ownership(false, true, false, false);
            state.set_emotion_damage_previous_turn(true);
            state.orbs.set_base_slots(3);
            state.orbs.set_slots(3);
            state
                .orbs
                .push(crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Frost, None).unwrap());
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, hp));
            let targets = state.rng.get(RngStream::Targets);
            after_player_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            assert_eq!(state.history.over, over, "enemy at {hp}");
            assert_eq!(state.block, block, "enemy at {hp}");
            assert_eq!(state.rng.get(RngStream::Targets), targets);
        }
    }

    /// The #3640 fixture: one 1,000-HP Toadpole, `relics` recorded in order
    /// (vouched or not), and `cards` interned in order. Three energy, the
    /// ordinary-actions phase.
    fn latch_fixture(
        cards: &[CardId],
        relics: &[RelicId],
        vouched: bool,
    ) -> (HotState, Catalog, Vec<crate::catalog::CardAtom>) {
        let mut builder = CatalogBuilder::new();
        let atoms = cards
            .iter()
            .map(|id| {
                builder
                    .intern_reachable(CardIdentity {
                        id: *id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap()
            })
            .collect();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.set_relics_ordered(relics, vouched).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 40;
        state.exact_piles = true;
        for stream in [RngStream::Rng, RngStream::Sel, RngStream::Targets] {
            state.rng.set(
                stream,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        state.monsters = std::sync::Arc::new(vec![monster]);
        (state, catalog, atoms)
    }

    fn with_hellraiser(state: &mut HotState) {
        state
            .powers
            .set(PowerId::Hellraiser, crate::powers::SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(state);
    }

    fn latch_push(state: &mut HotState, pile: PileId, cards: &[(u32, crate::catalog::CardAtom)]) {
        state
            .piles
            .get_mut(pile)
            .make_mut()
            .extend(cards.iter().map(|&(uid, atom)| HotCard {
                uid,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
    }

    /// Canonical round trip plus admission: the state is a document an
    /// admitted root can hold.
    fn latch_cold(state: &HotState, catalog: &Catalog) -> (HotState, Catalog) {
        use crate::boundary::HotBoundary;
        let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let loaded = HotBoundary::from_canonical(&wire, &loaded_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&loaded, &loaded_catalog).unwrap(),
            wire
        );
        assert_eq!(
            crate::engine::admission::admit(&wire, &loaded, &loaded_catalog),
            Ok(())
        );
        (loaded, loaded_catalog)
    }

    fn latch_play(state: &HotState, catalog: &Catalog, uid: u32) -> HotState {
        crate::engine::apply_action(
            state,
            catalog,
            &crate::engine::Action::Play {
                uid,
                target: Some(0),
                selection: crate::engine::SelectionRef::NONE,
            },
        )
        .unwrap()
        .state
    }

    /// The Ethereal Music Box clones in the Hand, by card.
    fn latch_clones(state: &HotState, catalog: &Catalog) -> Vec<CardId> {
        state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .filter(|card| state.card_states.get(card.uid).local_ethereal())
            .map(|card| catalog.spec(card.atom).unwrap().identity.id)
            .collect()
    }

    /// #3756: Game Piece's Power-play draw (`GamePiece/<AfterCardPlayed>d__6`
    /// `0x325930`, `CardPileCmd::Draw` at IL_0089) on either recorded side of
    /// Velvet Choker's count (`0x9d9a0`) and of Unsettling Lamp's latch
    /// (`0x9d606`). With no Hellraiser to nest a play in the draw, nothing
    /// reads either one between the two bodies, so each root is admitted in
    /// both orders, vouched or not, and all four reach one state: the Power
    /// played, one card drawn, one play counted.
    #[test]
    fn game_piece_draws_on_either_recorded_side_of_velvet_choker_and_unsettling_lamp() {
        let piece = RelicId::RelicGamePiece;
        for peer in [RelicId::RelicVelvetChoker, RelicId::RelicUnsettlingLamp] {
            let mut reached = Vec::new();
            for (order, vouched) in [
                ([piece, peer], true),
                ([peer, piece], true),
                ([piece, peer], false),
                ([peer, piece], false),
            ] {
                let (mut state, catalog, atoms) = latch_fixture(
                    &[CardId::DemonForm, CardId::DefendIronclad],
                    &order,
                    vouched,
                );
                if peer == RelicId::RelicUnsettlingLamp {
                    state.fanouts.set_unsettling_lamp_available(true);
                }
                latch_push(&mut state, PileId::Hand, &[(1, atoms[0])]);
                latch_push(&mut state, PileId::Draw, &[(2, atoms[1]), (3, atoms[1])]);
                // `latch_cold` asserts the root is admitted.
                let (state, catalog) = latch_cold(&state, &catalog);
                let play = crate::engine::legal_actions(&state, &catalog)
                    .into_iter()
                    .find(|action| matches!(action, crate::engine::Action::Play { uid: 1, .. }))
                    .unwrap_or_else(|| panic!("{order:?}: the Power is playable"));
                let done = crate::engine::apply_action(&state, &catalog, &play)
                    .unwrap_or_else(|refusal| panic!("{order:?} vouched={vouched}: {refusal:?}"))
                    .state;
                assert!(done.frames.is_empty(), "{order:?}");
                assert_eq!(
                    done.piles
                        .get(PileId::Hand)
                        .as_slice()
                        .iter()
                        .map(|card| card.uid)
                        .collect::<Vec<_>>(),
                    [2],
                    "{order:?} vouched={vouched}: Game Piece drew the top card"
                );
                assert_eq!(done.piles.get(PileId::Draw).len(), 1, "{order:?}");
                assert_eq!(done.history.owner_card_plays_finished_this_turn, 1);
                assert!(done.powers.value(PowerId::DemonForm) > 0, "{order:?}");
                reached.push(done);
            }
            for other in &reached[1..] {
                assert_eq!(&reached[0], other, "{peer:?}: one state in every order");
            }
        }
    }

    /// #3640: `MusicBox::BeforeCardPlayed` (`0x972ac`) latches the turn's
    /// first owner Attack to START, and `<AfterCardPlayed>d__13` (`0x32ae04`
    /// IL_0029-IL_002e) clones only that card. A Pommel Strike draws a
    /// Strike, Hellraiser AutoPlays it inside the Pommel Strike's own play,
    /// and the nested Strike FINISHES first. Its Before found the latch
    /// taken (IL_000d-IL_0019), its After is not the latched card, and the
    /// Pommel Strike is the card cloned. Before the latch was modeled this
    /// engine cloned the Strike and skipped the Pommel Strike.
    #[test]
    fn music_box_clones_the_outer_attack_not_a_nested_attack_that_finishes_first() {
        for vouched in [true, false] {
            let (mut state, catalog, atoms) = latch_fixture(
                &[
                    CardId::PommelStrike,
                    CardId::StrikeIronclad,
                    CardId::DefendIronclad,
                ],
                &[RelicId::RelicMusicBox],
                vouched,
            );
            let (pommel, strike, defend) = (atoms[0], atoms[1], atoms[2]);
            with_hellraiser(&mut state);
            latch_push(&mut state, PileId::Hand, &[(1, pommel)]);
            latch_push(&mut state, PileId::Draw, &[(2, strike), (3, defend)]);
            let (state, catalog) = latch_cold(&state, &catalog);

            let done = latch_play(&state, &catalog, 1);

            assert!(done.frames.is_empty());
            // The nested Strike finished first: it is under the Pommel
            // Strike in the Discard pile.
            let discard = done.piles.get(PileId::Discard).as_slice();
            assert_eq!(
                discard.iter().map(|card| card.uid).collect::<Vec<_>>(),
                [2, 1]
            );
            assert_eq!(done.history.card_plays_finished_combat, 2);
            assert_eq!(
                latch_clones(&done, &catalog),
                [CardId::PommelStrike],
                "vouched={vouched}"
            );
            assert_eq!(done.piles.get(PileId::Hand).len(), 1);
            assert!(done.fanouts.music_box_used_this_turn());
            assert_eq!(done.fanouts.music_box_card_uid(), None);
            latch_cold(&done, &catalog);
        }
    }

    /// #3640 beside #2909's inventory walk. Iron Club's fourth card draws a
    /// Strike inside the outer Bash's AfterCardPlayed walk and Hellraiser
    /// AutoPlays it there. Recorded before Music Box, the nested Strike's
    /// whole play runs before Music Box's body for the Bash; the latch still
    /// names the Bash, so that is the one clone in either order. The nested
    /// play's own walk advances Iron Club.
    #[test]
    fn music_box_clones_the_outer_attack_on_either_side_of_iron_club() {
        for order in [
            [RelicId::RelicMusicBox, RelicId::RelicIronClub],
            [RelicId::RelicIronClub, RelicId::RelicMusicBox],
        ] {
            let (mut state, catalog, atoms) = latch_fixture(
                &[CardId::Bash, CardId::StrikeIronclad, CardId::DefendIronclad],
                &order,
                true,
            );
            let (bash, strike, defend) = (atoms[0], atoms[1], atoms[2]);
            with_hellraiser(&mut state);
            assert!(state.fanouts.set_iron_club_cards(3));
            latch_push(&mut state, PileId::Hand, &[(1, bash)]);
            latch_push(&mut state, PileId::Draw, &[(2, strike), (3, defend)]);
            let (state, catalog) = latch_cold(&state, &catalog);

            let done = latch_play(&state, &catalog, 1);

            assert!(done.frames.is_empty(), "{order:?}");
            assert_eq!(done.history.card_plays_finished_combat, 2, "{order:?}");
            assert_eq!(latch_clones(&done, &catalog), [CardId::Bash], "{order:?}");
            assert_eq!(done.piles.get(PileId::Hand).len(), 1, "{order:?}");
            assert_eq!(done.fanouts.iron_club_cards(), 1, "{order:?}");
            assert!(done.fanouts.music_box_used_this_turn(), "{order:?}");
            assert_eq!(done.fanouts.music_box_card_uid(), None, "{order:?}");
            latch_cold(&done, &catalog);
        }
    }

    /// #3640: the plain case is unchanged, and `WasUsedThisTurn` (set at
    /// `0x32ae04` IL_00bf, read by Before at `0x972ac` IL_0034-IL_0040)
    /// keeps a second Attack of the turn from latching.
    #[test]
    fn music_box_clones_the_first_attack_of_the_turn_and_no_second() {
        let (mut state, catalog, atoms) = latch_fixture(
            &[CardId::StrikeIronclad, CardId::Bash],
            &[RelicId::RelicMusicBox],
            true,
        );
        let (strike, bash) = (atoms[0], atoms[1]);
        latch_push(&mut state, PileId::Hand, &[(1, strike), (2, bash)]);
        let (state, catalog) = latch_cold(&state, &catalog);

        let first = latch_play(&state, &catalog, 1);
        assert_eq!(latch_clones(&first, &catalog), [CardId::StrikeIronclad]);
        assert!(first.fanouts.music_box_used_this_turn());
        assert_eq!(first.fanouts.music_box_card_uid(), None);
        let (first, catalog) = latch_cold(&first, &catalog);

        let second = latch_play(&first, &catalog, 2);
        assert_eq!(latch_clones(&second, &catalog), [CardId::StrikeIronclad]);
        assert_eq!(second.piles.get(PileId::Hand).len(), 1);
        assert_eq!(second.fanouts.music_box_card_uid(), None);
    }

    /// #3640: Before has no `IsAutoPlay` test (`0x972ac` reads only
    /// `CardPlay.Card`), so an AutoPlayed Attack that is the turn's first
    /// latches like a manual one. Havoc AutoPlays the top Strike; it is
    /// cloned and the manual Bash after it is not.
    #[test]
    fn music_box_latches_an_autoplayed_first_attack() {
        let (mut state, catalog, atoms) = latch_fixture(
            &[CardId::Havoc, CardId::StrikeIronclad, CardId::Bash],
            &[RelicId::RelicMusicBox],
            true,
        );
        let (havoc, strike, bash) = (atoms[0], atoms[1], atoms[2]);
        latch_push(&mut state, PileId::Hand, &[(1, havoc), (3, bash)]);
        latch_push(&mut state, PileId::Draw, &[(2, strike)]);
        let (state, catalog) = latch_cold(&state, &catalog);

        let done = crate::engine::apply_action(
            &state,
            &catalog,
            &crate::engine::Action::Play {
                uid: 1,
                target: None,
                selection: crate::engine::SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(done.frames.is_empty());
        assert_eq!(latch_clones(&done, &catalog), [CardId::StrikeIronclad]);
        assert!(done.fanouts.music_box_used_this_turn());
        let (done, catalog) = latch_cold(&done, &catalog);

        let after_bash = latch_play(&done, &catalog, 3);
        assert_eq!(
            latch_clones(&after_bash, &catalog),
            [CardId::StrikeIronclad]
        );
    }

    /// #3640: the clone sets `WasUsedThisTurn` and clears `CardBeingPlayed`
    /// together (`0x32ae04` IL_00bf, IL_00c6) and Before never latches once
    /// used (`0x972ac` IL_0034-IL_0040), so a document holding both is no
    /// native state and does not publish. A latch alone does.
    #[test]
    fn a_used_music_box_with_a_latch_is_not_a_document() {
        use crate::boundary::HotBoundary;
        let (mut state, catalog, atoms) =
            latch_fixture(&[CardId::StrikeIronclad], &[RelicId::RelicMusicBox], true);
        latch_push(&mut state, PileId::Hand, &[(1, atoms[0])]);
        state.fanouts.set_music_box_card_uid(Some(1));
        assert!(HotBoundary::try_to_canonical(&state, &catalog).is_ok());
        state.fanouts.set_music_box_used_this_turn(true);
        assert!(HotBoundary::try_to_canonical(&state, &catalog).is_err());
        state.fanouts.set_music_box_card_uid(None);
        assert!(HotBoundary::try_to_canonical(&state, &catalog).is_ok());
    }

    /// #3640: the latch crosses a park. The Pommel Strike draws a Seeker
    /// Strike, Hellraiser AutoPlays it, and its selection suspends inside
    /// the Pommel Strike's play with `CardBeingPlayed` naming the Pommel
    /// Strike. The parked document carries that in `music_box_card_uid`,
    /// reloads cold, and every answer ends with the Pommel Strike cloned
    /// and no clone of the Seeker Strike.
    #[test]
    fn music_box_latch_survives_a_park_inside_the_outer_attack() {
        let (mut state, catalog, atoms) = latch_fixture(
            &[
                CardId::PommelStrike,
                CardId::SeekerStrike,
                CardId::DefendIronclad,
            ],
            &[RelicId::RelicMusicBox],
            true,
        );
        let (pommel, seeker, defend) = (atoms[0], atoms[1], atoms[2]);
        with_hellraiser(&mut state);
        latch_push(&mut state, PileId::Hand, &[(1, pommel)]);
        latch_push(
            &mut state,
            PileId::Draw,
            &[
                (2, seeker),
                (3, defend),
                (4, defend),
                (5, defend),
                (6, defend),
            ],
        );
        let (state, catalog) = latch_cold(&state, &catalog);

        let parked = latch_play(&state, &catalog, 1);
        assert!(parked.pending.is_some());
        assert_eq!(parked.fanouts.music_box_card_uid(), Some(1));
        assert!(!parked.fanouts.music_box_used_this_turn());

        let mut work = vec![parked];
        let mut terminals = 0;
        while let Some(state) = work.pop() {
            let (loaded, loaded_catalog) = latch_cold(&state, &catalog);
            assert_eq!(loaded, state);
            let actions = crate::engine::legal_actions(&loaded, &loaded_catalog);
            assert!(!actions.is_empty());
            for action in actions {
                let next = crate::engine::apply_action(&loaded, &loaded_catalog, &action)
                    .unwrap()
                    .state;
                if next.pending.is_some() {
                    work.push(next);
                    continue;
                }
                terminals += 1;
                assert!(next.frames.is_empty());
                assert_eq!(latch_clones(&next, &catalog), [CardId::PommelStrike]);
                assert!(next.fanouts.music_box_used_this_turn());
                assert_eq!(next.fanouts.music_box_card_uid(), None);
                latch_cold(&next, &catalog);
            }
        }
        assert!(terminals > 1);
    }

    /// The refusal arm of
    /// [`after_player_turn_start_template_pass`] is
    /// unreachable at v0.111.0, and this pins why: Mercury Hourglass is the
    /// only template relic with an `AfterPlayerTurnStart` hook, and no power
    /// subscribes to it. A second subscriber reddens this test before it can
    /// be skipped silently.
    #[test]
    fn mercury_hourglass_is_the_only_after_player_turn_start_template_subscriber() {
        let catalog = ordered_catalog(&crate::content_tables::EFFECTIVE_TEMPLATE_RELICS, true);
        let subjects = catalog
            .hooks()
            .subscribers(HookEvent::AfterPlayerTurnStart)
            .iter()
            .map(|subscriber| subscriber.subject)
            .collect::<Vec<_>>();
        assert_eq!(
            subjects,
            [HookSubject::Relic(RelicId::RelicMercuryHourglass)]
        );
        assert!(
            crate::hooks::POWER_SUBSCRIPTIONS
                .iter()
                .all(|(_, event)| *event != HookEvent::AfterPlayerTurnStart)
        );
        assert_eq!(after_player_turn_start_template_pass(&catalog), Ok(()));
    }

    /// #3635: the `AfterSideTurnStart` walk order. A vouched inventory runs
    /// its listeners as recorded, whatever the order, compiled templates and
    /// hand-written bodies interleaved. An unvouched one runs the early
    /// group, the templates in the order it lists them, then the late group.
    #[test]
    fn the_side_start_walk_runs_a_vouched_inventory_as_recorded_and_an_unvouched_one_as_fixed() {
        use RelicId::{
            RelicAnchor as ANCHOR, RelicBrimstone as BRIMSTONE, RelicCrossbow as CROSSBOW,
            RelicDiamondDiadem as DIADEM, RelicInfusedCore as INFUSED, RelicLantern as LANTERN,
            RelicSai as SAI, RelicStoneCalendar as CALENDAR, RelicSymbioticVirus as VIRUS,
        };
        let order = |relics: &[RelicId], vouched: bool| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(relics, vouched).unwrap();
            let catalog = builder.build();
            after_side_turn_start_order(&catalog).collect::<Vec<_>>()
        };
        let inventory = [
            VIRUS, SAI, CROSSBOW, ANCHOR, DIADEM, LANTERN, BRIMSTONE, CALENDAR, INFUSED,
        ];
        let mut orders = vec![
            inventory.to_vec(),
            inventory.iter().rev().copied().collect(),
        ];
        for shift in 1..inventory.len() {
            let mut rotated = inventory.to_vec();
            rotated.rotate_left(shift);
            orders.push(rotated);
        }
        for recorded in &orders {
            // Anchor does not listen on this hook.
            let listeners: Vec<_> = recorded
                .iter()
                .copied()
                .filter(|relic| *relic != ANCHOR)
                .collect();
            assert_eq!(order(recorded, true), listeners, "vouched {recorded:?}");
            let templates: Vec<_> = recorded
                .iter()
                .copied()
                .filter(|relic| [SAI, DIADEM].contains(relic))
                .collect();
            let fixed: Vec<_> = [LANTERN, CROSSBOW]
                .into_iter()
                .chain(templates)
                .chain([INFUSED, VIRUS, BRIMSTONE])
                .collect();
            assert_eq!(order(recorded, false), fixed, "unvouched {recorded:?}");
        }
    }

    /// The listener table is the three groups plus the five listeners that
    /// write nothing here, twenty-nine distinct relics, and the template group
    /// is exactly the hook's compiled subscribers.
    #[test]
    fn the_side_start_listener_table_is_its_groups_and_the_compiled_subscribers() {
        let groups: Vec<_> = AFTER_SIDE_TURN_START_EARLY
            .into_iter()
            .chain(AFTER_SIDE_TURN_START_TEMPLATES)
            .chain(AFTER_SIDE_TURN_START_LATE)
            .collect();
        assert_eq!(
            AFTER_SIDE_TURN_START_RELIC_DISPATCH[..groups.len()],
            groups[..]
        );
        let distinct: std::collections::BTreeSet<_> = AFTER_SIDE_TURN_START_RELIC_DISPATCH
            .iter()
            .map(|relic| *relic as u16)
            .collect();
        assert_eq!(distinct.len(), 29);

        let catalog = catalog(&AFTER_SIDE_TURN_START_RELIC_DISPATCH);
        let subscribers: Vec<_> = catalog
            .hooks()
            .subscribers(HookEvent::AfterSideTurnStart)
            .iter()
            .map(|subscriber| subscriber.subject)
            .collect();
        assert_eq!(
            subscribers,
            AFTER_SIDE_TURN_START_TEMPLATES.map(HookSubject::Relic)
        );
        assert_eq!(after_side_turn_start_template_pass(&catalog), Ok(()));
        // The five silent listeners never act.
        let state = HotState::at_defaults();
        for relic in &AFTER_SIDE_TURN_START_RELIC_DISPATCH[groups.len()..] {
            assert!(!after_side_turn_start_relic_acts(&catalog, &state, *relic));
        }
    }

    /// #3635, native: the turn-one orb relics in each recorded order.
    ///
    /// The headless harness on the archived v0.111.0 DLL (Cultists at 42 and
    /// 53 HP, seed `TESTSEED`, relics granted in the listed order) gives, for
    /// an Ironclad (no base slots):
    ///
    /// * Infused Core, Symbiotic Virus: hits of 9, 9, 9 (the Dark channel
    ///   evokes the last Lightning); the reverse: 6, 9, 9 (the first Lightning
    ///   evokes the Dark orb);
    /// * Runic Capacitor, Infused Core: no hit; the reverse: 9, 9;
    /// * Symbiotic Virus, Runic Capacitor, Infused Core: no hit (four slots);
    ///   Runic Capacitor first: one hit of 6 (three slots, the Dark evoked).
    ///
    /// and for a Defect entering with two Lightning queued (three base
    /// slots): Infused Core, Symbiotic Virus 9, 9, 9; the reverse 9, 9, 6;
    /// Runic Capacitor, Infused Core no hit; the reverse 9, 9.
    #[test]
    fn turn_one_orb_relics_overflow_in_the_recorded_order_the_game_runs() {
        use crate::hot::OrbKind::{Dark as D, Lightning as L};
        use RelicId::{
            RelicInfusedCore as INFUSED, RelicRunicCapacitor as CAPACITOR,
            RelicSymbioticVirus as VIRUS,
        };
        let run = |relics: &[RelicId], base: u8, queued: usize| {
            let mut builder = CatalogBuilder::new();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.set_relics_ordered(relics, true).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 1;
            for hp in [42, 53] {
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, hp));
            }
            state.orbs.set_base_slots(base);
            state.orbs.set_slots(base);
            for _ in 0..queued {
                state
                    .orbs
                    .push(crate::hot::HotOrb::from_parts(L, None).unwrap());
            }
            let rng = Xoshiro256StarStar::from_seed(5);
            state.rng.set(
                RngStream::Targets,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            let kinds: Vec<_> = state.orbs.as_slice().iter().map(|orb| orb.kind()).collect();
            let lost = 95 - state.monsters.iter().map(|monster| monster.hp).sum::<i32>();
            (kinds, state.orbs.slots(), lost)
        };
        // Ironclad.
        assert_eq!(run(&[INFUSED, VIRUS], 0, 0), (vec![D], 1, 27));
        assert_eq!(run(&[VIRUS, INFUSED], 0, 0), (vec![L], 1, 24));
        assert_eq!(run(&[CAPACITOR, INFUSED], 0, 0), (vec![L, L, L], 3, 0));
        assert_eq!(run(&[INFUSED, CAPACITOR], 0, 0), (vec![L], 4, 18));
        assert_eq!(
            run(&[VIRUS, CAPACITOR, INFUSED], 0, 0),
            (vec![D, L, L, L], 4, 0)
        );
        assert_eq!(
            run(&[CAPACITOR, VIRUS, INFUSED], 0, 0),
            (vec![L, L, L], 3, 6)
        );
        // Defect, two Lightning queued by the earlier hook.
        assert_eq!(run(&[INFUSED, VIRUS], 3, 2), (vec![L, L, D], 3, 27));
        assert_eq!(run(&[VIRUS, INFUSED], 3, 2), (vec![L, L, L], 3, 24));
        assert_eq!(run(&[CAPACITOR, INFUSED], 3, 2), (vec![L; 5], 6, 0));
        assert_eq!(run(&[INFUSED, CAPACITOR], 3, 2), (vec![L, L, L], 6, 18));
    }

    /// #3635, native: the turn-one Hand generators in each recorded order,
    /// from the game's own `CombatCardGeneration` stream.
    ///
    /// The headless harness on the archived v0.111.0 DLL (Ironclad, seed
    /// `TESTSEED`, relics granted in the listed order) appends, behind the
    /// dealt five:
    ///
    /// * Crossbow, Orange Dough: Thrash, Volley, Fasten; the reverse: Secret
    ///   Weapon, Panic Button, Perfected Strike (stream counter 63 both ways);
    /// * Fencing Manual, Crossbow: Sovereign Blade, Thrash; the reverse:
    ///   Thrash, Sovereign Blade (counter 28);
    /// * Fencing Manual, Orange Dough: Sovereign Blade, Secret Weapon, Panic
    ///   Button; the reverse: the Blade last (counter 35);
    /// * Big Hat with or without Crossbow, either order: nothing of its own
    ///   and no draw (counter 0 alone, 28 beside Crossbow).
    ///
    /// The stream here is the one the game seeds for that run
    /// (`rng::run_stream_at_zero`), so the cards are the game's.
    #[test]
    fn turn_one_hand_generators_fill_the_hand_in_the_recorded_order_the_game_runs() {
        use crate::catalog::RewardPool;
        use CardId::{
            Fasten, PanicButton, PerfectedStrike, SecretWeapon, SovereignBlade, Thrash, Volley,
        };
        use RelicId::{
            RelicBigHat as HAT, RelicCrossbow as CROSSBOW, RelicFencingManual as MANUAL,
            RelicOrangeDough as DOUGH,
        };
        let run = |relics: &[RelicId], vouched: bool| {
            let mut builder = CatalogBuilder::new();
            let pools =
                crate::steps::neutral::crossbow_owner_attack_pool(RewardPool::Ironclad, Some(&[]))
                    .into_iter()
                    .chain(crate::steps::neutral::colorless_generation_pool(Some(&[])))
                    .chain([SovereignBlade]);
            for id in pools {
                builder
                    .intern_reachable(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            // The harness builds its player on an empty `UnlockState`: no
            // epoch revealed.
            builder.set_splash_unlock_epochs(Vec::new());
            builder.set_relics_ordered(relics, vouched).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 1;
            state.next_card_uid = 10;
            state.reward_card_pool = Some(RewardPool::Ironclad);
            state.entropy_card_pool = Some(RewardPool::Ironclad);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 42));
            let rng = crate::rng::run_stream_at_zero("TESTSEED", "combat_card_generation");
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            let hand = state.piles.get(PileId::Hand).as_slice();
            assert!(
                hand.windows(2).all(|pair| pair[0].uid < pair[1].uid),
                "{relics:?}: uids in arrival order"
            );
            (
                hand.iter()
                    .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                    .collect::<Vec<_>>(),
                state.rng.get(RngStream::Generation).counter,
            )
        };
        assert_eq!(run(&[CROSSBOW], true), (vec![Thrash], 28));
        assert_eq!(
            run(&[CROSSBOW, DOUGH], true),
            (vec![Thrash, Volley, Fasten], 63)
        );
        assert_eq!(
            run(&[DOUGH, CROSSBOW], true),
            (vec![SecretWeapon, PanicButton, PerfectedStrike], 63)
        );
        assert_eq!(
            run(&[MANUAL, CROSSBOW], true),
            (vec![SovereignBlade, Thrash], 28)
        );
        assert_eq!(
            run(&[CROSSBOW, MANUAL], true),
            (vec![Thrash, SovereignBlade], 28)
        );
        assert_eq!(
            run(&[MANUAL, DOUGH], true),
            (vec![SovereignBlade, SecretWeapon, PanicButton], 35)
        );
        assert_eq!(
            run(&[DOUGH, MANUAL], true),
            (vec![SecretWeapon, PanicButton, SovereignBlade], 35)
        );
        assert_eq!(run(&[HAT], true), (vec![], 0));
        for order in [[HAT, CROSSBOW], [CROSSBOW, HAT]] {
            for vouched in [true, false] {
                assert_eq!(run(&order, vouched), (vec![Thrash], 28), "{order:?}");
            }
        }
        // An unvouched inventory keeps the fixed sequence whatever it lists:
        // Crossbow, Orange Dough, then the Forge.
        for order in [[MANUAL, DOUGH, CROSSBOW], [DOUGH, CROSSBOW, MANUAL]] {
            assert_eq!(
                run(&order, false),
                (vec![Thrash, Volley, Fasten, SovereignBlade], 63),
                "{order:?}"
            );
        }
    }

    /// #3635: Phylactery Unbound behind the Infused Core that ended the
    /// combat, through the walk. This is the vouched pair the review of
    /// #3896 found refusing (`EndingSummonNotModeled`) when the state held
    /// no Osty.
    ///
    /// Native (headless; Necrobinder, both Cultists at 5 HP, no choice
    /// signalled): Infused Core recorded first leaves the 6/6 Osty at 6/8,
    /// MaxHp only; Phylactery Unbound recorded first summons live, 8/8. The
    /// absent and dead rows are the same `OstyCmd.Summon` the game ran behind
    /// Rolling Boulder (1/2 and a corpse, cited on
    /// `SoloPetState::summon_ending`), reached here through the relic's body.
    #[test]
    fn phylactery_unbound_behind_a_killing_infused_core_summons_on_the_ended_combat() {
        use RelicId::{RelicInfusedCore as INFUSED, RelicPhylacteryUnbound as UNBOUND};
        let run = |relics: &[RelicId], osty: Option<(i32, i32)>, corpse: bool| {
            let mut builder = CatalogBuilder::new();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.set_relics_ordered(relics, true).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 1;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 9));
            state.fanouts.set_osty(osty).unwrap();
            if corpse {
                let hp = osty.unwrap().0;
                state
                    .fanouts
                    .mutate_pet(|pet| pet.lose_hp(hp).map(|_| ()))
                    .unwrap();
                assert!(state.fanouts.pet().corpse());
            }
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new())
                .unwrap_or_else(|refusal| panic!("{relics:?} with {osty:?}: {refusal}"));
            assert!(state.history.over, "{relics:?}: the overflow evoke killed");
            let pet = state.fanouts.pet();
            (
                pet.osty().map(|osty| (osty.hp(), osty.max_hp())),
                pet.corpse(),
            )
        };
        assert_eq!(
            run(&[INFUSED, UNBOUND], Some((6, 6)), false),
            (Some((6, 8)), false)
        );
        assert_eq!(
            run(&[UNBOUND, INFUSED], Some((6, 6)), false),
            (Some((8, 8)), false)
        );
        assert_eq!(run(&[INFUSED, UNBOUND], None, false), (Some((1, 2)), false));
        assert_eq!(run(&[UNBOUND, INFUSED], None, false), (Some((2, 2)), false));
        assert_eq!(run(&[INFUSED, UNBOUND], Some((2, 2)), true), (None, true));
        assert_eq!(
            run(&[UNBOUND, INFUSED], Some((2, 2)), true),
            (Some((2, 2)), false)
        );
    }

    /// #3635, native: the templates fire at their recorded places. The
    /// harness (Sai, Diamond Diadem, Lantern, Ironclad) records the block of
    /// 7, then 20, then Blur, for 27 Block and four Energy. Beside a lethal
    /// first hit they are the witness of the ended-combat rule: a body behind
    /// the relic that ended the combat is a no-op where its command returns
    /// on an ending combat, keeps its own counter, and still draws the
    /// Generation stream (adding no card) where the game does. Native, with
    /// every enemy at 9 HP and Infused Core first: no Strength, no Blur, no
    /// Block and no Sovereign Blade behind the kill.
    #[test]
    fn a_side_start_relic_behind_one_that_ended_combat_runs_as_the_game_runs_it() {
        use crate::catalog::RewardPool;
        use RelicId::{
            RelicBread as BREAD, RelicBrimstone as BRIMSTONE, RelicCrossbow as CROSSBOW,
            RelicDiamondDiadem as DIADEM, RelicHappyFlower as FLOWER, RelicInfusedCore as INFUSED,
            RelicLantern as LANTERN, RelicOrangeDough as DOUGH, RelicSai as SAI,
            RelicSealOfGold as SEAL,
        };
        let run = |relics: &[RelicId], monster_hp: i32| {
            let mut builder = CatalogBuilder::new();
            let pools =
                crate::steps::neutral::crossbow_owner_attack_pool(RewardPool::Ironclad, None)
                    .into_iter()
                    .chain(crate::steps::neutral::colorless_generation_pool(None));
            for id in pools {
                builder
                    .intern_reachable(CardIdentity {
                        id,
                        upgrade: 0,
                        enchantment: None,
                    })
                    .unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.set_splash_unlock_epochs(
                crate::boundary::FULLY_UNLOCKED_CARD_POOL_EPOCHS.to_vec(),
            );
            builder.set_relics_ordered(relics, true).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 80;
            state.max_hp = 80;
            state.turn = 1;
            state.energy = 3;
            state.gold = 10;
            state.next_card_uid = 10;
            state.reward_card_pool = Some(RewardPool::Ironclad);
            state.fully_unlocked_card_pool_epochs = true;
            assert!(state.set_flower(2));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, monster_hp));
            let rng = Xoshiro256StarStar::from_seed(29);
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
            after_side_turn_start(&catalog, &mut state, true, &mut Vec::new()).map(|()| state)
        };
        // A live fight: the harness's Sai, Diadem, Lantern.
        let live = run(&[SAI, DIADEM, LANTERN], 100).unwrap();
        assert_eq!((live.block, live.energy), (27, 4));
        assert_eq!(live.powers.value(PowerId::Blur), 1);

        // Infused Core at no base slots: the second channel evokes the first
        // Lightning for 9, which kills a 9 HP monster and ends the combat.
        let quiet = [BREAD, BRIMSTONE, LANTERN, SAI, DIADEM, FLOWER, SEAL];
        let mut behind = vec![INFUSED];
        behind.extend(quiet);
        let state = run(&behind, 9).unwrap();
        assert!(state.history.over);
        assert_eq!(state.energy, 3, "no Energy moves on an ended combat");
        assert_eq!(state.block, 0);
        assert_eq!(state.powers.value(PowerId::Strength), 0);
        assert_eq!(state.powers.value(PowerId::Blur), 0);
        assert_eq!(state.flower(), 0, "Happy Flower still counts its turn");
        assert_eq!(state.gold, 7, "Seal of Gold still pays");
        // The same relics ahead of it all act.
        let mut ahead = quiet.to_vec();
        ahead.push(INFUSED);
        let state = run(&ahead, 9).unwrap();
        assert!(state.history.over);
        assert_eq!(state.energy, 3 - 2 + 1 + 1 + 1);
        assert_eq!(state.block, 27);
        assert_eq!(state.powers.value(PowerId::Strength), 2);
        assert_eq!(state.gold, 7);

        // The two generation relics still draw on an ended combat and add
        // no card. Native (every enemy at 9 HP, Infused Core first): the
        // stream ends where a live fight leaves it, and the Hand is the
        // dealt five.
        for relic in [CROSSBOW, DOUGH] {
            let ended = run(&[INFUSED, relic], 9).unwrap();
            assert!(ended.history.over);
            assert!(ended.piles.get(PileId::Hand).is_empty(), "{relic:?}");
            let first = run(&[relic, INFUSED], 9).unwrap();
            assert!(first.history.over);
            assert!(!first.piles.get(PileId::Hand).is_empty(), "{relic:?}");
            let live = run(&[INFUSED, relic], 100).unwrap();
            assert!(!live.piles.get(PileId::Hand).is_empty(), "{relic:?}");
            for other in [&first, &live] {
                assert_eq!(
                    ended.rng.get(RngStream::Generation),
                    other.rng.get(RngStream::Generation),
                    "{relic:?}: the draw happens either way"
                );
            }
        }
        // A pass entered on an ended combat runs nobody.
        let catalog = catalog(&[LANTERN, FLOWER, SEAL]);
        let mut ended = HotState::at_defaults();
        ended.turn = 1;
        ended.gold = 10;
        ended.history.over = true;
        let before = (ended.energy, ended.flower(), ended.gold);
        after_side_turn_start(&catalog, &mut ended, false, &mut Vec::new()).unwrap();
        assert_eq!((ended.energy, ended.flower(), ended.gold), before);
    }

    // -----------------------------------------------------------------
    // #3635: the `BeforeHandDraw` relic walk
    // -----------------------------------------------------------------

    /// A turn-one start shaped like the headless harness's run on the
    /// archived v0.111.0 DLL: seed `TESTSEED`, an Ironclad on an empty
    /// `UnlockState`, the Draw pile in the order the game shuffled it (given,
    /// top first), and the run's own `Shuffle`, `CombatCardSelection` and
    /// `CombatCardGeneration` streams. The game's shuffle of an `n`-card deck
    /// took `n - 1` draws of `Shuffle`, which is where that stream starts.
    fn harness_turn_one(relics: &[RelicId], draw: &[CardId], vouched: bool) -> (Catalog, HotState) {
        use crate::catalog::RewardPool;
        let mut builder = CatalogBuilder::new();
        let generated = [CardId::Dazed, CardId::Soul, CardId::Shiv, CardId::Luminesce];
        for id in draw
            .iter()
            .copied()
            .chain(generated)
            .chain(crate::steps::neutral::colorless_generation_pool(Some(&[])))
        {
            builder
                .intern_reachable(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
        }
        builder.set_splash_unlock_epochs(Vec::new());
        builder.set_relics_ordered(relics, vouched).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.turn = 1;
        state.reward_card_pool = Some(RewardPool::Ironclad);
        state.entropy_card_pool = Some(RewardPool::Ironclad);
        for hp in [42, 53] {
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, hp));
        }
        let mut uid = 0;
        for id in draw {
            uid += 1;
            let atom = catalog
                .atom(&CardIdentity {
                    id: *id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        state.next_card_uid = uid + 1;
        let mut shuffle = crate::rng::run_stream_at_zero("TESTSEED", "shuffle");
        shuffle.shuffle(&mut vec![(); draw.len()]).unwrap();
        for (stream, rng) in [
            (RngStream::Rng, shuffle),
            (
                RngStream::Sel,
                crate::rng::run_stream_at_zero("TESTSEED", "combat_card_selection"),
            ),
            (
                RngStream::Generation,
                crate::rng::run_stream_at_zero("TESTSEED", "combat_card_generation"),
            ),
        ] {
            state.rng.set(
                stream,
                RngStreamState {
                    words: rng.words,
                    counter: rng.counter,
                },
            );
        }
        if relics.contains(&RelicId::RelicPendulum) {
            assert!(state.set_pendulum(0));
        }
        if relics.contains(&RelicId::RelicPollinousCore) {
            assert!(state.set_pollinous_core(0));
        }
        (catalog, state)
    }

    fn pile_ids(catalog: &Catalog, state: &HotState, pile: PileId) -> Vec<CardId> {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity.id)
            .collect()
    }

    /// The whole turn-one start, as the game's `StartTurn` runs it.
    fn harness_start(
        relics: &[RelicId],
        draw: &[CardId],
        vouched: bool,
    ) -> (Catalog, HotState, Vec<CardId>, Vec<CardId>) {
        let (catalog, mut state) = harness_turn_one(relics, draw, vouched);
        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
        let hand = pile_ids(&catalog, &state, PileId::Hand);
        let rest = pile_ids(&catalog, &state, PileId::Draw);
        (catalog, state, hand, rest)
    }

    const S: CardId = CardId::StrikeIronclad;
    const D: CardId = CardId::DefendIronclad;
    /// The starter nine as the game shuffled them for `TESTSEED`.
    const NINE: [CardId; 9] = [S, S, D, D, D, S, S, S, D];
    /// Four Strikes, four Defends, Inflame, Barricade and Demon Form, as the
    /// game shuffled them for `TESTSEED`.
    const ELEVEN: [CardId; 11] = [
        D,
        CardId::Barricade,
        S,
        D,
        S,
        D,
        CardId::Inflame,
        S,
        S,
        D,
        CardId::DemonForm,
    ];

    /// #3635, native: every pair of turn-one `BeforeHandDraw` card writers in
    /// both recorded orders, and two seven-relic inventories.
    ///
    /// Each expected Hand, Draw pile and stream counter is what the headless
    /// harness recorded from the archived v0.111.0 DLL with the relics
    /// granted in the listed order (`Player.Relics` order). The engine starts
    /// from the game's shuffled pile and the run's own streams
    /// ([`harness_turn_one`]), so the random Draw positions and Jeweled
    /// Mask's pick are the game's.
    #[test]
    fn before_hand_draw_relics_run_in_the_recorded_order_the_game_runs() {
        use CardId::{Barricade, Dazed, DemonForm, Inflame, Luminesce, Shiv, Soul};
        use RelicId::{
            RelicBlessedAntler as ANTLER, RelicFuneraryMask as FUNERARY,
            RelicJeweledMask as JEWELED, RelicNinjaScroll as SCROLL, RelicPendulum as PENDULUM,
            RelicPollinousCore as CORE, RelicRadiantPearl as PEARL,
        };
        let run = |relics: &[RelicId], draw: &[CardId]| {
            let (catalog, state, hand, rest) = harness_start(relics, draw, true);
            assert!(state.pending.is_none(), "{relics:?}");
            let fresh: Vec<u32> = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .filter(|card| {
                    matches!(
                        catalog.spec(card.atom).unwrap().identity.id,
                        Shiv | Luminesce
                    )
                })
                .map(|card| card.uid)
                .collect();
            assert!(
                fresh.windows(2).all(|pair| pair[0] < pair[1]),
                "{relics:?}: generated Hand cards carry uids in walk order"
            );
            (
                hand,
                rest,
                state.rng.get(RngStream::Rng).counter,
                state.rng.get(RngStream::Sel).counter,
            )
        };
        let dealt_nine = [S, S, D, D, D];
        let left_nine = vec![S, S, S, D];
        // Two Hand writers: the Hand holds them in recorded order.
        assert_eq!(
            run(&[SCROLL, PEARL], &NINE),
            (
                [&[Shiv, Shiv, Shiv, Luminesce][..], &dealt_nine].concat(),
                left_nine.clone(),
                8,
                0
            )
        );
        assert_eq!(
            run(&[PEARL, SCROLL], &NINE),
            (
                [&[Luminesce, Shiv, Shiv, Shiv][..], &dealt_nine].concat(),
                left_nine.clone(),
                8,
                0
            )
        );
        // Two Draw writers: the second three positions are rolled against the
        // pile the first three left.
        assert_eq!(
            run(&[FUNERARY, ANTLER], &NINE),
            (
                vec![S, S, D, Soul, D],
                vec![D, S, S, Soul, S, Soul, D, Dazed, Dazed, Dazed],
                14,
                0
            )
        );
        assert_eq!(
            run(&[ANTLER, FUNERARY], &NINE),
            (
                vec![S, S, D, Dazed, D],
                vec![D, S, S, Dazed, S, Dazed, D, Soul, Soul, Soul],
                14,
                0
            )
        );
        // Jeweled Mask beside a Draw writer: the same Power is lifted (the
        // candidates keep their order), and the inserts are rolled against a
        // pile that has or has not lost it.
        let lifted = vec![DemonForm, D, Barricade, S, D, S];
        assert_eq!(
            run(&[JEWELED, FUNERARY], &ELEVEN),
            (
                lifted.clone(),
                vec![D, Inflame, S, S, Soul, D, Soul, Soul],
                13,
                1
            )
        );
        assert_eq!(
            run(&[FUNERARY, JEWELED], &ELEVEN),
            (
                lifted.clone(),
                vec![D, Inflame, S, S, D, Soul, Soul, Soul],
                13,
                1
            )
        );
        assert_eq!(
            run(&[JEWELED, ANTLER], &ELEVEN),
            (
                lifted.clone(),
                vec![D, Inflame, S, S, Dazed, D, Dazed, Dazed],
                13,
                1
            )
        );
        assert_eq!(
            run(&[ANTLER, JEWELED], &ELEVEN),
            (
                lifted.clone(),
                vec![D, Inflame, S, S, D, Dazed, Dazed, Dazed],
                13,
                1
            )
        );
        // Jeweled Mask beside a Hand writer.
        let dealt_eleven = [D, Barricade, S, D, S];
        let left_eleven = vec![D, Inflame, S, S, D];
        for (relics, head) in [
            ([JEWELED, SCROLL], vec![DemonForm, Shiv, Shiv, Shiv]),
            ([SCROLL, JEWELED], vec![Shiv, Shiv, Shiv, DemonForm]),
            ([JEWELED, PEARL], vec![DemonForm, Luminesce]),
            ([PEARL, JEWELED], vec![Luminesce, DemonForm]),
        ] {
            assert_eq!(
                run(&relics, &ELEVEN),
                (
                    [&head[..], &dealt_eleven].concat(),
                    left_eleven.clone(),
                    10,
                    1
                ),
                "{relics:?}"
            );
        }
        // Seven of the eight, two ways round.
        assert_eq!(
            run(
                &[PEARL, ANTLER, JEWELED, SCROLL, FUNERARY, PENDULUM, CORE],
                &ELEVEN
            ),
            (
                [&[Luminesce, DemonForm, Shiv, Shiv, Shiv][..], &dealt_eleven].concat(),
                vec![D, Inflame, S, S, D, Dazed, Dazed, Soul, Dazed, Soul, Soul],
                16,
                1
            )
        );
        assert_eq!(
            run(
                &[FUNERARY, SCROLL, CORE, JEWELED, PENDULUM, ANTLER, PEARL],
                &ELEVEN
            ),
            (
                [&[Shiv, Shiv, Shiv, DemonForm, Luminesce][..], &dealt_eleven].concat(),
                vec![D, Inflame, S, S, D, Soul, Soul, Dazed, Soul, Dazed, Dazed],
                16,
                1
            )
        );
    }

    /// #3635, native: Toolbox's choice splits the pass at Toolbox's recorded
    /// place.
    ///
    /// The harness pushed the game's `TestCardSelector` with a pending
    /// source, so the hook really suspended; the first tuple is the game at
    /// that pause and the second after the first option was picked (Secret
    /// Weapon, the head of the run's `CombatCardGeneration` draw, counter
    /// 35). A relic recorded ahead of Toolbox has run at the pause and one
    /// recorded behind it has not. With Orange Dough owned, `StartTurn`'s
    /// side-start tail has put its two cards in the Hand at the pause, ahead
    /// of whatever the relics behind Toolbox add on resume.
    #[test]
    fn toolbox_pauses_the_before_hand_draw_walk_where_the_game_does() {
        use CardId::{
            Barricade, Dazed, DemonForm, DramaticEntrance, Fisticuffs, Inflame, Luminesce,
            SecretWeapon, Shiv, Soul,
        };
        use RelicId::{
            RelicBlessedAntler as ANTLER, RelicFuneraryMask as FUNERARY,
            RelicJeweledMask as JEWELED, RelicNinjaScroll as SCROLL, RelicOrangeDough as DOUGH,
            RelicPendulum as PENDULUM, RelicPollinousCore as CORE, RelicRadiantPearl as PEARL,
            RelicToolbox as TOOLBOX,
        };
        type Piles = (Vec<CardId>, Vec<CardId>);
        let run = |relics: &[RelicId], draw: &[CardId], core: i16| -> (Piles, Piles, HotState) {
            let (catalog, mut state) = harness_turn_one(relics, draw, true);
            if relics.contains(&CORE) {
                assert!(state.set_pollinous_core(core));
            }
            let mut events = Vec::new();
            crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut events).unwrap();
            assert_eq!(
                state.fanouts.batch_nine_relic_pending().map(|p| p.kind),
                Some(RelicPendingKind::Toolbox),
                "{relics:?}"
            );
            let paused = (
                pile_ids(&catalog, &state, PileId::Hand),
                pile_ids(&catalog, &state, PileId::Draw),
            );
            let parked = state.clone();
            resume_relic_selection(&mut state, &catalog, 0, &mut events).unwrap();
            assert!(state.pending.is_none(), "{relics:?}");
            let done = (
                pile_ids(&catalog, &state, PileId::Hand),
                pile_ids(&catalog, &state, PileId::Draw),
            );
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                if relics.contains(&DOUGH) { 70 } else { 35 },
                "{relics:?}"
            );
            (paused, done, parked)
        };
        let dealt = [S, S, D, D, D];
        let left = vec![S, S, S, D];
        for (relics, at_pause, head) in [
            (
                [TOOLBOX, SCROLL],
                vec![],
                vec![SecretWeapon, Shiv, Shiv, Shiv],
            ),
            (
                [SCROLL, TOOLBOX],
                vec![Shiv, Shiv, Shiv],
                vec![Shiv, Shiv, Shiv, SecretWeapon],
            ),
            ([TOOLBOX, PEARL], vec![], vec![SecretWeapon, Luminesce]),
            (
                [PEARL, TOOLBOX],
                vec![Luminesce],
                vec![Luminesce, SecretWeapon],
            ),
        ] {
            let (paused, done, _) = run(&relics, &NINE, 0);
            assert_eq!(paused, (at_pause, NINE.to_vec()), "{relics:?}");
            assert_eq!(
                done,
                ([&head[..], &dealt].concat(), left.clone()),
                "{relics:?}"
            );
        }
        // A Draw writer: behind Toolbox it has not rolled at the pause.
        for (relics, id) in [
            ([TOOLBOX, FUNERARY], Soul),
            ([FUNERARY, TOOLBOX], Soul),
            ([TOOLBOX, ANTLER], Dazed),
            ([ANTLER, TOOLBOX], Dazed),
        ] {
            let (paused, done, parked) = run(&relics, &NINE, 0);
            let ahead = relics[1] == TOOLBOX;
            assert_eq!(
                paused,
                (
                    vec![],
                    if ahead {
                        vec![S, S, D, id, D, D, S, S, id, S, id, D]
                    } else {
                        NINE.to_vec()
                    }
                ),
                "{relics:?}"
            );
            assert_eq!(
                parked.rng.get(RngStream::Rng).counter,
                if ahead { 11 } else { 8 }
            );
            assert_eq!(
                done,
                (
                    vec![SecretWeapon, S, S, D, id, D],
                    vec![D, S, S, id, S, id, D]
                ),
                "{relics:?}"
            );
        }
        // Jeweled Mask: its Selection draw is on its own side of the pause.
        let left_eleven = vec![D, Inflame, S, S, D];
        let (paused, done, parked) = run(&[TOOLBOX, JEWELED], &ELEVEN, 0);
        assert_eq!(paused, (vec![], ELEVEN.to_vec()));
        assert_eq!(parked.rng.get(RngStream::Sel).counter, 0);
        assert_eq!(
            done,
            (
                vec![SecretWeapon, DemonForm, D, Barricade, S, D, S],
                left_eleven.clone()
            )
        );
        let (paused, done, parked) = run(&[JEWELED, TOOLBOX], &ELEVEN, 0);
        assert_eq!(paused, (vec![DemonForm], ELEVEN[..10].to_vec()));
        assert_eq!(parked.rng.get(RngStream::Sel).counter, 1);
        assert_eq!(
            done,
            (
                vec![DemonForm, SecretWeapon, D, Barricade, S, D, S],
                left_eleven
            )
        );
        // The counters. Pollinous Core entering at three is at four at the
        // pause when recorded ahead of Toolbox and still at three behind it;
        // either way the fourth turn draws two more and resets it.
        let seven = (vec![SecretWeapon, S, S, D, D, D, S, S], vec![S, D]);
        let (_, done, parked) = run(&[CORE, TOOLBOX], &NINE, 3);
        assert_eq!(parked.pollinous_core(), 4);
        assert_eq!(done, seven);
        let (_, done, parked) = run(&[TOOLBOX, CORE], &NINE, 3);
        assert_eq!(parked.pollinous_core(), 3);
        assert_eq!(done, seven);
        let (_, _, parked) = run(&[CORE, TOOLBOX], &NINE, 0);
        assert_eq!(parked.pollinous_core(), 1);
        let (_, _, parked) = run(&[TOOLBOX, CORE], &NINE, 0);
        assert_eq!(parked.pollinous_core(), 0);
        let (_, _, parked) = run(&[PENDULUM, TOOLBOX], &NINE, 0);
        assert_eq!(parked.pendulum(), 1);
        let (_, _, parked) = run(&[TOOLBOX, PENDULUM], &NINE, 0);
        assert_eq!(parked.pendulum(), 0);
        // The side-start tail at the pause: Orange Dough's two cards are in
        // the Hand before the Shivs of a Scroll recorded behind Toolbox, and
        // the deal stops at the ten-card Hand.
        let (paused, done, _) = run(&[TOOLBOX, SCROLL, DOUGH], &NINE, 0);
        assert_eq!(paused, (vec![DramaticEntrance, Fisticuffs], NINE.to_vec()));
        assert_eq!(
            done,
            (
                vec![
                    DramaticEntrance,
                    Fisticuffs,
                    SecretWeapon,
                    Shiv,
                    Shiv,
                    Shiv,
                    S,
                    S,
                    D,
                    D
                ],
                vec![D, S, S, S, D]
            )
        );
        let (paused, done, _) = run(&[SCROLL, TOOLBOX, DOUGH], &NINE, 0);
        assert_eq!(
            paused,
            (
                vec![Shiv, Shiv, Shiv, DramaticEntrance, Fisticuffs],
                NINE.to_vec()
            )
        );
        assert_eq!(
            done,
            (
                vec![
                    Shiv,
                    Shiv,
                    Shiv,
                    DramaticEntrance,
                    Fisticuffs,
                    SecretWeapon,
                    S,
                    S,
                    D,
                    D
                ],
                vec![D, S, S, S, D]
            )
        );
    }

    /// #3635, native: Pollinous Core's fourth turn beside Jeweled Mask.
    ///
    /// The game, entering with `TurnsSeen` at three, lifts Demon Form and
    /// deals seven in either order, and leaves the counter at zero. Before
    /// #3635 this engine returned from the relic pass at Pollinous Core's
    /// reset and never ran the Jeweled Mask behind it.
    #[test]
    fn pollinous_cores_fourth_turn_does_not_skip_the_relics_behind_it() {
        use CardId::{Barricade, DemonForm, Inflame};
        use RelicId::{RelicJeweledMask as JEWELED, RelicPollinousCore as CORE};
        for vouched in [true, false] {
            for relics in [[CORE, JEWELED], [JEWELED, CORE]] {
                let (catalog, mut state) = harness_turn_one(&relics, &ELEVEN, vouched);
                assert!(state.set_pollinous_core(3));
                crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new())
                    .unwrap();
                assert_eq!(
                    pile_ids(&catalog, &state, PileId::Hand),
                    [DemonForm, D, Barricade, S, D, S, D, Inflame],
                    "{relics:?} vouched {vouched}"
                );
                assert_eq!(pile_ids(&catalog, &state, PileId::Draw), [S, S, D]);
                assert_eq!(state.pollinous_core(), 0);
                assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
            }
        }
    }

    /// #3635, native: a listener that ends the combat inside the pass does
    /// not stop the relics behind it, and each does what the game does there.
    ///
    /// Smokestack (60, applied before the combat starts) answers Blessed
    /// Antler's first Dazed and kills both enemies at 5 HP. The game leaves
    /// that one Dazed in the Draw pile (one `Shuffle` roll, counter 9), adds
    /// neither of the other two, and then:
    ///
    /// * Ninja Scroll, Radiant Pearl and Funerary Mask add nothing and roll
    ///   nothing;
    /// * Pollinous Core entering at three is left at four, Pendulum at one;
    /// * Jeweled Mask draws `CombatCardSelection` (counter 1) and Demon Form
    ///   stays in the Draw pile;
    /// * Toolbox draws `CombatCardGeneration` (counter 35), offers nothing,
    ///   and nothing recorded behind it runs: a Pendulum behind it stays at
    ///   zero, one ahead of it is at one.
    ///
    /// The Hand is never dealt. With a Smokestack of 1 the enemies live (39
    /// and 50) and the pass is the ordinary one.
    #[test]
    fn before_hand_draw_relics_behind_a_listener_that_ended_combat_do_what_the_game_does() {
        use CardId::{Barricade, Dazed, DemonForm, Inflame, Shiv};
        use RelicId::{
            RelicBlessedAntler as ANTLER, RelicFuneraryMask as FUNERARY,
            RelicJeweledMask as JEWELED, RelicNinjaScroll as SCROLL, RelicPendulum as PENDULUM,
            RelicPollinousCore as CORE, RelicRadiantPearl as PEARL, RelicToolbox as TOOLBOX,
        };
        let run = |relics: &[RelicId], draw: &[CardId], smokestack: i32, core: i16| {
            let (catalog, mut state) = harness_turn_one(relics, draw, true);
            state
                .powers
                .set(PowerId::Smokestack, SlotWire::Int, smokestack);
            assert!(
                state
                    .fanouts
                    .set_local_generated_power_order(&[PowerId::Smokestack])
            );
            if smokestack > 1 {
                for monster in state.monsters_mut().iter_mut() {
                    monster.hp = 5;
                }
            }
            if relics.contains(&CORE) {
                assert!(state.set_pollinous_core(core));
            }
            crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
            assert!(state.pending.is_none(), "{relics:?}");
            (catalog, state)
        };
        let one_dazed = vec![S, S, D, D, D, S, S, Dazed, S, D];
        for relics in [
            vec![ANTLER],
            vec![ANTLER, SCROLL],
            vec![ANTLER, PEARL],
            vec![ANTLER, FUNERARY],
            vec![ANTLER, CORE],
            vec![ANTLER, PENDULUM],
            vec![ANTLER, TOOLBOX],
            vec![ANTLER, TOOLBOX, PENDULUM, CORE],
            vec![ANTLER, PENDULUM, TOOLBOX, CORE],
        ] {
            let (catalog, state) = run(&relics, &NINE, 60, 3);
            assert!(state.history.over, "{relics:?}");
            assert_eq!(pile_ids(&catalog, &state, PileId::Hand), [], "{relics:?}");
            assert_eq!(
                pile_ids(&catalog, &state, PileId::Draw),
                one_dazed,
                "{relics:?}"
            );
            assert_eq!(state.rng.get(RngStream::Rng).counter, 9, "{relics:?}");
            let toolbox = relics.iter().position(|relic| *relic == TOOLBOX);
            assert_eq!(
                state.rng.get(RngStream::Generation).counter,
                if toolbox.is_some() { 35 } else { 0 },
                "{relics:?}"
            );
            let ran = |relic: RelicId| {
                relics
                    .iter()
                    .position(|owned| *owned == relic)
                    .is_some_and(|at| toolbox.is_none_or(|toolbox| at < toolbox))
            };
            if relics.contains(&PENDULUM) {
                assert_eq!(state.pendulum(), i16::from(ran(PENDULUM)), "{relics:?}");
            }
            if relics.contains(&CORE) {
                assert_eq!(
                    state.pollinous_core(),
                    if ran(CORE) { 4 } else { 3 },
                    "{relics:?}"
                );
            }
        }
        // Jeweled Mask: the draw, and the card stays where it was.
        let (catalog, state) = run(&[ANTLER, JEWELED], &ELEVEN, 60, 0);
        assert_eq!(pile_ids(&catalog, &state, PileId::Hand), []);
        assert_eq!(
            pile_ids(&catalog, &state, PileId::Draw),
            [D, Barricade, S, D, S, D, Inflame, S, S, D, Dazed, DemonForm]
        );
        assert_eq!(state.rng.get(RngStream::Rng).counter, 11);
        assert_eq!(state.rng.get(RngStream::Sel).counter, 1);
        // The control: the same listener without the kill.
        let (catalog, state) = run(&[ANTLER, SCROLL], &NINE, 1, 0);
        assert!(!state.history.over);
        assert_eq!(
            state.monsters.iter().map(|m| m.hp).collect::<Vec<_>>(),
            [39, 50]
        );
        assert_eq!(
            pile_ids(&catalog, &state, PileId::Hand),
            [Shiv, Shiv, Shiv, S, S, D, Dazed, D]
        );
        assert_eq!(
            pile_ids(&catalog, &state, PileId::Draw),
            [D, S, S, Dazed, S, Dazed, D]
        );
        assert_eq!(state.rng.get(RngStream::Rng).counter, 11);
    }

    /// #3635: the walk's own branches, on states no fight is needed for.
    #[test]
    fn the_before_hand_draw_walk_gates_orders_and_refuses_by_name() {
        use RelicId::{
            RelicBlessedAntler as ANTLER, RelicFuneraryMask as FUNERARY,
            RelicJeweledMask as JEWELED, RelicNinjaScroll as SCROLL, RelicPendulum as PENDULUM,
            RelicPollinousCore as CORE, RelicRadiantPearl as PEARL, RelicToolbox as TOOLBOX,
        };
        let ordered = |relics: &[RelicId], vouched: bool| {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(relics, vouched).unwrap();
            builder.build()
        };
        let walk = |catalog: &Catalog| {
            relic_walk_order(catalog, &BEFORE_HAND_DRAW_RELIC_DISPATCH).collect::<Vec<_>>()
        };
        // A vouched inventory is walked as recorded, an unvouched one in the
        // fixed sequence this engine ran before the order was read.
        let reversed = [
            PEARL, JEWELED, CORE, SCROLL, FUNERARY, ANTLER, TOOLBOX, PENDULUM,
        ];
        assert_eq!(walk(&ordered(&reversed, true)), reversed);
        assert_eq!(
            walk(&ordered(&reversed, false)),
            [
                PENDULUM, TOOLBOX, ANTLER, FUNERARY, SCROLL, CORE, JEWELED, PEARL
            ]
        );
        // Entry gates: only the two counters act after turn one.
        let all = ordered(&reversed, true);
        let mut state = HotState::at_defaults();
        state.turn = 2;
        for relic in reversed {
            assert_eq!(
                before_hand_draw_relic_acts(&all, &state, relic),
                matches!(relic, PENDULUM | CORE),
                "{relic:?}"
            );
        }
        state.turn = 1;
        assert!(
            reversed
                .into_iter()
                .all(|relic| before_hand_draw_relic_acts(&all, &state, relic))
        );
        assert!(!before_hand_draw_relic_acts(
            &ordered(&[CORE], true),
            &state,
            SCROLL
        ));
        // A pass entered on an ended combat runs no relic.
        let counters = ordered(&[PENDULUM, CORE], true);
        let mut state = HotState::at_defaults();
        state.turn = 3;
        assert!(state.set_pendulum(0) && state.set_pollinous_core(0));
        state.history.over = true;
        assert_eq!(
            before_hand_draw(&counters, &mut state, false, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!((state.pendulum(), state.pollinous_core()), (0, 0));
        // One a listener ended runs both counters.
        assert_eq!(
            before_hand_draw(&counters, &mut state, true, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!((state.pendulum(), state.pollinous_core()), (1, 1));
        // The fold neither reads nor resets Pollinous Core on an ended combat.
        assert!(state.set_pollinous_core(4));
        assert_eq!(pollinous_core_modify_hand_draw(&counters, &mut state), 0);
        assert_eq!(state.pollinous_core(), 4);
        state.history.over = false;
        assert_eq!(pollinous_core_modify_hand_draw(&counters, &mut state), 2);
        assert_eq!(state.pollinous_core(), 0);
        // A counter that enters a live pass already at `Turns` counts on,
        // and the fold adds the two cards and resets it (#3915 review).
        assert!(state.set_pollinous_core(4));
        assert!(!before_hand_draw_walk_can_refuse(&counters, &state));
        assert_eq!(
            before_hand_draw(&counters, &mut state, true, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!(state.pollinous_core(), 5);
        assert_eq!(pollinous_core_modify_hand_draw(&counters, &mut state), 2);
        assert_eq!(state.pollinous_core(), 0);
        // Past what the stored counter holds, the walk refuses by name.
        assert!(state.set_pollinous_core(6));
        assert!(!state.set_pollinous_core(7));
        assert!(before_hand_draw_walk_can_refuse(&counters, &state));
        let before = state.pendulum();
        assert_eq!(
            before_hand_draw(&counters, &mut state, true, &mut Vec::new()),
            Err(EngineRefusal::CounterOverflow(
                "Pollinous Core TurnsSeen past its stored range"
            ))
        );
        assert_eq!(
            state.pendulum(),
            (before + 1) % 3,
            "the walk itself is not a rehearsal"
        );
        assert!(state.set_pollinous_core(0));
        assert!(!before_hand_draw_walk_can_refuse(&counters, &state));
        state.turn = 1;
        assert!(before_hand_draw_walk_can_refuse(&counters, &state));
        assert!(!before_hand_draw_walk_can_refuse(
            &ordered(&[PENDULUM], true),
            &state
        ));
        state.turn = 2;
        assert!(!before_hand_draw_walk_can_refuse(
            &ordered(&[SCROLL], true),
            &state
        ));
        // A resume point the walk does not hold.
        assert_eq!(
            resume_before_hand_draw_after_toolbox(&counters, &mut state, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "BeforeHandDraw relic walk resume point"
            ))
        );
        // A choice parked inside a body: with a later relic still to act the
        // walk refuses, with none it returns parked.
        let mut parked = HotState::at_defaults();
        parked.turn = 2;
        assert!(parked.set_pendulum(0) && parked.set_pollinous_core(0));
        parked.pending = Some(std::sync::Arc::new(PendingSelection::relic_selection()));
        assert_eq!(
            before_hand_draw(&counters, &mut parked.clone(), true, &mut Vec::new()),
            Err(EngineRefusal::PowerOrderNotModeled(
                "BeforeHandDraw relic behind a listener choice parked inside an earlier relic body"
            ))
        );
        assert_eq!(
            before_hand_draw(
                &ordered(&[PENDULUM], true),
                &mut parked,
                true,
                &mut Vec::new()
            ),
            Ok(false)
        );
        // Toolbox's pause on an unvouched inventory: any other relic of the
        // pass that acts this turn is on an unknown side of it.
        let mut turn_one = HotState::at_defaults();
        turn_one.turn = 1;
        let refusal = Err(EngineRefusal::PowerOrderNotModeled(
            "BeforeHandDraw relic order around Toolbox's paused choice",
        ));
        for peer in [PENDULUM, ANTLER, FUNERARY, SCROLL, CORE, JEWELED, PEARL] {
            assert_eq!(
                before_hand_draw_pause_order_is_native(
                    &ordered(&[TOOLBOX, peer], false),
                    &turn_one
                ),
                refusal,
                "{peer:?}"
            );
            assert_eq!(
                before_hand_draw_pause_order_is_native(&ordered(&[peer, TOOLBOX], true), &turn_one),
                Ok(()),
                "{peer:?}"
            );
        }
        assert_eq!(
            before_hand_draw_pause_order_is_native(&ordered(&[TOOLBOX], false), &turn_one),
            Ok(())
        );
    }

    /// #3635: a refusal inside the turn-one relic walk leaves the state as
    /// it was, because the turn is rehearsed on a clone.
    #[test]
    fn a_refusal_inside_the_before_hand_draw_walk_writes_nothing() {
        use RelicId::{
            RelicNinjaScroll as SCROLL, RelicPendulum as PENDULUM, RelicToolbox as TOOLBOX,
        };
        // An unvouched Toolbox beside a Scroll and a Pendulum: Pendulum has
        // counted and Toolbox has drawn when the pause-order refusal fires.
        let (catalog, mut state) = harness_turn_one(&[SCROLL, PENDULUM, TOOLBOX], &NINE, false);
        let before = state.clone();
        assert_eq!(
            crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()),
            Err(EngineRefusal::PowerOrderNotModeled(
                "BeforeHandDraw relic order around Toolbox's paused choice"
            ))
        );
        assert!(state == before);
        // The same inventory vouched for parks at Toolbox.
        let (catalog, mut state) = harness_turn_one(&[SCROLL, PENDULUM, TOOLBOX], &NINE, true);
        crate::engine::turn::begin_player_turn(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert!(state.pending.is_some());
        assert_eq!(state.pendulum(), 1);
    }
}

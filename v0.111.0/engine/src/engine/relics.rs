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
/// three relics; the third, `RELIC.TWISTED_FUNNEL`, is still gated by
/// `entry::opening`'s `TURN_ONE_RELIC_BODIES` and is deliberately absent — its
/// amount picks up `RELIC.SNECKO_SKULL` and its application allocates a Poison
/// instance identity, which the oracle's own opening call (made with
/// `state=None`, so `m.poison_uid = -1`, `_record_misery_debuff_application`) does not,
/// so it is a different shape and owes its own derivation.
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
const TURN_ONE_ALL_ENEMY_DEBUFFS: [(RelicId, PowerId, MiseryToken, i32); 2] = [
    (RelicId::RelicRedMask, PowerId::Weak, MiseryToken::Weak, 1),
    (
        RelicId::RelicBagOfMarbles,
        PowerId::Vuln,
        MiseryToken::Vuln,
        1,
    ),
];

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
/// of which these two writes can reach.
fn turn_one_all_enemy_debuffs(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn > 1 {
        return Ok(());
    }
    for (relic, power, token, amount) in TURN_ONE_ALL_ENEMY_DEBUFFS {
        if !catalog.hooks().owns(relic) {
            continue;
        }
        // `ICombatState::get_HittableEnemies` (`0x1373a5`) is enumerated fresh
        // per body, and neither Weak nor Vulnerable can change liveness.
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

/// Persistent relic listeners on `BeforeHandDraw`. Pollinous Core's native
/// value briefly reaches four; resetting it here is observationally identical
/// because no represented listener reads the counter before ModifyHandDraw.
pub(crate) fn before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    // `Pendulum::BeforeHandDraw` (v0.111.0 RVA `0x98fa0`): owner check
    // `IL_000c`-`IL_001a`, then `set_TurnsSeen((TurnsSeen + 1) % Turns)` at
    // `IL_001b`-`IL_003a` with `Turns = 3` from `get_CanonicalVars`
    // (`0x98f3a`). Everything after that write is presentation —
    // `set_Status(TurnsSeen == Turns - 1)` at `IL_003f`-`IL_0063` and the
    // `DoActivateVisuals` flash at `IL_0068`-`IL_0076` — and `AfterCombatEnd`
    // (`0x99093`) clears only `Status`, never `TurnsSeen`, which is why the
    // counter is a saved per-run value the opening seeds. The advance happens
    // once, **before** any Toolbox pause below, so the later ModifyHandDraw
    // contribution reads the completed value: oracle
    // `_begin_before_hand_draw_relics`, frozen Python, deleted #2827.
    if catalog.hooks().owns(RelicId::RelicPendulum) {
        let next = (state.pendulum() + 1) % 3;
        let written = state.set_pendulum(next);
        debug_assert!(written);
    }
    if state.turn == 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicToolbox) {
        begin_generation_relic_selection(state, catalog, RelicPendingKind::Toolbox, 3)?;
        crate::coverage::record_relic(RelicId::RelicToolbox);
        return Ok(false);
    }
    continue_before_hand_draw_after_toolbox(catalog, state, events)
}

pub(crate) fn continue_before_hand_draw_after_toolbox(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if state.turn == 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicBlessedAntler) {
        for _ in 0..3 {
            // The oracle commits each Dazed with `force_exact=True`
            // (`_add_one_blessed_antler_dazed`, frozen Python, deleted #2827),
            // and `_commit_live_card_piles` makes exact mode visible before
            // the card enters Draw, so the listeners below see
            // it. `exact_piles` is the solver's pile-identity promotion, not a
            // native field; its safety gate is skipped for Rust-opening roots
            // (#2952), the only roots that reach this turn-1 row (#2992).
            if !state.history.over {
                state.exact_piles = true;
            }
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
        crate::coverage::record_relic(RelicId::RelicBlessedAntler);
    }
    // `FuneraryMask/<BeforeHandDraw>d__6::MoveNext` (v0.111.0 RVA `0x325000`,
    // re-read for #3162): `player == Owner` (`IL_0020`-`IL_002e`),
    // `TurnNumber == 1` (`IL_0033`-`IL_0046`), `Flash`, then while
    // `i < DynamicVars.Cards.BaseValue` (`CardsVar(3)`, `get_CanonicalVars`
    // `0x93ddc`; loop test `IL_00fd`-`IL_011d`) one
    // `ICombatState::CreateCard(Card<Soul>, Owner)` and
    // `CardPileCmd::AddGeneratedCardToCombat(card, PileType.Draw, Owner,
    // CardPilePosition.Random)` (`IL_005d`-`IL_007d`), awaited per card;
    // `PreviewCardPileAdd` is presentation. No saved property.
    if state.turn == 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicFuneraryMask) {
        for _ in 0..3 {
            super::cards::inject_generated_draw_random(
                state,
                catalog,
                crate::catalog::CardIdentity {
                    id: CardId::Soul,
                    upgrade: 0,
                    enchantment: None,
                },
                events,
            )?;
        }
        crate::coverage::record_relic(RelicId::RelicFuneraryMask);
    }
    if catalog.hooks().owns(RelicId::RelicPollinousCore) && !state.history.over {
        if state.pollinous_core() == 3 {
            let written = state.set_pollinous_core(0);
            debug_assert!(written);
            crate::coverage::record_relic(RelicId::RelicPollinousCore);
            return Ok(true);
        }
        let written = state.set_pollinous_core(state.pollinous_core() + 1);
        debug_assert!(written);
    }
    jeweled_mask_before_hand_draw(catalog, state)?;
    Ok(false)
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
/// There is no ending gate in the body; the `history.over` guard here is the
/// engine's usual one for a hook that cannot be reached once combat ended.
/// `CardModel.Keywords` is the effective set, and the only combat-local
/// keywords this crate represents are Ethereal, Retain and Sly
/// (`hot::CardStates`), none of them Innate, so the spec's `innate` flag is the
/// whole answer for a Power (`Imbued`, the one Innate-sensitive enchantment in
/// the opening's fixup, enchants Skills only: `Imbued::CanEnchantCardType`
/// `0xd60a9` is `Type == 2`).
///
/// # Position inside the walk
///
/// Native runs relic `BeforeHandDraw` listeners in `Player.Relics` order.
/// This body runs after Blessed Antler's and Funerary Mask's Draw inserts and
/// Pollinous Core's counter, and before the `ModifyHandDraw` fold and Radiant
/// Pearl's Hand insert. Pendulum's and Pollinous Core's bodies write only their
/// counters, so they commute with it; every other represented same-hook body
/// moves a Draw or Hand card, and the opening refuses those co-ownerships by
/// name (`entry::opening`'s `refuse_unordered_hand_draw_card_peers`, Blessed
/// Antler's and Toolbox's gates).
fn jeweled_mask_before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.turn != 1 || state.history.over || !catalog.hooks().owns(RelicId::RelicJeweledMask) {
        return Ok(());
    }
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
    let mut chosen = draw.remove(index);
    let spec = catalog
        .spec(chosen.atom)
        .ok_or(EngineRefusal::UnknownAtom(chosen.atom))?;
    state
        .card_states
        .set_to_free_this_turn(chosen.uid, spec.cost)
        .ok_or(EngineRefusal::MalformedArgs("Jeweled Mask free card"))?;
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
/// property and no RNG draw. It runs after `ModifyHandDraw` is totalled here
/// and before the Draw; the total reads no pile, so that is the native order.
/// Its same-hook Hand/Draw peers are refused in the opening
/// (`entry::opening`'s `refuse_unordered_hand_draw_card_peers`).
pub(crate) fn radiant_pearl_before_hand_draw(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.turn == 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicRadiantPearl) {
        super::cards::inject_generated_bottom(
            state,
            catalog,
            crate::catalog::CardIdentity {
                id: CardId::Luminesce,
                upgrade: 0,
                enchantment: None,
            },
            1,
            PileId::Hand,
            events,
        )?;
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
/// The oracle is `_continue_after_toasty_mittens`' `s.turn <= 1 and s.bellows
/// and not s.over` block (frozen Python, deleted #2827), which calls
/// `_upgrade_hand_once`. It runs **first** in that fixed
/// suffix, ahead of Bone Tea, so it is placed first here too. The frozen Hand
/// snapshot and the atomic transaction are [`super::cards::upgrade_live_cards_once`],
/// the same command Bone Tea's pass below uses. Bone Tea is the only same-hook
/// peer admission lets co-own it, because two clipped upgrade passes commute;
/// every other order-observable pairing is refused by name (admission's Vexing
/// Puzzlebox list, and `entry::opening`'s `AfterPlayerTurnStart` peer gate).
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
/// The oracle is `s.turn == 1 and s.popper and not s.over`, then
/// `damage_monster(s, m, 9.0, powered=False, dealer=PLAYER)` for each
/// `m in s.alive()` (frozen Python `_continue_after_toasty_mittens`, deleted #2827); `s.alive()` is a list
/// built before the first hit. That is exactly the shape of
/// Mr Struggles' block below — the same `alive_targets` snapshot and the same
/// catalog-authenticated unpowered, blockable `damage_monster_with_catalog`
/// call — and Popper sits between Bone Tea and Mr Struggles in the oracle's
/// fixed suffix, so it is placed there. The same-hook peers whose order around
/// it is observable (a second enemy-damage body — Mercury Hourglass, Mr
/// Struggles — Royal Poison, Bone Tea, and the Hand-choice and generation
/// relics) are refused by name at the opening
/// (`entry::opening::after_player_turn_start_peers_are_ordered`) and in
/// admission (`turn_start_damage_relics` and the Bone Tea / Vexing Puzzlebox
/// lists). The oracle's one recorded-order Queen trio is refused rather than
/// admitted, which is stricter than the oracle and never wrong. The one
/// exceptions are Toasty Mittens (#2884) and Gambling Chip under a vouched
/// inventory: Popper then runs on its recorded side of that relic, ahead of
/// the Hand choice when it was recorded first
/// ([`festive_popper_precedes_toasty_mittens`],
/// [`festive_popper_precedes_gambling_chip`]).
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

fn continue_after_toasty_mittens(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    bellows_after_player_turn_start(catalog, state)?;
    if state.turn <= 1
        && !state.history.over
        && catalog.hooks().owns(RelicId::RelicBoneTea)
        && state.fanouts.bone_tea_combats_left() > 0
    {
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
    }
    if !festive_popper_precedes_toasty_mittens(catalog)
        && !festive_popper_precedes_gambling_chip(catalog)
    {
        festive_popper_after_player_turn_start(catalog, state, events)?;
    }
    if !state.history.over && catalog.hooks().owns(RelicId::RelicMrStruggles) {
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
    }
    if state.turn <= 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicRoyalPoison) {
        super::damage::damage_player_from_power_with_catalog(state, catalog, 4, false, events)?;
        crate::coverage::record_relic(RelicId::RelicRoyalPoison);
    }
    if state.emotion_chip_owned() && state.emotion_damage_previous_turn() && !state.history.over {
        let orb_count = state.orbs.as_slice().len();
        for index in 0..orb_count {
            super::orbs::passive_at_affected_by_hooks(state, catalog, index, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicEmotionChip);
    }
    Ok(())
}

/// v0.111.0 AfterPlayerTurnStart awaits inventory-order listeners.
/// VexingPuzzlebox MoveNext (RVA 0x333de0) adds its turn-1 generated card
/// before ToastyMittens MoveNext (RVA 0x33285c) selects and exhausts from
/// Hand, then grants Strength. This fixed suffix requires that recorded
/// order; the reverse would offer a different selection pool.
pub(crate) fn vexing_toasty_order_is_exact(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    if !hooks.owns(RelicId::RelicVexingPuzzlebox) || !hooks.owns(RelicId::RelicToastyMittens) {
        return true;
    }
    hooks.dispatch_ordered()
        && hooks
            .relics()
            .iter()
            .position(|r| *r == RelicId::RelicVexingPuzzlebox)
            < hooks
                .relics()
                .iter()
                .position(|r| *r == RelicId::RelicToastyMittens)
}

/// Whether Festive Popper's turn-one damage runs **ahead of** Toasty Mittens'
/// Hand choice (#2884): both owned, the inventory vouched for as dispatch
/// order, and every Festive Popper recorded before every Toasty Mittens.
///
/// Native awaits `Hook::AfterPlayerTurnStart`'s relic listeners one by one in
/// `Player.Relics` order (`Hook/<AfterPlayerTurnStart>d__56::MoveNext`, v0.111.0
/// RVA `0x3cff0c`: `IterateCombatHookListeners` at `IL_012d`, then each
/// listener's `AbstractModel::AfterPlayerTurnStart` awaited at `IL_017a`, with
/// no ending check between listeners). The two bodies do not commute:
///
/// * `FestivePopper/<AfterPlayerTurnStart>d__4::MoveNext` (RVA `0x324b94`)
///   deals its 9 to `HittableEnemies` at `IL_0072`-`IL_0094` on
///   `TurnNumber == 1` (`IL_0044`-`IL_0055`);
/// * `ToastyMittens/<AfterPlayerTurnStart>d__6::MoveNext` (RVA `0x33285c`) has
///   no turn gate — only `player == Owner` (`IL_0033`-`IL_0042`) — and pauses
///   on `CardSelectCmd::FromHand` (`IL_0068`), then `CardCmd::Exhaust`
///   (`IL_00f2`) and `Apply<StrengthPower>` (`IL_01af`).
///
/// Popper first means the enemies are already hit (or dead, ending the fight)
/// when the Hand choice is offered and when the side-start tail runs at that
/// pause (#3050); Toasty first means the reverse. So the fixed suffix, which
/// runs Popper after Toasty, is exact only for the Toasty-first inventory, and
/// this predicate moves Popper to just before Toasty for the other one. Every
/// other same-hook peer Popper does not commute with (Choices Paradox,
/// Vexing Puzzlebox, Bone Tea, Mr Struggles, Mercury Hourglass, Royal Poison)
/// is refused beside Popper by the opening
/// (`entry::opening::after_player_turn_start_peers_are_ordered`) and, where
/// the engine can see them, by admission, so moving Popper past Bellows and
/// Bone Tea reorders it only against bodies it is never co-owned with here.
/// [`after_player_turn_start_dispatch`] carries the same move for the pause
/// order check.
pub(crate) fn festive_popper_precedes_toasty_mittens(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    if !hooks.owns(RelicId::RelicFestivePopper)
        || !hooks.owns(RelicId::RelicToastyMittens)
        || !hooks.dispatch_ordered()
        // Popper already ran, ahead of the earlier Gambling Chip choice.
        || festive_popper_precedes_gambling_chip(catalog)
    {
        return false;
    }
    let relics = hooks.relics();
    let last_popper = relics
        .iter()
        .rposition(|relic| *relic == RelicId::RelicFestivePopper);
    let first_toasty = relics
        .iter()
        .position(|relic| *relic == RelicId::RelicToastyMittens);
    last_popper
        .zip(first_toasty)
        .is_some_and(|(popper, toasty)| popper < toasty)
}

/// Whether Festive Popper's turn-one damage runs **ahead of** Gambling Chip's
/// Hand choice: both owned, the inventory vouched for as dispatch order, and
/// every Festive Popper recorded before every Gambling Chip.
///
/// The listener walk is [`festive_popper_precedes_toasty_mittens`]'s
/// (`Hook/<AfterPlayerTurnStart>d__56::MoveNext`, v0.111.0 RVA `0x3cff0c`,
/// `Player.Relics` order, no ending check between listeners). The two bodies
/// do not commute:
///
/// * `FestivePopper/<AfterPlayerTurnStart>d__4::MoveNext` (RVA `0x324b94`)
///   gates `player == Owner` (`IL_0027`-`IL_002c`) and `TurnNumber == 1`
///   (`IL_0045`-`IL_0055`), then awaits `CreatureCmd::Damage` over
///   `HittableEnemies` (`IL_0079`-`IL_0094`);
/// * `GamblingChip/<AfterPlayerTurnStart>d__2::MoveNext` (RVA `0x325788`)
///   gates `player == Owner` (`IL_002e`-`IL_0033`) and `TurnNumber <= 1`
///   (`IL_003b`-`IL_004b`), then pauses on
///   `CardSelectCmd::FromHandForDiscard` (`IL_0071`) and awaits
///   `CardCmd::DiscardAndDraw` (`IL_00f0`).
///
/// Popper first means the enemies are already hit (or dead, ending the fight)
/// when the discard choice is offered and when the side-start tail runs at
/// that pause (#3050); Chip first means the reverse. The fixed suffix runs
/// Popper after Chip, so it is exact only for the Chip-first inventory, and
/// this predicate moves Popper to just before Chip for the other one. The
/// bodies between the two places (Vexing Puzzlebox, Toasty Mittens, Bellows,
/// Bone Tea) are either refused beside Popper by the opening
/// (`entry::opening::after_player_turn_start_peers_are_ordered`) or, for
/// Toasty Mittens, held to the recorded order at the pause
/// ([`after_player_turn_start_pause_order_is_native`]).
pub(crate) fn festive_popper_precedes_gambling_chip(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    if !hooks.owns(RelicId::RelicFestivePopper)
        || !hooks.owns(RelicId::RelicGamblingChip)
        || !hooks.dispatch_ordered()
    {
        return false;
    }
    let relics = hooks.relics();
    let last_popper = relics
        .iter()
        .rposition(|relic| *relic == RelicId::RelicFestivePopper);
    let first_chip = relics
        .iter()
        .position(|relic| *relic == RelicId::RelicGamblingChip);
    last_popper
        .zip(first_chip)
        .is_some_and(|(popper, chip)| popper < chip)
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

fn continue_after_gambling_chip(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !vexing_toasty_order_is_exact(catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Toasty Mittens + Vexing Puzzlebox recorded order",
        ));
    }
    // A Puzzlebox recorded before Choices Paradox already ran ahead of the
    // Paradox grid (#3321, `after_player_turn_start`).
    if !vexing_puzzlebox_precedes_choices_paradox(catalog) {
        vexing_puzzlebox_after_player_turn_start(catalog, state, events)?;
    }
    // A Festive Popper recorded before Toasty Mittens deals its damage ahead of
    // the Hand choice, and `continue_after_toasty_mittens` then skips it (#2884).
    if festive_popper_precedes_toasty_mittens(catalog) {
        festive_popper_after_player_turn_start(catalog, state, events)?;
    }
    if !state.history.over && catalog.hooks().owns(RelicId::RelicToastyMittens) {
        // `ToastyMittens/<AfterPlayerTurnStart>d__6` (RVA `0x33285c`) has no
        // Hand gate before `CardSelectCmd::FromHand` (IL_0068), which pauses
        // before it reads the Hand (#3050): the side-start tail runs at that
        // pause and can still hand it a card. It runs here, ahead of the Hand
        // read, whatever the Hand holds, because the candidate count decides
        // whether native prompts at all (#3273, [`toasty_mittens_select`]).
        super::turn::run_side_start_ahead_of_setup_remainder(
            state,
            catalog,
            RelicId::RelicToastyMittens,
            events,
        )?;
        if !toasty_mittens_select(catalog, state, events)? {
            return Ok(());
        }
    }
    continue_after_toasty_mittens(catalog, state, events)
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

fn continue_after_choices_paradox(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // A Festive Popper recorded before Gambling Chip deals its damage ahead of
    // the discard choice, and the later places then skip it.
    if festive_popper_precedes_gambling_chip(catalog) {
        festive_popper_after_player_turn_start(catalog, state, events)?;
    }
    if state.turn <= 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicGamblingChip) {
        if state.piles.get(PileId::Hand).is_empty() {
            // `GamblingChip/<AfterPlayerTurnStart>d__2` (RVA `0x325788`)
            // gates only owner and `TurnNumber <= 1` (IL_0045-004b) before
            // `CardSelectCmd::FromHandForDiscard` (IL_0071), whose `FromHand`
            // pauses before it reads the Hand (#3050): a side-start tail that
            // runs at that pause can still hand it a card.
            super::turn::run_side_start_ahead_of_setup_remainder(
                state,
                catalog,
                RelicId::RelicGamblingChip,
                events,
            )?;
        }
        if !state.piles.get(PileId::Hand).is_empty() {
            begin_hand_relic_selection(state, RelicPendingKind::GamblingChip)?;
            crate::coverage::record_relic(RelicId::RelicGamblingChip);
            return Ok(());
        }
    }
    continue_after_gambling_chip(catalog, state, events)
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
                continue_after_choices_paradox(catalog, state, events)?;
            } else {
                super::turn::resume_player_turn_start_after_toolbox(state, catalog, events)?;
            }
        }
        RelicPendingKind::GamblingChip => {
            let selected = gambling_selection(&pending.entries, ordinal).ok_or(
                EngineRefusal::MalformedArgs("Gambling Chip selection ordinal"),
            )?;
            if selected.is_empty() {
                continue_after_gambling_chip(catalog, state, events)?;
            } else {
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
                continue_after_gambling_chip(catalog, state, events)?;
            }
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
            continue_after_toasty_mittens(catalog, state, events)?;
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

/// Rust's `AfterPlayerTurnStart` relic dispatch: the hand-authored sequence
/// ([`after_player_turn_start`] through [`continue_after_toasty_mittens`]),
/// then the one template subscriber, Mercury Hourglass, in
/// `fire_hook(AfterPlayerTurnStart)`.
///
/// A v0.111.0 method-table scan of `sts2.dll` (DLL 9cb4f1ad) finds exactly
/// these eleven relic types overriding `AbstractModel::AfterPlayerTurnStart`
/// (base RVA `0x7a1f2`): Bellows `0x906dc`, BoneTea `0x90ffc`, ChoicesParadox
/// `0x92214`, EmotionChip `0x9306c`, FestivePopper `0x93918`, GamblingChip
/// `0x94424`, MercuryHourglass `0x96c60`, MrStruggles `0x970e8`, RoyalPoison
/// `0x9a8dc`, ToastyMittens `0x9c984`, VexingPuzzlebox `0x9db70`. No type
/// overrides `AfterPlayerTurnStartEarly` (base `0x7a1eb`), and only Blood Vial
/// and Fake Blood Vial override `AfterPlayerTurnStartLate`, whose pass
/// (`Hook/<AfterPlayerTurnStart>d__56` IL_027c) follows the whole ordinary
/// pass and so always lies in a paused remainder.
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

/// The order this engine runs [`AFTER_PLAYER_TURN_START_RELIC_DISPATCH`] in
/// for `catalog`: the fixed sequence, except that Festive Popper runs just
/// before Gambling Chip when the inventory records it first
/// ([`festive_popper_precedes_gambling_chip`]) or else just before Toasty
/// Mittens when the inventory records it first
/// ([`festive_popper_precedes_toasty_mittens`], #2884), Vexing Puzzlebox runs
/// just before Choices Paradox when recorded first
/// ([`vexing_puzzlebox_precedes_choices_paradox`], #3321), and a leading
/// Mercury Hourglass runs first ([`mercury_hourglass_leads`], #3321).
fn after_player_turn_start_dispatch(catalog: &Catalog) -> [RelicId; 11] {
    let mut order = AFTER_PLAYER_TURN_START_RELIC_DISPATCH;
    if festive_popper_precedes_gambling_chip(catalog) {
        let chip = dispatch_position(&order, RelicId::RelicGamblingChip);
        let popper = dispatch_position(&order, RelicId::RelicFestivePopper);
        order[chip..=popper].rotate_right(1);
    } else if festive_popper_precedes_toasty_mittens(catalog) {
        let toasty = dispatch_position(&order, RelicId::RelicToastyMittens);
        let popper = dispatch_position(&order, RelicId::RelicFestivePopper);
        order[toasty..=popper].rotate_right(1);
    }
    if vexing_puzzlebox_precedes_choices_paradox(catalog) {
        let paradox = dispatch_position(&order, RelicId::RelicChoicesParadox);
        let puzzlebox = dispatch_position(&order, RelicId::RelicVexingPuzzlebox);
        order[paradox..=puzzlebox].rotate_right(1);
    }
    if mercury_hourglass_leads(catalog) {
        let hourglass = dispatch_position(&order, RelicId::RelicMercuryHourglass);
        order[..=hourglass].rotate_right(1);
    }
    order
}

fn dispatch_position(order: &[RelicId; 11], relic: RelicId) -> usize {
    order
        .iter()
        .position(|candidate| *candidate == relic)
        .expect("every moved relic is in the dispatch table")
}

/// Whether every `first` precedes every `second` in the inventory vouched for
/// as dispatch order (both owned).
fn inventory_precedes(catalog: &Catalog, first: RelicId, second: RelicId) -> bool {
    let hooks = catalog.hooks();
    if !hooks.dispatch_ordered() {
        return false;
    }
    let relics = hooks.relics();
    relics
        .iter()
        .rposition(|relic| *relic == first)
        .zip(relics.iter().position(|relic| *relic == second))
        .is_some_and(|(last_first, first_second)| last_first < first_second)
}

/// Whether Mercury Hourglass runs **first** in this engine's
/// `AfterPlayerTurnStart` relic pass (#3321), rather than last as the one
/// template subscriber of [`super::fire_hook`].
///
/// Native awaits the relic listeners in `Player.Relics` order
/// (`Hook/<AfterPlayerTurnStart>d__56::MoveNext` RVA `0x3cff0c`,
/// `IterateCombatHookListeners` at `IL_012d`, each listener awaited at
/// `IL_017a`, no ending check between listeners).
/// `MercuryHourglass/<AfterPlayerTurnStart>d__4::MoveNext` (v0.111.0 RVA
/// `0x32a854`) gates only `player == Owner` (`IL_001d`-`IL_002b`), then
/// awaits one `CreatureCmd::Damage(HittableEnemies, DynamicVars.Damage,
/// Owner.Creature)` at `IL_0036`-`IL_0067`: no turn gate and no RNG draw. So
/// the fixed tail position is native exactly when Hourglass is recorded after
/// every same-hook peer that acts, and the front is native when it is recorded
/// **before every other owned** member of
/// [`AFTER_PLAYER_TURN_START_RELIC_DISPATCH`] in a vouched inventory: then no
/// peer, acting or not, runs ahead of it. That is this predicate. It is false
/// when Hourglass is the pass's only owned member, where front and tail are
/// the same place. A mixed position still refuses where it is observable (the
/// pause check [`after_player_turn_start_pause_order_is_native`], and
/// admission's Vexing Puzzlebox list through
/// [`mercury_hourglass_vexing_order_is_exact`]).
pub(crate) fn mercury_hourglass_leads(catalog: &Catalog) -> bool {
    if !catalog.hooks().owns(RelicId::RelicMercuryHourglass) {
        return false;
    }
    mercury_hourglass_leads_rare(catalog)
}

#[cold]
#[inline(never)]
fn mercury_hourglass_leads_rare(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    let mut peers = AFTER_PLAYER_TURN_START_RELIC_DISPATCH
        .into_iter()
        .filter(|relic| *relic != RelicId::RelicMercuryHourglass && hooks.owns(*relic))
        .peekable();
    peers.peek().is_some()
        && peers.all(|peer| inventory_precedes(catalog, RelicId::RelicMercuryHourglass, peer))
}

/// Whether this engine runs Mercury Hourglass and Vexing Puzzlebox in their
/// recorded order (#3321): Hourglass ahead of Puzzlebox exactly when it leads
/// the pass ([`mercury_hourglass_leads`]), and otherwise behind it, which is
/// native only when every Puzzlebox is recorded before every Hourglass.
/// Admission read the pair as unordered until this predicate.
pub(crate) fn mercury_hourglass_vexing_order_is_exact(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    if !hooks.owns(RelicId::RelicMercuryHourglass) || !hooks.owns(RelicId::RelicVexingPuzzlebox) {
        return true;
    }
    mercury_hourglass_leads(catalog)
        || inventory_precedes(
            catalog,
            RelicId::RelicVexingPuzzlebox,
            RelicId::RelicMercuryHourglass,
        )
}

/// Whether Vexing Puzzlebox's turn-one card runs **ahead of** Choices
/// Paradox's grid (#3321): both owned, the inventory vouched for as dispatch
/// order, every Puzzlebox recorded before every Paradox, and no Gambling Chip,
/// whose Hand choice sits between them in the fixed sequence and would be
/// passed over by the move.
///
/// The two bodies do not commute. Both draw `RunRngSet.CombatCardGeneration`
/// through `CardFactory::GetDistinctForCombat`:
/// `VexingPuzzlebox/<AfterPlayerTurnStart>d__2::MoveNext` (v0.111.0 RVA
/// `0x333de0`) at `IL_0087`-`IL_009d`, then `AddGeneratedCardToCombat` into
/// the Hand at `IL_00b7`; `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext`
/// (RVA `0x321b0c`) at `IL_009e`-`IL_00b3`, then pauses on
/// `CardSelectCmd::FromSimpleGrid` at `IL_0144`, where the side-start tail runs
/// (#3050). Each gates only owner and `TurnNumber == 1` (Puzzlebox
/// `IL_0020`-`IL_0044`, Paradox `IL_0027`-`IL_004b`). Native awaits them in
/// `Player.Relics` order (`Hook/<AfterPlayerTurnStart>d__56::MoveNext` RVA
/// `0x3cff0c`, `IL_012d`/`IL_017a`), so a Puzzlebox recorded first draws the
/// stream first and its card is in the Hand before the pause.
pub(crate) fn vexing_puzzlebox_precedes_choices_paradox(catalog: &Catalog) -> bool {
    let hooks = catalog.hooks();
    hooks.owns(RelicId::RelicVexingPuzzlebox)
        && hooks.owns(RelicId::RelicChoicesParadox)
        && !hooks.owns(RelicId::RelicGamblingChip)
        && inventory_precedes(
            catalog,
            RelicId::RelicVexingPuzzlebox,
            RelicId::RelicChoicesParadox,
        )
}

/// The `AfterPlayerTurnStart` template fire point once a leading Mercury
/// Hourglass already ran ([`mercury_hourglass_lead`]): it records the fire
/// point, and any other compiled subscriber refuses by name rather than run
/// out of place (none exists at v0.111.0: Hourglass is this hook's only
/// template relic).
pub(crate) fn after_player_turn_start_template_pass_after_leading_hourglass(
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
            "AfterPlayerTurnStart template subscriber behind a leading Mercury Hourglass",
        ));
    }
    Ok(())
}

/// Mercury Hourglass's template body, run at the front of the pass when it
/// leads ([`mercury_hourglass_leads`]); `turn::resume_player_turn_start_after_relics`
/// then skips the template fire point so it runs once. The compiled rule is
/// the same one [`super::fire_hook`] interprets.
///
/// Native has no ending check between listeners (`IL_017a` awaits the next
/// listener unconditionally), so a peer still acts after Hourglass kills the
/// last enemy, where this engine's bodies stop on `history.over`. That shape
/// is not modeled: it refuses by name when a peer would act this turn.
fn mercury_hourglass_lead(
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
    )?;
    if state.history.over
        && AFTER_PLAYER_TURN_START_RELIC_DISPATCH
            .into_iter()
            .any(|relic| {
                relic != RelicId::RelicMercuryHourglass
                    && after_player_turn_start_relic_acts(catalog, state, relic)
            })
    {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "AfterPlayerTurnStart relic peer after a leading Mercury Hourglass ended combat",
        ));
    }
    Ok(())
}

/// Whether an `AfterPlayerTurnStart` relic body acts this turn, by its own
/// entry gates (the bodies below and Mercury Hourglass's `is_owner` template).
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
/// so exactly the relics before the paused one ran ahead of the tail. Rust
/// ran the prefix of [`after_player_turn_start_dispatch`] instead. The two
/// sets must agree for every relic that acts this turn, and an unrecorded
/// acquisition order cannot show that they do: both refuse by name.
pub(crate) fn after_player_turn_start_pause_order_is_native(
    catalog: &Catalog,
    state: &HotState,
    paused: RelicId,
) -> Result<(), EngineRefusal> {
    const REFUSAL: EngineRefusal = EngineRefusal::PowerOrderNotModeled(
        "AfterPlayerTurnStart relic order around a paused SetupPlayerTurn choice",
    );
    let hooks = catalog.hooks();
    let dispatch = after_player_turn_start_dispatch(catalog);
    let dispatch_position = |relic| dispatch.iter().position(|candidate| *candidate == relic);
    let inventory_position = |relic| hooks.relics().iter().position(|owned| *owned == relic);
    let paused_dispatch = dispatch_position(paused).ok_or(REFUSAL)?;
    for (position, relic) in dispatch.into_iter().enumerate() {
        if position == paused_dispatch || !after_player_turn_start_relic_acts(catalog, state, relic)
        {
            continue;
        }
        if !hooks.dispatch_ordered() {
            return Err(REFUSAL);
        }
        let native_before = inventory_position(relic) < inventory_position(paused);
        if native_before != (position < paused_dispatch) {
            return Err(REFUSAL);
        }
    }
    Ok(())
}

/// Hand-authored owner `AfterPlayerTurnStart` relics in Python's fixed
/// post-power sequence. Same-hook noncommuting ownership pairs are rejected
/// by admission because acquisition order is absent from the run payload,
/// except where a vouched inventory moves a body to its recorded place: a
/// leading Mercury Hourglass ([`mercury_hourglass_leads`]) and a Vexing
/// Puzzlebox recorded before Choices Paradox
/// ([`vexing_puzzlebox_precedes_choices_paradox`]) run here, ahead of the
/// Paradox grid and in that order (#3321), and Festive Popper runs before
/// Toasty Mittens (#2884).
pub(crate) fn after_player_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if mercury_hourglass_leads(catalog) {
        mercury_hourglass_lead(catalog, state, events)?;
    }
    if vexing_puzzlebox_precedes_choices_paradox(catalog) {
        vexing_puzzlebox_after_player_turn_start(catalog, state, events)?;
    }
    if state.turn == 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicChoicesParadox)
    {
        begin_generation_relic_selection(state, catalog, RelicPendingKind::ChoicesParadox, 5)?;
        crate::coverage::record_relic(RelicId::RelicChoicesParadox);
        return Ok(());
    }
    continue_after_choices_paradox(catalog, state, events)
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

/// The hand-authored energy relics that run after owner power listeners and
/// before the generated template tail of `AfterSideTurnStart`.
pub(crate) fn after_side_turn_start(
    catalog: &Catalog,
    state: &mut HotState,
    hook_started: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !hook_started {
        return Ok(());
    }
    // `RELIC.LANTERN` — one Energy on the owner's first side turn, and the
    // oracle's **first** relic listener on this hook (frozen Python, deleted #2827; immediately after `_run_after_side_turn_start_power_order`).
    //
    // `Lantern/<AfterSideTurnStart>d__6::MoveNext` RVA `0x328268`:
    // `Contains<Creature>(participants, Owner.Creature)` at `IL_0020`-`IL_0038`,
    // the turn guard `get_TurnNumber; ldc.i4.1; ble.s` at `IL_0043`-`IL_004e`
    // whose fall-through `leave`s at `IL_0050`, `RelicModel::Flash` at
    // `IL_0056` (presentation), and `PlayerCmd::GainEnergy` at `IL_0071` with
    // `DynamicVars.Energy.BaseValue` (`IL_0061`-`IL_0066`) and `Owner`
    // (`IL_006c`). `Lantern::get_CanonicalVars` (`0x95adf`) is
    // `ldc.i4.1; newobj EnergyVar::.ctor`, so the amount is 1 —
    // `combat_sim.LANTERN_ENERGY` (frozen Python `_relic_templates`, deleted #2827).
    //
    // Additive, never exclusive: it is one row beside Booming Conch,
    // Candelabra / Chandelier, the flowers, Venerable Tea Set's
    // `AfterEnergyReset` grant and Very Hot Cocoa's compiled template rule,
    // because `PlayerCmd/<GainEnergy>d__3::MoveNext` (`0x3ee8a0`) is a
    // per-listener command and `combat_sim._gain_owner_energy` (frozen Python, deleted #2827) is
    // one statement per relic. The `NoEnergyGainPower::ModifyEnergyGain`
    // (`0xa4f9d`) suppression and the ending gate are that command's, so they
    // are spelled here exactly as the peers above spell them.
    if state.turn <= 1 && catalog.hooks().owns(RelicId::RelicLantern) && !state.history.over {
        if !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Lantern energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicLantern);
    }
    if catalog.hooks().owns(RelicId::RelicPaelsEye) {
        state
            .fanouts
            .set_paels_eye_was_owner_part_last_player_turn(true);
        crate::coverage::record_relic(RelicId::RelicPaelsEye);
    }
    if state.turn <= 1 && state.booming_conch_elite() && !state.history.over {
        if !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Booming Conch energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicBoomingConch);
    }
    if !state.history.over {
        let gated = match state.turn {
            2 if catalog.hooks().owns(RelicId::RelicCandelabra) => {
                Some((RelicId::RelicCandelabra, 2_i16))
            }
            3 if catalog.hooks().owns(RelicId::RelicChandelier) => {
                Some((RelicId::RelicChandelier, 3_i16))
            }
            _ => None,
        };
        if let Some((relic, amount)) = gated {
            state.energy = state
                .energy
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("turn-gated relic energy"))?;
            crate::coverage::record_relic(relic);
        }
    }
    for (relic, current, period, amount) in [
        (RelicId::RelicHappyFlower, state.flower(), 3_i16, 1_i16),
        (
            RelicId::RelicFakeHappyFlower,
            state.fake_flower(),
            5_i16,
            1_i16,
        ),
    ] {
        if !catalog.hooks().owns(relic) {
            continue;
        }
        let next = (current + 1) % period;
        let written = match relic {
            RelicId::RelicHappyFlower => state.set_flower(next),
            RelicId::RelicFakeHappyFlower => state.set_fake_flower(next),
            _ => unreachable!(),
        };
        debug_assert!(written);
        if next == 0 && !state.history.over && !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(amount)
                .ok_or(EngineRefusal::CounterOverflow("flower energy"))?;
        }
        crate::coverage::record_relic(relic);
    }
    if state.turn <= 1 && catalog.hooks().owns(RelicId::RelicBigHat) {
        // The fully-unlocked solo-Ironclad Ethereal generation pool is empty,
        // so native returns before consuming Generation RNG
        // (`BigHat/<AfterSideTurnStart>d__6::MoveNext` `0x31f64c`
        // IL_00af-IL_00b8). Another owner's Ethereal pool is not, and this
        // body draws nothing, so it refuses rather than open that fight
        // without its two cards (#3264: the owner-pool draw is read from IL,
        // but its oracle digest is not yet reproduced).
        if state.reward_card_pool != Some(crate::catalog::RewardPool::Ironclad) {
            return Err(EngineRefusal::MalformedArgs(
                "Big Hat owner Ethereal pool beyond Ironclad (#3264)",
            ));
        }
        crate::coverage::record_relic(RelicId::RelicBigHat);
    }
    if catalog.hooks().owns(RelicId::RelicCrossbow) {
        // A vouched inventory that records Brimstone ahead of Crossbow runs
        // Brimstone here, at its native place (#3381); see
        // [`brimstone_leads_crossbow`].
        if !state.history.over && brimstone_leads_crossbow(catalog) {
            brimstone_after_side_turn_start(state, events)?;
        }
        if !state.history.over {
            crossbow_after_side_turn_start(catalog, state, events)?;
        }
    }
    if state.turn <= 1 && !state.history.over && catalog.hooks().owns(RelicId::RelicOrangeDough) {
        let pool = colorless_generation_relic_pool(
            catalog,
            state,
            "Orange Dough Colorless pool provenance",
        )?;
        let shuffled = super::cards::shuffle_generation_slice(state, &pool)?;
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
    }
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

/// Whether Brimstone runs just ahead of Crossbow in this engine's
/// `AfterSideTurnStart` relic pass, rather than at its fixed place in
/// [`after_side_turn_start_late`] (#3381).
///
/// Native awaits every `AfterSideTurnStart` listener in the order
/// `Hook::IterateCombatHookListeners` yields it
/// (`Hook/<AfterSideTurnStart>d__75::MoveNext` RVA `0x3d15ac`: the iterator at
/// IL_0024, each `AfterSideTurnStart` awaited at IL_0065-IL_0092), and relics
/// come in `Player.Relics` order. Beside Crossbow, admission lets only
/// Lantern, Candelabra, Happy Flower and Pael's Legion share the hook with
/// Brimstone (every other relic override of `AfterSideTurnStart` in the
/// assembly is on Crossbow's refused peer list in `engine::admission`,
/// including the four compiled templates `fire_hook` would run between the
/// two passes), and those four commute with both bodies. So the one order
/// that is observable is Crossbow's against Brimstone's, through a live
/// Arsenal: Crossbow's generated Attack raises `AfterCardGeneratedForCombat`,
/// whose `ArsenalPower` listener applies Strength
/// (`<AfterCardGeneratedForCombat>d__6` `0x334ec8` IL_0061) against
/// Brimstone's own Strength application. The fixed place is Crossbow first,
/// which is native when the inventory records Crossbow first. A vouched
/// inventory recording every Brimstone before every Crossbow makes this true,
/// and Brimstone then runs where native runs it. An unvouched or interleaved
/// inventory keeps the fixed order, and admission refuses it beside a
/// reachable Arsenal.
pub(crate) fn brimstone_leads_crossbow(catalog: &Catalog) -> bool {
    catalog.hooks().owns(RelicId::RelicCrossbow)
        && relic_recorded_before(catalog, RelicId::RelicBrimstone, RelicId::RelicCrossbow)
}

/// Hand-authored relic listeners that Python places after the generated
/// `AfterSideTurnStart` template group. The immutable inventory order is not
/// projected; admission rejects the combinations whose results do not
/// commute.
pub(crate) fn after_side_turn_start_late(
    catalog: &Catalog,
    state: &mut HotState,
    hook_started: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !hook_started {
        return Ok(());
    }
    // `PhylacteryUnbound/<AfterSideTurnStart>d__11::MoveNext` (v0.111.0 RVA
    // `0x32e134`): owner-participates test at `IL_002e`, then
    // `OstyCmd::Summon(Owner, DynamicVars["StartOfTurn"] = 2)` at `IL_005b`,
    // with no turn test, so turn one summons too (the oracle's
    // frozen Python `_turn_start_after_royal`, deleted #2827). The opening reaches it through
    // `engine::deal_opening_hand` (#2827). The body reads no combat state
    // between the owner test and `Summon` (IL_003a-IL_005b), and
    // `<Summon>d__0` (`0x3ee040`) gates only its SFX on `IsInProgress`
    // (IL_00c6-IL_00d0), so a summon after an earlier turn-start peer ended
    // the combat runs under IsEnding: `summon_osty`'s projection (#3279).
    if catalog.hooks().owns(RelicId::RelicPhylacteryUnbound) {
        super::summon_osty(state, 2, "Phylactery Unbound Osty")?;
        crate::coverage::record_relic(RelicId::RelicPhylacteryUnbound);
    }
    if catalog.hooks().owns(RelicId::RelicPaelsLegion) {
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
    if catalog.hooks().owns(RelicId::RelicBread) && state.turn == 1 && !state.history.over {
        state.energy = state.energy.saturating_sub(2).max(0);
        crate::coverage::record_relic(RelicId::RelicBread);
    }
    if catalog.hooks().owns(RelicId::RelicPaelsTears)
        && state.paels_tears_had_leftover_energy()
        && !state.history.over
    {
        if !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(2)
                .ok_or(EngineRefusal::CounterOverflow("Pael's Tears energy"))?;
        }
        crate::coverage::record_relic(RelicId::RelicPaelsTears);
    }
    fencing_manual_after_side_turn_start(catalog, state, events)?;
    symbiotic_virus_after_side_turn_start(catalog, state, events)?;
    if catalog.hooks().owns(RelicId::RelicBrimstone)
        && !state.history.over
        && !brimstone_leads_crossbow(catalog)
    {
        brimstone_after_side_turn_start(state, events)?;
    }
    if catalog.hooks().owns(RelicId::RelicSealOfGold) && state.gold >= 3 {
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
/// order (`CombatState::IterateHookListeners` `0x137409`), which the run
/// payload does not vouch for. This sits where the frozen oracle ran it
/// (`begin_player_turn`, between Pael's Tears and Symbiotic Virus, frozen
/// Python `_turn_start_after_royal`, deleted #2827). The order is unobservable
/// against every represented same-hook peer except the other turn-one Hand
/// writers — Crossbow, Big Hat and Orange Dough, whose generated cards and
/// uids would interleave differently with the Blade — and Infused Core, whose
/// Lightning channel the oracle refused beside it; `engine::admission`
/// (`Fencing Manual + Infused Core turn-start order`, and the Crossbow and
/// Big Hat same-hook lists) and `entry::opening`'s
/// `fencing_manual_turn_start_peers_are_ordered` refuse those, except Orange
/// Dough recorded (vouched) before Fencing Manual, where this fixed order —
/// Dough in the early group, the Forge in the late one — is the native one.
/// A loaded document carries no inventory order, so `engine::admission`
/// refuses Fencing Manual + Orange Dough whenever the Forge can still run
/// (turn one, no Sovereign Blade in any pile: `Fencing Manual + Orange Dough
/// turn-one Forge order`).
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
/// does this call in [`after_side_turn_start_late`]. The oracle's listeners in
/// between are Fencing Manual, which runs immediately before this call
/// ([`fencing_manual_after_side_turn_start`], #3090; its Forge writes no orb),
/// and Letter Opener's turn>1 reset, Runic Capacitor and Infused Core, which
/// are either inert on turn one or still gated in the opening.
///
/// The same-hook peers whose order around this channel is observable are the
/// oracle's `start_combat` refusals (frozen Python, deleted #2827: Infused Core; Runic
/// Capacitor at zero base slots; Cracked Core at zero base slots with Fencing
/// Manual or Brimstone). `engine::admission` refuses each by name for a loaded
/// document, beside the Crossbow and Big Hat same-hook refusals that also list
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
///   those counts see, so it is held to the same table.
pub(crate) fn counter_precedes_out_of_walk_peer_commutes(counter: RelicId, peer: RelicId) -> bool {
    match peer {
        RelicId::RelicPocketwatch | RelicId::RelicRippleBasin => true,
        RelicId::RelicPenNib => counter != RelicId::RelicIronClub,
        _ => false,
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
            super::draw::draw_cards(state, catalog, 1, super::draw::DrawSource::Command, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicIronClub);
    }
    Ok(())
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
fn mummified_hand_after_card_played(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    let hand = state.piles.get(PileId::Hand).as_slice();
    let candidates_for = |require_base: bool, require_current: bool| {
        hand.iter()
            .copied()
            .filter(|card| {
                let Some(candidate) = catalog.spec(card.atom) else {
                    return false;
                };
                let base_positive = candidate.cost > 0 || candidate.row.star_cost > 0;
                let current_positive =
                    super::play::resolved_local_energy_cost(state, *card, candidate) > 0
                        || super::play::resolved_star_cost(state, *card, candidate)
                            .is_some_and(|cost| cost > 0);
                (!require_base || base_positive) && (!require_current || current_positive)
            })
            .collect::<Vec<_>>()
    };
    let candidates = [(true, true), (false, true), (true, false), (false, false)]
        .into_iter()
        .map(|(base, current)| candidates_for(base, current))
        .find(|candidates| !candidates.is_empty())
        .unwrap_or_default();
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
        mummified_hand_after_card_played(catalog, state)?;
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
        super::draw::draw_cards(state, catalog, 1, super::draw::DrawSource::Command, events)?;
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
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();

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
        let pollinous = before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
        let bonus = modifier_total(&catalog, HookEvent::ModifyHandDraw, &state).unwrap()
            + pollinous_hand_draw_bonus(pollinous);

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
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
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
        before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(state.piles.get(PileId::Draw).len(), 4);
        radiant_pearl_before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
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
        after_player_turn_start(&vexing_catalog, &mut vexing, &mut Vec::new()).unwrap();
        assert_eq!(vexing.piles.get(PileId::Hand).len(), 1);

        let royal_catalog = catalog(&[RelicId::RelicRoyalPoison]);
        let mut royal = HotState::at_defaults();
        royal.hp = 80;
        royal.max_hp = 80;
        royal.turn = 1;
        after_player_turn_start(&royal_catalog, &mut royal, &mut Vec::new()).unwrap();
        assert_eq!(royal.hp, 76);
    }

    /// #3381: Brimstone runs at its recorded side of Crossbow.
    ///
    /// With a live Arsenal (1) and Ruined Helmet, whichever Strength
    /// application comes first is doubled. Crossbow first: Arsenal's 1 is
    /// doubled to 2, then Brimstone adds 2, for 4. Brimstone first: its 2 is
    /// doubled to 4, then Arsenal adds 1, for 5. A vouched Brimstone-first
    /// inventory takes the second order. Crossbow-first, and an unvouched
    /// inventory in either order, keep the fixed Crossbow-first order, which
    /// admission refuses for the unvouched case. Each enemy gets its 1 Strength
    /// exactly once in every order.
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
            after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
            assert_eq!(state.piles.get(PileId::Hand).len(), 1, "one Crossbow card");
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Strength),
                1,
                "Brimstone's enemy Strength, once"
            );
            (
                brimstone_leads_crossbow(&catalog),
                state.powers.value(PowerId::Strength),
            )
        };
        use RelicId::{RelicBrimstone as B, RelicCrossbow as C, RelicRuinedHelmet as H};
        assert_eq!(run([B, C, H], true), (true, 5));
        assert_eq!(run([C, B, H], true), (false, 4));
        assert_eq!(run([B, C, H], false), (false, 4));
        assert_eq!(run([C, B, H], false), (false, 4));
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
        // Ending combat skips the body before any draw.
        let mut over = live.clone();
        over.history.over = true;
        let before = over.clone();
        after_side_turn_start(&crossbow, &mut over, true, &mut Vec::new()).unwrap();
        assert_eq!(over, before);

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
        before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
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
        after_side_turn_start_late(&catalog, &mut pet, true, &mut Vec::new()).unwrap();
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

        after_player_turn_start(&catalog, &mut state, &mut Vec::new()).unwrap();
        assert_eq!(relic_selection_action_count(&state, &catalog), Ok(5));
        resume_relic_selection(&mut state, &catalog, 0, &mut Vec::new()).unwrap();

        assert!(state.pending.is_none());
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        let selected = state.piles.get(PileId::Hand).as_slice()[0];
        assert!(state.card_states.get(selected.uid).local_retain);
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
                true,
            ),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.set_relics_ordered(&order, recorded).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            let before = state.clone();
            assert!(!vexing_toasty_order_is_exact(&catalog));
            assert!(continue_after_gambling_chip(&catalog, &mut state, &mut Vec::new()).is_err());
            assert_eq!(state, before);
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
    /// ending refuses by name. Before #3279 every arm healed.
    #[test]
    fn phylactery_unbound_turn_start_summon_reads_the_ending_projection() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[RelicId::RelicPhylacteryUnbound])
            .unwrap();
        let catalog = builder.build();
        for (live_enemy, osty, expected) in [
            (true, Some((3, 5)), Ok(Some((5, 7)))),
            (false, Some((3, 5)), Ok(Some((3, 7)))),
            (
                false,
                None,
                Err(EngineRefusal::EndingSummonNotModeled(
                    "Phylactery Unbound Osty",
                )),
            ),
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
            let result = after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new())
                .map(|()| {
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
                before_hand_draw(&catalog, &mut state, &mut Vec::new()),
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
        before_hand_draw(&catalog, &mut state, &mut Vec::new()).unwrap();
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
            before_hand_draw(&catalog, &mut state, &mut Vec::new()),
            refused
        );
        let (catalog, mut state) = setup(Some(RewardPool::Silent), Some(full), 1);
        assert_eq!(
            before_hand_draw(&catalog, &mut state, &mut Vec::new()),
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
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert!(state.orbs.as_slice().is_empty(), "turn 2 channels nothing");

        let mut state = live();
        state.turn = 1;
        state.history.over = true;
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert!(
            state.orbs.as_slice().is_empty(),
            "an over combat channels nothing"
        );

        let mut state = live();
        state.turn = 1;
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
        assert_eq!(state.orbs.as_slice().len(), SYMBIOTIC_VIRUS_CHANNELS);
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
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
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
        after_side_turn_start_late(&catalog, &mut state, true, &mut Vec::new()).unwrap();
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
            after_side_turn_start_late(catalog, state, started, &mut Vec::new()).unwrap();
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
    /// Selection draw and no pile change. Past turn one, after the combat
    /// ended, or unowned, the body is inert.
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
        let (catalog, _, mut state) = jeweled_mask_fixture(true, &[(CardId::Inflame, 0)]);
        state.history.over = true;
        untouched(&catalog, state);
        let (catalog, _, state) = jeweled_mask_fixture(false, &[(CardId::Inflame, 0)]);
        untouched(&catalog, state);
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

    fn ordered_catalog(relics: &[RelicId], ordered: bool) -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder.set_relics_ordered(relics, ordered).unwrap();
        builder.build()
    }

    #[test]
    fn mercury_hourglass_leads_only_ahead_of_every_owned_peer_in_a_vouched_inventory() {
        use RelicId::{
            RelicAkabeko as AKABEKO, RelicBellows as BELLOWS, RelicMercuryHourglass as HOURGLASS,
            RelicToastyMittens as TOASTY, RelicVexingPuzzlebox as PUZZLEBOX,
            RelicWhetstone as UNRELATED,
        };
        for (relics, ordered, leads) in [
            (&[HOURGLASS, PUZZLEBOX][..], true, true),
            (
                &[AKABEKO, HOURGLASS, UNRELATED, TOASTY, BELLOWS],
                true,
                true,
            ),
            // Alone in the pass: front and tail are the same place.
            (&[HOURGLASS, UNRELATED], true, false),
            (&[HOURGLASS, PUZZLEBOX], false, false),
            (&[PUZZLEBOX, HOURGLASS], true, false),
            (&[TOASTY, HOURGLASS, BELLOWS], true, false),
            (&[PUZZLEBOX, TOASTY], true, false),
        ] {
            let catalog = ordered_catalog(relics, ordered);
            assert_eq!(
                mercury_hourglass_leads(&catalog),
                leads,
                "{relics:?} vouched {ordered}"
            );
        }
    }

    #[test]
    fn hourglass_and_puzzlebox_are_admitted_only_in_the_order_the_engine_runs() {
        use RelicId::{
            RelicMercuryHourglass as HOURGLASS, RelicToastyMittens as TOASTY,
            RelicVexingPuzzlebox as PUZZLEBOX,
        };
        for (relics, ordered, exact) in [
            // Hourglass leads, so it runs first: native.
            (&[HOURGLASS, PUZZLEBOX][..], true, true),
            // Puzzlebox recorded first, Hourglass at its fixed tail: native.
            (&[PUZZLEBOX, HOURGLASS], true, true),
            (&[PUZZLEBOX, TOASTY, HOURGLASS], true, true),
            // Hourglass recorded before Puzzlebox but not leading: the tail
            // would run it after Puzzlebox.
            (&[TOASTY, HOURGLASS, PUZZLEBOX], true, false),
            (&[HOURGLASS, PUZZLEBOX], false, false),
            (&[PUZZLEBOX, HOURGLASS], false, false),
            // Either relic alone is not this pair.
            (&[HOURGLASS], false, true),
            (&[PUZZLEBOX], false, true),
        ] {
            let catalog = ordered_catalog(relics, ordered);
            assert_eq!(
                mercury_hourglass_vexing_order_is_exact(&catalog),
                exact,
                "{relics:?} vouched {ordered}"
            );
        }
    }

    #[test]
    fn puzzlebox_precedes_paradox_only_when_recorded_first_without_gambling_chip() {
        use RelicId::{
            RelicChoicesParadox as PARADOX, RelicGamblingChip as CHIP,
            RelicVexingPuzzlebox as PUZZLEBOX,
        };
        for (relics, ordered, precedes) in [
            (&[PUZZLEBOX, PARADOX][..], true, true),
            (&[PARADOX, PUZZLEBOX], true, false),
            (&[PUZZLEBOX, PARADOX], false, false),
            (&[PUZZLEBOX, PARADOX, CHIP], true, false),
            (&[PUZZLEBOX], true, false),
        ] {
            let catalog = ordered_catalog(relics, ordered);
            assert_eq!(
                vexing_puzzlebox_precedes_choices_paradox(&catalog),
                precedes,
                "{relics:?} vouched {ordered}"
            );
        }
    }

    #[test]
    fn the_dispatch_carries_both_moves_for_the_pause_check() {
        use RelicId::{
            RelicChoicesParadox as PARADOX, RelicMercuryHourglass as HOURGLASS,
            RelicVexingPuzzlebox as PUZZLEBOX,
        };
        let catalog = ordered_catalog(&[HOURGLASS, PUZZLEBOX, PARADOX], true);
        assert_eq!(
            after_player_turn_start_dispatch(&catalog)[..3],
            [HOURGLASS, PUZZLEBOX, PARADOX]
        );
        let catalog = ordered_catalog(&[PARADOX, PUZZLEBOX, HOURGLASS], true);
        assert_eq!(
            after_player_turn_start_dispatch(&catalog),
            AFTER_PLAYER_TURN_START_RELIC_DISPATCH
        );
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
    /// [`after_player_turn_start_template_pass_after_leading_hourglass`] is
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
        assert_eq!(
            after_player_turn_start_template_pass_after_leading_hourglass(&catalog),
            Ok(())
        );
    }
}

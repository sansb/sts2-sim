//! The damage pipeline.
//!
//! The shape here is Python's, not a shortcut: `player_attack` (frozen Python, deleted #2827) folds
//! every additive then every multiplicative modifier, floors **once**, fires
//! Thorns' `BeforeDamageReceived` retaliation, and only then hands a fixed
//! amount to `damage_monster`, which applies block, truncates once
//! more at `LoseHpInternal`, records the damage result, and runs the death
//! cascade. `monster_attack_player` is the mirror image, with its own
//! frozen per-hit snapshot.
//!
//! Each admitted modifier occupies the same ordered fold as Python. The
//! *structure* is kept even where only one term is live — the remaining
//! modifiers land as additional terms in these folds, not as rewrites — and
//! each absent term is named at the site it would occupy.
//!
//! # Arithmetic
//!
//! Damage is [`DotNetDecimal`], the game's `System.Decimal`, throughout. That
//! matters: `Hook::ModifyDamage` floors the value once, at the end of the fold
//! (v0.109.0 0x2b1214 IL_01b5–01c0), and `Creature.DamageBlockInternal`
//! (0x11d568) converts the *blocked* portion to `Int32` before subtracting it
//! from the integer Block field. Subtracting first and truncating the
//! remainder is observably wrong. Python uses `Fraction` for the same reason
//! (`SOLVER_INVARIANTS.md` I7); every multiplier the slice reaches is exact in
//! both.

use crate::catalog::{CardIdentity, CardSpec, Catalog};
use crate::decimal::DotNetDecimal;
use crate::hot::{
    AfterDamageReceivedPower, AfterPowerAmountChangedRecord, DrawCaller, HotMonster, HotState,
    LocalCostExpiration, LocalCostModifier, LocalCostModifierKind, MiseryToken, MonsterFollowUp,
    MonsterOverride, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, EnchantmentId, MonsterKind, PotionId, PowerId, RelicId};
use crate::powers::{SlotWire, Slots};

use super::admission::{
    CORPSE_SLUG_GLOMP_INDEX, CORPSE_SLUG_GOOP_INDEX, CORPSE_SLUG_WHIP_SLAP_INDEX,
};
use super::{EngineRefusal, Event, Subject};
use crate::rng::Xoshiro256StarStar;

/// `combat_sim` truncates the final HP loss into a signed 32-bit field; the
/// hot slots are `i32`, so anything past this range refuses rather than wraps.
const HP_LOSS_CLAMP: i64 = i32::MAX as i64;

/// Skulking Colony's per-side-turn HP-loss ceiling —
/// `combat_sim.HARDENED_SHELL`, the bound `damage_monster` (frozen Python, deleted #2827) caps each
/// further loss against.
pub(crate) const HARDENED_SHELL: i32 = 20;
const NATIVE_PLAYER_BLOCK_CAP: i64 = 999_999_999;

fn overflow(site: &'static str) -> EngineRefusal {
    EngineRefusal::CounterOverflow(site)
}

/// Compute one positive monster-Strength writer while preserving the exact
/// temporary-Strength restoration invariant. Native removes a negative
/// TempStrength amount later by subtracting it from the live Strength; every
/// writer that can raise Strength must therefore prove both the immediate sum
/// and that deferred restoration fit signed `i32` before publishing anything.
pub(crate) fn checked_monster_strength_successor(
    monster: &HotMonster,
    delta: i32,
    site: &'static str,
) -> Result<i32, EngineRefusal> {
    let updated = monster
        .powers
        .value(PowerId::Strength)
        .checked_add(delta)
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    let temporary_strength = monster.powers.value(PowerId::TempStrength);
    if temporary_strength < 0 && updated.checked_sub(temporary_strength).is_none() {
        return Err(EngineRefusal::CounterOverflow("monster strength"));
    }
    Ok(updated)
}

/// The **one** writer of a monster's `PowerId::Strength` scalar, and the only
/// place its physical attachment ledger row is created, moved off its amount
/// or removed (#2693 S1).
///
/// `tests/hot_path_contract.rs::monster_strength_writer_census_is_exact`
/// derives every production `.set(PowerId::Strength` site from the tree and
/// fails on any monster-side one outside this body, so a new family cannot
/// reintroduce a raw scalar write that leaves the ledger behind.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * **A fresh attach appends.** `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`
///   does the stacking lookup at IL_00a3 and only the not-found fall-through
///   reaches `set_Applier` IL_013d and `ApplyInternal` IL_0360;
///   `Creature::ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f is a plain
///   `List.Add` onto `Creature::_powers`, which `Creature::get_Powers`
///   `0x11d8ae` returns unsorted.
/// * **A stacking application moves nothing and rewrites no applier.** The
///   found branch is `PowerCmd::ModifyAmount` `0x3f032c`, which reaches
///   `PowerModel::SetAmount` `0x83f8c` — a write to `_amount` (IL_0033) that
///   never touches `_powers` — and never reaches `set_Applier`.
/// * **Removal is at exactly zero, in place.** `StrengthPower` is
///   `AllowNegative` (`0xa8949`), so `PowerModel::ShouldRemoveDueToAmount`
///   `0x83b0d` IL_0012-IL_0023 removes it when `Amount == 0` and at no other
///   amount — a sign crossing keeps the same instance, its position and its
///   applier. `PowerModel::RemoveInternal` `0x84048` IL_001f ->
///   `Creature::RemovePowerInternal` `0x11db0b` IL_0015-IL_001c is a
///   `List.Remove`, so surviving instances keep their relative order, and a
///   later re-application appends at the end rather than reclaiming the old
///   slot.
/// * **A zero amount never attaches.** `PowerModel::ApplyInternal` `0x84012`
///   IL_0001-IL_000e returns before setting the owner.
///
/// The `applier` argument is the one native passes to
/// `PowerCmd.Apply<StrengthPower>` and is consulted **only on a fresh
/// attach**. It is `Applier::Player` for a card, potion or temporary-wrapper
/// application (`Malaise/<OnPlay>d__9::MoveNext` `0x3ab428` IL_00fc-IL_0109
/// passes `card.Owner.Player.Creature`;
/// `TemporaryStrengthPower/<BeforeApplied>d__20::MoveNext` `0x348d20`
/// IL_003e-IL_004b forwards the wrapper's own applier),
/// `Applier::Monster(uid)` for an enemy move or monster-owned power
/// (`<WriggleMove>d__19::MoveNext` `0x376510` IL_0161-IL_0175,
/// `<BarrageMove>d__18::MoveNext` `0x36e8b4` IL_00cd-IL_00e6,
/// `<DebilitatingSmogMove>d__13::MoveNext` `0x3703a4` IL_00ab-IL_00c5 and
/// IL_0120-IL_013e, `RitualPower/<AfterSideTurnEnd>d__11::MoveNext`
/// `0x342a94` IL_0052-IL_0071, `EnragePower/<AfterCardPlayed>d__4::MoveNext`
/// `0x339ce4` IL_009c-IL_00bb — every one passes the acting creature itself),
/// and `Applier::None` for a relic, which passes a literal null
/// (`PhilosophersStone::AfterCreatureAddedToCombat` `0x994f0` IL_004b-IL_004e
/// and `<AfterRoomEntered>d__8::MoveNext` `0x32dff0` IL_0098-IL_009b;
/// `Brimstone/<AfterSideTurnStart>d__8::MoveNext` `0x32098c` IL_012d-IL_0130).
///
/// **Upkeep is gated** on `misery_attachment_upkeep`, the per-fight mirror of
/// [`crate::catalog::Catalog::misery_is_reachable`]. `Misery` is the ledger's
/// only reader, so a fight that cannot reach it writes the scalar alone and
/// projects byte-identically to the pre-#2693 engine. Where upkeep is on but
/// an existing nonzero scalar carries no row — a legacy checkpoint — this
/// **gives up** rather than inventing a position, exactly as
/// [`record_misery_debuff_application`] gives up on an unreconstructible
/// scalar multiset. Absence of a row then continues to mean "provenance not
/// recorded", which [`misery_scalar_state_is_exact`] refuses on exactly where
/// it refused before.
pub(crate) fn write_monster_strength(
    monster: &mut HotMonster,
    updated: i32,
    applier: crate::hot::Applier,
    upkeep: bool,
) {
    let previous = monster.powers.value(PowerId::Strength);
    monster
        .powers
        .set(PowerId::Strength, SlotWire::Int, updated);
    if !upkeep || previous == updated {
        return;
    }
    let existing = super::monsters::strength_attachment(monster);
    match (previous == 0, updated == 0, existing) {
        // A fresh attach: `ApplyInternal` sets the owner and
        // `ApplyPowerInternal` appends, after `set_Applier` wrote the applier.
        (true, false, None) => {
            monster
                .misery_debuff_order
                .push_attachment(crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Strength,
                    applier,
                    amount: updated,
                })
        }
        // `ShouldRemoveDueToAmount` at exactly zero, erased in place.
        (false, true, Some(index)) => {
            let removed = monster.misery_debuff_order.remove_attachment(index);
            debug_assert!(removed, "a located attachment removes");
        }
        // `ModifyAmount` -> `SetAmount`: a value, never a position, and never
        // the applier.
        (false, false, Some(index)) => {
            let written = monster
                .misery_debuff_order
                .set_attachment_amount(index, updated);
            debug_assert!(written, "a located nonzero attachment restacks");
        }
        // Give up: an unrecorded legacy instance whose position this writer
        // cannot know, or a row the caller's own bookkeeping already broke.
        // Leaving the ledger alone keeps "unrecorded" as the state, which
        // every reader refuses on where it is observable.
        _ => {}
    }
}

/// Record the provenance of a `StrengthPower` a monster walks into this root
/// already carrying, wherever its position is uniquely determined (#2693 S4).
///
/// #2693 S1 never infers: a monster entering a root with a nonzero
/// `PowerId::Strength` — `initial_powers`, an ascension effect, or any
/// mid-fight checkpoint written before the ledger existed — carried no row,
/// so [`write_monster_strength`] gave up on the next write and the scalar
/// stayed *unrecorded* for the whole fight. When `Misery` is reachable and
/// something later drives that Strength negative, the play then refuses
/// **late, inside an admitted root**, which #2693 forbids.
///
/// What makes this exact rather than a guess is that only one thing about the
/// instance is unknown. Its **amount** is the scalar. Its **position** is
/// uniquely determined exactly where
/// [`super::monsters::entering_strength_position_is_determined`] holds —
/// nothing else is recorded, so it precedes everything that can be added
/// later, which is what `Creature::ApplyPowerInternal` `0x11da0c`
/// IL_0063-IL_006f's append guarantees. Its **applier** is genuinely
/// unobserved, and is recorded as [`crate::hot::Applier::Unknown`] rather
/// than invented: `StrengthPower` takes the base
/// `PowerModel::get_InstanceType` `0x83751` = 0, so
/// `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058 resolves it
/// by model id and never reads the applier, and the only readers of the
/// applier at all are the two given-side amount modifiers gated on
/// `ICombatState.ContainsCreature(applier)`
/// (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_01d3-IL_01ec) — Snecko
/// Skull, which is `PoisonPower`-only (`0x9bb8d`), and the Unsettling Lamp
/// (`0x9d5a8`), which admission refuses beside a non-player applier.
///
/// Where the position is **not** determined this does nothing and the root
/// refuses at admission by name instead of at play time.
///
/// Gated on [`crate::hot::HotFanouts::misery_attachment_upkeep`] like every
/// other ledger write, so a fight that cannot reach `Misery` records nothing
/// and projects byte-identically to the pre-#2693 engine.
pub(crate) fn materialize_entering_strength_provenance(state: &mut HotState) {
    if !state.fanouts.misery_attachment_upkeep() {
        return;
    }
    let pending = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| {
            // The ledger holds no attachment in the determined case, so this
            // is exactly "a nonzero scalar with no row".
            !super::monsters::strength_provenance_is_recorded(monster)
                && super::monsters::entering_strength_position_is_determined(monster)
        })
        .map(|(index, monster)| (index, monster.powers.value(PowerId::Strength)))
        .collect::<Vec<_>>();
    for (index, amount) in pending {
        state.monsters_mut()[index]
            .misery_debuff_order
            .push_attachment(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Unknown,
                amount,
            });
    }
}

/// Does `kind`'s own `AfterAddedToRoom` apply `StrengthPower` to itself, with
/// itself as applier (#3364)? Only then can an **opening** name the applier
/// of the Strength the monster spawns with.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// **The census.** Every `<AfterAddedToRoom>d__N::MoveNext` in the DLL (65
/// async bodies) and every synchronous `AfterAddedToRoom` (69, each a
/// state-machine thunk or `MonsterModel::AfterAddedToRoom` `0x823b0`, which
/// is `Task.CompletedTask`) was read with `dump_il.py`. Exactly one calls
/// `PowerCmd.Apply<StrengthPower>`: `MysteriousKnight/<AfterAddedToRoom>d__0`
/// `0x3642a0`. The only other Strength-named call is `TheLost/<AfterAddedToRoom>d__11`
/// `0x3702cc` IL_0030, which is `Apply<PossessStrengthPower>` — a different
/// model, never a `StrengthPower` row. The three helpers those bodies call
/// (`LagavulinMatriarch/<Sleep>d__40` `0x3613b8`, which applies Plating and
/// Asleep; `ToughEgg::Hatch`; `FabricatorNormal::SetBotFallPosition`) apply
/// no Strength, and the two non-generic `PowerCmd::Apply` calls
/// (`Aeonglass` `0x3524e8` IL_0108, `GremlinMerc` `0x35d898` IL_0166) apply
/// `WitheringPresencePower` and `ThieveryPower` mutables.
///
/// **The Mysterious Knight.** `<AfterAddedToRoom>d__0::MoveNext` `0x3642a0`
/// awaits the base hook (`<>n__0`, IL_002d — `FlailKnight` does not override
/// it, so it is `MonsterModel::AfterAddedToRoom`'s completed task), then
/// IL_0087-IL_00a0 is `PowerCmd.Apply<StrengthPower>(ctx, get_Creature, 6,
/// get_Creature, null, false)`: the Knight's own `Creature` as **both**
/// target (IL_008d) and applier (IL_0099). The amount is the
/// `MONSTER_MODELS` `StrengthPower` row.
pub(crate) fn spawn_strength_is_self_applied(kind: MonsterKind) -> bool {
    kind == MonsterKind::MysteriousKnight
}

/// Name the applier of a monster's spawn Strength on an **opening** root,
/// where the IL proves it is the monster's own `AfterAddedToRoom` (#3364).
///
/// [`materialize_entering_strength_provenance`] records every entering
/// instance `Unknown`, and must: a mid-fight document cannot say whether its
/// Strength is still the spawn instance or a later one — a Strength driven
/// to exactly zero is removed (`PowerModel::ShouldRemoveDueToAmount`
/// `0x83b0d` IL_0012-IL_0023), and the next `Apply` re-attaches with *its*
/// applier (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_013d writes
/// `set_Applier` on the fresh-attach fall-through only). The opening can:
/// its pre-hook roster is the state `AfterAddedToRoom` leaves, before any
/// room-entry or combat-start hook, card, or turn has run. So this is called
/// once, from `entry::opening::build`, straight after the pre-hook document
/// is hydrated, and nowhere else.
///
/// It rewrites a row only when everything agrees with that spawn:
///
/// * the kind is one whose `AfterAddedToRoom` self-applies Strength
///   ([`spawn_strength_is_self_applied`]);
/// * the ledger holds nothing but one `Strength` row, recorded `Unknown` by
///   [`materialize_entering_strength_provenance`] — so its position is
///   already forced and only the applier changes;
/// * the row, the live scalar and the native spawn amount
///   ([`super::monsters::native_initial_power`]) are the same number.
///
/// Anything else is left `Unknown`, so a shape this does not recognise keeps
/// refusing by name rather than being attributed by guess. `StrengthPower`
/// stacks by model id (`PowerCmd::FindExistingInstanceForStacking`
/// `0x1338d8` IL_0058), so the applier rides along through every later
/// `ModifyAmount` exactly as [`write_monster_strength`] already keeps it.
pub(crate) fn attribute_spawn_strength_to_its_owner(state: &mut HotState) {
    let pending = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| {
            if !spawn_strength_is_self_applied(monster.kind) {
                return false;
            }
            let order = &monster.misery_debuff_order;
            let mut rows = order.attachments();
            let (Some(row), None) = (rows.next(), rows.next()) else {
                return false;
            };
            let scalar = monster.powers.value(PowerId::Strength);
            order.as_slice().is_empty()
                && order.knockdown().is_empty()
                && row.power == crate::hot::AttachedPowerModel::Strength
                && row.applier == crate::hot::Applier::Unknown
                && row.amount == scalar
                && super::monsters::native_initial_power(state, monster.kind, "StrengthPower")
                    == Some(scalar)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    for index in pending {
        let monster = &mut state.monsters_mut()[index];
        let owner = crate::hot::Applier::Monster(monster.uid);
        let amount = monster.powers.value(PowerId::Strength);
        let removed = monster.misery_debuff_order.remove_attachment(0);
        debug_assert!(removed, "the lone Unknown row removes");
        monster
            .misery_debuff_order
            .push_attachment(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: owner,
                amount,
            });
    }
}

/// [`write_monster_strength`] for the common enemy self-buff, where the
/// acting creature is both target and applier.
///
/// Every monster move and monster-owned power that raises its own Strength
/// passes its own `Creature` as the applier — `<WriggleMove>d__19::MoveNext`
/// `0x376510` IL_0161-IL_0175 and `<BarrageMove>d__18::MoveNext` `0x36e8b4`
/// IL_00cd-IL_00e6 load `get_Creature` twice, once for the target and once
/// for the applier, and `RitualPower/<AfterSideTurnEnd>d__11::MoveNext`
/// `0x342a94` IL_0052-IL_0071 does the same with `PowerModel::get_Owner`.
/// A move that buffs a *different* creature keeps the **acting** monster as
/// applier (`<DebilitatingSmogMove>d__13::MoveNext` `0x3703a4`
/// IL_00ab-IL_00c5 applies to its targets with its own `Creature`), so those
/// callers spell the applier out rather than using this helper.
pub(crate) fn write_monster_self_strength(monster: &mut HotMonster, updated: i32, upkeep: bool) {
    let owner = crate::hot::Applier::Monster(monster.uid);
    write_monster_strength(monster, updated, owner, upkeep);
}

/// Erase every temporary-Strength wrapper row this monster carries, leaving
/// its aggregate scalar behind as *unrecorded*.
///
/// This is the give-up edge, and it is deliberately not silent about what it
/// costs: after it runs the scalar no longer has recorded provenance, so
/// [`misery_scalar_state_is_exact`] refuses on it exactly where the
/// categorical `TempStrength != 0` disjunct refuses today. It is reached only
/// where the alternative would be a row that disagrees with the scalar — an
/// approximation I5 forbids — and never where a position could be known.
fn abandon_monster_temp_strength_provenance(monster: &mut HotMonster) {
    loop {
        let Some(index) = monster
            .misery_debuff_order
            .attachments()
            .position(|record| record.power.is_temporary_strength_wrapper())
        else {
            return;
        };
        let removed = monster.misery_debuff_order.remove_attachment(index);
        debug_assert!(removed, "a located attachment removes");
    }
}

/// The **one** writer that applies a concrete temporary-Strength wrapper to a
/// monster (#2693 S2), and — with
/// [`unwind_monster_temp_strength_wrappers`] and
/// [`clear_monster_temp_strength_wrappers`] — one of only three places a
/// monster's `PowerId::TempStrength` scalar or a wrapper row moves.
///
/// `tests/hot_path_contract.rs::monster_temp_strength_writer_census_is_exact`
/// derives every production `.set(PowerId::TempStrength` site from the tree
/// and fails on any monster-side one outside these three bodies.
///
/// `amount` is the **positive** native `Amount` this application contributes,
/// as it stands *after* the Artifact/Lamp gate — the callers run that gate
/// themselves, because it is `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`
/// IL_0248's `Hook::ModifyPowerAmountReceived` and it fires before anything
/// here can happen.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * **The nested Strength lands BEFORE the wrapper attaches.** `Apply`
///   `0x3efbac` calls `PowerModel::BeforeApplied` at IL_02d9 and
///   `PowerModel::ApplyInternal` — which is what reaches
///   `Creature::ApplyPowerInternal` `0x11da0c` IL_0063's `List.Add` — only at
///   IL_0360. `TemporaryStrengthPower/<BeforeApplied>d__20::MoveNext`
///   `0x348d20` IL_001d-IL_004b is the nested
///   `PowerCmd.Apply<StrengthPower>(target, Sign * amount, applier,
///   cardSource)`. So on a fresh attach with no live Strength the ledger gets
///   `[… , strength, wrapper]`, in that order, and this body writes them in
///   that order for that reason.
/// * **A restack moves a value, not a position.** The found branch is
///   `PowerCmd::ModifyAmount` `0x3f032c`, whose `PowerModel::SetAmount`
///   `0x83f8c` IL_0033 never touches `_powers`; the nested Strength then
///   comes from `<AfterPowerAmountChanged>d__21::MoveNext` `0x348a88`
///   IL_004b-IL_007a, which runs at `ModifyAmount` IL_02eb — *after*
///   `SetAmount` IL_01d6. Writing Strength first is still correct there,
///   because the wrapper's position does not move either way.
/// * **The lookup is by model, never by applier** — see
///   [`super::monsters::temp_strength_wrapper_attachment`].
/// * **`SetAmount` clamps to +/-999999999** (`0x83f8c` IL_0013-IL_0022). The
///   aggregate scalar is *not* clamped the same way, so a stacked row that
///   would clamp can no longer mirror it. Rather than record a row that
///   disagrees, this gives the monster's wrapper provenance up entirely; the
///   scalar arithmetic is byte-identical either way, so no root that admits
///   today can lose admission over it.
///
/// Returns the new `(temp_strength, strength)` scalars so each caller can keep
/// emitting its own `note_power` pair in its own established order.
pub(crate) fn write_monster_temp_strength_wrapper(
    monster: &mut HotMonster,
    model: crate::hot::AttachedPowerModel,
    amount: i32,
    applier: crate::hot::Applier,
    upkeep: bool,
) -> Result<(i32, i32), EngineRefusal> {
    debug_assert!(
        model.is_temporary_strength_wrapper(),
        "only a loss wrapper carries a temporary-Strength row"
    );
    if amount <= 0 {
        // `ShouldRemoveDueToAmount` `0x83b0d` IL_0001-IL_0010 removes a
        // `!AllowNegative` power at any non-positive amount, and
        // `ApplyInternal` `0x84012` IL_0001-IL_000e never attaches a zero, so
        // a non-positive application is not a native state.
        return Err(EngineRefusal::MalformedArgs(
            "temporary monster Strength wrapper amount",
        ));
    }
    let delta = -amount;
    let previous_temp = monster.powers.value(PowerId::TempStrength);
    let temp = previous_temp
        .checked_add(delta)
        .ok_or(EngineRefusal::CounterOverflow("temporary monster strength"))?;
    let strength = monster
        .powers
        .value(PowerId::Strength)
        .checked_add(delta)
        .ok_or(EngineRefusal::CounterOverflow("monster strength"))?;

    // Whether the rows can be kept in step is decided BEFORE anything moves,
    // against the state as it stands: a scalar this writer never recorded
    // (a legacy checkpoint, or a fight whose upkeep gate was off) cannot be
    // attributed to a position now.
    let recorded = super::monsters::temp_strength_provenance_is_recorded(monster);
    let existing = super::monsters::temp_strength_wrapper_attachment(monster, model);
    let stacked = existing
        .and_then(|index| {
            monster
                .misery_debuff_order
                .attachments()
                .get(index)
                .copied()
        })
        .map_or(Some(amount), |record| record.amount.checked_add(amount))
        .filter(|value| *value <= 999_999_999);

    write_monster_strength(monster, strength, applier, upkeep);
    monster
        .powers
        .set(PowerId::TempStrength, SlotWire::Int, temp);

    if upkeep {
        match (recorded, stacked, existing) {
            (true, Some(stacked), Some(index)) => {
                let written = monster
                    .misery_debuff_order
                    .set_attachment_amount(index, stacked);
                debug_assert!(written, "a located nonzero attachment restacks");
            }
            (true, Some(_), None) => {
                monster
                    .misery_debuff_order
                    .push_attachment(crate::hot::AttachmentRecord {
                        power: model,
                        applier,
                        amount,
                    })
            }
            // Either the scalar already carried provenance this writer never
            // recorded, or the clamp would make the row disagree with it.
            _ => abandon_monster_temp_strength_provenance(monster),
        }
        debug_assert!(
            !recorded
                || stacked.is_none()
                || super::monsters::temp_strength_provenance_is_recorded(monster),
            "a recorded ledger stays in step with its scalar"
        );
    }
    Ok((temp, strength))
}

/// Run the native side-end unwind of every temporary-Strength wrapper this
/// monster carries and return its restored `PowerId::Strength`.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `TemporaryStrengthPower/<AfterSideTurnEnd>d__22::MoveNext` `0x348ba8`
/// returns unless `participants.Contains(Owner)` (IL_0024-IL_0037), then
/// `Flash`es, **removes the wrapper first** (`PowerCmd::Remove` IL_0043,
/// awaited) and only then applies `-Sign * Amount` Strength — reading the
/// amount off the already-detached model — with the **`Owner`** as applier
/// and a null card source (IL_009d-IL_00c4). The applier is the owner, not
/// the wrapper's original one, which is why this hands `write_monster_strength`
/// `Applier::Monster(uid)` however the wrappers were applied.
///
/// **The order across several wrappers is `Creature.Powers` order, and it is
/// observable.** `Hook/<AfterSideTurnEnd>d__81::MoveNext` `0x3d11c0`
/// IL_0051-IL_00b0 walks `Hook::IterateCombatHookListeners` `0x103056` ->
/// `CombatState::IterateHookListeners` `0x137409`, whose
/// `<IterateHookListeners>d__69::MoveNext` `0x3f9720` builds a `List` at
/// IL_003a-IL_0048 and fills it per creature with
/// `AddRange(creature.Powers)` at IL_008f-IL_0097. Two facts follow: the walk
/// is a *snapshot*, so a wrapper removing itself mid-walk cannot disturb the
/// rest, and within a creature it is ledger order. That matters because each
/// restoration is its own `Apply<StrengthPower>`: if one of them lands
/// Strength on exactly zero the instance is removed (`ShouldRemoveDueToAmount`
/// `0x83b0d` IL_0012-IL_0023, `AllowNegative`), and the next re-attaches it at
/// the END of the ledger with the owner as its applier.
///
/// With no recorded provenance this falls back to the single aggregate
/// restoration the pre-#2693 engine performed, which is the same scalar
/// result — the rows only ever decide where the Strength row ends up.
pub(crate) fn unwind_monster_temp_strength_wrappers(
    monster: &mut HotMonster,
    upkeep: bool,
) -> Result<i32, EngineRefusal> {
    let temporary_strength = monster.powers.value(PowerId::TempStrength);
    if temporary_strength == 0 {
        return Ok(monster.powers.value(PowerId::Strength));
    }
    let owner = crate::hot::Applier::Monster(monster.uid);
    let restored = monster
        .powers
        .value(PowerId::Strength)
        .checked_sub(temporary_strength)
        .ok_or(EngineRefusal::CounterOverflow("monster strength"))?;
    let recorded = super::monsters::temp_strength_provenance_is_recorded(monster);
    monster.powers.set(PowerId::TempStrength, SlotWire::Int, 0);
    if !upkeep {
        write_monster_strength(monster, restored, owner, upkeep);
        return Ok(restored);
    }
    if !recorded {
        abandon_monster_temp_strength_provenance(monster);
        write_monster_strength(monster, restored, owner, upkeep);
        return Ok(restored);
    }
    // Always the FIRST surviving wrapper row, re-read each pass: the Strength
    // writes between them can remove or append a row of their own and shift
    // every later index.
    loop {
        let Some((index, record)) = super::monsters::temp_strength_wrapper_rows(monster).next()
        else {
            break;
        };
        let removed = monster.misery_debuff_order.remove_attachment(index);
        debug_assert!(removed, "a located attachment removes");
        let next = monster
            .powers
            .value(PowerId::Strength)
            .checked_add(record.amount)
            .ok_or(EngineRefusal::CounterOverflow("monster strength"))?;
        write_monster_strength(monster, next, owner, upkeep);
    }
    debug_assert_eq!(
        monster.powers.value(PowerId::Strength),
        restored,
        "the per-wrapper restorations sum to the aggregate one"
    );
    Ok(restored)
}

/// Remove every temporary-Strength wrapper outright, **without** restoring
/// the Strength they applied.
///
/// This is not the side-end lifecycle: it is the two represented whole-
/// instance wipes — `PlowPower`'s threshold crossing and the segment revive —
/// where the Strength scalar is separately zeroed by its own writer rather
/// than unwound. Every row goes with the instances it describes.
pub(crate) fn clear_monster_temp_strength_wrappers(monster: &mut HotMonster, upkeep: bool) {
    monster.powers.set(PowerId::TempStrength, SlotWire::Int, 0);
    if upkeep {
        abandon_monster_temp_strength_provenance(monster);
    }
}

/// Authenticate the target-owned Conqueror carrier before Sovereign Blade
/// reads it. Admission accepts only a nonnegative Int; direct/nested callers
/// must refuse the same malformed wire or negative amount instead of treating
/// it as an ordinary absent/positive scalar.
///
/// Current v0.111.0 ARM64 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `ConquerorPower.ModifyDamageMultiplicative` RVA `0xa08b0` is the
/// target-owned powered-source x2 listener. Its Amount is duration, so every
/// positive value contributes exactly one multiplier.
#[cold]
#[inline(never)]
fn sovereign_conqueror_is_positive(state: &HotState, target: usize) -> Result<bool, EngineRefusal> {
    match state.monsters[target].powers.get(PowerId::Conqueror) {
        None => Ok(false),
        Some(slot) if slot.wire == SlotWire::Int && slot.value >= 0 => Ok(slot.value > 0),
        Some(_) => Err(EngineRefusal::MalformedArgs("Conqueror power state")),
    }
}

/// One card attack command: `hits` hits at `base` damage against each of
/// `targets`, in order.
///
/// `combat_sim.player_attack` (frozen Python, deleted #2827). The command is two nested loops, and
/// the nesting is the mechanic rather than an implementation detail: an
/// all-opponents card is **one** command whose per-hit body sweeps the whole
/// target list, not N single-target commands. Vigor, Gigantification, Pen Nib
/// and the `AfterAttack` tail are scoped around the outer command; the
/// liveness re-read and the Thorns retaliation point are per target *within* a
/// hit.
///
/// The target list is a roster-index slice captured by the caller before the
/// command starts, and every hit addresses those same creatures: a target
/// killed by an earlier hit stays in the list and is skipped by the liveness
/// gate. That is native `Targeting(creature)` semantics. An all-opponents
/// card body does NOT use this entry: `TargetingAllOpponents` re-reads the
/// living opponents on every hit ([`player_attack_all_from_card`], #3023).
///
/// The per-target body is, in order:
///
/// 1. re-read the live target — an earlier hit may have killed it;
/// 2. fold the additive modifiers (Strength, temporary Strength, Vigor,
///    Accuracy, …). Player Strength, Vigor, and physical-Shiv Accuracy are the
///    admitted terms; Phantom Blades stays refused with per-card Retain;
/// 3. fold the multiplicative modifiers. Player Weak, Vulnerable,
///    Double Damage, player-owned Cruelty, Weak-target Tracking, and Shrink
///    are the admitted terms;
/// 4. floor once (`dmg = max(0, dmg)`), the shared final floor;
/// 5. `ThornsPower.BeforeDamageReceived` (`0xaa0ac` -> `d__4::MoveNext`
///    `0x349954`; the previously cited `0x38ffb0` is not this body on the
///    v0.111.0 archive — re-derived for #2647) retaliates **per powered damage
///    instance, after the calculation and before the hit applies** — fully
///    blocked hits and the killing blow included;
/// 6. commit through [`damage_monster`].
///
/// A hit count of zero runs the outer loop zero times and is legal: Whirlwind
/// at X = 0 executes the command with no damage instances (d__6 0x40ce34;
/// `AttackCommand.Execute` reaches `AfterAttack` regardless).
pub fn player_attack(
    state: &mut HotState,
    source: &CardSpec,
    targets: &[usize],
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base,
            decimal_base: None,
            hits,
            source_uid: None,
            frozen_target_identity: None,
            catalog: None,
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::Player,
        },
        events,
    )
}

/// One private result published by a powered attack command.
///
/// The receiver uid is captured before death handling, so a same-slot respawn
/// cannot inherit the old command's result. `total_damage` is native blocked
/// plus unblocked damage; `overkill_damage` is the unblocked amount beyond the
/// receiver's pre-hit HP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttackDamageResult {
    pub target: usize,
    pub receiver_uid: u32,
    pub total_damage: i32,
    pub overkill_damage: i32,
    pub was_target_killed: bool,
}

/// Exact powered-attack dealer. A player-owned pet shares ownership-sensitive
/// target modifiers, but never inherits the player's personal attack stats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttackDealer {
    Player,
    PlayerPet,
}

/// How one powered `AttackCommand` chooses the receivers of each hit.
///
/// `AttackCommand.<Execute>d__90::MoveNext` (v0.111.0 RVA `0x3f19c0`) calls
/// `GetPossibleTargets` afresh at the top of EVERY iteration of its per-hit
/// loop (loop body IL_0156-IL_07e0, call at IL_0168) and filters it with
/// `IsAlive` (`<>c::<Execute>b__90_1`, `0x3f18b6`) into `validTargets`
/// (IL_0196). `GetPossibleTargets` (`0x1348ac`) returns the fixed
/// `_singleTarget` when one was set (IL_000c-IL_001f) and otherwise the live
/// `CombatState.GetOpponentsOf(Attacker)` (IL_0050-IL_005c). The two
/// therefore differ only for a command whose roster changes between hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttackTargeting<'a> {
    /// A `Targeting(creature)` command: every hit addresses the same
    /// creature identity. Also carries the deliberate caller-frozen rosters.
    Fixed(&'a [usize]),
    /// A `TargetingAllOpponents` command: each hit re-reads the living
    /// opponents, so a creature spawned by an earlier hit's death (Phrog's
    /// Wrigglers, Gremlin Merc's pair, a respawned Axebot) is hit by the later
    /// hits, and an empty living set ends the hit loop (`validTargets.Count ==
    /// 0` with a live combat branches out of the loop at IL_01a6-IL_01b3).
    AllOpponents,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackContextMode {
    Ordinary,
    EchoingSlash,
    Omnislice,
}

struct AttackPlan<'targets, 'results> {
    targeting: AttackTargeting<'targets>,
    base: i64,
    decimal_base: Option<DotNetDecimal>,
    hits: i64,
    source_uid: Option<u32>,
    /// The enclosing CardPlay's original Creature identity for a fixed
    /// single-enemy attack. Stock may replace the roster entry in place
    /// between hits or between earlier and later body commands; those later
    /// hits still address the dead original and therefore no-op.
    frozen_target_identity: Option<(i32, u32)>,
    catalog: Option<&'targets Catalog>,
    context_mode: AttackContextMode,
    result_sink: Option<&'results mut Vec<AttackDamageResult>>,
    dealer: AttackDealer,
}

fn frozen_card_target_identity(source_uid: u32, targets: &[usize]) -> Option<(i32, u32)> {
    (targets.len() == 1)
        .then(|| super::play::active_card_target_identity(source_uid).flatten())
        .flatten()
}

struct AttackObservation<'results> {
    source_uid: Option<u32>,
    result_sink: Option<&'results mut Vec<AttackDamageResult>>,
    dealer: AttackDealer,
    catalog: Option<&'results Catalog>,
}

/// A card body's powered attack with its exact physical source identity.
///
/// Curl Up is the first admitted target listener that retains the physical
/// card across damage and consumes it at AfterCardPlayed. Validate the live
/// physical UID once, before the command mutates, whenever an attacked live
/// Louse currently owns Curl Up. `source` is the running card spec already
/// resolved by `StepCtx`; the UID check proves that it still has exactly one
/// physical carrier.
pub fn player_attack_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    targets: &[usize],
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    if hits > 0
        && targets.iter().copied().any(|target| {
            state
                .monsters
                .get(target)
                .is_some_and(|monster| monster.hp > 0 && monster.powers.value(PowerId::CurlUp) > 0)
        })
    {
        require_live_card_source(state, catalog, source, source_uid)?;
    }
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: frozen_card_target_identity(source_uid, targets),
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::Player,
        },
        events,
    )
}

/// A physical card attack whose native `Damage.BaseValue` is a Decimal.
///
/// Thrash is the only admitted source: its post-play growth retains the exact
/// result of a prior `ModifyDamage` fold, including Weak/Shrink fractions.
pub(crate) fn player_attack_decimal_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    targets: &[usize],
    base: DotNetDecimal,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    if source.identity.id != CardId::Thrash {
        return Err(EngineRefusal::MalformedArgs("decimal card attack source"));
    }
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base: 0,
            decimal_base: Some(base),
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: frozen_card_target_identity(source_uid, targets),
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::Player,
        },
        events,
    )
}

/// One powered all-opponents card attack (`TargetingAllOpponents`).
///
/// Every hit re-resolves the living opponents in roster order (see
/// [`AttackTargeting::AllOpponents`] for the IL): a creature an earlier hit's
/// death spawned is a receiver of the later hits, and once no opponent is
/// alive the remaining hits do not run.
pub fn player_attack_all_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::AllOpponents,
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: None,
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::Player,
        },
        events,
    )
}

/// The player's Osty issuing one all-opponents card attack; targeting as
/// [`player_attack_all_from_card`], dealer as [`player_pet_attack_from_card`].
pub(crate) fn player_pet_attack_all_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::AllOpponents,
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: None,
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::PlayerPet,
        },
        events,
    )
}

/// One powered card attack with a per-hit live `CombatTargets` roll.
pub fn player_attack_random_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Random,
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: None,
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::Player,
        },
        events,
    )
}

/// One card-authored attack issued by the player's singular Osty.
///
/// The physical card remains the command source for target listeners and
/// result routing, while the dealer stays `PLAYER_PET` for every modifier,
/// retaliation, history, and AfterDamageGiven decision.
pub(crate) fn player_pet_attack_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    targets: &[usize],
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: frozen_card_target_identity(source_uid, targets),
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::PlayerPet,
        },
        events,
    )
}

pub(crate) fn player_pet_attack_random_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Random,
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: None,
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: None,
            dealer: AttackDealer::PlayerPet,
        },
        events,
    )
}

/// One ordinary targeted powered card attack that returns its private result
/// list to the fused card body.
pub fn player_attack_results_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    targets: &[usize],
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<Vec<AttackDamageResult>, EngineRefusal> {
    let (catalog, source, source_uid) = source;
    if hits > 0 && !targets.is_empty() {
        require_live_card_source_if_curl_up(state, catalog, source, source_uid, hits)?;
    }
    let mut results = Vec::new();
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base,
            decimal_base: None,
            hits,
            source_uid: Some(source_uid),
            frozen_target_identity: frozen_card_target_identity(source_uid, targets),
            catalog: Some(catalog),
            context_mode: AttackContextMode::Ordinary,
            result_sink: Some(&mut results),
            dealer: AttackDealer::Player,
        },
        events,
    )?;
    Ok(results)
}

/// The two exact card-private multi-batch AttackContext modes.
pub fn player_attack_context_from_card(
    state: &mut HotState,
    source: (&Catalog, &CardSpec, u32),
    targets: &[usize],
    base: i64,
    mode: AttackContextMode,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let (catalog, source, source_uid) = source;
    let valid = matches!(
        (mode, source.identity.id, targets),
        (AttackContextMode::EchoingSlash, CardId::EchoingSlash, [])
            | (AttackContextMode::Omnislice, CardId::Omnislice, [_])
    );
    if !valid {
        return Err(EngineRefusal::MalformedArgs("attack context mode"));
    }
    require_live_card_source_if_curl_up(state, catalog, source, source_uid, 1)?;
    let mut results = Vec::new();
    player_attack_inner(
        state,
        source,
        AttackPlan {
            targeting: AttackTargeting::Fixed(targets),
            base,
            decimal_base: None,
            hits: 1,
            source_uid: Some(source_uid),
            frozen_target_identity: frozen_card_target_identity(source_uid, targets),
            catalog: Some(catalog),
            context_mode: mode,
            result_sink: Some(&mut results),
            dealer: AttackDealer::Player,
        },
        events,
    )
}

fn require_live_card_source_if_curl_up(
    state: &HotState,
    catalog: &Catalog,
    source: &CardSpec,
    source_uid: u32,
    hits: i64,
) -> Result<(), EngineRefusal> {
    if hits > 0
        && state
            .monsters
            .iter()
            .any(|monster| monster.hp > 0 && monster.powers.value(PowerId::CurlUp) > 0)
    {
        require_live_card_source(state, catalog, source, source_uid)?;
    }
    Ok(())
}

/// One powered player or Osty `AttackCommand`.
///
/// A command issued while the combat is over or ending does nothing at all,
/// whichever of the two dealers issues it (Osty since #3466, the player since
/// #3483): `AttackCommand/<Execute>d__90::MoveNext` RVA `0x3f19c0` tests
/// `CombatManager::get_IsOverOrEnding` (IL_0067-006c; RVA `0x1358be` is
/// `IsEnding || !IsInProgress`) and, for a live combat
/// (`CombatState::IsLiveCombat` RVA `0x1377db` is constant true;
/// IL_0073-0086), leaves at IL_0088-008a. That is before BeforeAttack
/// (IL_00d8, so Vigor and Gigantification never bind the command), any hit,
/// the `CombatHistory::CreatureAttacked` record (IL_07e5-082a) and AfterAttack
/// (IL_0845). So no Osty attack is counted for Flatten or Rattle, and a player
/// command leaves Vigor where it was.
///
/// The test is live, not a latch: `get_IsEnding` RVA `0x135834` calls
/// `CombatManager::IsCombatEnding` RVA `0x135854`, which is true for a pending
/// loss (IL_0016-0025) or when no enemy passes `IsAlive && IsPrimaryEnemy`
/// (`<IsCombatEnding>b__82_0` RVA `0x3f2532`) and no `ShouldStopCombatFromEnding`
/// listener holds it open (IL_0026-0069). [`damage_combat_is_ending`] is the
/// shared projection of that test.
///
/// It sits at command entry only. The per-hit loop (IL_0156-07e0) never
/// re-tests it: a multi-hit command whose earlier hit ends the combat leaves
/// the loop at the next hit's empty `validTargets` (IL_01a6-01b3) or dead
/// attacker (IL_0161), and both branch to the record site, so that command
/// still records and runs AfterAttack. A later command in the same card body
/// is the one that returns here. The engine's step runner keeps running a
/// card body after the combat is over while the player lives, so this entry
/// return, not a caller gate, is what keeps such a command inert.
///
/// A dead attacker returns next (IL_00a2-00b1: `Attacker.IsDead`, leave at
/// IL_00b1), also before BeforeAttack and the record site (#3502). A dead
/// player makes the combat ending (pending loss), so only Osty reaches it
/// with the combat live: its killed or never-summoned state is
/// `pet().osty().is_none()`. Every step caller already tests that before
/// issuing the command (`osty_body`, `sic_em_exact`, `rattle_exact`); the
/// return keeps a direct command caller from counting an Osty attack for
/// Flatten or Rattle, or binding Gigantification, with no Osty.
fn player_attack_inner(
    state: &mut HotState,
    source: &CardSpec,
    plan: AttackPlan<'_, '_>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if damage_combat_is_ending(state) {
        return Ok(());
    }
    if plan.dealer == AttackDealer::PlayerPet && state.fanouts.pet().osty().is_none() {
        return Ok(());
    }
    let fixed_target_identities = match plan.targeting {
        AttackTargeting::Fixed(targets) => {
            let mut identities = targets
                .iter()
                .map(|target| {
                    state
                        .monsters
                        .get(*target)
                        .map(|monster| (monster.slot, monster.uid))
                })
                .collect::<Option<Vec<_>>>();
            if let (Some(identities), Some(identity)) =
                (&mut identities, plan.frozen_target_identity)
                && identities.len() == 1
            {
                identities[0] = identity;
            }
            identities
        }
        AttackTargeting::AllOpponents | AttackTargeting::Random => None,
    };
    let hive_reachable = plan.hits > 0
        && match (plan.context_mode, plan.targeting) {
            (AttackContextMode::EchoingSlash, _)
            | (_, AttackTargeting::AllOpponents | AttackTargeting::Random) => state
                .monsters
                .iter()
                .any(|monster| monster.hp > 0 && monster.powers.value(PowerId::Hive) > 0),
            (_, AttackTargeting::Fixed(targets)) => targets.iter().copied().any(|target| {
                state.monsters.get(target).is_some_and(|monster| {
                    monster.hp > 0 && monster.powers.value(PowerId::Hive) > 0
                })
            }),
        };
    let knockdown_reachable = require_live_knockdown_state(state)?;
    // A retaliation can remove/replace the in-flight original receiver. The
    // retained-object guard below must remain atomic for direct command users.
    // Inferno and Puzzle-drawn AutoPlay are the represented player-retaliation
    // writers of enemy state. Ordinary Thorns keeps the allocation-free path;
    // pet retaliation already owns the unconditional pet-command rehearsal.
    let thorns_reachable = plan.hits > 0
        && (state.powers.value(PowerId::Inferno) > 0 || state.fanouts.puzzle_armed())
        && state
            .monsters
            .iter()
            .any(|monster| monster.hp > 0 && monster.powers.value(PowerId::Thorns) > 0);

    // Osty attacks add command-scoped history and can publish Flatten's
    // physical-card rewrite after every damage/listener result. Rehearse the
    // complete command even when Hive is absent so a late counter/catalog/
    // local-state refusal cannot leak any earlier hit through direct callers.
    if hive_reachable && !super::monsters::entomancer_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs("Entomancer Hive owner/state"));
    }
    let hex_reachable = state.powers.value(PowerId::HexPower) > 0;
    let gremlin_merc_reachable = state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::GremlinMerc | MonsterKind::FatGremlin
        )
    });
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_owner_reachable(state);
    let gigantification_reachable =
        matches!(plan.dealer, AttackDealer::Player | AttackDealer::PlayerPet)
            && source.is_attack
            && state.fanouts.gigantification() > 0
            && !state.fanouts.gigantification_bound();
    if hex_reachable && !super::cards::hex_power_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Spectral Hex attack entry"));
    }
    if gremlin_merc_reachable && !super::monsters::gremlin_merc_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs("Gremlin Merc attack entry"));
    }
    if hopper_reachable {
        let catalog = plan.catalog.ok_or(EngineRefusal::MalformedArgs(
            "Thieving Hopper attack catalog",
        ))?;
        if !super::monsters::thieving_hopper_state_is_valid(state)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(state, catalog)
        {
            return Err(EngineRefusal::MalformedArgs("Thieving Hopper attack entry"));
        }
    }
    if dampen_reachable {
        let catalog = plan
            .catalog
            .ok_or(EngineRefusal::MalformedArgs("Dampen attack catalog"))?;
        if !super::cards::dampen_state_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs("Dampen attack entry"));
        }
    }
    if aeonglass_reachable {
        let catalog = plan
            .catalog
            .ok_or(EngineRefusal::MalformedArgs("Aeonglass attack catalog"))?;
        if !super::cards::aeonglass_state_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs("Aeonglass attack entry"));
        }
    }
    if (hive_reachable || plan.dealer == AttackDealer::PlayerPet) && plan.catalog.is_none() {
        return Err(EngineRefusal::MalformedArgs("Hive attack catalog"));
    }
    if hive_reachable
        || thorns_reachable
        || plan.dealer == AttackDealer::PlayerPet
        || knockdown_reachable
        || hex_reachable
        || gremlin_merc_reachable
        || hopper_reachable
        || dampen_reachable
        || aeonglass_reachable
        || gigantification_reachable
    {
        let mut probe = state.clone();
        let mut probe_events = Vec::new();
        let mut probe_results = Vec::new();
        let probe_has_results = plan.result_sink.is_some();
        player_attack_inner_apply(
            &mut probe,
            source,
            AttackPlan {
                targeting: plan.targeting,
                base: plan.base,
                decimal_base: plan.decimal_base,
                hits: plan.hits,
                source_uid: plan.source_uid,
                frozen_target_identity: plan.frozen_target_identity,
                catalog: plan.catalog,
                context_mode: plan.context_mode,
                result_sink: probe_has_results.then_some(&mut probe_results),
                dealer: plan.dealer,
            },
            fixed_target_identities.as_deref(),
            &mut probe_events,
        )?;
    }
    player_attack_inner_apply(
        state,
        source,
        plan,
        fixed_target_identities.as_deref(),
        events,
    )
}

/// Apply one already-rehearsed powered attack command.
///
/// Personal Hive can publish several fallible physical-card transactions
/// after a surviving damage result. [`player_attack_inner`] runs this whole
/// command on a cloned hot state first whenever that reader is reachable, so
/// an overflow or generated-listener refusal cannot leak the hit preceding
/// the failed suffix.
fn player_attack_inner_apply(
    state: &mut HotState,
    source: &CardSpec,
    mut plan: AttackPlan<'_, '_>,
    fixed_target_identities: Option<&[(i32, u32)]>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // VigorPower.BeforeAttack snapshots the complete live amount for this
    // powered player AttackCommand. Every hit and target reads the snapshot;
    // AfterAttack removes the whole power only when the command completes.
    let vigor = if plan.dealer == AttackDealer::Player {
        state.powers.value(PowerId::Vigor)
    } else {
        0
    };
    // GigantificationPower BeforeAttack/AfterAttack (RVA 0xa2e3c and
    // 0xa2f28; `<AfterAttack>d__8` 0x33b668) binds the first owner-card
    // AttackCommand issued by that player or their Osty, not an individual
    // hit: BeforeAttack stores the command in `Data.commandToModify`
    // (IL_005c-0073), and AfterAttack clears it and decrements (IL_0034-003c). The latch also prevents nested
    // commands from taking a second stack while this command is open. Python
    // `player_attack` keeps the owner CardModel for `PLAYER_PET` (frozen Python, deleted #2827); a non-Attack or absent source never enters this API.
    let gigantification = matches!(plan.dealer, AttackDealer::Player | AttackDealer::PlayerPet)
        && source.is_attack
        && state.fanouts.gigantification() > 0
        && !state.fanouts.gigantification_bound();
    if gigantification && !state.fanouts.set_gigantification_bound(true) {
        return Err(EngineRefusal::MalformedArgs("Gigantification bound state"));
    }
    let one_for_all = one_for_all_additive(state, source, &plan)?;
    let lethality = lethality_multiplier(state, source, &plan)?;
    let strike_dummy = source.strike_tag
        && plan
            .catalog
            .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicStrikeDummy));
    // Ownership only: the source's `IsUpgraded` is re-read per receiver below
    // ([`miniature_cannon_source_is_upgraded`]), because a death inside this
    // command can restore a Dampened source's upgrade (#3245).
    let miniature_cannon = plan
        .catalog
        .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicMiniatureCannon));
    let fake_strike_dummy = source.strike_tag
        && plan
            .catalog
            .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicFakeStrikeDummy));
    let mystic_lighter = source.identity.enchantment.is_some()
        && plan
            .catalog
            .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicMysticLighter));
    let vitruvian_minion = source.row.tags.contains(&"Minion")
        && plan
            .catalog
            .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicVitruvianMinion));
    let paper_phrog = plan
        .catalog
        .is_some_and(|catalog| catalog.hooks().owns(RelicId::RelicPaperPhrog));
    if strike_dummy {
        crate::coverage::record_relic(RelicId::RelicStrikeDummy);
    }
    if fake_strike_dummy {
        crate::coverage::record_relic(RelicId::RelicFakeStrikeDummy);
    }
    if mystic_lighter {
        crate::coverage::record_relic(RelicId::RelicMysticLighter);
    }
    // The command-scoped damaged set SkittishPower.AfterAttack walks, built
    // as `player_attack` (frozen Python, deleted #2827) builds `damaged`, consumed at the AfterAttack
    // tail below. Purely local to this one command: no carrier field, no size
    // pin.
    //
    // Collection is gated on a live Skittish owner being on the roster, and
    // that gate is load-bearing rather than an optimisation: `Vec::new()` does
    // not allocate, but the first `push` does, and this is the attack hot path
    // — collecting unconditionally raised measured allocations per transition
    // from 12 to 13 and broke the ceiling. The gate is observationally exact
    // because the only consumer tests `skittish > 0` per owner anyway, and
    // Skittish is an encounter-entry power (Phantasmal Gardener) that no card
    // or move can apply mid-command, so its presence cannot change inside this
    // loop.
    let skittish_present = state
        .monsters
        .iter()
        .any(|monster| monster.powers.value(PowerId::Skittish) > 0);
    let mut damaged: Vec<u32> = Vec::new();
    let mut pending_hits = plan.hits;
    while pending_hits > 0 {
        pending_hits -= 1;
        // Native's two per-hit exits (#3495), replacing the frozen-Python
        // `if s.over: break`. `AttackCommand/<Execute>d__90::MoveNext`
        // (`0x3f19c0`) re-tests nothing about the combat's outcome per hit:
        // - IL_0156-0161: a dead attacker leaves for the record site. The
        //   player's death, or Osty's, including Osty killed by Thorns while
        //   the combat continues.
        // - IL_0168-01b3: `validTargets` (`IsAlive`, `b__90_1` `0x3f18b6`) is
        //   empty in a live combat (`CombatState.IsLiveCombat` `0x1377db` is
        //   constant true). Each targeting arm below tests it.
        // A won combat has no live enemy, and a lost one has a dead attacker
        // (Kill `0x3ebe90` IL_06ac-06ca kills Osty with its owner), so these
        // cover every exit the `over` test took.
        // The player's death is projected as `hp <= 0` with the outcome
        // latched: `resolve_player_lethal_with_catalog` sets `history.over`
        // once no reviver saved the player, and a revive restores HP instead.
        let attacker_dead = match plan.dealer {
            AttackDealer::Player => state.history.over && state.hp <= 0,
            AttackDealer::PlayerPet => state.fanouts.pet().osty().is_none(),
        };
        if attacker_dead {
            break;
        }
        let hit_targets = match (plan.context_mode, plan.targeting) {
            (AttackContextMode::EchoingSlash, _) => {
                let targets = alive_targets(state);
                if targets.is_empty() {
                    break;
                }
                targets
            }
            (_, AttackTargeting::Random) => match roll_target(state)? {
                Some(target) => vec![target],
                None => break,
            },
            // `validTargets` re-read per hit (IL_0168-IL_0196); an empty set
            // in a live combat leaves the loop (IL_01a6-IL_01b3).
            (_, AttackTargeting::AllOpponents) => {
                let targets = alive_targets(state);
                if targets.is_empty() {
                    break;
                }
                targets
            }
            // `GetPossibleTargets` (`0x1348ac` IL_000c-001f) is the fixed
            // target alone, filtered by `IsAlive`: once none is alive the
            // command leaves rather than entering an empty Damage batch.
            (_, AttackTargeting::Fixed(targets)) => {
                if !targets.iter().any(|target| {
                    state
                        .monsters
                        .get(*target)
                        .is_some_and(|monster| monster.hp > 0)
                }) {
                    break;
                }
                targets.to_vec()
            }
        };
        // This slice's new callbacks use synchronous unpowered batches. A
        // Strike-tagged child issuing an AoE still needs the separate native
        // AttackCommand batch port; never silently accept the old target loop.
        if state.fanouts.synchronous_damage_is_active() && hit_targets.len() > 1 {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "powered multi-target damage inside Damage callback",
            ));
        }
        let result_start = plan.result_sink.as_deref().map_or(0, Vec::len);
        // Native `AttackCommand.<Execute>d__90::MoveNext` (`0x3f19c0`) makes
        // exactly ONE `CreatureCmd.Damage(IEnumerable<Creature>)` per hit:
        // IL_0702-IL_0710 chooses `singleTarget` or the whole live
        // `validTargets` list, IL_0724-IL_0743 issues the call, IL_07a5 folds
        // its frozen results into `_results`. A multi-receiver hit is therefore
        // a native BATCH (`0x3e96c8`), not a per-target damage loop: every
        // receiver commits before the first result callback, and every result
        // callback runs before the first death drains. See
        // [`enter_powered_damage_batch`] for the phase citations.
        //
        // A one-receiver hit takes the unchanged sequential path below: with a
        // single frozen result, "commit all, dispatch all, drain all" and
        // "commit, dispatch, drain" are the same walk, and keeping it avoids
        // re-gating every single-target attack in the crate.
        let batched = hit_targets.len() > 1;
        let frame_depth = state.frames.len();
        let mut committed: Vec<FrozenMonsterDamage> = Vec::new();
        if batched {
            enter_powered_damage_batch(state)?;
        }
        for (target_cursor, target) in hit_targets.into_iter().enumerate() {
            let Some(monster) = state.monsters.get(target) else {
                continue;
            };
            if fixed_target_identities
                .and_then(|identities| identities.get(target_cursor))
                .is_some_and(|identity| (monster.slot, monster.uid) != *identity)
            {
                continue;
            }
            // `player_attack`: `if s.over or m.hp <= 0: continue` (frozen Python, deleted #2827) —
            // the next target, not the next hit.
            //
            // Inside a native batch the combat-over half of that test does not
            // exist: `0x3e96c8` IL_0167-IL_0172 skips a receiver only for
            // `originalTarget.IsDead`, and the one dealer-liveness gate is at
            // Damage ENTRY (IL_0078-IL_00af), before the first receiver. A
            // Thorns receiver that kills the dealer therefore cancels the next
            // HIT (`0x3f19c0` IL_0156-IL_0161) and the entry of the next Damage
            // call, never the receivers already snapshotted in this one.
            if (!batched && state.history.over) || monster.hp <= 0 {
                continue;
            }
            // `CombatState.IterateHookListeners` (`0x3f9720`) rebuilds the
            // listener list on every `Hook.ModifyDamage` enumeration, and
            // `Hook.ModifyDamageInternal` (`0x106aa0` IL_002b, IL_0098) goes
            // through `IRunState.IterateHookListeners`, which is NOT ending
            // gated. Each creature contributes its Powers unconditionally
            // (IL_008f-IL_0097), but a player's relics, potions, orbs and pile
            // cards are gated on `Player.IsActiveForHooks` (IL_00bb-IL_00c2),
            // which `Player.DeactivateHooks` clears inside Kill. So a receiver
            // reached after the dealer's own death still folds the owner's
            // powers and none of the owner's relics — the command-scoped relic
            // flags above cannot hand a later receiver a stale slot. The Boot
            // already carries the identical gate inside the commit.
            let relics_live = !state.fanouts.player_hooks_deactivated();
            let strike_dummy = strike_dummy && relics_live;
            let miniature_cannon = miniature_cannon
                && relics_live
                && miniature_cannon_source_is_upgraded(state, source, &plan)?;
            if miniature_cannon {
                crate::coverage::record_relic(RelicId::RelicMiniatureCannon);
            }
            let fake_strike_dummy = fake_strike_dummy && relics_live;
            let mystic_lighter = mystic_lighter && relics_live;
            let vitruvian_minion = vitruvian_minion && relics_live;
            let paper_phrog = paper_phrog && relics_live;
            let vulnerable = monster.powers.value(PowerId::Vuln) > 0;
            let target_weak = monster.powers.value(PowerId::Weak) > 0;
            let thorns = monster.powers.value(PowerId::Thorns);
            let original_uid = monster.uid;

            // (2) `player_attack`'s additive fold (frozen Python, deleted #2827): `player_strength(s) +
            // s.temp_strength + vigor`, plus Calcify, One For All, Accuracy,
            // Phantom Blades and Strike Dummy. Accuracy is live only for the
            // owner's powered physical Shiv source. Phantom Blades shares
            // those source/dealer/powered gates and contributes only while no
            // own Shiv CardPlayFinished entry exists this turn. The current
            // physical play is recorded after its complete body, so all hits
            // of the first finished Shiv receive the frozen live amount.
            //
            // Red Skull's 3 is not a term here: it is a real `StrengthPower`
            // already in the scalar (#3044,
            // [`red_skull_after_player_hp_changed`]).
            let strength = if plan.dealer == AttackDealer::Player {
                i64::from(state.powers.value(PowerId::Strength))
            } else {
                0
            };
            let temp_strength = if plan.dealer == AttackDealer::Player {
                state.temp_strength
            } else {
                0
            };
            let calcify = if plan.dealer == AttackDealer::PlayerPet {
                state.powers.value(PowerId::Calcify)
            } else {
                0
            };
            let (accuracy, phantom_blades) =
                if plan.dealer == AttackDealer::Player && source.row.tags.contains(&"Shiv") {
                    (
                        state.powers.value(PowerId::Accuracy),
                        if state.history.shiv_plays_finished_this_turn == 0 {
                            state.powers.value(PowerId::PhantomBlades)
                        } else {
                            0
                        },
                    )
                } else {
                    (0, 0)
                };
            let sharp = match source.identity.enchantment {
                Some(enchantment) if matches!(enchantment.id, EnchantmentId::Sharp) => {
                    i64::from(enchantment.amount)
                }
                // `Vigorous::EnchantDamageAdditive` RVA `0xd66a4`: Sharp's
                // `Amount` behind `Status == 0` (IL_0001-000e), read live per
                // hit through `Hook::ModifyDamage` RVA `0x10511c` IL_0041. The
                // Status is slot 5 of the source's physical uid; a source
                // without one cannot be placed on either side of the gate.
                // See `play::vigorous_identity_is_exact`.
                Some(enchantment) if matches!(enchantment.id, EnchantmentId::Vigorous) => {
                    let uid = plan
                        .source_uid
                        .ok_or(EngineRefusal::MalformedArgs("VIGOROUS source uid"))?;
                    if !crate::engine::play::vigorous_identity_is_exact(source) {
                        return Err(EngineRefusal::MalformedArgs("VIGOROUS owner"));
                    }
                    match state
                        .card_states
                        .get_ref(uid)
                        .and_then(|instance| instance.enchantment_state.get())
                    {
                        Some(0) => i64::from(enchantment.amount),
                        Some(1) => 0,
                        _ => return Err(EngineRefusal::MalformedArgs("VIGOROUS status")),
                    }
                }
                // `TezcatarasEmber::EnchantDamageAdditive` RVA `0xd6673`:
                // Sharp's body with `DynamicVars.Damage.BaseValue` — the
                // constant `DamageVar(3m, …)` of `get_CanonicalVars` RVA
                // `0xd6630` — in place of `Amount` (IL_000f-001f). See
                // `play::tezcataras_ember_identity_is_exact`.
                Some(enchantment) if matches!(enchantment.id, EnchantmentId::TezcatarasEmber) => {
                    if !crate::engine::play::tezcataras_ember_identity_is_exact(source) {
                        return Err(EngineRefusal::MalformedArgs("TEZCATARAS_EMBER owner"));
                    }
                    3
                }
                _ => 0,
            };
            let additive = sharp
                .checked_add(strength)
                .and_then(|value| value.checked_add(i64::from(temp_strength)))
                .and_then(|value| value.checked_add(i64::from(vigor)))
                .and_then(|value| value.checked_add(i64::from(calcify)))
                .and_then(|value| value.checked_add(i64::from(one_for_all)))
                .and_then(|value| value.checked_add(i64::from(accuracy)))
                .and_then(|value| value.checked_add(i64::from(phantom_blades)))
                .and_then(|value| value.checked_add(3 * i64::from(strike_dummy)))
                .and_then(|value| value.checked_add(3 * i64::from(miniature_cannon)))
                .and_then(|value| value.checked_add(i64::from(fake_strike_dummy)))
                .and_then(|value| value.checked_add(9 * i64::from(mystic_lighter)))
                .ok_or_else(|| overflow("attack damage"))?;
            // `Corrupted::EnchantDamageMultiplicative` RVA `0xd5eb5` returns
            // `new Decimal(15, 0, 0, 0, 1)` — exactly `3/2` — for a powered
            // Attack and `Decimal.One` otherwise. Python applies it inside
            // `_powered_card_damage_base` (frozen Python, deleted #2827), which multiplies the
            // card's raw DamageVar plus the enchantment additive and only then
            // adds relic_card_damage_bonus and the creature additives. So the
            // factor lands on the base alone, never on the additive fold — and
            // `sharp` above cannot be in that fold at the same time, because a
            // CardModel carries at most one EnchantmentModel.
            let corrupted = source
                .identity
                .enchantment
                .is_some_and(|enchantment| matches!(enchantment.id, EnchantmentId::Corrupted))
                && crate::engine::play::onplay_enchantment_identity_is_exact(source);
            let base = match plan.decimal_base {
                Some(base) => Some(base),
                None if corrupted => Some(DotNetDecimal::from_i64(plan.base)),
                None => None,
            };
            let mut damage = if let Some(base) = base {
                let base = if corrupted {
                    base.checked_mul(
                        DotNetDecimal::ratio(3, 2).map_err(|_| overflow("attack damage"))?,
                    )
                    .map_err(|_| overflow("attack damage"))?
                } else {
                    base
                };
                base.checked_add(DotNetDecimal::from_i64(additive))
                    .map_err(|_| overflow("attack damage"))?
            } else {
                DotNetDecimal::from_i64(
                    plan.base
                        .checked_add(additive)
                        .ok_or_else(|| overflow("attack damage"))?,
                )
            };
            // (3) multiplicative fold in Python order.  Player Weak is the
            // outgoing 3/4 term; Shrink's 7/10 sits after target-side
            // Vulnerable and Tracking.  All factors remain exact decimal
            // values until the one shared floor below.
            damage = damage
                .checked_mul(lethality)
                .map_err(|_| overflow("attack damage"))?;
            if state.powers.value(PowerId::DoubleDamage) > 0 {
                // Current v0.111.0 `DoubleDamagePower::
                // ModifyDamageMultiplicative` RVA 0xa1ca8: one x2 listener
                // for a powered Attack with a non-null CardModel source when
                // the dealer is the owner or an owned Pet. Every caller of
                // this shared path supplies that source, and AttackDealer's
                // closed vocabulary is exactly those two eligible dealers.
                // Amount is duration only, so any positive stack is one x2.
                damage = damage
                    .checked_mul(DotNetDecimal::from_i64(2))
                    .map_err(|_| overflow("attack damage"))?;
            }
            // `PenNib::ModifyDamageMultiplicative` RVA `0x9917c`: a powered
            // attack (IL_000c-IL_0012) with a card source (IL_001a-IL_001c)
            // whose dealer is the owner's Creature (IL_0024-IL_0031) OR the
            // owner's Osty (IL_0033-IL_0040) — any other dealer returns One
            // at IL_0042 — is doubled when that card is `AttackToDouble`
            // (IL_0082-IL_008d). Both [`AttackDealer`] variants are eligible,
            // so an Osty-issued body of the tenth Attack (Poke, #3438) is
            // doubled exactly like the owner's own hit.
            if source.is_attack
                && matches!(plan.dealer, AttackDealer::Player | AttackDealer::PlayerPet)
                && relics_live
                && super::play::active_play_pen_double()
            {
                damage = damage
                    .checked_mul(DotNetDecimal::from_i64(2))
                    .map_err(|_| overflow("Pen Nib attack damage"))?;
                crate::coverage::record_relic(RelicId::RelicPenNib);
            }
            if gigantification {
                // GigantificationPower.ModifyDamageMultiplicative RVA
                // 0xa2a34: x3 inside the shared multiplier fold, before the
                // one final floor, for every target and hit of the command.
                damage = damage
                    .checked_mul(DotNetDecimal::from_i64(3))
                    .map_err(|_| overflow("attack damage"))?;
            }
            // ConquerorPower.ModifyDamageMultiplicative RVA `0xa08b0` is a target-owned,
            // powered-source listener. Its amount is duration, not a stack
            // multiplier: any positive amount applies one x2 exactly to a
            // Sovereign Blade result. Branch on source identity before the
            // sparse monster lookup so every non-Sovereign attack retains its
            // old reader/allocation footprint. Native order places this after
            // Double Damage/Gigantification and before Weak/Vulnerable/
            // Tracking, with one live read per AoE target.
            if matches!(source.identity.id, CardId::SovereignBlade)
                && sovereign_conqueror_is_positive(state, target)?
            {
                damage = damage
                    .checked_mul(DotNetDecimal::from_i64(2))
                    .map_err(|_| overflow("attack damage"))?;
            }
            if plan.dealer == AttackDealer::Player && state.powers.value(PowerId::PlayerWeak) > 0 {
                damage = damage
                    .checked_mul(weak_multiplier(0, false)?)
                    .map_err(|_| overflow("attack damage"))?;
            }
            if vulnerable {
                if paper_phrog && plan.dealer == AttackDealer::Player {
                    crate::coverage::record_relic(RelicId::RelicPaperPhrog);
                }
                damage = damage
                    .checked_mul(vulnerable_multiplier(
                        state,
                        true,
                        state.monsters[target].powers.value(PowerId::Debilitate),
                        paper_phrog && plan.dealer == AttackDealer::Player,
                    )?)
                    .map_err(|_| overflow("attack damage"))?;
            }
            if target_weak {
                let tracking = state.powers.value(PowerId::Tracking);
                if tracking > 0 {
                    damage = damage
                        .checked_mul(
                            DotNetDecimal::ratio(100_i64 + i64::from(tracking), 100)
                                .map_err(|_| overflow("tracking multiplier"))?,
                        )
                        .map_err(|_| overflow("attack damage"))?;
                }
            }
            // Every live KnockdownPower instance is an independent
            // target-owned powered-attack multiplier. The local player is
            // the sole admitted applier and therefore receives x1; its Osty
            // pet is a distinct dealer and receives each positive Amount in
            // acquisition order. This sits after Tracking and before the
            // still-refused Flanking family, with the one final floor below.
            if !knockdown_state_is_exact(monster) {
                return Err(EngineRefusal::MalformedArgs(
                    "Knockdown distinct-instance state",
                ));
            }
            if plan.dealer == AttackDealer::PlayerPet {
                for amount in monster.misery_debuff_order.knockdown() {
                    damage = damage
                        .checked_mul(DotNetDecimal::from_i64(i64::from(*amount)))
                        .map_err(|_| overflow("Knockdown multiplier"))?;
                }
            }
            if vitruvian_minion {
                damage = damage
                    .checked_mul(DotNetDecimal::from_i64(2))
                    .map_err(|_| overflow("attack damage"))?;
                crate::coverage::record_relic(RelicId::RelicVitruvianMinion);
            }
            if plan.dealer == AttackDealer::Player && state.powers.value(PowerId::PlayerShrink) > 0
            {
                damage = damage
                    .checked_mul(
                        DotNetDecimal::ratio(7, 10).map_err(|_| overflow("shrink multiplier"))?,
                    )
                    .map_err(|_| overflow("attack damage"))?;
            }
            if matches!(source.identity.id, CardId::Hang) {
                let hang = state.monsters[target].powers.value(PowerId::Hang);
                if hang > 0 {
                    damage = damage
                        .checked_mul(DotNetDecimal::from_i64(i64::from(hang)))
                        .map_err(|_| overflow("hang multiplier"))?;
                }
            }
            if monster.powers.value(PowerId::Soar) > 0 {
                // SoarPower (#197) is an owner-local x50/100 listener on
                // powered incoming attacks (`player_attack` (frozen Python, deleted #2827)). It is
                // deliberately in this shared multiplicative fold, before
                // the one final floor; direct/unpowered damage never enters
                // this path and therefore never reads Soar.
                damage = damage
                    .checked_mul(
                        DotNetDecimal::ratio(1, 2).map_err(|_| overflow("soar multiplier"))?,
                    )
                    .map_err(|_| overflow("attack damage"))?;
            }
            if monster.powers.value(PowerId::Flutter) > 0 {
                // FlutterPower.ModifyDamageMultiplicative (v0.111.0 RVA
                // `0xa2733`) is the target owner's exact x0.5 term for a
                // powered player attack. The one floor remains below the
                // complete multiplicative walk.
                damage = damage
                    .checked_mul(
                        DotNetDecimal::ratio(1, 2).map_err(|_| overflow("Flutter multiplier"))?,
                    )
                    .map_err(|_| overflow("attack damage"))?;
            }
            if monster.kind == MonsterKind::BygoneEffigy {
                // SlowPower.ModifyDamageMultiplicative (v0.111.0 RVA
                // `0xa7e1c`): owner-only (IL_000c-IL_001a returns Decimal.One
                // for any other target), then `IsPoweredAttack` gated
                // (IL_001b-IL_0028), then `One + 0.1 * SlowAmount` — the
                // Decimal ctor at IL_0029-IL_0033 is `(1,0,0,0,1)`, i.e. 1
                // with scale 1 = 0.1.
                //
                // Both gates are already discharged by this position: the fold
                // is the powered player/Osty command path (direct/unpowered
                // damage never enters it, exactly as the Soar comment above
                // records), and `monster` is the target. Placed immediately
                // after Flutter to match `player_attack` (frozen Python, deleted #2827), whose
                // `Fraction(10 + m.slow, 10)` is the same value expressed to
                // avoid binary float (I7); `DotNetDecimal::ratio` is the exact
                // equivalent here.
                let slow = monster.bygone_effigy_slow();
                if slow > 0 {
                    damage = damage
                        .checked_mul(
                            DotNetDecimal::ratio(i64::from(slow) + 10, 10)
                                .map_err(|_| overflow("Slow multiplier"))?,
                        )
                        .map_err(|_| overflow("attack damage"))?;
                }
            }
            // (4) the single shared floor.
            if damage < DotNetDecimal::zero() {
                damage = DotNetDecimal::zero();
            }

            // (5) Thorns retaliation, before the hit lands.
            if thorns > 0 {
                // The thorny monster is the DEALER of the retaliation
                // (`0x349954` IL_0065–IL_0086 passes `Owner` as the dealer and
                // the incoming dealer as the receiver), so its identity must
                // survive into the owner-side AfterDamageGiven walk. `target`
                // still indexes it here: the retained/replaced-receiver guard
                // below runs only after the retaliation.
                let thorns_owner_uid = original_uid;
                if plan.dealer == AttackDealer::PlayerPet {
                    thorns_retaliation_pet(state, plan.catalog, thorns, thorns_owner_uid, events)?;
                } else {
                    thorns_retaliation(state, plan.catalog, thorns, thorns_owner_uid, events)?;
                }
                // Native Damage0x3e96c8 resumes after BeforeDamageReceived
                // (02b9) directly into block/HP commit (02be–02f3). Dealer death
                // cancels later hits, never the already computed current hit.
                // This deliberately corrects frozen Python `player_attack` (deleted #2827)
                // (its dead dealer return and dead receiver skip);
                // neither shortcut exists at the native await return site.
                // We retain roster identity; a removed/replaced original
                // receiver needs a distinct object projection and refuses.
                // The one admitted retained-dead shape commits native's zero
                // result (see [`thorns_receiver_after_retaliation`]).
                match thorns_receiver_after_retaliation(
                    state,
                    target,
                    original_uid,
                    batched,
                    plan.dealer,
                )? {
                    ThornsReceiver::Live => {}
                    ThornsReceiver::RetainedDeadAtCombatEnd => {
                        commit_thorns_dead_receiver_zero_result(
                            state,
                            target,
                            plan.result_sink.as_deref_mut(),
                            events,
                        );
                        continue;
                    }
                }
            }

            // (6) commit. The realised HP loss decides membership of the
            // command-scoped damaged set that SkittishPower.AfterAttack walks
            // below (`player_attack` (frozen Python, deleted #2827): `damaged.append(m)` iff
            // `damage_monster(...) > 0`). Identity is by uid, not index: a
            // death inside this same command shifts the roster.
            let uid = state.monsters[target].uid;
            if batched {
                // Batch phase 1 only: `0x3e96c8` IL_02be-IL_02f8 subtracts
                // Block and IL_04a4-IL_04c1 commits HP for this receiver, then
                // IL_0a99-IL_0aa4 moves straight to the next one. Nothing
                // dispatches a result and nothing kills until every receiver
                // snapshotted at IL_00c6 has committed.
                if let Some(result) = commit_monster_damage(
                    state,
                    target,
                    damage,
                    true,
                    true,
                    AttackObservation {
                        source_uid: plan.source_uid,
                        result_sink: plan.result_sink.as_deref_mut(),
                        dealer: plan.dealer,
                        catalog: plan.catalog,
                    },
                    events,
                )? {
                    if result.hp_after <= 0 {
                        state.fanouts.register_batch_death(result.uid);
                    }
                    if skittish_present && result.hp_lost_int > 0 {
                        damaged.push(uid);
                    }
                    committed.push(result);
                }
                continue;
            }
            let realised = damage_monster_inner(
                state,
                target,
                damage,
                true,
                true,
                AttackObservation {
                    source_uid: plan.source_uid,
                    result_sink: plan.result_sink.as_deref_mut(),
                    dealer: plan.dealer,
                    catalog: plan.catalog,
                },
                events,
            )?;
            if skittish_present && realised > 0 {
                damaged.push(uid);
            }
            apply_personal_hive_after_powered_hit(state, plan.catalog, target, events)?;
        }
        if batched {
            finish_powered_damage_batch(state, plan.catalog, frame_depth, &committed, events)?;
        }
        if plan.context_mode == AttackContextMode::EchoingSlash {
            let killed = plan.result_sink.as_deref().map_or(0_usize, |results| {
                results[result_start..]
                    .iter()
                    .filter(|result| result.was_target_killed)
                    .count()
            });
            if killed > 0 {
                let killed: i64 = killed
                    .try_into()
                    .map_err(|_| overflow("echoing slash wave count"))?;
                pending_hits = pending_hits
                    .checked_add(killed)
                    .ok_or_else(|| overflow("echoing slash wave count"))?;
            }
        }
    }
    if plan.context_mode == AttackContextMode::Omnislice {
        let Some(results) = plan.result_sink else {
            return Err(EngineRefusal::MalformedArgs("omnislice result context"));
        };
        if let (Some(first), AttackTargeting::Fixed([original])) =
            (results.first().copied(), plan.targeting)
        {
            let spill = first
                .total_damage
                .checked_add(first.overkill_damage)
                .ok_or_else(|| overflow("omnislice spill"))?;
            for target in alive_targets(state) {
                if target == *original || state.history.over {
                    continue;
                }
                let spill_uid = state.monsters[target].uid;
                let spill_realised = damage_monster_inner(
                    state,
                    target,
                    DotNetDecimal::from_i64(i64::from(spill)),
                    false,
                    true,
                    AttackObservation {
                        source_uid: plan.source_uid,
                        result_sink: Some(results),
                        dealer: plan.dealer,
                        catalog: plan.catalog,
                    },
                    events,
                )?;
                if skittish_present && spill_realised > 0 {
                    damaged.push(spill_uid);
                }
            }
        }
    }
    // SkittishPower.AfterAttack (RVA `0xa7ac4`): the command's whole damaged
    // set gains Block equal to Amount, once per turn per owner
    // (`player_attack` (frozen Python, deleted #2827)). The latch is the power instance's own
    // `Data.hasGainedBlockThisTurn` (`get_` `0xa7a9a`), so a monster hit twice
    // by one command — or by two commands in a turn — gains Block once; that
    // is why `damaged` may legitimately hold duplicate uids and this walk does
    // not de-duplicate. `AfterSideTurnEnd` `0xa7b10` clears the latch.
    for uid in damaged {
        if damage_combat_is_ending(state) {
            break;
        }
        let Some(index) = state.monsters.iter().position(|monster| monster.uid == uid) else {
            continue;
        };
        let monster = &state.monsters[index];
        let skittish = monster.powers.value(PowerId::Skittish);
        if monster.hp <= 0 || skittish <= 0 || monster.skittish_used() {
            continue;
        }
        let block = monster
            .block
            .checked_add(skittish)
            .ok_or_else(|| overflow("Skittish block"))?;
        let monster = &mut state.monsters_mut()[index];
        monster.set_skittish_used(true);
        // Monster Block carries no event here, matching Plating's gain in the
        // side-start walk; the canonical projection is what lockstep compares.
        monster.block = block;
    }
    if plan.dealer == AttackDealer::PlayerPet {
        state
            .fanouts
            .mutate_pet(|pet| pet.record_attack())
            .map_err(|_| overflow("osty attacks this turn"))?;
    }
    if vigor != 0 && !damage_combat_is_ending(state) && !state.fanouts.player_hooks_deactivated() {
        state.powers.set(PowerId::Vigor, SlotWire::Int, 0);
        note_power(events, Subject::Player, PowerId::Vigor, 0);
    }
    // AfterAttack (`0x3f19c0` IL_0845) runs after the hit loop even when a hit
    // ended the combat, but `Hook.AfterAttack` (`<AfterAttack>d__3` 0x3cb644
    // IL_001d) walks `Hook.IterateCombatHookListeners`
    // (`<IterateCombatHookListeners>d__0` 0x3d3bc0), which yields nothing
    // while `IsOverOrEnding && !IsStarting` (IL_0028-0042). So a command whose
    // earlier hit ended the combat never reaches
    // `GigantificationPower.AfterAttack`: native keeps `commandToModify`
    // bound and the stack undecremented in the terminal state, as here
    // (#3495). The same gate keeps Vigor above.
    if gigantification
        && !damage_combat_is_ending(state)
        && !state.fanouts.player_hooks_deactivated()
    {
        if !state.fanouts.set_gigantification_bound(false) {
            return Err(EngineRefusal::MalformedArgs("Gigantification bound state"));
        }
        let remaining = state
            .fanouts
            .gigantification()
            .checked_sub(1)
            .ok_or(EngineRefusal::CounterOverflow("gigantification"))?;
        if !state.fanouts.set_gigantification(remaining) {
            return Err(EngineRefusal::MalformedArgs("Gigantification amount"));
        }
    }
    if plan.dealer == AttackDealer::PlayerPet
        && !damage_combat_is_ending(state)
        && !state.fanouts.player_hooks_deactivated()
    {
        let catalog = plan
            .catalog
            .ok_or(EngineRefusal::MalformedArgs("Osty attack catalog"))?;
        super::relics::after_pet_attack(catalog, state, events)?;
        if damage_combat_is_ending(state) {
            return Ok(());
        }
        let flatten_uids = PileId::ALL
            .into_iter()
            .flat_map(|pile| state.piles.get(pile).as_slice())
            .filter_map(|card| {
                catalog
                    .spec(card.atom)
                    .is_some_and(|spec| matches!(spec.identity.id, CardId::Flatten))
                    .then_some(card.uid)
            })
            .collect::<Vec<_>>();
        if !flatten_uids.is_empty() {
            for uid in flatten_uids {
                state.card_states.append_local_cost_modifier(
                    uid,
                    LocalCostModifier {
                        kind: LocalCostModifierKind::Set,
                        amount: 0,
                        expiration: LocalCostExpiration::ThisTurn,
                        reduce_only: false,
                    },
                );
            }
            // Python's `_map_card_id_in_all_piles(..., force_exact=True)`
            // publishes exact mode whenever the retained AfterAttack listener
            // sees at least one Flatten, even when only one copy is live.
            state.exact_piles = true;
        }
    }
    Ok(())
}

/// Open one native powered `CreatureCmd.Damage` batch.
///
/// Authority: archived v0.111.0 `sts2.dll`, SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// `AttackCommand.<Execute>d__90::MoveNext` (RVA `0x3f19c0`) is a per-hit loop.
/// Its head re-reads `Attacker.IsDead` (IL_0156-IL_0161) and rebuilds the live
/// `validTargets` list (IL_0168-IL_0196) before every hit, then issues ONE
/// `CreatureCmd.Damage(IEnumerable<Creature>, …)` for the whole receiver list
/// (IL_0702-IL_0743) and folds the returned results into `_results`
/// (IL_07a5). `CreatureCmd.<Damage>d__12::MoveNext` (RVA `0x3e96c8`) then runs
/// three ordered phases over ONE snapshot (`targets.ToList()`, IL_00c6):
///
/// 1. **commit**, per original receiver: skip a dead receiver
///    (IL_0167-IL_0172), `Hook.ModifyDamage` mask 14 (IL_017a-IL_01b7),
///    `AfterModifyingDamageAmount` (IL_01cf), `BeforeDamageReceived`
///    (IL_0261 — where Thorns retaliates), and on its return, with no further
///    liveness test, `DamageBlockInternal` (IL_02e2-IL_02f8), `ModifyHpLost`
///    (IL_0345/IL_0430) and `LoseHpInternal` (IL_04bc). The per-receiver
///    results join the command list at IL_0a65-IL_0a71 and the loop advances
///    at IL_0a99-IL_0aa4;
/// 2. **result dispatch**, over the now-frozen list (IL_0ad9-IL_0ae4):
///    `AfterBlockBroken` (IL_0b53), `AfterCurrentHpChanged` (IL_0be4),
///    `AfterDamageGiven` (IL_0cc4), then EITHER queue the receiver into
///    `killedCreatures` when the frozen result was lethal and it is still dead
///    (IL_0d21-IL_0d47) OR `AfterDamageReceived` (IL_0d86);
/// 3. **death drain**: one `CreatureCmd.Kill(killedCreatures, false)` at
///    IL_0ead-IL_0eb4.
///
/// That is what #2654's Kaiser Crab needs: a killing AoE commits HP to BOTH
/// crabs in phase 1, so the CrabRage death hook in phase 3 can only ever see a
/// sibling that has already taken its own damage.
///
/// The dealer-liveness gate inside Damage is at ENTRY only (IL_0078-IL_00af,
/// which returns one zero result per target). Nothing re-tests it per receiver,
/// which is why a Thorns kill during phase 1 cancels the next hit and not the
/// rest of this one.
///
/// Entry authentication mirrors the unpowered batch
/// ([`damage_monsters_after_catalog_auth`]): the private pending-death receipt
/// this opens authenticates retained corpse reads during phases 2 and 3, so the
/// Queen/Amalgam roster quotient must be exact first. (Phrog's Infested spawn
/// now runs at its native position after Gremlin Horn, #2656, so a Horn owner
/// no longer refuses here.) The remaining internal-damage entry
/// quotients (Hex, Dampen, Hopper, Aeonglass, Knockdown) are already validated
/// for every powered command by [`player_attack_inner`].
fn enter_powered_damage_batch(state: &mut HotState) -> Result<(), EngineRefusal> {
    if state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        )
    }) && !super::monsters::queen_roster_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs("Queen/Amalgam attack batch"));
    }
    state.fanouts.enter_damage_batch();
    Ok(())
}

/// Run phases 2 and 3 of one powered `CreatureCmd.Damage` batch and close it.
///
/// Phase 2 walks the frozen results in commit order (`0x3e96c8` IL_0ad9-IL_0e84)
/// through the shared [`dispatch_monster_damage_result`], which already carries
/// the AfterBlockBroken/AfterCurrentHpChanged/AfterDamageGiven walk and the
/// lethal-versus-AfterDamageReceived branch at IL_0d21-IL_0d86 and reports
/// whether this result queues a kill. `PersonalHivePower.AfterDamageReceived`
/// (`0x340220`) is part of that hook, so it belongs in the non-lethal branch
/// here — before any death drains — rather than after the receiver's own death
/// as the single-receiver path runs it.
///
/// Phase 3 is the single `Kill(killedCreatures, false)` at IL_0ead-IL_0eb4.
///
/// The retained/replaced-receiver guards are the ones #2651 landed and are
/// deliberately NOT widened: a listener that removes or replaces a queued
/// receiver needs a distinct object projection and refuses here.
fn finish_powered_damage_batch(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    frame_depth: usize,
    results: &[FrozenMonsterDamage],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let mut killed = Vec::new();
    for result in results.iter().copied() {
        if state
            .monsters
            .get(result.target)
            .is_none_or(|monster| monster.uid != result.uid)
        {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "attack Damage retained receiver replacement",
            ));
        }
        if dispatch_monster_damage_result(state, catalog, result, events)? {
            killed.push(result);
        } else {
            if result.hp_after <= 0 {
                state.fanouts.cancel_pending_batch_death(result.uid);
            }
            apply_personal_hive_after_powered_hit(state, catalog, result.target, events)?;
        }
    }
    for result in killed {
        if state
            .monsters
            .get(result.target)
            .is_none_or(|monster| monster.uid != result.uid)
        {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "attack Damage retained death replacement",
            ));
        }
        if state.monsters[result.target].hp > 0 {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "attack Damage restored queued receiver",
            ));
        }
        finish_damage_result_death(state, catalog, result, events)?;
    }
    if state.frames.len() != frame_depth || !state.fanouts.leave_damage_batch() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if !state.fanouts.damage_batch_is_active() && !state.any_monster_alive() && !state.history.over
    {
        state.history.over = true;
        events.push(Event::CombatOver {
            player_won: state.hp > 0,
        });
    }
    Ok(())
}

/// Freeze OneForAllPower's command-scoped additive before the first hit.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `OneForAllPower.ModifyDamageAdditive` (RVA `0xa52d8`) requires a non-null
/// same-owner source (IL_0017-IL_004b), rejects X cost through the live
/// CardPlay card or the bare CardModel (IL_0056-IL_008e), then requires
/// `CardPlay.Resources.EnergySpent == 0` (IL_0093-IL_00ad) or, without a
/// CardPlay, `EnergyCost.GetWithModifiers(-1) == 0` (IL_00ae-IL_00ce).
/// It deliberately does not inspect `CardModel.Type`: a zero-spent Skill or
/// Power that issues an AttackCommand receives the same additive. Python's
/// `_one_for_all_attack_additive` now carries the same source-type-neutral
/// predicate, closing #1796.
fn one_for_all_additive(
    state: &HotState,
    source: &CardSpec,
    plan: &AttackPlan<'_, '_>,
) -> Result<i32, EngineRefusal> {
    let amount = state.powers.value(PowerId::OneForAll);
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("OneForAllPower amount"));
    }
    if amount == 0 || source.x_cost {
        return Ok(0);
    }
    let Some(source_uid) = plan.source_uid else {
        return Ok(if source.cost == 0 { amount } else { 0 });
    };
    let catalog = plan.catalog.ok_or(EngineRefusal::MalformedArgs(
        "OneForAll physical card source",
    ))?;
    live_card_source_pile(
        state,
        catalog,
        source,
        source_uid,
        "OneForAll physical card source",
    )?;
    let spent = super::play::active_card_energy_spent(source_uid).ok_or(
        EngineRefusal::MalformedArgs("OneForAll active play context"),
    )?;
    Ok(if spent == 0 { amount } else { 0 })
}

/// Miniature Cannon's live `CardModel.IsUpgraded` read for one receiver.
///
/// Current v0.111.0 IL (sts2.dll 9cb4f1ad…):
/// - `MiniatureCannon::ModifyDamageAdditive` (RVA `0x96ec8`) gates on
///   `IsPoweredAttack` (IL_000c-IL_0019) and a non-null `cardSource`
///   (IL_001a-IL_0023), then calls `cardSource.IsUpgraded` at IL_0024-IL_002b
///   before the owner gates and the `ExtraDamage` BaseValue (IL_0033-IL_006c).
///   Nothing caches the answer: it is a property read on the live CardModel.
/// - `CreatureCmd/<Damage>d__12::MoveNext` (RVA `0x3e96c8`) calls
///   `Hook.ModifyDamage` at IL_01b2 once per receiver, inside the `targetList`
///   enumeration (IL_0156-IL_0172 skips a dead `originalTarget`), passing the
///   same `cardSource` object each time. `AttackCommand.<Execute>d__90`
///   (RVA `0x3f19c0`) issues one such `Damage` call per hit (IL_0724-IL_0743).
///
/// So each receiver of each hit reads the card's CURRENT upgrade. A source
/// Dampened at L0 whose Magi Knight dies on an earlier hit of the same command
/// has its upgrade restored by `DampenPower::AfterRemoved` (see
/// [`crate::engine::cards::remove_dampen_after_death`]), and every later hit
/// then takes the +3 (#3245, 5URDMEV66CZW n45 One-Two Punch → Conflagration).
///
/// Every catalog-bearing [`AttackPlan`] carries a physical `source_uid`; the
/// live row is found by that UID and must be unique and still the same card
/// identity apart from its upgrade level, or the read refuses by name.
///
/// Cold and out of line: it runs only while Miniature Cannon is owned, and
/// keeping it out of the per-receiver fold leaves that loop's codegen as it was.
#[cold]
#[inline(never)]
fn miniature_cannon_source_is_upgraded(
    state: &HotState,
    source: &CardSpec,
    plan: &AttackPlan<'_, '_>,
) -> Result<bool, EngineRefusal> {
    let (Some(catalog), Some(source_uid)) = (plan.catalog, plan.source_uid) else {
        return Err(EngineRefusal::MalformedArgs(
            "MiniatureCannon physical card source",
        ));
    };
    let mut matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice().iter())
        .filter(|card| card.uid == source_uid);
    let first = matches.next();
    let count = usize::from(first.is_some()) + matches.count();
    let Some(card) = first.filter(|_| count == 1) else {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: count,
        });
    };
    let live = catalog
        .spec(card.atom)
        .filter(|live| {
            CardIdentity {
                upgrade: source.identity.upgrade,
                ..live.identity
            } == source.identity
        })
        .ok_or(EngineRefusal::MalformedArgs(
            "MiniatureCannon physical card source",
        ))?;
    Ok(live.identity.upgrade > 0)
}

/// Freeze LethalityPower's exact command-scoped multiplier before the first
/// hit. Native requires a non-null owned CardModel source, but deliberately
/// does not gate on dealer or Card.Type. A Play source qualifies only at
/// authenticated native `CardPlay.CurrentPlayIndex == 0` **and** while no
/// prior same-owner Attack play has started; its current Attack body makes the
/// owner-filtered count one. A direct non-Play source contributes no current
/// Play entry and therefore qualifies only while that count is zero. The two
/// gates are independent because a non-Attack physical source may itself issue
/// a powered attack on every replay. Current v0.111.0/41cef1ea IL:
/// `LethalityPower.ModifyDamageMultiplicative` (`0xa4634`), active-play index
/// gate IL_0042-IL_005e and owner-filtered history threshold IL_0066-IL_00c0.
fn lethality_multiplier(
    state: &HotState,
    source: &CardSpec,
    plan: &AttackPlan<'_, '_>,
) -> Result<DotNetDecimal, EngineRefusal> {
    let amount = state.powers.value(PowerId::Lethality);
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("LethalityPower amount"));
    }
    let Some(source_uid) = plan.source_uid else {
        return Ok(DotNetDecimal::from_i64(1));
    };
    if amount == 0 {
        return Ok(DotNetDecimal::from_i64(1));
    }
    let catalog = plan.catalog.ok_or(EngineRefusal::MalformedArgs(
        "Lethality physical card source",
    ))?;
    let pile = live_card_source_pile(
        state,
        catalog,
        source,
        source_uid,
        "Lethality physical card source",
    )?;
    let eligible = if pile == PileId::Play {
        let play_index = super::play::active_card_play_index(source_uid).ok_or(
            EngineRefusal::MalformedArgs("Lethality active play context"),
        )?;
        play_index == 0 && state.history.owner_attack_plays_started_this_turn <= 1
    } else {
        state.history.owner_attack_plays_started_this_turn <= 0
    };
    if !eligible {
        return Ok(DotNetDecimal::from_i64(1));
    }
    DotNetDecimal::ratio(100_i64 + i64::from(amount), 100)
        .map_err(|_| overflow("lethality multiplier"))
}

/// PersonalHivePower/<AfterDamageReceived>d__6::MoveNext (current v0.111.0
/// RVA 0x340220; powered gate IL_004d, CreateCard<Dazed> IL_00af,
/// AddGeneratedCardToCombat IL_00d1, loop test IL_015a-IL_0166).
///
/// The power fires once for every powered result its owner survives, even
/// when Block absorbed all damage. Each live Hive stack is a separate
/// null-creator generated Dazed+0 transaction at a random Draw position: one
/// uid, generated-hook token, Shuffle draw, physical-entry tail, pile event,
/// and local generated-power suffix apiece. Owner-created history deliberately
/// does not advance.
fn apply_personal_hive_after_powered_hit(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(owner) = state.monsters.get(target) else {
        return Ok(());
    };
    let hive = owner.powers.value(PowerId::Hive);
    if owner.hp <= 0 || hive == 0 {
        return Ok(());
    }
    let catalog = catalog.ok_or(EngineRefusal::MalformedArgs("Hive attack catalog"))?;
    let count: usize = hive
        .try_into()
        .map_err(|_| EngineRefusal::MalformedArgs("Entomancer Hive amount"))?;
    let identity = CardIdentity {
        id: CardId::Dazed,
        upgrade: 0,
        enchantment: None,
    };
    for _ in 0..count {
        super::cards::inject_generated_null_random(state, catalog, identity, PileId::Draw, events)?;
    }
    Ok(())
}

fn require_live_card_source(
    state: &HotState,
    catalog: &Catalog,
    source: &CardSpec,
    source_uid: u32,
) -> Result<(), EngineRefusal> {
    live_card_source_pile(
        state,
        catalog,
        source,
        source_uid,
        "CurlUp physical card source",
    )?;
    Ok(())
}

fn live_card_source_pile(
    state: &HotState,
    catalog: &Catalog,
    source: &CardSpec,
    source_uid: u32,
    mismatch_site: &'static str,
) -> Result<PileId, EngineRefusal> {
    let mut matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| {
            state
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .map(move |card| (pile, card))
        })
        .filter(|(_, card)| card.uid == source_uid);
    let first = matches.next();
    let second = matches.next();
    let count = usize::from(first.is_some()) + usize::from(second.is_some()) + matches.count();
    if count != 1 {
        return Err(EngineRefusal::ActiveCardNotUnique {
            uid: source_uid,
            matches: count,
        });
    }
    let (pile, card) = first.expect("one live physical card source was proven");
    if catalog.spec(card.atom) != Some(source) {
        return Err(EngineRefusal::MalformedArgs(mismatch_site));
    }
    Ok(pile)
}

/// The roster indices of every living monster, in roster order
/// (`combat_sim.State.alive`).
///
/// A powered all-opponents `AttackCommand` calls this once per hit
/// ([`AttackTargeting::AllOpponents`], #3023). An unpowered all-enemies
/// damage call (one `CreatureCmd.Damage` over a list) snapshots it once.
pub fn alive_targets(state: &HotState) -> Vec<usize> {
    (0..state.monsters.len())
        .filter(|index| state.monsters[*index].hp > 0)
        .collect()
}

/// Select one live monster with `CombatTargets` RNG (`_roll_target`).
///
/// The live pool is rebuilt for every call. Empty pools consume no draw;
/// every nonempty pool, including a singleton, consumes exactly one draw.
pub fn roll_target(state: &mut HotState) -> Result<Option<usize>, EngineRefusal> {
    let targets = alive_targets(state);
    if targets.is_empty() {
        return Ok(None);
    }
    let live = state.rng.get(RngStream::Targets);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound: i32 = targets
        .len()
        .try_into()
        .map_err(|_| overflow("CombatTargets pool"))?;
    let selected: usize = rng
        .next_bounded(bound)
        .map_err(|_| overflow("CombatTargets pool"))?
        .try_into()
        .map_err(|_| overflow("CombatTargets result"))?;
    state.rng.set(
        RngStream::Targets,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(Some(targets[selected]))
}

/// `VulnerablePower`'s multiplier walk (`_vulnerable_multiplier`, frozen Python, deleted #2827).
///
/// Paper Phrog's 7/4 base, Cruelty's additive percentage, and Debilitate's
/// `2x - 1` remap all live in this function in Python. Cruelty is admitted for
/// a player-owned dealer only. Debilitate then remaps the complete preceding
/// multiplier once per live owner stack; Paper Phrog selects the initial base.
fn vulnerable_multiplier(
    state: &HotState,
    player_owned_dealer: bool,
    debilitate: i32,
    paper_phrog: bool,
) -> Result<DotNetDecimal, EngineRefusal> {
    let base = DotNetDecimal::ratio(
        if paper_phrog { 7 } else { 3 },
        if paper_phrog { 4 } else { 2 },
    )
    .map_err(|_| overflow("vulnerable multiplier"))?;
    let cruelty = state.powers.value(PowerId::Cruelty);
    let before_debilitate = if player_owned_dealer && cruelty > 0 {
        base.checked_add(
            DotNetDecimal::ratio(i64::from(cruelty), 100)
                .map_err(|_| overflow("vulnerable multiplier"))?,
        )
        .map_err(|_| overflow("vulnerable multiplier"))?
    } else {
        base
    };
    if debilitate > 0 {
        before_debilitate
            .checked_add(
                before_debilitate
                    .checked_sub(DotNetDecimal::from_i64(1))
                    .map_err(|_| overflow("vulnerable multiplier"))?,
            )
            .map_err(|_| overflow("vulnerable multiplier"))
    } else {
        Ok(before_debilitate)
    }
}

/// `WeakPower`'s ordered multiplier walk (`_monster_weak_multiplier`, frozen Python, deleted #2827).
///
/// Paper Krane's 3/5 base and Debilitate's `2x - 1` remap are the two terms
/// that can move it. Debilitate remaps the base
/// as `x - (1 - x)`. Unlike Vulnerable this one is *not* dyadic-safe by accident — 3/4 is
/// exact in [`DotNetDecimal`], and the single truncation at the end of the
/// snapshot is what makes the fractional result observable at all.
fn weak_multiplier(debilitate: i32, paper_krane: bool) -> Result<DotNetDecimal, EngineRefusal> {
    let base = DotNetDecimal::ratio(3, if paper_krane { 5 } else { 4 })
        .map_err(|_| overflow("weak multiplier"))?;
    if debilitate > 0 {
        base.checked_sub(
            DotNetDecimal::from_i64(1)
                .checked_sub(base)
                .map_err(|_| overflow("weak multiplier"))?,
        )
        .map_err(|_| overflow("weak multiplier"))
    } else {
        Ok(base)
    }
}

/// One damage instance against a monster (`combat_sim.damage_monster`, frozen Python, deleted #2827).
///
/// Returns the integer HP actually lost. `blockable` mirrors the native
/// `ValueProp.Unblockable` bit: an unblockable instance skips block but not
/// the HP-loss or damage-received bookkeeping.
pub fn damage_monster(
    state: &mut HotState,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    damage_monster_with_context(state, None, target, amount, powered, blockable, events)
}

fn damage_monster_with_context(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let queen_roster = state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        )
    });
    let hex_carrier = state.powers.value(PowerId::HexPower) > 0;
    let gremlin_merc_reachable = state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::GremlinMerc | MonsterKind::FatGremlin
        )
    });
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_owner_reachable(state);
    let knockdown_reachable = require_live_knockdown_state(state)?;
    if hopper_reachable {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper damage catalog",
        ));
    }
    if dampen_reachable {
        return Err(EngineRefusal::MalformedArgs("Dampen damage catalog"));
    }
    if aeonglass_reachable {
        return Err(EngineRefusal::MalformedArgs("Aeonglass damage catalog"));
    }
    if queen_roster || hex_carrier || gremlin_merc_reachable || knockdown_reachable {
        if queen_roster && !super::monsters::queen_roster_state_is_valid(state) {
            return Err(EngineRefusal::MalformedArgs("Queen/Amalgam damage entry"));
        }
        if hex_carrier && !super::cards::hex_power_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs("Spectral Hex damage entry"));
        }
        if gremlin_merc_reachable && !super::monsters::gremlin_merc_state_is_valid(state) {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc damage entry"));
        }
        let mut next = state.clone();
        let mut emitted = Vec::new();
        let lost = damage_monster_inner(
            &mut next,
            target,
            amount,
            powered,
            blockable,
            AttackObservation {
                source_uid: None,
                result_sink: None,
                dealer: AttackDealer::Player,
                catalog,
            },
            &mut emitted,
        )?;
        *state = next;
        events.extend(emitted);
        return Ok(lost);
    }
    damage_monster_inner(
        state,
        target,
        amount,
        powered,
        blockable,
        AttackObservation {
            source_uid: None,
            result_sink: None,
            dealer: AttackDealer::Player,
            catalog,
        },
        events,
    )
}

/// Catalog-authenticated direct damage for the fixed Hopper/DeckVersion unit.
///
/// A bare [`damage_monster`] call cannot prove that cold master atoms and
/// physical payloads belong to the caller's catalog, so that seam remains
/// fail-closed whenever Hopper is reachable.
pub fn damage_monster_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    require_live_knockdown_state(state)?;
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_reachable(state, catalog);
    if !hopper_reachable
        && !dampen_reachable
        && !aeonglass_reachable
        && !state.fanouts.gremlin_horn_owned()
    {
        return damage_monster_with_context(
            state,
            Some(catalog),
            target,
            amount,
            powered,
            blockable,
            events,
        );
    }
    if hopper_reachable
        && (!super::monsters::thieving_hopper_state_is_valid(state)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(state, catalog))
    {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper damage entry"));
    }
    if dampen_reachable && !super::cards::dampen_state_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs("Dampen damage entry"));
    }
    if aeonglass_reachable && !super::cards::aeonglass_state_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs("Aeonglass damage entry"));
    }
    let mut next = state.clone();
    let mut emitted = Vec::new();
    let lost = damage_monster_inner(
        &mut next,
        target,
        amount,
        powered,
        blockable,
        AttackObservation {
            source_uid: None,
            result_sink: None,
            dealer: AttackDealer::Player,
            catalog: Some(catalog),
        },
        &mut emitted,
    )?;
    *state = next;
    events.extend(emitted);
    Ok(lost)
}

/// Preserve the caller's catalog through an already-authenticated command.
/// Native v111 Damage `0x3e96c8` awaits Kill `0x3ebe90`, whose AfterDeath
/// listeners may draw and AutoPlay physical cards before Damage resumes.
/// An orb or power callback therefore needs the same catalog as its parent
/// command; authentication is not permission to discard that context.
pub(crate) fn damage_monster_after_catalog_auth(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    damage_monster_with_optional_catalog(
        state,
        Some(catalog),
        target,
        amount,
        powered,
        blockable,
        events,
    )
}

/// Native CreatureCmd.Damage (v111 0x3e96c8) snapshots its targets (00c6),
/// commits every target through 0aa4, dispatches frozen results (0ad9–0d86),
/// then drains the retained killed list (0eb4). A death-triggered draw must
/// therefore observe ALL committed HP, not a partly damaged target list.
///
/// This internal entry inherits the caller's catalog authentication and atomic
/// public action checkpoint. Private execution receipts only justify pending
/// corpse fields during synchronous callbacks; they cannot cross a cold boundary.
pub(crate) fn damage_monsters_after_catalog_auth(
    state: &mut HotState,
    catalog: &Catalog,
    targets: &[usize],
    amount: DotNetDecimal,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    damage_monsters_with_optional_catalog(state, Some(catalog), targets, amount, blockable, events)
}

/// [`damage_monsters_after_catalog_auth`] for an internal caller that carries
/// its parent command's optional catalog, as the single-target
/// [`damage_monster_with_optional_catalog`] does (Inferno's nested Damage,
/// #3274). `Some` is exactly the authenticated batch.
fn damage_monsters_with_optional_catalog(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    targets: &[usize],
    amount: DotNetDecimal,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.hp <= 0 {
        return Ok(());
    }
    if targets.len() <= 1 {
        if let Some(target) = targets.first() {
            damage_monster_with_optional_catalog(
                state, catalog, *target, amount, false, blockable, events,
            )?;
        }
        return Ok(());
    }
    // Preserve the same authentication surface as the former single-target
    // helper before creating any private pending-death receipt.
    if state
        .monsters
        .iter()
        .any(|m| matches!(m.kind, MonsterKind::Queen | MonsterKind::TorchHeadAmalgam))
        && !super::monsters::queen_roster_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs("Queen/Amalgam damage entry"));
    }
    if state
        .monsters
        .iter()
        .any(|m| matches!(m.kind, MonsterKind::GremlinMerc | MonsterKind::FatGremlin))
        && !super::monsters::gremlin_merc_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs("Gremlin Merc damage entry"));
    }
    if state.powers.value(PowerId::HexPower) > 0 && !super::cards::hex_power_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Spectral Hex damage entry"));
    }
    if state.card_states.dampen().is_some() && !super::cards::dampen_internal_state_is_exact(state)
    {
        return Err(EngineRefusal::MalformedArgs("Dampen internal damage entry"));
    }
    if (state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|m| m.kind == MonsterKind::ThievingHopper))
        && !super::monsters::thieving_hopper_internal_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper internal damage entry",
        ));
    }
    if super::cards::aeonglass_owner_reachable(state)
        && !super::cards::aeonglass_internal_state_is_exact(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Aeonglass internal damage entry",
        ));
    }
    require_live_knockdown_state(state)?;
    let frame_depth = state.frames.len();
    state.fanouts.enter_damage_batch();
    let mut results = Vec::with_capacity(targets.len());
    for &target in targets {
        if let Some(result) = commit_monster_damage(
            state,
            target,
            amount,
            false,
            blockable,
            AttackObservation {
                source_uid: None,
                result_sink: None,
                dealer: AttackDealer::Player,
                catalog,
            },
            events,
        )? {
            if result.hp_after <= 0 {
                state.fanouts.register_batch_death(result.uid);
            }
            results.push(result);
        }
    }
    let mut killed = Vec::new();
    for result in results {
        if state
            .monsters
            .get(result.target)
            .is_none_or(|monster| monster.uid != result.uid)
        {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Damage retained receiver replacement",
            ));
        }
        if dispatch_monster_damage_result(state, catalog, result, events)? {
            killed.push(result);
        } else if result.hp_after <= 0 {
            state.fanouts.cancel_pending_batch_death(result.uid);
        }
    }
    for result in killed {
        if state
            .monsters
            .get(result.target)
            .is_none_or(|monster| monster.uid != result.uid)
        {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Damage retained death replacement",
            ));
        }
        if state.monsters[result.target].hp > 0 {
            return Err(EngineRefusal::PowerOrderNotModeled(
                "Damage restored queued receiver",
            ));
        }
        finish_damage_result_death(state, catalog, result, events)?;
    }
    if state.frames.len() != frame_depth || !state.fanouts.leave_damage_batch() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if !state.fanouts.damage_batch_is_active() && !state.any_monster_alive() && !state.history.over
    {
        state.history.over = true;
        events.push(Event::CombatOver {
            player_won: state.hp > 0,
        });
    }
    Ok(())
}

/// Preserve the authenticated catalog through internal retaliation and deaths.
fn damage_monster_with_optional_catalog(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_owner_reachable(state);
    if !hopper_reachable
        && !dampen_reachable
        && !aeonglass_reachable
        && !state.fanouts.gremlin_horn_owned()
    {
        return damage_monster_with_context(
            state, catalog, target, amount, powered, blockable, events,
        );
    }
    if hopper_reachable && !super::monsters::thieving_hopper_internal_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper internal damage entry",
        ));
    }
    if dampen_reachable && !super::cards::dampen_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Dampen internal damage entry"));
    }
    if aeonglass_reachable && !super::cards::aeonglass_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Aeonglass internal damage entry",
        ));
    }
    let mut next = state.clone();
    let mut emitted = Vec::new();
    let lost = damage_monster_inner(
        &mut next,
        target,
        amount,
        powered,
        blockable,
        AttackObservation {
            source_uid: None,
            result_sink: None,
            dealer: AttackDealer::Player,
            catalog,
        },
        &mut emitted,
    )?;
    *state = next;
    events.extend(emitted);
    Ok(lost)
}

fn damage_monster_inner(
    state: &mut HotState,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    observation: AttackObservation<'_>,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let catalog = observation.catalog;
    let Some(result) = commit_monster_damage(
        state,
        target,
        amount,
        powered,
        blockable,
        observation,
        events,
    )?
    else {
        return Ok(0);
    };
    if dispatch_monster_damage_result(state, catalog, result, events)? {
        finish_damage_result_death(state, catalog, result, events)?;
    }
    Ok(result.hp_lost_int)
}

fn finish_damage_result_death(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    result: FrozenMonsterDamage,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let FrozenMonsterDamage { target, kind, .. } = result;
    if kind == MonsterKind::ThievingHopper
        || state.card_states.dampen().is_some()
        || kind == MonsterKind::Aeonglass
        || state.fanouts.damage_batch_is_active()
    {
        // Hopper/Dampen/Aeonglass public damage boundaries already own the complete
        // catalog-bearing command checkpoint. Enter the private death
        // suffix directly so it can authenticate the exact post-damage
        // transient rather than require another stable-root proof after
        // HP has been committed.
        finish_monster_death_inner(state, target, catalog, events)?;
    } else if let Some(catalog) = catalog {
        finish_monster_death_with_catalog(state, catalog, target, events)?;
    } else {
        finish_monster_death(state, target, events)?;
    }
    Ok(())
}

fn commit_monster_damage(
    state: &mut HotState,
    target: usize,
    amount: DotNetDecimal,
    powered: bool,
    blockable: bool,
    observation: AttackObservation<'_>,
    events: &mut Vec<Event>,
) -> Result<Option<FrozenMonsterDamage>, EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Ok(None);
    };
    if monster.hp <= 0 {
        return Ok(None);
    }
    let block_before = monster.block;
    let hp_before = monster.hp;
    let uid = monster.uid;
    let kind = monster.kind;
    let plow_threshold = monster.powers.value(PowerId::PlowThreshold);
    let intangible = monster.powers.value(PowerId::Intangible);
    if plow_threshold > 0 && !super::monsters::beast_plow_damage_owner_is_valid(state, target) {
        return Err(EngineRefusal::MalformedArgs(
            "Ceremonial Beast Plow owner/state",
        ));
    }
    if powered && monster.powers.value(PowerId::CurlUp) > 0 && observation.source_uid.is_none() {
        return Err(EngineRefusal::MalformedArgs("CurlUp physical card source"));
    }

    let intangible_owner_exact = match kind {
        MonsterKind::SoulFysh => {
            matches!(intangible, 1 | 2)
                && (super::monsters::soul_fysh_state_is_valid(state)
                    || super::monsters::soul_fysh_side_end_state_is_valid(state))
        }
        MonsterKind::TestSubject => {
            intangible == 1 && super::monsters::test_subject_state_is_valid(state)
        }
        _ => false,
    };
    if intangible != 0 && !intangible_owner_exact {
        return Err(EngineRefusal::MalformedArgs(
            "monster Intangible owner/state",
        ));
    }

    // Intangible caps the pre-block amount, before block subtraction and for
    // both blockable and unblockable damage. Soul Fysh and Test Subject are
    // the admitted monster owners.
    let amount = if intangible > 0 {
        amount.min(DotNetDecimal::from_i64(1))
    } else {
        amount
    };
    // HardToKillPower.ModifyDamageCap `0x24dd44` (#2481 slot 3) is the second
    // pre-block cap and sits immediately after Intangible, matching
    // `damage_monster` (frozen Python, deleted #2827). `CreatureCmd.<Damage>d__12::MoveNext`
    // `0x432a14` calls `Hook.ModifyDamage` with mask 14 for every Damage
    // command, and the cap ignores both `DamageProps` and powered status — so
    // blockable/unblockable and powered/unpowered instances share it, exactly
    // like the Intangible cap above.
    let hard_to_kill = state.monsters[target].powers.value(PowerId::HardToKill);
    let amount = if hard_to_kill > 0 {
        amount.min(DotNetDecimal::from_i64(i64::from(hard_to_kill)))
    } else {
        amount
    };
    let block_decimal = DotNetDecimal::from_i64(i64::from(block_before));
    let blocked = if blockable {
        block_decimal.min(amount)
    } else {
        DotNetDecimal::zero()
    };
    // Creature.DamageBlockInternal converts the blocked Decimal to Int32
    // before subtracting it from the integer Block field.
    let blocked_int: i32 = blocked
        .trunc_i64()
        .map_err(|_| overflow("blocked damage"))?
        .try_into()
        .map_err(|_| overflow("blocked damage"))?;

    let mut hp_lost = amount
        .checked_sub(blocked)
        .map_err(|_| overflow("unblocked damage"))?;
    if hp_lost < DotNetDecimal::zero() {
        hp_lost = DotNetDecimal::zero();
    }
    // Skulking Colony's hardened shell (`Hook::ModifyHpLost`,
    // `damage_monster` (frozen Python, deleted #2827)): each further loss this side turn is capped at
    // whatever remains of `HARDENED_SHELL` after the window already realised,
    // and floored at zero so an already-exhausted window absorbs completely.
    // Python places it immediately before the Slippery cap below, and both
    // before the after-osty evaluator.
    if kind == MonsterKind::SkulkingColony {
        let remaining = HARDENED_SHELL
            .checked_sub(state.monsters[target].skulking_colony_shell_window())
            .ok_or_else(|| overflow("Skulking Colony shell window"))?;
        let remaining = DotNetDecimal::from_i64(i64::from(remaining.max(0)));
        if hp_lost > remaining {
            hp_lost = remaining;
        }
    }
    // SlipperyPower.ModifyHpLostAfterOsty `0xab329` is owner-only and returns
    // `Decimal.One` iff the incoming post-block loss is >= 1 — a cap to one,
    // not a floor (`damage_monster` (frozen Python, deleted #2827)).
    //
    // Order is load-bearing and is the reason this sits ABOVE The Boot rather
    // than at the old placeholder comment below it. Python applies the cap and
    // only then calls `_evaluate_player_hp_loss(..., after_osty_only=True)`,
    // which is where The Boot's 1 -> 5 raise lives; the Boot is
    // `AfterOstyLate`, so it may legitimately raise this capped one back to
    // five for an eligible powered owner hit. Applying the cap after the Boot
    // instead would turn that 5 into a 1 — the arithmetic would be silently
    // wrong rather than refused.
    if state.monsters[target].powers.value(PowerId::Slippery) > 0
        && hp_lost >= DotNetDecimal::from_i64(1)
    {
        hp_lost = DotNetDecimal::from_i64(1);
    }
    if powered
        && state.the_boot_owned()
        && !state.fanouts.player_hooks_deactivated()
        && hp_lost >= DotNetDecimal::from_i64(1)
        && hp_lost < DotNetDecimal::from_i64(5)
    {
        hp_lost = DotNetDecimal::from_i64(5);
        crate::coverage::record_relic(RelicId::RelicTheBoot);
    }
    // Colony's hardened-shell cap sits with the Slippery cap above in Python
    // (`damage_monster` (frozen Python, deleted #2827), immediately before it) and remains refused.
    let raw = hp_lost
        .trunc_i64()
        .map_err(|_| overflow("monster hp loss"))?;
    let hp_lost_int: i32 = raw
        .clamp(0, HP_LOSS_CLAMP)
        .try_into()
        .map_err(|_| overflow("monster hp loss"))?;
    if state.hand_drill_owned()
        && !state.fanouts.player_hooks_deactivated()
        && state.powers.value(PowerId::Vicious) > 0
        && blocked_int > 0
        && block_before - blocked_int <= 0
        && hp_before - hp_lost_int > 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "Hand Drill block break with Vicious requires a catalog-bearing command",
        ));
    }

    let monster = &mut state.monsters_mut()[target];
    monster.block -= blocked_int;
    // Python `damage_monster` latches `block_broken` at the subtraction
    // (frozen Python, deleted #2827): the flag is
    // frozen before any AfterDamageReceived consumer can run. Re-deriving it
    // from the post-hook Block value would go silently wrong the day a
    // block-granting consumer (Curl Up, monster Plating) is ported, so the
    // latch is taken here even though nothing admitted can move Block yet.
    let block_broken = blocked_int > 0 && monster.block <= 0;
    monster.hp -= hp_lost_int;
    // CombatHistory records DamageReceived before the damage-given and death
    // hooks. A result counts even when both its amounts are zero.
    if powered {
        let (counter, label) = match observation.dealer {
            AttackDealer::Player => (
                &mut monster.owner_powered_damage_results_this_turn,
                "owner_powered_damage_results_this_turn",
            ),
            AttackDealer::PlayerPet => (
                &mut monster.nonowner_same_side_powered_damage_results_this_turn,
                "nonowner_same_side_powered_damage_results_this_turn",
            ),
        };
        *counter = counter.checked_add(1).ok_or_else(|| overflow(label))?;
    }
    let hp_after = monster.hp;
    events.push(Event::MonsterDamaged {
        uid,
        blocked: blocked_int,
        unblocked: hp_lost_int,
        hp: hp_after,
    });
    if let Some(results) = observation.result_sink {
        results.push(AttackDamageResult {
            target,
            receiver_uid: uid,
            total_damage: blocked_int
                .checked_add(hp_lost_int.min(hp_before))
                .ok_or_else(|| overflow("attack result total damage"))?,
            overkill_damage: hp_lost_int.saturating_sub(hp_before).max(0),
            was_target_killed: hp_lost_int >= hp_before,
        });
    }

    Ok(Some(FrozenMonsterDamage {
        target,
        uid,
        kind,
        hp_after,
        block_broken,
        blocked_int,
        hp_lost_int,
        powered,
        source_uid: observation.source_uid,
        dealer: observation.dealer,
    }))
}

/// Native Damage retains these result facts across the whole target batch.
#[derive(Clone, Copy)]
struct FrozenMonsterDamage {
    target: usize,
    uid: u32,
    kind: MonsterKind,
    hp_after: i32,
    block_broken: bool,
    blocked_int: i32,
    hp_lost_int: i32,
    powered: bool,
    source_uid: Option<u32>,
    dealer: AttackDealer,
}

fn dispatch_monster_damage_result(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    result: FrozenMonsterDamage,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let FrozenMonsterDamage {
        target,
        uid,
        kind,
        hp_after,
        block_broken,
        blocked_int,
        hp_lost_int,
        powered,
        source_uid,
        dealer,
    } = result;
    // Native CreatureCmd.<Damage>d__12::MoveNext 0x3e96c8 dispatches
    // AfterBlockBroken (IL_0b53) before AfterDamageGiven (IL_0cc4).
    // Keep the outer result frozen: a listener's nested kill owns its own
    // death suffix and must not cause this result to dispatch Kill again.
    if block_broken {
        after_monster_block_broken(state, catalog, target, events)?;
    }
    // Native AfterCurrentHpChanged runs here (Damage0x3e96c8 IL0be4).
    // The complete v111 override census has no monster-local gameplay writer:
    // Crab handlers are animations, MeatOnTheBone only updates owner UI, and
    // NecroMastery filters Osty. RedSkull is global (0x32f5ec/0x32f790) and
    // does run here, but on a monster's HP change it is a no-op: its latch
    // equals the player's HP quotient, which only player HP writes move, and
    // each of those runs `red_skull_after_player_hp_changed` itself (#3044).
    // The one stale latch the port admits (the opening's Planisphere case) is
    // consumed by the turn-one Blood Vial heal before anything reads Strength,
    // so a turn-start monster HP change that would resync it earlier natively
    // is unobservable (`entry::opening::red_skull_room_entry_lifts`).
    let sic_em_amount = state.monsters[target].powers.value(PowerId::SicEm);
    powers_after_damage_given(
        state,
        catalog,
        DamageGivenResult {
            target,
            receiver_uid: uid,
            pending_kill: hp_after <= 0,
            blocked: blocked_int,
            unblocked: hp_lost_int,
            powered,
            dealer,
        },
        events,
    )?;

    // Sic Em belongs to the damaged target and runs after the complete
    // player-power walk, but before that target's Kill. A lethal pet hit can
    // therefore re-summon Osty before death cleanup.
    if powered && dealer == AttackDealer::PlayerPet {
        // Native freezes the PowerModel, then tests owner membership rather
        // than current power attachment (IterateHookListeners 0x3f9720,
        // Contains 0x137564). A retained corpse still dispatches its detached
        // Sic Em instance; a fresh Stock replacement is a different receiver.
        let retained_owner = matches!(
            kind,
            MonsterKind::TestSubject
                | MonsterKind::Parafright
                | MonsterKind::EyeWithTeeth
                | MonsterKind::DecimillipedeSegment
                | MonsterKind::WaterfallGiant
        );
        let amount = if state.monsters[target].uid == uid
            && (hp_after <= 0 || state.monsters[target].hp > 0 || retained_owner)
        {
            sic_em_amount
        } else {
            0
        };
        // `SicEmPower/<AfterDamageGiven>d__6` (0x3447a0) IL_00a7 awaits
        // `OstyCmd.Summon`. On the killing hit of the last enemy the combat is
        // already ending, so the re-summon raises MaxHp and its Heal returns
        // early (`super::summon_osty`, #3246).
        if amount > 0 {
            super::summon_osty(state, amount, "Sic Em summon")?;
        }
    }

    // Native Damage (0x3e96c8, IL_0d27..0d86) skips AfterDamageReceived
    // only when this frozen result was lethal AND the target remains dead.
    // A nonlethal result whose Given listener killed the target still takes
    // the received-hook branch; the nested command already owned Kill.
    let target_killed = hp_after <= 0 && state.monsters[target].hp <= 0;
    if !target_killed && state.monsters[target].uid == uid {
        // CurlUpPower.AfterDamageReceived runs after the represented player-owned
        // AfterDamageGiven listeners and before Kill. It records the first
        // powered physical source even when Block absorbed the complete hit.
        if powered && state.monsters[target].powers.value(PowerId::CurlUp) > 0 {
            let source_uid =
                source_uid.ok_or(EngineRefusal::MalformedArgs("CurlUp physical card source"))?;
            let monster = &mut state.monsters_mut()[target];
            // First physical card latches; further results from that same card are
            // tolerated; a different card is rejected until its own AfterCardPlayed.
            if monster.curl_up_card_uid == -1 {
                monster.curl_up_card_uid = source_uid as i32;
            }
        }

        // PlowPower.AfterDamageReceived (`damage_monster`, frozen Python, deleted #2827) runs after the
        // player-owned AfterDamageGiven walk and before Kill. Any positive
        // unblocked result crossing the current-HP threshold removes all shared
        // and temporary Strength, removes Plow, then installs one cosmetic
        // STUN_MOVE whose dynamic successor is BEAST_CRY_MOVE.
        let plow = state.monsters[target].powers.value(PowerId::PlowThreshold);
        if kind == MonsterKind::CeremonialBeast
            && plow > 0
            && hp_lost_int > 0
            && (1..=plow).contains(&state.monsters[target].hp)
        {
            let live = state.monsters[target].hp > 0 && !state.history.over;
            let upkeep = state.fanouts.misery_attachment_upkeep();
            let monster = &mut state.monsters_mut()[target];
            // Plow removes both instances outright — no side-end restoration
            // runs — so every temporary-Strength wrapper row goes with the
            // wrappers it describes (#2693 S2), and the Strength row goes at
            // exactly the `ShouldRemoveDueToAmount` `0x83b0d` zero edge.
            clear_monster_temp_strength_wrappers(monster, upkeep);
            write_monster_strength(monster, 0, crate::hot::Applier::None, upkeep);
            monster.powers.set(PowerId::PlowThreshold, SlotWire::Int, 0);
            if live {
                monster.override_state = MonsterOverride::BeastStun;
                monster.forced_follow_up = MonsterFollowUp::BeastCry;
            }
            note_power(events, Subject::Monster(uid), PowerId::TempStrength, 0);
            note_power(events, Subject::Monster(uid), PowerId::Strength, 0);
            note_power(events, Subject::Monster(uid), PowerId::PlowThreshold, 0);
        }

        if kind == MonsterKind::ThievingHopper
            && powered
            && hp_lost_int > 0
            && state.monsters[target].powers.value(PowerId::Flutter) > 0
        {
            let amount = state.monsters[target]
                .powers
                .value(PowerId::Flutter)
                .checked_sub(1)
                .ok_or_else(|| overflow("Flutter amount"))?;
            let can_stun = state.monsters[target].hp > 0 && !state.history.over;
            let monster = &mut state.monsters_mut()[target];
            monster.powers.set(PowerId::Flutter, SlotWire::Int, amount);
            note_power(events, Subject::Monster(uid), PowerId::Flutter, amount);
            if amount == 0 && can_stun {
                let follow_up = match monster.loop_pos {
                    2 => MonsterFollowUp::HopperNab,
                    3 | 4 => MonsterFollowUp::HopperEscape,
                    _ => {
                        return Err(EngineRefusal::MalformedArgs(
                            "Flutter delayed stun successor",
                        ));
                    }
                };
                monster.override_state = MonsterOverride::Stunned;
                monster.forced_follow_up = follow_up;
            }
        }

        // The realised unblocked damage joins the Colony's side-turn window
        // (`damage_monster` (frozen Python, deleted #2827)), after the cap above has already
        // limited it. The window is reset at the owner's own side start and again
        // in `begin_player_turn`.
        if kind == MonsterKind::SkulkingColony && hp_lost_int != 0 && state.monsters[target].hp > 0
        {
            let next = state.monsters[target]
                .skulking_colony_shell_window()
                .checked_add(hp_lost_int)
                .ok_or_else(|| overflow("Skulking Colony shell window"))?;
            let monster = &mut state.monsters_mut()[target];
            if !monster.set_skulking_colony_shell_window(next) {
                return Err(overflow("Skulking Colony shell window"));
            }
        }

        // SlumberPower.AfterDamageReceived 0x34500c: one decrement for
        // each positive UnblockedDamage, with no attack/property filter.
        // The enclosing native receiver gate suppresses lethal results.
        // At zero, Stun queues WakeUpMove -> ROLL_OUT_MOVE; Plating remains
        // until that queued move actually executes.
        if kind == MonsterKind::SlumberingBeetle
            && !state.history.over
            && state.monsters[target].hp > 0
            && state.monsters[target].powers.value(PowerId::Slumber) > 0
            && hp_lost_int > 0
        {
            null_applier_power_amount_changed_is_exact(state)?;
            let monster = &mut state.monsters_mut()[target];
            let amount = monster.powers.value(PowerId::Slumber) - 1;
            monster.powers.set(PowerId::Slumber, SlotWire::Int, amount);
            note_power(events, Subject::Monster(uid), PowerId::Slumber, amount);
            if amount == 0 {
                monster.override_state = MonsterOverride::BeetleWake;
            }
        }

        // AsleepPower `<AfterDamageReceived>d__4::MoveNext` RVA `0x335094`
        // (v0.111.0 DLL `9cb4f1ad…`; `damage_monster` (frozen Python, deleted #2827)): only
        // for `target == Owner` (`IL_0030`-`IL_003e`) and a nonzero
        // `UnblockedDamage` (`IL_0043`-`IL_0050`), it removes Plating first
        // (`IL_0055`-`IL_006d`), sets `IsAwake` (`IL_0156`-`IL_015d`), stuns
        // to the cosmetic `WAKE_UP_MOVE` with a `SLASH_MOVE` follow-up
        // (`IL_0162`-`IL_017e`), and only then removes Asleep itself
        // (`IL_01db`-`IL_01dc`). No command between them reads Asleep, so the
        // removal order below is not observable. `Creature.StunInternal`
        // rejects a dead target, so a lethal wake clears both powers without
        // installing any state — which is why the removals happen
        // unconditionally and only the stun is gated on survival.
        if kind == MonsterKind::LagavulinMatriarch
            && state.monsters[target].powers.value(PowerId::Asleep) > 0
            && hp_lost_int > 0
        {
            let uid = state.monsters[target].uid;
            let survives = state.monsters[target].hp > 0 && !state.history.over;
            let already_forced =
                state.monsters[target].forced_follow_up != crate::hot::MonsterFollowUp::None;
            let monster = &mut state.monsters_mut()[target];
            monster.powers.set(PowerId::Mplating, SlotWire::Int, 0);
            monster.powers.set(PowerId::Asleep, SlotWire::Int, 0);
            if survives && !already_forced {
                monster.override_state = crate::hot::MonsterOverride::LagavulinWakeUp;
                monster.forced_follow_up = crate::hot::MonsterFollowUp::LagavulinSlash;
            }
            note_power(events, Subject::Monster(uid), PowerId::Mplating, 0);
            note_power(events, Subject::Monster(uid), PowerId::Asleep, 0);
        }

        // SlipperyPower.AfterDamageReceived `<AfterDamageReceived>d__7` `0x342acc`
        // reads the final `DamageResult.UnblockedDamage` and decrements exactly
        // one stack (`damage_monster` (frozen Python, deleted #2827)). It reads the *final* value,
        // so it sits after the whole hp-loss fold — The Boot's raise included —
        // and is deliberately not gated on `powered`: Python's condition is only
        // `m.slippery > 0 and hp_lost_int >= 1`.
        if state.monsters[target].powers.value(PowerId::Slippery) > 0 && hp_lost_int >= 1 {
            let amount = state.monsters[target]
                .powers
                .value(PowerId::Slippery)
                .checked_sub(1)
                .ok_or_else(|| overflow("Slippery amount"))?;
            let monster = &mut state.monsters_mut()[target];
            monster.powers.set(PowerId::Slippery, SlotWire::Int, amount);
            note_power(events, Subject::Monster(uid), PowerId::Slippery, amount);
        }
    }

    if !target_killed
        && kind == MonsterKind::TerrorEel
        && state.monsters[target].uid == uid
        && hp_lost_int > 0
        // ShriekPower's Amount is the HP threshold: `TerrorEel::get_ShriekAmount`
        // `0xbfda2` `GetValueIfAscension(8, 75, 70)` (#2539).
        && super::monsters::native_initial_power(state, kind, "ShriekPower")
            .is_some_and(|threshold| hp_after <= threshold)
        && state.monsters[target].powers.value(PowerId::Shriek) > 0
    {
        let monster = &mut state.monsters_mut()[target];
        monster.powers.set(PowerId::Shriek, SlotWire::Bool, 0);
        monster.override_state = MonsterOverride::Stunned;
        note_power(events, Subject::Monster(uid), PowerId::Shriek, 0);
    }
    Ok(target_killed)
}

/// Immutable receiver/result facts held by native Damage across its hooks.
#[derive(Clone, Copy)]
struct DamageGivenResult {
    target: usize,
    receiver_uid: u32,
    pending_kill: bool,
    blocked: i32,
    unblocked: i32,
    powered: bool,
    dealer: AttackDealer,
}

/// #2647 D2 / §5.4 — an Imbalanced carrier may not deal a fully blocked
/// result outside its own attack.
///
/// `ThornsPower` `0x349954` IL_0065–IL_0086 makes the thorny monster the
/// **dealer** of its retaliation, so a carrier's retaliation is a second dealer
/// path into `Hook.AfterDamageGiven`. This refuses it **before** any mutation,
/// keeping the command's refusals atomic as its `"Thorns retained dead or
/// replaced receiver"` sibling already is.
///
/// **The justification is now per-representation, not one argument stretched
/// over both** (#2693 B1). The carrier set grew, so the inherited reasoning
/// had to be re-derived rather than kept:
///
/// * **`BowlbugRock` (by kind).** The harm is the collapsed `_isOffBalance`
///   latch becoming observable: D2 models the latch as the immediate `Dizzy`
///   state `HeadbuttMove` `0x354048` IL_00bf–IL_00c7 consumes at the end of
///   the same move, and a retaliation-set latch would instead survive to the
///   *next* Headbutt. The collapse has no representation for that.
/// * **An attached carrier (#2693 B1).** There is no latch here — the arm
///   installs a real generic stun — so the D2 argument does not transfer and
///   would have been the wrong reason. The harm is that the stun parks
///   `StateLog.Last()` (`StunInternal` `0x11d7cc` IL_0031–IL_0055), and
///   [`crate::engine::monsters::parked_telegraph`] identifies that with the
///   row `loop_pos` names. That identification is established for a creature
///   stunned during its own action; a Thorns retaliation fires inside
///   *another* creature's attack, where the carrier's machine may be
///   mid-transition, and #2647 open question 2 is exactly whether the two
///   coincide off that path. Unestablished, so refuse (I5).
///
/// Both refusals are also unreachable from a public root today: admission
/// refuses any Imbalanced carrier holding Thorns at all, `buff_thorns` is
/// self-only, and no `BuffThorns` owner shares a roster with a carrier. This
/// is the guard that lets [`monsters_after_damage_given`] treat its one call
/// site as the carrier's own attack. #2655 is where this path grows a real
/// listener.
fn refuse_imbalanced_thorns_owner(
    state: &HotState,
    thorns_owner_uid: u32,
) -> Result<(), EngineRefusal> {
    if state.monsters.iter().any(|monster| {
        monster.uid == thorns_owner_uid
            && monster.hp > 0
            && super::monsters::owner_carries_imbalanced(monster)
    }) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Imbalanced owner outside its own attack",
        ));
    }
    Ok(())
}

/// Monster-owned `Hook.AfterDamageGiven` listeners for one `DamageResult`.
///
/// `Hook::AfterDamageGiven` `0x103804` -> `d__24::MoveNext` `0x3cd4f4` walks
/// `ICombatState::IterateHookListeners`, i.e. **every** registered listener,
/// not only the dealer's. Each of the nine concrete overrides opens with the
/// same `dealer != Owner -> return` guard — `ImbalancedPower` `0x33cd60`
/// IL_0020–IL_002e and `PaperCutsPower` `0x3400f8` IL_0020–IL_002e are the two
/// a monster can own. That guard filters by *dealer*, not by power, so one
/// creature carrying both would fire both; it is why this walk resolves the
/// dealer and dispatches on its powers rather than iterating the roster, but it
/// is NOT an argument that only one listener runs.
///
/// Their relative order is unobservable for a narrower reason (#2647 open
/// question 6): the two write disjoint state — PaperCuts loses the player max
/// HP, Imbalanced writes the dealer's own override — and neither reads the
/// other's. No admitted roster puts both on one creature either, PaperCuts
/// being intrinsic to Scroll of Biting and Imbalanced to Bowlbug. Slice B may
/// not inherit this: a third listener, or one that reads what another writes,
/// re-opens the question. `AbstractModel::AfterDamageGiven` `0x7a040` is a
/// `CompletedTask` no-op base.
///
/// The one listener is `ImbalancedPower` `0x33cd60`, whose predicate is
/// exactly `dealer == Owner && result.WasFullyBlocked` — no HP, liveness or
/// phase test. Its two arms are
/// `Owner.Monster as BowlbugRock` (IL_004b–IL_0061, set the off-balance latch)
/// and everything else (IL_0068–IL_006F, `CreatureCmd::Stun(Owner, null)`).
///
/// **Both arms are now live.** Slice A had to refuse the second, because
/// [`crate::engine::monsters::owner_carries_imbalanced`] was by kind and no
/// non-Bowlbug creature could carry the power. #2693 B1 adds the physical
/// ledger attachment, so the arm is reachable and installs slice A's modeled
/// generic stun; `owner_carries_imbalanced_is_bowlbug_or_an_attachment`
/// replaces the old unreachability pin and still enumerates every
/// `MonsterKind`. Admission is what keeps the arm exact: a carrier is
/// admissible only when
/// [`crate::engine::monsters::generic_stun_owner_state_is_representable`]
/// holds for it, i.e. the stun this arm installs resolves at **every**
/// position its loop can reach.
///
/// The Bowlbug arm writes `MonsterOverride::Dizzy` rather than a latch field:
/// #2647 D2 collapses `_isOffBalance` into the state `HeadbuttMove` `0x354048`
/// IL_00bf–IL_00c7 would install at the end of this same move. The collapse is
/// exact only while Bowlbug cannot deal a fully blocked result outside HEADBUTT,
/// which is why its loop is pinned to the single `(HEADBUTT, 16, 1)` row, why
/// [`refuse_imbalanced_thorns_owner`] guards the other dealer path, and why
/// `bowlbug_rock_may_not_carry_thorns` refuses a document that claims one.
///
/// Liveness: `Creature::StunInternal` `0x11d7cc` IL_0028 returns silently on a
/// corpse, so a dealer killed inside its own attack — by player Thorns, or by a
/// later listener — gets no Dizzy. Requiring a live dealer here is what makes
/// the collapse faithful, and IL_001f is the `history.over` arm.
pub(crate) fn monsters_after_damage_given(
    state: &mut HotState,
    dealer_uid: u32,
    was_fully_blocked: bool,
) -> Result<(), EngineRefusal> {
    if !was_fully_blocked {
        return Ok(());
    }
    let Some(dealer) = state
        .monsters
        .iter()
        .position(|monster| monster.uid == dealer_uid && monster.hp > 0)
    else {
        return Ok(());
    };
    if !super::monsters::owner_carries_imbalanced(&state.monsters[dealer]) {
        return Ok(());
    }
    if state.monsters[dealer].kind != MonsterKind::BowlbugRock {
        // `0x33cd60` IL_0056's `isinst BowlbugRock` yields null, so IL_005d
        // falls through to IL_0068-IL_006f: `get_Owner`, `ldnull`, and an
        // awaited `CreatureCmd::Stun(Owner, null)`. That is exactly the
        // generic parked-telegraph stun slice A modeled, so this arm installs
        // it instead of refusing. It became reachable when #2693 B1 let a
        // non-Bowlbug creature carry the power as a ledger attachment, which
        // is slice A's deferred witness (15).
        //
        // `install_generic_stun` re-checks `history.over` and owner liveness
        // itself, matching `StunInternal` `0x11d7cc` IL_001f / IL_0028, and
        // refuses rather than overwriting a foreign override.
        //
        // #2647: an awake Slumbering Beetle (a `Misery` clone recipient in
        // `SLUMBERING_BEETLE_NORMAL`) is the one bespoke owner this arm now
        // reaches; `install_imbalanced_self_stun` routes it to its own
        // exactly-determined ROLL_OUT_MOVE stun and every other owner to
        // `install_generic_stun` unchanged.
        return super::monsters::install_imbalanced_self_stun(state, dealer);
    }
    if state.history.over {
        return Ok(());
    }
    state.monsters_mut()[dealer].override_state = MonsterOverride::Dizzy;
    Ok(())
}

/// Player-owned AfterDamageGiven listeners, frozen in first-application order.
fn powers_after_damage_given(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    result: DamageGivenResult,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.fanouts.player_hooks_deactivated() {
        return Ok(());
    }
    let DamageGivenResult {
        target,
        receiver_uid,
        pending_kill,
        blocked,
        unblocked,
        powered,
        dealer,
    } = result;
    let order = state.fanouts.after_damage_given_order().to_vec();
    for power in order {
        // Stock creates a new Axebot in the same roster slot. The native
        // command holds originalTarget, so later callbacks cannot use the
        // replacement's powers or apply a debuff to it.
        let same_receiver = state.monsters[target].uid == receiver_uid;
        match power {
            PowerId::Envenom if dealer == AttackDealer::Player && powered && unblocked > 0 => {
                if state.history.over || !same_receiver {
                    continue;
                }
                let source = if state.monsters[target].hp <= 0 {
                    if !pending_kill || state.fanouts.pet().has_die_for_you() {
                        continue;
                    }
                    PlayerMonsterDebuffSource::PowerBeforeKill(catalog, receiver_uid)
                } else {
                    PlayerMonsterDebuffSource::Power(catalog)
                };
                apply_player_monster_debuff(
                    state,
                    source,
                    target,
                    PowerId::Poison,
                    MiseryToken::Poison,
                    state.powers.value(power),
                    events,
                )?;
            }
            PowerId::MonarchsGaze
                if dealer == AttackDealer::Player
                    && powered
                    && !state.history.over
                    && same_receiver
                    && (state.monsters[target].hp > 0
                        || pending_kill && !state.fanouts.pet().has_die_for_you()) =>
            {
                // Native MonarchsGaze (0x33e544) applies a visible Type-2
                // temporary wrapper. Artifact blocks that wrapper before its
                // inner Strength decrease or Sleight can run (0x348d20).
                let Some(amount) = player_monster_debuff_amount(
                    state,
                    target,
                    state.powers.value(power),
                    false,
                    events,
                )?
                else {
                    continue;
                };
                // #3338: a nested Sleight kill of a retained-death owner is
                // the one place the fresh-attach order is observable. A known
                // restack returns false and keeps the shared writer below.
                if temp_strength_wrapper_order_is_observable(state, target)
                    && temp_strength_wrapper_on_retained_owner(
                        state,
                        catalog,
                        target,
                        crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown,
                        amount,
                        crate::hot::Applier::Player,
                        events,
                    )?
                {
                    continue;
                }
                let upkeep = state.fanouts.misery_attachment_upkeep();
                let monster = &mut state.monsters_mut()[target];
                // The wrapper's nested application
                // (`TemporaryStrengthPower/<BeforeApplied>d__20::MoveNext`
                // `0x348d20` IL_003e-IL_004b) carries the wrapper's own
                // applier, which `MonarchsGazePower/<AfterDamageGiven>d__4::
                // MoveNext` `0x33e544` IL_005a-IL_0061 passes as the
                // listener's `PowerModel::get_Owner` — the player, since it is
                // a player-owned power.
                let (temp, strength) = write_monster_temp_strength_wrapper(
                    monster,
                    crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown,
                    amount,
                    crate::hot::Applier::Player,
                    upkeep,
                )?;
                let uid = monster.uid;
                note_power(events, Subject::Monster(uid), PowerId::TempStrength, temp);
                note_power(events, Subject::Monster(uid), PowerId::Strength, strength);
                powers_after_power_amount_changed(
                    state,
                    catalog,
                    target,
                    Some(PowerId::Strength),
                    -amount,
                    false,
                    // The wrapper's nested `Apply<StrengthPower>` forwards the
                    // applier the wrapper application was handed
                    // (`0x348d20` IL_003e-IL_004b), and `0x33e544` IL_005a
                    // passes this player-owned listener's own `Owner`.
                    crate::hot::Applier::Player,
                    events,
                )?;
            }
            PowerId::ReaperForm if powered && blocked + unblocked > 0 => {
                // Native Apply<Doom> (0x3ef988) gates ending/detached targets
                // before recording PowerReceived. Death's Door reads that
                // history, so an earlier nested kill cannot invent a discount.
                if state.history.over || !same_receiver {
                    continue;
                }
                let amount = (blocked + unblocked)
                    .checked_mul(state.powers.value(power))
                    .ok_or_else(|| overflow("reaper form doom"))?;
                let source = if state.monsters[target].hp <= 0 {
                    if !pending_kill || state.fanouts.pet().has_die_for_you() {
                        continue;
                    }
                    PlayerMonsterDebuffSource::PowerBeforeKill(catalog, receiver_uid)
                } else {
                    PlayerMonsterDebuffSource::Power(catalog)
                };
                apply_player_monster_debuff(
                    state,
                    source,
                    target,
                    PowerId::Doom,
                    MiseryToken::Doom,
                    amount,
                    events,
                )?;
            }
            PowerId::Envenom | PowerId::MonarchsGaze | PowerId::ReaperForm => {}
            _ => {
                return Err(EngineRefusal::MalformedArgs(
                    "after-damage-given listener order",
                ));
            }
        }
    }
    Ok(())
}

/// The admitted consumers of one monster `AfterBlockBroken` dispatch.
///
/// Python: `_burrowed_after_block_broken` (frozen, deleted #2827). A live Burrowed instance is
/// owner-validated at admission. Native BurrowedPower.AfterBlockBroken
/// (MoveNext 0x33622c) removes Burrowed even on a lethal block break, while
/// Creature.Stun skips a dead owner. Hand Drill may itself kill through Sleight
/// before the Burrowed listener is reached.
fn after_monster_block_broken(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.hand_drill_owned() && !state.fanouts.player_hooks_deactivated() {
        if let Some(catalog) = catalog {
            apply_power_monster_debuff_with_catalog(
                state,
                catalog,
                target,
                PowerId::Vuln,
                MiseryToken::Vuln,
                2,
                events,
            )?;
        } else {
            apply_power_monster_debuff(state, target, PowerId::Vuln, MiseryToken::Vuln, 2, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicHandDrill);
    }
    let live = state.monsters[target].hp > 0 && !state.history.over;
    let monster = &mut state.monsters_mut()[target];
    if monster.kind == MonsterKind::Tunneler && monster.powers.value(PowerId::Burrowed) > 0 {
        monster.powers.set(PowerId::Burrowed, SlotWire::Bool, 0);
        if live {
            monster.override_state = MonsterOverride::Dizzy;
        }
        note_power(events, Subject::Monster(monster.uid), PowerId::Burrowed, 0);
    }
    Ok(())
}

/// Remove all positive Block from a live monster and fire the same
/// AfterBlockBroken subscribers as the damage path.
///
/// Python: `_lose_all_monster_block` (frozen, deleted #2827), used by Expose.
pub fn lose_all_monster_block(
    state: &mut HotState,
    target: usize,
    events: &mut Vec<Event>,
) -> bool {
    let Some(monster) = state.monsters.get(target) else {
        return false;
    };
    if state.history.over || monster.hp <= 0 || monster.block <= 0 {
        return false;
    }
    state.monsters_mut()[target].block = 0;
    let _ = after_monster_block_broken(state, None, target, events);
    true
}

pub fn lose_all_monster_block_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Ok(false);
    };
    if state.history.over || monster.hp <= 0 || monster.block <= 0 {
        return Ok(false);
    }
    state.monsters_mut()[target].block = 0;
    after_monster_block_broken(state, Some(catalog), target, events)?;
    Ok(true)
}

/// The original powered-attack receiver as the native Damage command finds
/// it when Thorns' `BeforeDamageReceived` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThornsReceiver {
    /// Still the same live creature: the ordinary commit follows.
    Live,
    /// The same creature, killed by a nested command the retaliation
    /// caused, and that kill ended the combat (#2655, witness KD13JGCDPB3U
    /// fight 0: Thorns 2 -> Inferno 6 over the last Toadpole at 1 HP).
    RetainedDeadAtCombatEnd,
}

/// Classify the receiver after [`thorns_retaliation`] returns (#2655).
///
/// Native `CreatureCmd.<Damage>d__12::MoveNext` RVA `0x3e96c8` tests receiver
/// liveness exactly once, at receiver ENTRY (IL_0167-IL_0172
/// `originalTarget.IsDead`). After `Hook.BeforeDamageReceived` returns
/// (IL_0261, resumed at IL_02b9) it goes straight to `DamageBlockInternal`
/// (IL_02f3), `ModifyHpLost` (IL_0345/IL_0430), `ModifyUnblockedDamageTarget`
/// (IL_03e5) and `LoseHpInternal` (IL_04bc) with no second liveness test, so
/// a receiver the retaliation killed still takes the already computed hit.
/// [`commit_thorns_dead_receiver_zero_result`] carries what that commit
/// produces; this function admits it only where every consumer of the result
/// is proven inert, and names every other shape as a refusal:
///
/// - **Replaced receiver** (roster slot gone or holding another uid): the
///   native object is the retained original creature, a distinct projection
///   this roster cannot carry. Refused.
/// - **Inside a powered batch** (`batched`): the zero result would enter the
///   phase-2 result walk beside live receivers. Refused.
/// - **Pet dealer**: Osty-dealt results reach SicEm/Reaper Form/Underworld
///   dealer-side gates this walk does not audit. Refused.
/// - **Combat continues** (`!history.over`): native records
///   `CombatHistory.DamageReceived` for the zero result (IL_072c-IL_0764 is
///   gated only on `IsInProgress && !IsEnding`) and per-turn result readers
///   would see it. Refused.
/// - **Player dead**: a different terminal shape. Refused.
/// - **Retained Block**: `DamageBlockInternal` RVA `0x11d568` IL_001f-IL_002b
///   would absorb `min(Block, amount)` into a dead creature and reach the
///   `WasBlockBroken`/`AfterBlockBroken` arm (IL_04de-IL_0500, IL_0b53).
///   Kill does not clear Block. Refused.
/// - **Lagavulin Matriarch**: the one `MonsterModel` `AfterDamageReceived`
///   override (RVA `0xb8020`), live while a retaining power keeps the corpse
///   in combat. Refused.
/// - **Monarch's Gaze on the player**: `MonarchsGazePower/<AfterDamageGiven>
///   d__4::MoveNext` RVA `0x33e544` IL_001d-IL_0061 applies
///   `MonarchsGazeStrengthDownPower` to the target for any owner-dealt powered
///   result, with no result-amount gate, i.e. to the corpse. Refused.
fn thorns_receiver_after_retaliation(
    state: &HotState,
    target: usize,
    original_uid: u32,
    batched: bool,
    dealer: AttackDealer,
) -> Result<ThornsReceiver, EngineRefusal> {
    let Some(monster) = state
        .monsters
        .get(target)
        .filter(|monster| monster.uid == original_uid)
    else {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Thorns replaced receiver",
        ));
    };
    if monster.hp > 0 {
        return Ok(ThornsReceiver::Live);
    }
    let refusal = if batched {
        "Thorns retained dead receiver inside a damage batch"
    } else if dealer != AttackDealer::Player {
        "Thorns retained dead receiver of a pet attack"
    } else if !state.history.over {
        "Thorns retained dead receiver while combat continues"
    } else if state.hp <= 0 {
        "Thorns retained dead receiver after player death"
    } else if monster.block != 0 {
        "Thorns retained dead receiver with Block"
    } else if monster.kind == MonsterKind::LagavulinMatriarch {
        "Thorns retained dead Lagavulin Matriarch receiver"
    } else if state.powers.value(PowerId::MonarchsGaze) != 0 {
        "Thorns retained dead receiver under Monarch's Gaze"
    } else {
        return Ok(ThornsReceiver::RetainedDeadAtCombatEnd);
    };
    Err(EngineRefusal::PowerOrderNotModeled(refusal))
}

/// Native's commit of an already computed powered hit onto a receiver that a
/// nested kill during Thorns retaliation removed while ending the combat
/// (#2655). Admitted only by [`thorns_receiver_after_retaliation`].
///
/// `Creature.LoseHpInternal` RVA `0x11d5b8`: IL_000c-IL_0029 latches
/// `wasKilled = CurrentHp > 0 && amount >= CurrentHp`, false at 0 HP;
/// IL_004c-IL_005b writes `Max(0 - n, 0) = 0`; IL_0068-IL_0070 sets
/// `UnblockedDamage = 0 - 0`; IL_007c-IL_008c leaves `OverkillDamage` 0 for an
/// unkilled result. With Block 0, `DamageBlockInternal` (`0x11d568`) returns
/// 0, and Damage `0x3e96c8` IL_04de-IL_0554 derives `WasBlockBroken` and
/// `WasFullyBlocked` both false. So the result is all zero whatever
/// `ModifyHpLost` returned, and that hook's only side effects are receiver-
/// owned (Hardened Shell, Buffer, Intangible, Slippery: all removed at death,
/// `PowerModel.ShouldPowerBeRemovedAfterOwnerDeath` RVA `0x840dd` returns true
/// and none of the seven overrides is one of them) or The Boot's
/// `AfterModifyingHpLostAfterOsty` `0x9c6fe`, which only flashes. The
/// player-owned Tungsten Rod (`ModifyHpLostAfterOsty` `0x9d0c4`) and Beating
/// Remnant (`0x904a8`) gate on `target == Owner.Creature`, so they are inert
/// for a monster receiver.
/// `ModifyUnblockedDamageTarget`'s one override, DieForYou `0xa181c`
/// IL_0001-IL_001c, redirects only its pet owner's creature.
///
/// Result phase, with the combat already ending:
/// - `CombatHistory.DamageReceived` is skipped (IL_0731-IL_0742
///   `IsInProgress && !IsEnding`), so no per-turn result counter moves.
/// - IL_0b2f `WasBlockBroken` false; IL_0bb6-IL_0bbc `UnblockedDamage <= 0`
///   skips only `AfterCurrentHpChanged`. The branch lands on IL_0c41, the
///   player `DamageDealt += UnblockedDamage` block, which adds 0 (no
///   modeled state).
/// - IL_0cc4 `Hook.AfterDamageGiven` (`0x3cd4f4`, not ending gated) reaches
///   every override inert for a zero player-dealt result: Concoct `0x337100`
///   and Envenom `0x33a04c` need `UnblockedDamage > 0` (IL_004b); Imbalanced
///   `0x33cd60` needs its monster owner as dealer; PaperCuts `0x3400f8` a
///   player target; Reaper Form `0x341b90` IL_0077 and Underworld `0x34a2e0`
///   IL_0092 `TotalDamage > 0` (`0x11deb0` = Blocked + Unblocked); SicEm
///   `0x3447a0` an Osty dealer; SkillIronclad2Achievement `0xf4f7b`
///   `UnblockedDamage >= 999`. Monarch's Gaze is refused above.
/// - IL_0d27 `WasTargetKilled` false, so IL_0d86 `Hook.AfterDamageReceived`
///   (`0x3cd698`, not ending gated) runs. Every override gates on
///   `target == Owner` (relics: BeatingRemnant `0x90508`, CentennialPuzzle
///   `0x321758`, DemonTongue `0x322adc`, EmotionChip `0x93018`, LavaLamp
///   `0x95f98`, SelfFormingClay `0x330ae4`; powers: Asleep `0x335094`, CurlUp
///   `0xa10cc`, FlameBarrier `0x33a358`, Flutter `0x33a620`, HardenedShell
///   `0xa3368`, Inferno `0x33d084`, PersonalHive `0x340220`, Plow `0x340848`,
///   Reflect `0x342314`, Rupture `0x342fec`, Shriek `0x344348`, Slippery
///   `0x344f20`, Slumber `0x34500c`, TheGambit `0x349584`), and the corpse
///   owns none of them after death. Lagavulin Matriarch is refused above.
/// - IL_0eb4 `Kill` receives an empty killed list.
///
/// The result still joins the command's `_results` (IL_0a6c), so the sink
/// gets a zero, unkilled row; Echoing Slash counts no kill from it.
fn commit_thorns_dead_receiver_zero_result(
    state: &HotState,
    target: usize,
    result_sink: Option<&mut Vec<AttackDamageResult>>,
    events: &mut Vec<Event>,
) {
    let uid = state.monsters[target].uid;
    events.push(Event::MonsterDamaged {
        uid,
        blocked: 0,
        unblocked: 0,
        hp: state.monsters[target].hp,
    });
    if let Some(results) = result_sink {
        results.push(AttackDamageResult {
            target,
            receiver_uid: uid,
            total_damage: 0,
            overkill_damage: 0,
            was_target_killed: false,
        });
    }
}

/// `ThornsPower/<BeforeDamageReceived>d__4::MoveNext` RVA `0x349954`,
/// player-side.
///
/// IL_0065–IL_0086 is
/// `CreatureCmd::Damage(choiceContext, /*receiver*/ dealer, Amount,
/// /*props*/ 20, /*dealer*/ Owner, null, null)` — so the **thorny monster is
/// the dealer** of this retaliation, which is why `thorns_owner_uid` is
/// threaded in (#2647 §1.3; the parameter is also what #2655 needs).
///
/// `props = 20` decomposes over `MegaCrit.Sts2.Core.ValueProps.ValueProp` as
/// `Unpowered(4) | SkipHurtAnim(16)`, with `Unblockable(2)` **clear** and
/// `Move(8)` clear. So the retaliation is blockable — `WasFullyBlocked`'s
/// first conjunct `!props.HasFlag(Unblockable)` (`0x3e96c8` IL_0505) holds and
/// this result can be fully blocked — while
/// `ValuePropExtensions::IsPoweredAttack` `0xd6e0` (`HasFlag(Move) &&
/// !HasFlag(Unpowered)`) is false, so it can re-trigger neither Thorns nor
/// PaperCuts (#2647 open question 7).
fn thorns_retaliation(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    thorns: i32,
    thorns_owner_uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    refuse_imbalanced_thorns_owner(state, thorns_owner_uid)?;
    let block_before = state.block;
    let blocked = block_before.min(thorns);
    state.block -= blocked;
    let unblocked = thorns - blocked;
    let hp_lost = commit_player_hp_loss(state, i64::from(unblocked), blocked, events)?;
    if resolve_player_lethal_with_catalog(state, catalog, events, true)? {
        rupture_after_owner_hp_loss(state, hp_lost, events)?;
        inferno_after_player_hp_loss(state, catalog, hp_lost, events)?;
        after_owner_damage_received_relics(state, catalog, hp_lost, events)?;
    }
    Ok(())
}

/// Monster Thorns against `PLAYER_PET`: owner Block is still the shared
/// defense layer, but only the live Osty receives the unblocked remainder.
/// Pet death never ends combat and never cancels the already-built attack.
fn thorns_retaliation_pet(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    thorns: i32,
    thorns_owner_uid: u32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // `WasFullyBlocked`'s second conjunct reads `originalTarget.Block`
    // (`0x3e96c8` IL_0505–IL_0554), and the original target here is the pet,
    // whose own Block is not represented — the shared owner layer consumed
    // below is the player's. So this path never computes the flag: an
    // owner-side listener that would read it refuses first (I5, #2647 §5.10).
    refuse_imbalanced_thorns_owner(state, thorns_owner_uid)?;
    let blocked = state.block.min(thorns);
    state.block -= blocked;
    let unblocked = thorns - blocked;
    let mut actual_loss = 0;
    state
        .fanouts
        .mutate_pet(|pet| {
            actual_loss = pet.lose_hp(unblocked)?;
            Ok(())
        })
        .map_err(|_| overflow("Osty thorns damage"))?;
    let died = actual_loss > 0 && state.fanouts.pet().osty().is_none();
    // Necro Mastery's damage keeps the attack's catalog, so a kill reaches
    // Gremlin Horn's AfterDeath Draw (#3172).
    necro_mastery_after_pet_hp_lost_with_catalog(state, catalog, actual_loss, events)?;
    if died {
        super::cards::physical_cards_after_actual_death(state)?;
    }
    Ok(())
}

/// `NecroMasteryPower.OnHpChanged`: one callback for actual owner-Osty HP
/// loss, dispatching serial unpowered/unblockable damage over the frozen live
/// enemy roster. The live stacked Amount is read only after the HP mutation.
fn necro_mastery_after_pet_hp_lost_with_catalog(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    actual_loss: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if actual_loss <= 0 || state.fanouts.player_hooks_deactivated() {
        return Ok(());
    }
    let amount = state.powers.value(PowerId::NecroMastery);
    if amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("necro mastery amount"));
    }
    let damage = i64::from(actual_loss)
        .checked_mul(i64::from(amount))
        .ok_or_else(|| overflow("necro mastery damage"))?;
    let targets = alive_targets(state);
    for target in targets {
        if state.history.over {
            break;
        }
        damage_monster_with_optional_catalog(
            state,
            catalog,
            target,
            DotNetDecimal::from_i64(damage),
            false,
            false,
            events,
        )?;
    }
    Ok(())
}

/// Drain a retained solo Osty's queued actual death after all hit results.
/// Native Kill(list) 0x3ebb88 IL0070–00a0 does not discard revived targets.
/// KillWithoutCheckingWinCondition 0x3ebe90 IL01bc–0200 removes restored HP
/// and publishes a fresh HPChanged before death observers. NecroMastery
/// 0x33e950 observes this second loss. DieForYou 0xa1861/0xa186f retains Osty's
/// combat membership and power, including during a combat-ending transition.
fn finish_queued_osty_death(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if let Some(osty) = state.fanouts.pet().osty() {
        let loss = osty.hp();
        state
            .fanouts
            .mutate_pet(|pet| pet.lose_hp(loss).map(|_| ()))
            .map_err(|_| overflow("queued Osty death"))?;
        necro_mastery_after_pet_hp_lost_with_catalog(state, catalog, loss, events)?;
    }
    super::cards::physical_cards_after_actual_death(state)
}

/// Live native CombatManager.IsEnding (0x135854) before the killed-list
/// drain is complete. Primary liveness changes at HP commit, not AfterDeath.
/// The complete v111 veto override census is Adaptable0x9f605,
/// Infested0xa4017, SteamEruption0xa85fb, Stock0xa86c7, Surprise0xa8b9b.
/// Infested/Surprise are implicit in the admitted pre-spawn monster lineage.
///
/// This is the shared IsEnding projection for ending-gated reset commands
/// (#2669): GainStars/GainEnergy/Channel skip while it holds, Remove does
/// not, and Decrement delegates to ModifyAmount, which returns at IsEnding.
pub(crate) fn damage_combat_is_ending(state: &HotState) -> bool {
    if state.history.over {
        return true;
    }
    if state
        .monsters
        .iter()
        .any(|monster| monster.hp > 0 && !super::monsters::is_secondary_enemy(monster))
    {
        return false;
    }
    !state.monsters.iter().any(|monster| {
        monster.powers.value(PowerId::Adaptable) > 0
            || monster.powers.value(PowerId::Stock) > 0
            || monster.powers.value(PowerId::SteamPressure) > 0
            || (monster.kind == MonsterKind::PhrogParasite && infested_is_pending(state))
            || (monster.kind == MonsterKind::GremlinMerc && state.monsters.len() == 1)
    })
}

/// Test fixture for the ending-but-not-over window (#3502, #3515).
///
/// Appends a dead primary Toadpole and a live secondary Gas Bomb, so no
/// living primary remains while a creature is still alive: native
/// `IsCombatEnding` (`0x135854` IL_0026-0069) is true and `history.over` has
/// not latched. With `vetoed`, the dead primary carries Adaptable, whose
/// `ShouldStopCombatFromEnding` keeps the same roster live: the control.
/// Returns the Gas Bomb's roster index, a live target for targeted bodies.
#[cfg(test)]
pub(crate) fn push_ending_window_roster(state: &mut HotState, vetoed: bool) -> usize {
    let base = u32::try_from(state.monsters.len()).expect("small test roster");
    let mut primary = HotMonster::new(MonsterKind::Toadpole, 100);
    primary.hp = 0;
    primary.uid = base;
    primary.slot = i32::try_from(base).expect("small test roster");
    if vetoed {
        primary
            .powers
            .set(PowerId::Adaptable, crate::powers::SlotWire::Int, 1);
    }
    state.monsters_mut().push(primary);
    // Enough HP that no witness attack kills it and latches `history.over`.
    let mut bomb = HotMonster::new(MonsterKind::GasBomb, 100);
    bomb.uid = base + 1;
    bomb.slot = i32::try_from(base + 1).expect("small test roster");
    state.monsters_mut().push(bomb);
    assert!(!state.history.over);
    assert_eq!(damage_combat_is_ending(state), !vetoed);
    state.monsters.len() - 1
}

/// Witness harness for an ending-gated body (#3515).
///
/// Replaces `template`'s roster with [`push_ending_window_roster`] and runs
/// `run` with the Gas Bomb's index as the live target. The ending roster must
/// leave the state untouched; the Adaptable-vetoed control must change it,
/// which proves the fixture reaches the write the gate guards.
#[cfg(test)]
pub(crate) fn assert_ending_window_gate(
    template: &HotState,
    label: &str,
    mut run: impl FnMut(&mut HotState, usize) -> Result<(), EngineRefusal>,
) {
    for vetoed in [false, true] {
        let mut state = template.clone();
        state.monsters_mut().clear();
        let target = push_ending_window_roster(&mut state, vetoed);
        let before = state.clone();
        run(&mut state, target)
            .unwrap_or_else(|error| panic!("{label} vetoed={vetoed}: {error:?}"));
        assert_eq!(state == before, !vetoed, "{label} vetoed={vetoed}");
    }
}

/// Whether a Phrog Parasite still owns its `InfestedPower`.
///
/// `InfestedPower.ShouldStopCombatFromEnding` (`0xa4017`) returns true for as
/// long as the power exists: while its owner lives, and across the dead
/// owner's own `Hook.AfterDeath` walk until `RemoveAllPowersAfterDeath`
/// (`KillWithoutCheckingWinCondition` `0x3ebe90` IL_04f1) strips it. The
/// power's only callback spawns the Wrigglers, and nothing else creates a
/// Wriggler, so the retained dead Phrog row stops vetoing exactly when they
/// exist (#2656). This replaces the former `monsters.len() == 1` proxy, which
/// dropped the veto whenever any peer row was present. The witness is exact
/// for one Phrog per roster, which is every encounter that builds one
/// (`PHROG_PARASITE_ELITE` is the Phrog alone).
fn infested_is_pending(state: &HotState) -> bool {
    !state
        .monsters
        .iter()
        .any(|monster| monster.kind == MonsterKind::Wriggler)
}

/// Ordinary monster death (`_finish_monster_death`, frozen Python, deleted #2827) plus the terminal
/// boundary (`_finish_death_terminal_boundary`).
///
/// `RemoveAllPowersAfterDeath` strips the debuff family and the acquisition
/// order; Vulnerable, Thorns, Weak and Strength survive an ordinary death.
/// IllusionPower is the represented exception: Weak/Vulnerable are stripped,
/// temporary Strength is retained, and the corpse spends its next action on a
/// full heal. Test Subject's first two forms are the other represented
/// exception: Adaptable retains the cleaned corpse until its next action.
///
/// **Poison and Intangible do not survive.** `_finish_monster_death` (frozen Python, deleted #2827)
/// clears Doom, Oblivion, Strangle, Poison, Hang, Debilitate, and Intangible
/// unconditionally, and resets the Poison instance identity with them.
/// Poison's original missing
/// cleanup surfaced when a poison tick landed a killing blow; Soul Fysh's
/// admitted Intangible reader makes the same native death-removal rule live
/// for that power too.
pub(crate) fn finish_monster_death(
    state: &mut HotState,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    finish_monster_death_with_context(state, None, target, events)
}

/// [`finish_monster_death`] with the enclosing command's catalog, when it has
/// one. The catalog only reaches the death body's AfterDeath listeners
/// (Gremlin Horn's Draw, #3166); every authentication below is unchanged.
fn finish_monster_death_with_context(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let knockdown_reachable = require_knockdown_death_entry(state, target)?;
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    if state.card_states.dampen().is_some() {
        return Err(EngineRefusal::MalformedArgs("Dampen death catalog"));
    }
    if super::cards::aeonglass_owner_reachable(state) {
        return Err(EngineRefusal::MalformedArgs("Aeonglass death catalog"));
    }
    if hopper_reachable {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper death catalog",
        ));
    }
    let gremlin_merc_reachable = state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::GremlinMerc | MonsterKind::FatGremlin
        )
    });
    if gremlin_merc_reachable {
        let mut entry = state.clone();
        let Some(dying) = entry.monsters_mut().get_mut(target) else {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc death target"));
        };
        if matches!(
            dying.kind,
            MonsterKind::GremlinMerc | MonsterKind::FatGremlin
        ) && dying.hp <= 0
        {
            dying.hp = 1;
        }
        if !super::monsters::gremlin_merc_state_is_valid(&entry) {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc death entry"));
        }
        let mut next = state.clone();
        let mut emitted = Vec::new();
        finish_monster_death_inner(&mut next, target, catalog, &mut emitted)?;
        *state = next;
        events.extend(emitted);
        return Ok(());
    }
    let hex_reachable = state.powers.value(PowerId::HexPower) > 0;
    if hex_reachable {
        let mut entry = state.clone();
        let Some(dying) = entry.monsters_mut().get_mut(target) else {
            return Err(EngineRefusal::MalformedArgs("Spectral Hex death target"));
        };
        if dying.kind == MonsterKind::SpectralKnight && dying.hp <= 0 {
            // Public death completion enters after lethal HP was committed.
            // Reconstitute only that coordinate to authenticate the stable
            // live Hex owner boundary, as the Queen roster path below does.
            dying.hp = 1;
        }
        if !super::cards::hex_power_state_is_exact(&entry) {
            return Err(EngineRefusal::MalformedArgs("Spectral Hex death entry"));
        }
        let mut next = state.clone();
        let mut emitted = Vec::new();
        finish_monster_death_inner(&mut next, target, catalog, &mut emitted)?;
        *state = next;
        events.extend(emitted);
        return Ok(());
    }
    match crate::moves::spawned::constrict_carrier_state(state) {
        crate::moves::spawned::ConstrictCarrierState::Absent => {}
        crate::moves::spawned::ConstrictCarrierState::Reachable => {
            let dying_uid = state.monsters[target].uid;
            if !crate::moves::spawned::constrict_death_state_is_exact(state, dying_uid) {
                return Err(EngineRefusal::MalformedArgs("constrict death state"));
            }
            let mut next = state.clone();
            let mut emitted = Vec::new();
            finish_monster_death_inner(&mut next, target, catalog, &mut emitted)?;
            *state = next;
            events.extend(emitted);
            return Ok(());
        }
        crate::moves::spawned::ConstrictCarrierState::Malformed => {
            return Err(EngineRefusal::MalformedArgs("ConstrictPower amount"));
        }
    }
    match crate::moves::spawned::tender_carrier_state(state) {
        crate::moves::spawned::TenderCarrierState::Absent => {}
        crate::moves::spawned::TenderCarrierState::Reachable => {
            if !crate::moves::spawned::tender_private_state_is_exact(state) {
                return Err(EngineRefusal::MalformedArgs("tender death state"));
            }
            let mut next = state.clone();
            let mut emitted = Vec::new();
            finish_monster_death_inner(&mut next, target, catalog, &mut emitted)?;
            *state = next;
            events.extend(emitted);
            return Ok(());
        }
        crate::moves::spawned::TenderCarrierState::Malformed => {
            return Err(EngineRefusal::MalformedArgs("TenderPower state"));
        }
    }
    if knockdown_reachable {
        let mut next = state.clone();
        let mut emitted = Vec::new();
        finish_monster_death_inner(&mut next, target, catalog, &mut emitted)?;
        *state = next;
        events.extend(emitted);
        return Ok(());
    }
    finish_monster_death_inner(state, target, catalog, events)
}

pub fn finish_monster_death_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    require_knockdown_death_entry(state, target)?;
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_reachable(state, catalog);
    // Gremlin Horn's AfterDeath draws through the catalog. Retain it even
    // when the dying monster has no catalog-dependent private mechanic.
    if !hopper_reachable
        && !dampen_reachable
        && !aeonglass_reachable
        && !state.fanouts.gremlin_horn_owned()
    {
        return finish_monster_death(state, target, events);
    }
    let mut entry = state.clone();
    let Some(dying) = entry.monsters_mut().get_mut(target) else {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper death target"));
    };
    if matches!(
        dying.kind,
        MonsterKind::ThievingHopper
            | MonsterKind::Aeonglass
            | MonsterKind::FlailKnight
            | MonsterKind::MagiKnight
            | MonsterKind::SpectralKnight
    ) && dying.hp <= 0
    {
        dying.hp = 1;
    }
    if hopper_reachable
        && (!super::monsters::thieving_hopper_state_is_valid(&entry)
            || !super::monsters::thieving_hopper_deck_payload_is_exact(&entry, catalog))
    {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper death entry"));
    }
    if dampen_reachable && !super::cards::dampen_state_is_exact(&entry, catalog) {
        return Err(EngineRefusal::MalformedArgs("Dampen death entry"));
    }
    if aeonglass_reachable && !super::cards::aeonglass_state_is_exact(&entry, catalog) {
        return Err(EngineRefusal::MalformedArgs("Aeonglass death entry"));
    }
    let mut next = state.clone();
    let mut emitted = Vec::new();
    finish_monster_death_inner(&mut next, target, Some(catalog), &mut emitted)?;
    *state = next;
    events.extend(emitted);
    Ok(())
}

/// A death inside a catalog-bearing command whose entry already
/// authenticated the private card payloads (Doom's enemy-side batch, End of
/// Days). Its entry keeps the parent command's catalog (it was
/// `finish_monster_death_after_hopper_auth`, which dropped it), so the death's `Hook.AfterDeath` walk runs
/// Gremlin Horn's Draw instead of refusing (#3166):
/// `GremlinHorn/<AfterDeath>d__6::MoveNext` RVA `0x326170` (v0.111.0 DLL
/// 9cb4f1ad…) gates only on `target.Side != Owner.Creature.Side`
/// (IL_0024-IL_003f), then awaits `PlayerCmd.GainEnergy` (IL_0062) and
/// `CardPileCmd.Draw` (IL_00d9): nothing in it depends on how the creature
/// died, so a Doom `Kill` reaches it exactly as a lethal hit does.
pub(crate) fn finish_monster_death_after_catalog_auth(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let catalog = Some(catalog);
    if state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper)
    {
        return finish_monster_death_inner(state, target, catalog, events);
    }
    if state.card_states.dampen().is_some() {
        if !super::cards::dampen_death_transient_is_exact(state, target) {
            return Err(EngineRefusal::MalformedArgs("Dampen internal death entry"));
        }
        return finish_monster_death_inner(state, target, catalog, events);
    }
    if super::cards::aeonglass_owner_reachable(state) {
        if !super::cards::aeonglass_internal_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass internal death entry",
            ));
        }
        return finish_monster_death_inner(state, target, catalog, events);
    }
    finish_monster_death_with_context(state, catalog, target, events)
}

#[inline(always)]
fn finish_monster_death_inner(
    state: &mut HotState,
    target: usize,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let tracked = state.fanouts.synchronous_damage_is_active();
    let uid = state.monsters[target].uid;
    finish_monster_death_body(state, target, catalog, events)?;
    if tracked && !state.fanouts.finish_batch_death_cleanup(uid) {
        return Err(EngineRefusal::MalformedArgs("Damage death cleanup receipt"));
    }
    Ok(())
}

fn finish_monster_death_body(
    state: &mut HotState,
    target: usize,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.fanouts.gremlin_horn_owned() && catalog.is_none() {
        return Err(EngineRefusal::MalformedArgs("Gremlin Horn death catalog"));
    }
    let dying_kind = state.monsters[target].kind;
    if dying_kind == MonsterKind::ThievingHopper {
        // SwipePower.BeforeDeath is the first Hopper death callback: it
        // restores the exact DeckVersion row before generic power cleanup,
        // MonsterDied publication, and every AfterDeath listener group.
        super::monsters::hopper_return_stolen_card(state, target)?;
    }
    let gremlin_merc_death = matches!(
        dying_kind,
        MonsterKind::GremlinMerc | MonsterKind::FatGremlin
    );
    if gremlin_merc_death {
        let mut entry = state.clone();
        entry.monsters_mut()[target].hp = 1;
        if !super::monsters::gremlin_merc_state_is_valid(&entry) {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc death lifecycle"));
        }
    }
    if matches!(
        dying_kind,
        MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
    ) || state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        )
    }) {
        // Damage has committed the lethal HP value, but the serialized state
        // immediately before this command was a stable live-owner boundary.
        // Reconstitute only that HP coordinate to authenticate provenance.
        let mut entry = state.clone();
        entry.monsters_mut()[target].hp = 1;
        if !super::monsters::queen_roster_state_is_valid(&entry) {
            return Err(EngineRefusal::MalformedArgs("Queen/Amalgam death entry"));
        }
    }
    // #3191: every Kin death, Follower or Priest, starts from the roster's
    // identity quotient. Liveness is not checked here: inside one powered
    // batch a pending Priest corpse may still stand beside a live Follower
    // until its own Kill reaches the secondary cascade below.
    let kin_roster = super::monsters::kin_roster_reachable(state);
    if kin_roster && !super::monsters::kin_roster_shape_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Kin boss death entry"));
    }
    let test_subject_retains = if dying_kind == MonsterKind::TestSubject {
        let mut live_probe = state.clone();
        live_probe.monsters_mut()[target].hp = 1;
        if !super::monsters::test_subject_state_is_valid(&live_probe) {
            return Err(EngineRefusal::MalformedArgs("Test Subject death lifecycle"));
        }
        state.monsters[target].test_subject_respawns() < 2
    } else {
        false
    };
    if (matches!(dying_kind, MonsterKind::TheLost | MonsterKind::TheForgotten)
        || state.monsters.iter().any(|monster| {
            matches!(
                monster.kind,
                MonsterKind::TheLost | MonsterKind::TheForgotten
            )
        }))
        && !super::monsters::possess_roster_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "TheLostAndForgottenNormal roster lifecycle",
        ));
    }
    let axebot_respawns = dying_kind == MonsterKind::Axebot
        && state.monsters[target].powers.value(PowerId::Stock) > 0;
    if axebot_respawns {
        // Stock's complete singleton/amount/identity contract must refuse
        // before ordinary death cleanup clears any observable state.
        super::monsters::require_axebot_respawn(state, state.monsters[target].uid)?;
    }
    state
        .fanouts
        .begin_batch_death_cleanup(state.monsters[target].uid);
    let dying_was_primary = !super::monsters::is_secondary_enemy(&state.monsters[target]);
    let illusion = matches!(
        state.monsters[target].kind,
        MonsterKind::Parafright | MonsterKind::EyeWithTeeth
    );
    // `ReattachPower/<AfterDeath>d__11::MoveNext` RVA `0x341d30` IL_0040-0056
    // vetoes the revive with `AreAllOtherSegmentsDead && Owner.IsDead`, so the
    // last segment to fall stays dead and ends the encounter. Python spells
    // the same predicate as `elif m.kind == SEGMENT and s.alive():`
    // (`_finish_monster_death` (frozen Python, deleted #2827)) — `State.alive` is every monster
    // at `hp > 0`, and the dying segment's HP is already zero here, so this is
    // "some peer is still standing".
    let segment_peer_alive = state.monsters[target].kind == MonsterKind::DecimillipedeSegment
        && state
            .monsters
            .iter()
            .enumerate()
            .any(|(index, monster)| index != target && monster.hp > 0);
    if dying_kind == MonsterKind::FatGremlin {
        let heist = state.monsters[target].powers.value(PowerId::HeistGold);
        let Some(merc_index) = state
            .monsters
            .iter()
            .position(|monster| monster.kind == MonsterKind::GremlinMerc && monster.uid == 0)
        else {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc Heist owner"));
        };
        if !state.monsters_mut()[merc_index].set_gremlin_merc_returned_gold(heist) {
            return Err(EngineRefusal::MalformedArgs("Gremlin Merc returned gold"));
        }
    }
    // `ConstrictPower/<AfterDeath>d__5::MoveNext` RVA `0x3373cc` identity-
    // compares this dead creature with the retained applier and removes the
    // matching singleton before generic RemoveAllPowersAfterDeath cleanup.
    if dying_kind == MonsterKind::SlitheringStrangler && state.fanouts.constrict_amount() != 0 {
        super::turn::prepare_after_side_turn_end_singleton_write(
            state,
            crate::hot::AfterSideTurnEndPowerToken::Constrict,
            true,
            false,
        )?;
        let written = state.fanouts.set_constrict_amount(0);
        debug_assert!(written);
    }
    remove_player_shrink_after_applier_death(state, dying_kind, events)?;
    let (uid, kind) = {
        let upkeep = state.fanouts.misery_attachment_upkeep();
        let monster = &mut state.monsters_mut()[target];
        if !illusion {
            // The temporary wrapper's side-end restoration removes each
            // wrapper first and re-applies with the OWNER as applier
            // (`TemporaryStrengthPower/<AfterSideTurnEnd>d__22::MoveNext`
            // `0x348ba8` IL_0043, IL_009d-IL_00c4), not the wrapper's original
            // one. The whole ledger is dropped on the next line either way, so
            // only the restored scalar survives this call.
            unwind_monster_temp_strength_wrappers(monster, upkeep)?;
        }
        monster.misery_debuff_order = Default::default();
        monster.powers.set(PowerId::Doom, SlotWire::Int, 0);
        monster.powers.set(PowerId::Demise, SlotWire::Int, 0);
        monster.powers.set(PowerId::Shrink, SlotWire::Int, 0);
        monster.powers.set(PowerId::Oblivion, SlotWire::Int, 0);
        monster.powers.set(PowerId::Strangle, SlotWire::Int, 0);
        monster.powers.set(PowerId::Hang, SlotWire::Int, 0);
        monster.powers.set(PowerId::CurlUp, SlotWire::Int, 0);
        monster.powers.set(PowerId::Poison, SlotWire::Int, 0);
        monster.powers.set(PowerId::SicEm, SlotWire::Int, 0);
        monster.powers.set(PowerId::Artifact, SlotWire::Int, 0);
        monster.powers.set(PowerId::Conqueror, SlotWire::Int, 0);
        monster.powers.set(PowerId::Debilitate, SlotWire::Int, 0);
        monster.powers.set(PowerId::HatchPower, SlotWire::Int, 0);
        monster.powers.set(PowerId::HighVoltage, SlotWire::Int, 0);
        monster.powers.set(PowerId::HeistGold, SlotWire::Int, 0);
        monster.powers.set(PowerId::Intangible, SlotWire::Int, 0);
        monster.powers.set(PowerId::Mplating, SlotWire::Int, 0);
        // Rampart is an ordinary owner power. A dead Living Shield cannot
        // grant later side-start block, and its deferred completed-Slam
        // receipt cannot survive ordinary native death cleanup.
        monster.powers.set(PowerId::Rampart, SlotWire::Int, 0);
        if monster.kind == MonsterKind::LivingShield {
            monster.forced_follow_up = MonsterFollowUp::None;
        }
        monster.powers.set(PowerId::EscapeArtist, SlotWire::Int, 0);
        monster.powers.set(PowerId::Flutter, SlotWire::Int, 0);
        // Native KillWithoutCheckingWinCondition (0x3ebe90) removes these
        // ordinary listener instances before the outer Damage command resumes.
        // Its fresh AfterDamageReceived walk must not redispatch a removed one.
        monster.powers.set(PowerId::PlowThreshold, SlotWire::Int, 0);
        monster.powers.set(PowerId::Asleep, SlotWire::Int, 0);
        monster.powers.set(PowerId::Slumber, SlotWire::Int, 0);
        monster.powers.set(PowerId::Shriek, SlotWire::Bool, 0);
        monster.powers.set(PowerId::Burrowed, SlotWire::Bool, 0);
        // `_finish_monster_death` (frozen Python, deleted #2827) clears both of these with the
        // rest of the dying creature's ordinary power state. They join the list
        // as their readers land (#2481 slots 2 and 3) — a power that is written
        // and read but never cleared on death projects a live amount on a
        // corpse, which is what the corpus caught as
        // `monsters[N].ravenous: python='<absent>' rust=5`.
        monster.powers.set(PowerId::Slippery, SlotWire::Int, 0);
        monster.powers.set(PowerId::Ravenous, SlotWire::Int, 0);
        // `_finish_monster_death` (frozen Python, deleted #2827) clears CrabRage with the rest of the
        // dying creature's ordinary power state. Native reaches the same place
        // through `Creature.RemoveAllPowersAfterDeath` `0x11dbac`:
        // `CrabRagePower` does not override the base
        // `ShouldPowerBeRemovedAfterOwnerDeath` `0x840dd`, so the instance goes
        // with its owner and can never rage for a later sibling death.
        monster.set_crab_rage(false);
        // `_finish_monster_death` (frozen Python, deleted #2827) clears Suck with the rest of the
        // dying creature's ordinary power state. Its reader is the AfterAttack
        // tail above, which a Thorns kill can reach mid-command: without this
        // the corpse would both keep projecting `suck: 3` and still count the
        // in-flight hit group.
        monster.powers.set(PowerId::Suck, SlotWire::Int, 0);
        // SandpitPower is ordinary death-removed state. Its AfterRemoved
        // callback observes a dead owner and therefore cannot force-kill the
        // player from this path (`_finish_monster_death`, frozen Python, deleted #2827).
        monster.powers.set(PowerId::Sandpit, SlotWire::Int, 0);
        monster.poison_uid = -1;
        monster.curl_up_card_uid = -1;
        monster.set_louse_curled(false);
        monster.set_demise_after_intangible(false);
        monster.set_demise_before_ritual(false);
        monster.override_state = MonsterOverride::None;
        monster.forced_follow_up = MonsterFollowUp::None;
        // Ordinary owners preserve Ritual across death in Python. Test
        // Subject is the explicit current-form reset: RemoveAllPowersAfterDeath
        // clears every current-form/player-applied power except the two native
        // veto owners, Adaptable and Painful Stabs. The latter survives form
        // two's retaliation death long enough to finish its frozen Wound tail
        // (`_finish_monster_death`, frozen Python, deleted #2827).
        if monster.kind == MonsterKind::TestSubject {
            let adaptable = monster.powers.value(PowerId::Adaptable);
            let painful_stabs = monster.powers.value(PowerId::PainfulStabs);
            monster.powers = Slots::new();
            if adaptable > 0 {
                monster
                    .powers
                    .set(PowerId::Adaptable, SlotWire::Int, adaptable);
            }
            if painful_stabs > 0 {
                monster
                    .powers
                    .set(PowerId::PainfulStabs, SlotWire::Int, painful_stabs);
            }
            monster.ritual_fresh = false;
            monster.set_test_subject_nemesis_apply_intangible(false);
        }
        if illusion {
            monster.powers.set(PowerId::Weak, SlotWire::Int, 0);
            monster.powers.set(PowerId::Vuln, SlotWire::Int, 0);
            monster.revive_stage = 1;
        }
        // `ReattachPower/<AfterDeath>d__11::MoveNext` RVA `0x341d30`
        // IL_005a-IL_008c sets `Data.isReviving` and immediately
        // `SetMoveImmediate(DeadState, 0)` on the retained corpse. Python
        // represents the whole DEAD -> REATTACH -> RAND cycle as
        // `revive_stage` (`_finish_monster_death` (frozen Python, deleted #2827)), and that arm
        // is an `elif` **after** the shared power-removal pass above, so it
        // re-zeroes Strength outright rather than only unwinding the temporary
        // wrapper: a segment comes back with no Strength at all, which is why
        // #2498 saw `strength=2 / weak=1` standing on the Rust peer. Block is
        // part of the same wipe and is not otherwise cleared on death.
        if segment_peer_alive {
            // A whole-instance wipe, i.e. the exactly-zero removal edge
            // (`ShouldRemoveDueToAmount` `0x83b0d`); the ledger was already
            // reset above, so this only keeps the one-writer census total.
            write_monster_strength(monster, 0, crate::hot::Applier::None, upkeep);
            clear_monster_temp_strength_wrappers(monster, upkeep);
            monster.powers.set(PowerId::Weak, SlotWire::Int, 0);
            monster.powers.set(PowerId::Vuln, SlotWire::Int, 0);
            monster.powers.set(PowerId::Poison, SlotWire::Int, 0);
            monster.powers.set(PowerId::Hang, SlotWire::Int, 0);
            monster.block = 0;
            monster.revive_stage = super::monsters::SEGMENT_REVIVE_DEAD;
        }
        if monster.kind == MonsterKind::WaterfallGiant
            && monster.powers.value(PowerId::SteamPressure) > 0
            && !monster.is_about_to_blow()
        {
            // SteamEruptionPower.AfterDeath(false) runs after the ordinary
            // power-removal pass but before every other represented death
            // listener and the terminal check (`_finish_monster_death`, frozen Python, deleted #2827). This was an actual death, yet the one Steam Eruption
            // instance survives and replaces the owner with its sentinel
            // ABOUT form. Clearing the complete slot vector before restoring
            // Steam Pressure also makes future newly represented powers obey
            // native RemoveAllPowersAfterDeath by construction.
            let pressure = monster.powers.value(PowerId::SteamPressure);
            monster.powers = Slots::new();
            monster
                .powers
                .set(PowerId::SteamPressure, SlotWire::Int, pressure);
            monster.hp = 999_999_999;
            monster.max_hp = 999_999_999;
            monster.set_is_about_to_blow(true);
            monster.loop_pos = 6;
        }
        (monster.uid, monster.kind)
    };
    if test_subject_retains {
        super::monsters::latch_test_subject_after_death(state, target)?;
    }
    events.push(Event::MonsterDied { uid });
    if axebot_respawns {
        super::monsters::respawn_axebot(state, uid, events)?;
    }
    // The represented listeners of the deferred-choice `Hook.AfterDeath`
    // walk, which has no `WhenAll`: a choice begun in one resolves after the
    // rest of the walk and of the enclosing command (#3386,
    // `puzzle::DeferredChoiceListener`).
    let after_death_listener =
        super::puzzle::DeferredChoiceListener::enter(Some(super::puzzle::AFTER_DEATH_WALL));
    // DampenPower is the first represented player-power AfterDeath listener.
    // It restores the exact tracked atoms before Surrounded, Hex, Gremlin
    // Horn, physical-card listeners, and identity normalization.
    if kind == MonsterKind::MagiKnight && state.card_states.dampen().is_some() {
        super::cards::remove_dampen_after_death(state, uid)?;
    }
    // SurroundedPower is a player power, so its AfterDeath listener also
    // precedes the player-relic group. Face the sole surviving side before a
    // Gremlin Horn draw can AutoPlay a targeted card — the
    // `_kaiser_surrounded_after_actual_death` call inside
    // `_finish_monster_death`, frozen Python, deleted #2827.
    super::monsters::kaiser_surrounded_after_actual_death(state, target)?;
    // HexPower is a player-power AfterDeath listener and therefore precedes
    // every player relic, physical-card, and monster-model listener. The
    // narrow Knights quotient authenticates its one fixed Spectral owner.
    if kind == MonsterKind::SpectralKnight && state.powers.value(PowerId::HexPower) > 0 {
        let exact_owner = matches!(state.monsters.as_slice(), [flail, spectral, magi]
            if (flail.kind, spectral.kind, magi.kind)
                == (MonsterKind::FlailKnight, MonsterKind::SpectralKnight, MonsterKind::MagiKnight)
                && (spectral.slot, spectral.uid, Some(spectral.max_hp))
                    == (1, 1, super::monsters::native_fixed_hp(state, MonsterKind::SpectralKnight))
                && target == 1 && spectral.hp <= 0);
        if !exact_owner || state.powers.value(PowerId::HexPower) != 2 {
            return Err(EngineRefusal::MalformedArgs("Spectral Hex death owner"));
        }
        super::cards::remove_hex_power(state);
        note_power(events, Subject::Player, PowerId::HexPower, 0);
    }
    if state.fanouts.gremlin_horn_owned() {
        let catalog = catalog.expect("Gremlin Horn catalog preflighted");
        if !damage_combat_is_ending(state) && !state.fanouts.no_energy_gain() {
            state.energy = state
                .energy
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("Gremlin Horn energy"))?;
        }
        if !damage_combat_is_ending(state) {
            // A choice this Draw begins resolves in a queued hook action
            // after the rest of this walk and of the enclosing action
            // (#3387, `engine::hook_action`).
            super::draw::gremlin_horn_draw(state, catalog, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicGremlinHorn);
    }
    // The player's pile cards are the last allied listeners in the
    // `Hook.AfterDeath` walk (see the Infested spawn just below). Its all-piles
    // mapping validates/allocates every legacy card identity even when no
    // Melancholy is present. Keep this boundary unconditional: several deaths
    // in one Doom batch re-enter it, and the shared normalizer is idempotent.
    super::cards::physical_cards_after_actual_death(state)?;
    // `InfestedPower.AfterDeath` (`0xa3f70` -> `<AfterDeath>d__4::MoveNext`
    // `0x33d3ec`, `CreatureCmd.Add` of each Wriggler at IL_008b) is an ENEMY
    // power listener. `Hook.AfterDeath` (`<AfterDeath>d__28::MoveNext`
    // `0x3cd984` IL_0040) walks one `IterateHookListeners` snapshot, and
    // `CombatState.<IterateHookListeners>d__69::MoveNext` (`0x3f9720`) lists
    // `_allies` before `_enemies` (IL_0056-IL_0079), each player's powers,
    // relics, potions, orbs and pile cards (IL_008f-IL_019b) before any enemy
    // power. So Surrounded, Hex, Dampen, Gremlin Horn's GainEnergy + Draw and
    // the pile-card listeners above all run while the dead Phrog still owns
    // Infested and no Wriggler exists (#2656): a Hellraiser Strike drawn by
    // the Horn has no Wriggler to hit. Kill runs this walk (IL_03bc) before
    // `RemoveCreature` (IL_04d5) and `RemoveAllPowersAfterDeath` (IL_04f1),
    // which is why Infested's unconditional `ShouldStopCombatFromEnding`
    // (`0xa4017`) still vetoes the ending across the Horn window
    // ([`damage_combat_is_ending`]).
    if kind == MonsterKind::PhrogParasite {
        spawn_wrigglers(state)?;
    }
    if kind == MonsterKind::GremlinMerc {
        super::monsters::spawn_gremlin_merc_pair(state, target)?;
        state.monsters_mut()[target]
            .powers
            .set(PowerId::StolenGold, SlotWire::Int, 0);
    }
    // `_finish_monster_death_suffix` (frozen Python, deleted #2827) runs this cascade before both of the
    // hooks below, and Corpse Slug's Ravenous is its one admitted member.
    corpse_slug_ravenous_after_death(state, target, events)?;
    // `_content_monsters_after_actual_death` (frozen Python, deleted #2827) dispatches the Kaiser
    // branch from the same cascade, one `elif` after Corpse Slug's.
    super::monsters::crab_rage_after_death(state, target, events)?;
    super::monsters::queen_after_monster_death(state, dying_kind)?;
    restore_possess_debit_after_owner_death(state, target, events)?;
    // A select that resolved without a prompt in a listener other than
    // Gremlin Horn's (whose Draw counts its own, #3387) refuses here (#3485).
    after_death_listener.settle(Ok(()))?;
    if axebot_respawns || test_subject_retains {
        // The fresh replacement is already live; Stock's
        // ShouldStopCombatFromEnding result suppresses both the secondary
        // cascade and the terminal recomputation for this death.
        return Ok(());
    }
    let remaining: Vec<usize> = state
        .monsters
        .iter()
        .enumerate()
        .filter_map(|(index, monster)| (monster.hp > 0).then_some(index))
        .collect();
    if dying_was_primary
        && !remaining.is_empty()
        && remaining
            .iter()
            .all(|index| super::monsters::is_secondary_enemy(&state.monsters[*index]))
    {
        finish_secondary_death_cascade(state, &remaining, events)?;
    }
    if !state.any_monster_alive() && !state.fanouts.damage_batch_is_active() && !state.history.over
    {
        state.history.over = true;
        events.push(Event::CombatOver { player_won: true });
    }
    if state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        )
    }) && !super::monsters::queen_roster_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Queen/Amalgam stable death result",
        ));
    }
    if gremlin_merc_death && !super::monsters::gremlin_merc_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Gremlin Merc stable death result",
        ));
    }
    if kind == MonsterKind::ThievingHopper
        && !super::monsters::thieving_hopper_state_is_valid(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper stable death result",
        ));
    }
    Ok(())
}

/// `RavenousPower.AfterDeath` for the Corpse Slug roster
/// (`_content_monsters_after_actual_death` (frozen Python, deleted #2827)).
///
/// Current-build IL (v0.111.0, DLL 9cb4f1ad): `RavenousPower::AfterDeath` RVA
/// `0xa63e0` takes `(choiceContext, target, wasRemovalPrevented)` and its body
/// lives in `RavenousPower/<AfterDeath>d__6::MoveNext`; the dynamic move it
/// installs is `RavenousPower::StunnedMove` RVA `0xa643c`. The listener gates
/// on self, opposite side, a dead owner and prevented removal — every
/// surviving sibling qualifies, which is why the Python comment says so and
/// this walks the whole roster.
///
/// Each survivor is stunned to the one-shot `RAVENOUS_STUN_MOVE` parked over
/// the telegraph it was already showing, then gains Strength equal to its own
/// Ravenous amount. `_stun_monster` (frozen Python, deleted #2827) no-ops on a dead owner or a finished
/// combat, and `_set_monster_move_immediate` refuses to overwrite an
/// existing forced follow-up (native `SetMoveImmediate` with `force=false`
/// bails on an unperformed STUNNED), so a second stun before the first
/// performs is silently dropped — and the Strength still applies, because
/// Python gates that on the owner rather than on `changed`.
fn corpse_slug_ravenous_after_death(
    state: &mut HotState,
    dying: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.monsters[dying].kind != MonsterKind::CorpseSlug {
        return Ok(());
    }
    // `dying` is deliberately NOT passed here. `_finish_monster_death` has
    // already zeroed the dead slug's Ravenous (frozen Python, deleted #2827) by the time
    // `_finish_monster_death_suffix` reaches this cascade, so Python
    // validates with the default `dying=None` and the corpse is expected to
    // hold 0 like any other dead slug. Passing the dying index would demand
    // the live amount on a creature whose power was just cleared.
    if !super::monsters::corpse_slug_roster_is_valid(state, None) {
        return Err(EngineRefusal::MalformedArgs("Corpse Slug roster"));
    }
    for index in 0..state.monsters.len() {
        if index == dying || state.monsters[index].hp <= 0 {
            continue;
        }
        let monster = &state.monsters[index];
        // The parked telegraph: an already-forced follow-up wins, otherwise
        // the ordinary loop move this sibling is showing.
        let follow_up = if monster.forced_follow_up != MonsterFollowUp::None {
            monster.forced_follow_up
        } else {
            match monster.loop_pos {
                CORPSE_SLUG_WHIP_SLAP_INDEX => MonsterFollowUp::SlugWhipSlap,
                CORPSE_SLUG_GLOMP_INDEX => MonsterFollowUp::SlugGlomp,
                CORPSE_SLUG_GOOP_INDEX => MonsterFollowUp::SlugGoop,
                _ => {
                    return Err(EngineRefusal::MalformedArgs(
                        "Corpse Slug Ravenous telegraph",
                    ));
                }
            }
        };
        let ravenous = monster.powers.value(PowerId::Ravenous);
        let already_forced = monster.forced_follow_up != MonsterFollowUp::None;
        if !state.history.over {
            let monster = &mut state.monsters_mut()[index];
            if !already_forced {
                monster.override_state = MonsterOverride::RavenousStun;
                monster.forced_follow_up = follow_up;
            }
        }
        if !state.history.over && state.monsters[index].hp > 0 {
            let strength = state.monsters[index]
                .powers
                .value(PowerId::Strength)
                .checked_add(ravenous)
                .ok_or_else(|| overflow("Ravenous strength"))?;
            let uid = state.monsters[index].uid;
            let upkeep = state.fanouts.misery_attachment_upkeep();
            // `RavenousPower/<AfterDeath>d__6::MoveNext` IL_017d-IL_019c
            // applies to `Owner` with `Owner` as the applier.
            write_monster_strength(
                &mut state.monsters_mut()[index],
                strength,
                crate::hot::Applier::Monster(uid),
                upkeep,
            );
            note_power(events, Subject::Monster(uid), PowerId::Strength, strength);
        }
    }
    if !super::monsters::corpse_slug_roster_is_valid(state, None) {
        return Err(EngineRefusal::MalformedArgs("Corpse Slug roster result"));
    }
    Ok(())
}

/// PossessPower.AfterDeath for the admitted no-relic solo roster. Native runs
/// this after player/card death listeners and before the secondary cascade and
/// terminal boundary. Only the exact dead owner restores its own accumulated
/// source debit, and the dictionary is cleared after the serial Power command.
///
/// The restore is `PowerCmd.Apply<StrengthPower|DexterityPower>`
/// (`PossessSpeedPower/<AfterDeath>d__11::MoveNext` RVA `0x340f54`
/// IL_0077-IL_007f; `PossessStrengthPower/<AfterDeath>d__11::MoveNext` RVA
/// `0x3410d4`), and `PowerCmd/<Apply>d__1`1::MoveNext` (RVA `0x3ef988`)
/// returns at IL_0020-IL_0034 while `CombatManager.IsEnding` holds. So the
/// LAST primary owner's death restores nothing: its HP commit already made
/// the combat ending (#3214, fd41bbfb8f5a0529 step 26: native player
/// Dexterity stays -3 after The Forgotten dies last with a -4 debit). The
/// gate is the shared IsEnding projection, [`damage_combat_is_ending`].
fn restore_possess_debit_after_owner_death(
    state: &mut HotState,
    target: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(owner) = state.monsters.get(target) else {
        return Err(EngineRefusal::MalformedArgs("Possess death owner"));
    };
    if owner.hp > 0 {
        return Ok(());
    }
    let (power, debit, site) = match owner.kind {
        MonsterKind::TheLost => (
            PowerId::Strength,
            owner.possess_strength_debit,
            "Possess Strength restore",
        ),
        MonsterKind::TheForgotten => (
            PowerId::Dexterity,
            owner.possess_speed_debit,
            "Possess Speed restore",
        ),
        _ => return Ok(()),
    };
    if debit > 0 {
        return Err(EngineRefusal::MalformedArgs("Possess debit ledger"));
    }
    let amount = debit
        .checked_neg()
        .ok_or(EngineRefusal::CounterOverflow(site))?;
    if !damage_combat_is_ending(state) && amount != 0 {
        let updated = state
            .powers
            .value(power)
            .checked_add(amount)
            .ok_or(EngineRefusal::CounterOverflow(site))?;
        state.powers.set(power, SlotWire::Int, updated);
        note_power(events, Subject::Player, power, updated);
    }
    let owner = &mut state.monsters_mut()[target];
    match owner.kind {
        MonsterKind::TheLost => owner.possess_strength_debit = 0,
        MonsterKind::TheForgotten => owner.possess_speed_debit = 0,
        _ => unreachable!("Possess owner kind was matched above"),
    }
    Ok(())
}

/// Python `_continue_secondary_death_cascade` (frozen, deleted #2827), restricted to
/// the no-relic/no-suspension admission surface. The live secondary snapshot
/// is frozen in slot order and killed without entering Illusion revival.
fn finish_secondary_death_cascade(
    state: &mut HotState,
    cascade: &[usize],
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    for &index in cascade {
        let monster = state
            .monsters
            .get(index)
            .ok_or(EngineRefusal::CounterOverflow("secondary death cascade"))?;
        if monster.hp <= 0 {
            continue;
        }
        let dying_kind = monster.kind;
        let illusion = matches!(
            dying_kind,
            MonsterKind::Parafright | MonsterKind::EyeWithTeeth
        );
        state.monsters_mut()[index].hp = 0;
        remove_player_shrink_after_applier_death(state, dying_kind, events)?;
        let upkeep = state.fanouts.misery_attachment_upkeep();
        let monster = &mut state.monsters_mut()[index];
        if !illusion {
            // Owner-as-applier side-end restoration, removing each wrapper
            // before its own restoration (`0x348ba8` IL_0043,
            // IL_009d-IL_00c4); the ledger is reset below either way.
            unwind_monster_temp_strength_wrappers(monster, upkeep)?;
        }
        monster.powers.set(PowerId::Doom, SlotWire::Int, 0);
        monster.powers.set(PowerId::Demise, SlotWire::Int, 0);
        monster.powers.set(PowerId::Shrink, SlotWire::Int, 0);
        monster.powers.set(PowerId::Oblivion, SlotWire::Int, 0);
        monster.powers.set(PowerId::Strangle, SlotWire::Int, 0);
        monster.powers.set(PowerId::Hang, SlotWire::Int, 0);
        monster.powers.set(PowerId::CurlUp, SlotWire::Int, 0);
        monster.powers.set(PowerId::Poison, SlotWire::Int, 0);
        monster.powers.set(PowerId::SicEm, SlotWire::Int, 0);
        monster.powers.set(PowerId::Debilitate, SlotWire::Int, 0);
        monster.powers.set(PowerId::HatchPower, SlotWire::Int, 0);
        monster.powers.set(PowerId::HighVoltage, SlotWire::Int, 0);
        monster.powers.set(PowerId::Mplating, SlotWire::Int, 0);
        monster.poison_uid = -1;
        monster.curl_up_card_uid = -1;
        monster.set_louse_curled(false);
        monster.set_demise_after_intangible(false);
        monster.set_demise_before_ritual(false);
        monster.revive_stage = 0;
        monster.override_state = MonsterOverride::None;
        monster.forced_follow_up = MonsterFollowUp::None;
        monster.misery_debuff_order = Default::default();
        events.push(Event::MonsterDied { uid: monster.uid });
        super::cards::physical_cards_after_actual_death(state)?;
    }
    Ok(())
}

/// `ShrinkPower/<AfterDeath>d__16::MoveNext` RVA `0x3444e0` removes the
/// infinite player singleton when its retained Shrinker Beetle applier dies.
/// The compact solo projection pays no separate applier UID, so admission
/// proves the source-derived one-owner roster before this identity-equivalent
/// cleanup runs.
fn remove_player_shrink_after_applier_death(
    state: &mut HotState,
    dying_kind: MonsterKind,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if dying_kind != MonsterKind::ShrinkerBeetle || state.powers.value(PowerId::PlayerShrink) == 0 {
        return Ok(());
    }
    let unique_source = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::ShrinkerBeetle)
        .count()
        == 1;
    if state.powers.value(PowerId::PlayerShrink) != 1 || !unique_source {
        return Err(EngineRefusal::MalformedArgs(
            "player Shrink applier identity",
        ));
    }
    state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 0);
    note_power(events, Subject::Player, PowerId::PlayerShrink, 0);
    Ok(())
}

/// `InfestedPower.AfterDeath`: four uniquely rolled, initially stunned
/// Wrigglers join in creation order.
///
/// Each Wriggler is a `CreatureCmd.Add` creation and takes the next native
/// creature id, which a combat-start pet (Byrdpip, Osty) has already
/// advanced past the Phrog (#3039,
/// [`super::monsters::allocate_creature_uids`]). The uids are allocated
/// after the four HP rolls so a refused roll publishes nothing; the uid is
/// not an input to any roll.
fn spawn_wrigglers(state: &mut HotState) -> Result<(), EngineRefusal> {
    let live = state.rng.get(RngStream::Niche);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let mut taken: Vec<i32> = state
        .monsters
        .iter()
        .map(|monster| monster.max_hp)
        .collect();
    // `Wriggler` `GetValueIfAscension(8, 18..22, 17..21)` at the fight's
    // ascension (#2539).
    let (low, high) =
        crate::encounters::hp_band(crate::ids::MonsterKind::Wriggler, state.fanouts.ascension())
            .map_err(|_| EngineRefusal::MalformedArgs("Wriggler HP band"))?;
    let mut spawned = Vec::with_capacity(4);
    for offset in 0..4_u32 {
        let candidates: Vec<i32> = (low..=high).filter(|hp| !taken.contains(hp)).collect();
        let bound: i32 = candidates
            .len()
            .try_into()
            .map_err(|_| overflow("Wriggler HP candidates"))?;
        let pick: usize = rng
            .next_bounded(bound)
            .map_err(|_| overflow("Wriggler HP candidates"))?
            .try_into()
            .map_err(|_| overflow("Wriggler HP result"))?;
        let hp = candidates[pick];
        taken.push(hp);
        let mut monster = HotMonster::new(crate::ids::MonsterKind::Wriggler, hp);
        monster.max_hp = hp;
        monster.loop_pos = i32::try_from(offset % 2).unwrap_or(0);
        monster.spawn_noop = true;
        spawned.push(monster);
    }
    let next_uid = super::monsters::allocate_creature_uids(state, 4)?;
    for (offset, monster) in (0..4_u32).zip(spawned.iter_mut()) {
        monster.uid = next_uid
            .checked_add(offset)
            .ok_or_else(|| overflow("monster uid"))?;
    }
    state.monsters_mut().extend(spawned);
    for offset in 0..4_u32 {
        let uid = next_uid
            .checked_add(offset)
            .ok_or_else(|| overflow("monster uid"))?;
        super::monsters::fur_coat_after_opponent_added(state, uid)?;
    }
    state.rng.set(
        RngStream::Niche,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(())
}

/// One monster attack command (`monster_attack_player`, frozen Python, deleted #2827).
///
/// Python's loop body returns after the first iteration and recurses through
/// `_monster_attack_after_paper_cuts` for the remaining hits; the flat loop
/// here is that recursion unrolled. Scroll of Biting's fixed Paper Cuts 2
/// result is synchronous because its enemy-side max-HP loss reaches no owner
/// relic listener (Centennial Puzzle's receipt-owned Draw, #3114, fires from
/// the hit's own AfterDamageReceived walk, never from Paper Cuts);
/// represented Reflect and Flame Barrier results are likewise synchronous
/// against admitted rosters. The positive-result receipt closes Test
/// Subject's synchronous Painful Stabs continuation without changing any
/// existing attack caller's terminal behavior.
#[inline(always)]
pub(crate) fn monster_attack_player_positive_results(
    state: &mut HotState,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<i64, EngineRefusal> {
    if state.fanouts.puzzle_armed() {
        return Err(EngineRefusal::MalformedArgs(
            "Centennial Puzzle monster attack requires catalog",
        ));
    }
    // A live Necro Mastery callback can fail after pet loss, player spill, or
    // one or more enemy deaths (including the physical-card death walk).
    // Rehearse the complete multi-hit AttackCommand before publishing any of
    // those prefixes. The ordinary no-pet/no-power hot path remains unchanged.
    if state.fanouts.pet().osty().is_some() && state.powers.value(PowerId::NecroMastery) != 0
        || state.after_damage_received_order_is_reachable()
    {
        rehearse_necro_mastery_monster_attack(state, None, actor, base, hits)?;
    }
    monster_attack_player_positive_results_apply(state, None, actor, base, hits, events)
}

pub(crate) fn monster_attack_player_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    monster_attack_player_positive_results_with_catalog(state, catalog, actor, base, hits, events)
        .map(|_| ())
}

/// [`monster_attack_player_positive_results`] for a move body, which always
/// holds the catalog. Every refusal the catalogless entry adds is a missing
/// catalog: Centennial Puzzle, the Hopper/Dampen/Aeonglass public guard, and
/// a player-Thorns retaliation kill reaching Gremlin Horn's AfterDeath draw
/// in `finish_monster_death_body` (#2927).
pub(crate) fn monster_attack_player_positive_results_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<i64, EngineRefusal> {
    if state.fanouts.pet().osty().is_some() && state.powers.value(PowerId::NecroMastery) != 0
        || state.after_damage_received_order_is_reachable()
        || state.fanouts.puzzle_armed()
    {
        rehearse_necro_mastery_monster_attack(state, Some(catalog), actor, base, hits)?;
    }
    monster_attack_player_positive_results_apply(state, Some(catalog), actor, base, hits, events)
}

/// Execute the one-hit catalog-bearing attack used by Bowlbug Headbutt.
///
/// `CreatureCmd/<Damage>d__12::MoveNext` 0x3e96c8 calculates `WasFullyBlocked`
/// before Osty interposes (IL0505–0554), then copies that same flag to the
/// original player result (IL069f–06d3). Since #2647 slice A the flag has no
/// out-parameter: its one listener is the owner-side
/// [`monsters_after_damage_given`] walk, dispatched inside the command at the
/// native hook point, so no move body can derive it from post-command HP
/// either. What survives here is the single-result pin — #2647 §5.9, and the
/// reason Bowlbug's `(16, 1)` row is the only shape `headbutt` accepts.
pub(crate) fn monster_attack_player_with_catalog_result(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if hits != 1 {
        return Err(EngineRefusal::MalformedArgs(
            "single-result monster attack hit count",
        ));
    }
    if state.fanouts.pet().osty().is_some() && state.powers.value(PowerId::NecroMastery) != 0
        || state.after_damage_received_order_is_reachable()
        || state.fanouts.puzzle_armed()
    {
        rehearse_necro_mastery_monster_attack(state, Some(catalog), actor, base, hits)?;
    }
    monster_attack_player_positive_results_apply(state, Some(catalog), actor, base, hits, events)?;
    Ok(())
}

/// Cold whole-command rehearsal for the live Necro Mastery reader.
///
/// Keeping the clone and second attack walk out of the ordinary attack
/// wrapper lets its always-inlined absent branch hand off directly to the
/// unchanged large attack body.
#[cold]
#[inline(never)]
fn rehearse_necro_mastery_monster_attack(
    state: &HotState,
    catalog: Option<&Catalog>,
    actor: usize,
    base: i64,
    hits: i64,
) -> Result<(), EngineRefusal> {
    let mut probe = state.clone();
    monster_attack_player_positive_results_apply(
        &mut probe,
        catalog,
        actor,
        base,
        hits,
        &mut Vec::new(),
    )?;
    Ok(())
}

/// One monster attack's frozen per-hit damage against the player.
///
/// This is the single computation behind both the executed hit (the loop in
/// [`monster_attack_player_positive_results_apply`]) and the intent the game
/// displays for it ([`crate::engine::turn::monster_intents`]). Native agrees
/// that the two are one computation: v0.111.0 (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// `AttackIntent::GetSingleDamage` RVA `0x79890` IL_0018-IL_0052 calls
/// `Hook::ModifyDamage` with the local player's creature as target, the
/// monster as dealer, `DamageCalc()` as amount, props `8` (`ValueProp.Move`),
/// no card source and hook type `14`, and `CreatureCmd/<Damage>d__12::MoveNext`
/// RVA `0x3e96c8` IL_0184-IL_01b2 makes the same call for the executed hit
/// with the attack's `DamageProps` (`AttackCommand::.ctor` RVA `0x1349a0`
/// IL_0014 stores `8`) and the same hook type `14`, a null card for a monster
/// dealer. The intent then truncates the decimal and floors it at zero
/// (IL_0058-IL_0064), the same `trunc` this snapshot ends with.
///
/// Pure: it records no coverage and writes nothing, so a read-only intent
/// query can call it. The executed hit records Paper Krane's coverage itself.
#[inline]
pub(crate) fn monster_attack_hit_damage(
    state: &HotState,
    monster: &HotMonster,
    base: i64,
) -> Result<i64, EngineRefusal> {
    // The frozen per-hit snapshot (`_monster_attack_damage_snapshot`, frozen Python, deleted #2827): base + Strength + Vigor + Tainted, then the Weak, player
    // Vulnerable, Colossus and Intercept/Covered factors, then the
    // Intangible cap, one floor,
    // and **then a truncation to an integer**. Shrink and Kaiser's
    // Surrounded back-attack factor are both represented below; Intercept
    // is the represented zero/doubling factor.
    // Every admitted term is read before player Thorns can mutate the
    // dealer, so the in-flight hit remains frozen if retaliation kills it.
    let strength = monster.powers.value(PowerId::Strength);
    let vigor = monster.powers.value(PowerId::Vigor);
    let tainted = state.powers.value(PowerId::Tainted);
    let weak = monster.powers.value(PowerId::Weak) > 0;
    let vulnerable = state.powers.value(PowerId::PlayerVuln) > 0;
    let mut damage = DotNetDecimal::from_i64(
        base.checked_add(i64::from(strength))
            .and_then(|value| value.checked_add(i64::from(vigor)))
            .and_then(|value| value.checked_add(i64::from(tainted)))
            .ok_or_else(|| overflow("monster attack base"))?,
    );
    if weak {
        damage = damage
            .checked_mul(weak_multiplier(
                monster.powers.value(PowerId::Debilitate),
                state.paper_krane_owned(),
            )?)
            .map_err(|_| overflow("monster attack damage"))?;
    }
    if vulnerable {
        damage = damage
            .checked_mul(vulnerable_multiplier(state, false, 0, false)?)
            .map_err(|_| overflow("monster attack damage"))?;
    }
    if monster.powers.value(PowerId::Shrink) > 0 {
        // ShrinkPower.ModifyDamageMultiplicative RVA 0xa784c: an
        // owner-dealt powered attack receives one exact x7/10 factor,
        // after player Vulnerable and before Colossus. Amount is finite
        // duration only, never a repeated multiplier.
        damage = damage
            .checked_mul(
                DotNetDecimal::ratio(7, 10).map_err(|_| overflow("monster Shrink multiplier"))?,
            )
            .map_err(|_| overflow("monster attack damage"))?;
    }
    if state.powers.value(PowerId::Colossus) > 0 && monster.powers.value(PowerId::Vuln) > 0 {
        damage = damage
            .checked_mul(DotNetDecimal::ratio(1, 2).map_err(|_| overflow("colossus multiplier"))?)
            .map_err(|_| overflow("monster attack damage"))?;
    }
    let covered = state.fanouts.intercept_covered_mask();
    if covered != 0 {
        let multiplier = if covered & 0b01 != 0 {
            0
        } else {
            1 + covered.count_ones()
        };
        damage = damage
            .checked_mul(DotNetDecimal::from_i64(i64::from(multiplier)))
            .map_err(|_| overflow("Intercept incoming damage"))?;
    }
    // `SurroundedPower::ModifyDamageMultiplicative` `0xa8bc4` is an
    // ordinary multiplicative listener on the PLAYER, so it folds in the
    // same `Hook.ModifyDamageInternal` `0x106aa0` pass (IL_0098) as Weak,
    // Vulnerable, Shrink and Colossus. Python's frozen snapshot applies it
    // last among the factors and before the floor
    // (`_monster_attack_damage_snapshot`, frozen Python, deleted #2827), which is the position
    // taken here. The 3/2 is the `Decimal(15, 0, 0, false, 1)` at IL_0057.
    if super::monsters::kaiser_back_attacks(state, monster) {
        damage = damage
            .checked_mul(
                DotNetDecimal::ratio(3, 2)
                    .map_err(|_| overflow("Kaiser back-attack multiplier"))?,
            )
            .map_err(|_| overflow("monster attack damage"))?;
    }
    if undying_sigil_halves(state, monster) {
        damage = damage
            .checked_mul(
                DotNetDecimal::ratio(1, 2).map_err(|_| overflow("Undying Sigil multiplier"))?,
            )
            .map_err(|_| overflow("monster attack damage"))?;
    }
    if damage < DotNetDecimal::zero() {
        damage = DotNetDecimal::zero();
    }
    if state.powers.value(PowerId::Intangible) > 0 {
        damage = damage.min(DotNetDecimal::from_i64(1));
    }
    // The truncation closes the snapshot. Python's
    // `_monster_attack_damage_snapshot` applies `int(damage)` (frozen Python, deleted #2827)
    // only for Test Subject; `monster_attack_player` passes
    // `truncate=m.kind == TEST_SUBJECT`. Every other monster
    // carries the Fraction into `_commit_monster_attack_hit`, blocks via
    // `min(s.block, dmg)` /
    // `int(blocked)`, and truncates the remainder in
    // `_commit_player_hp_loss`. Truncating up front instead is
    // equivalent for admitted entries (proven in the #1385 review: Block
    // is integral, so the fractional part never changes which side of the
    // block boundary a point of damage lands on). Weak is the first
    // modifier that can make the amount fractional.
    damage
        .trunc_i64()
        .map_err(|_| overflow("monster attack damage"))
}

/// Whether the player's Undying Sigil halves this monster's powered attack.
///
/// v0.111.0 (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// `UndyingSigil::ModifyDamageMultiplicative` RVA `0x9d400`, params
/// `(target, amount, props, dealer, cardSource)`:
/// - IL_000c-IL_0015: a null `dealer` returns `Decimal.One`;
/// - IL_0016-IL_0023: `props.IsPoweredAttack()` is required;
/// - IL_0024-IL_0037: `target` must be the owner's creature (the player);
/// - IL_0038-IL_004c: `dealer` must not be the owner's creature;
/// - IL_004d-IL_0062: `dealer.CurrentHp <= dealer.GetPowerAmount<DoomPower>()`
///   (MethodSpec `0x2b0009af` instantiates `GetPowerAmount` over
///   `MegaCrit.Sts2.Core.Models.Powers.DoomPower`) — the DEALER's own Doom;
/// - IL_0063-IL_0078: returns the `DamageDecrease` DynamicVar, which
///   `get_CanonicalVars` (RVA `0x9d3c5`) builds as `Decimal(5, 0, 0, 0, 1)`
///   = exactly 1/2.
///
/// So a doomed enemy's attack on the player is halved and the player's own
/// attacks are never touched. The one powered monster-to-player computation
/// is [`monster_attack_hit_damage`] (props `8`, the player as target, the
/// monster as dealer), which both the executed hit and the displayed intent
/// read (`AttackIntent::GetSingleDamage` RVA `0x79890` calls the same
/// `Hook::ModifyDamage`). The relic is a player-owned listener, gated on
/// `Player.IsActiveForHooks` like every relic; a player whose hooks are
/// deactivated has died, and a dead player is never the target of a further
/// hit. The factor folds in the same `Hook.ModifyDamageInternal` `0x106aa0`
/// multiplicative pass as Weak/Vulnerable/Shrink/Colossus/Surrounded,
/// before the zero floor and the Intangible cap; every admitted factor is an
/// exact small-denominator decimal, so its position inside the product
/// cannot change the value.
#[inline]
fn undying_sigil_halves(state: &HotState, monster: &HotMonster) -> bool {
    monster.hp <= monster.powers.value(PowerId::Doom) && state.fanouts.undying_sigil_owned()
}

/// Native CreatureCmd.Damage d12 0x3e96c8 (v111 DLL
/// 9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4)
/// publishes the [Osty, player] HP split, retaining a zero-spill player
/// result and its original Block flags (IL04bc–06d3). Each result runs its
/// HPChanged/Given hooks, then either queues a frozen lethal/live-dead
/// receiver or runs Received (IL0bb0–0d86). Only after every result does
/// Kill drain (IL0ead–0eb4). FlameBarrier0x33a358 and Reflect0x342314 target
/// only their owner's player result; zero spill does not suppress them.
/// Solo queued-player prevention belongs to final Kill, never before Received.
fn monster_attack_player_positive_results_apply(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<i64, EngineRefusal> {
    let mut positive_results = 0_i64;
    // `SuckPower/<AfterAttack>d__6::MoveNext` RVA `0x346c58` counts *hit
    // groups*, not results: for each group it drops the player's own result
    // whenever a pet of the player absorbed in that group
    // (`<>c__DisplayClass6_0::<AfterAttack>b__2` RVA `0x346c32`), then
    // increments once if anything left in the group took unblocked damage
    // (`<>c::<AfterAttack>b__6_1` RVA `0x346c1f`). Python represents that as
    // `_commit_monster_attack_hit` (frozen Python, deleted #2827): `(pet_lost > 0) if had_ally else
    // (hp_lost > 0)`, gated on the **live** owner amount — a Thorns kill
    // during the in-flight hit clears Suck before this increment, so the
    // group that killed the owner does not count. Hoisting the entry
    // ownership test keeps the ordinary no-Suck attack at one bool per hit.
    let suck_owner = state
        .monsters
        .get(actor)
        .is_some_and(|monster| monster.powers.value(PowerId::Suck) > 0);
    let mut suck_triggers = 0_i64;
    for _ in 0..hits {
        // AttackCommand re-checks `Attacker.IsDead` at the head of every hit
        // iteration (0x3ef17c IL_0156-0161): a retaliation kill during hit N
        // cancels hits N+1.. but never hit N itself (#799).
        if state.history.over {
            return Ok(positive_results);
        }
        let Some(monster) = state.monsters.get(actor) else {
            break;
        };
        if monster.hp <= 0 {
            break;
        }
        // `Hook.AfterDamageGiven` carries the dealer `Creature`, not a roster
        // index, and a death inside this command shifts indices — so the
        // owner-side walk below is keyed by creation-order uid.
        let dealer_uid = monster.uid;

        let damage = monster_attack_hit_damage(state, monster, base)?;
        if monster.powers.value(PowerId::Weak) > 0 && state.paper_krane_owned() {
            crate::coverage::record_relic(RelicId::RelicPaperKrane);
        }
        if undying_sigil_halves(state, monster) {
            crate::coverage::record_relic(RelicId::RelicUndyingSigil);
        }

        // Player Thorns retaliates after the snapshot but before block. A
        // lethal retaliation never cancels this already-started hit; only the
        // next AttackCommand loop head re-checks the dealer (#799).
        let thorns = state.powers.value(PowerId::Thorns);
        if thorns > 0 {
            if state.monsters[actor].kind == MonsterKind::ThievingHopper
                && super::monsters::thieving_hopper_internal_state_is_valid(state)
                && !super::monsters::thieving_hopper_state_is_valid(state)
            {
                // THIEVERY installs Swipe before its attack.  That exact
                // loop-zero prefix is internal to the action's clone, so the
                // retaliation must not re-enter the stable public damage
                // boundary after the theft has already changed DeckVersion.
                damage_monster_inner(
                    state,
                    actor,
                    DotNetDecimal::from_i64(i64::from(thorns)),
                    false,
                    true,
                    AttackObservation {
                        source_uid: None,
                        result_sink: None,
                        dealer: AttackDealer::Player,
                        catalog,
                    },
                    events,
                )?;
            } else {
                damage_monster_with_optional_catalog(
                    state,
                    catalog,
                    actor,
                    DotNetDecimal::from_i64(i64::from(thorns)),
                    false,
                    true,
                    events,
                )?;
            }
        }
        // `_commit_monster_attack_hit` (frozen Python, deleted #2827): `blocked = min(s.block, dmg)`
        // and `unblocked = max(0, dmg - blocked)`, both integer.
        let block_before = state.block;
        let blocked_int: i32 = i64::from(block_before)
            .min(damage)
            .try_into()
            .map_err(|_| overflow("player blocked damage"))?;
        state.block -= blocked_int;
        let mut unblocked_int = (damage - i64::from(blocked_int)).max(0);
        // `CreatureCmd/<Damage>d__12::MoveNext` 0x3e96c8 IL0505–0554 records
        // the result flag
        // before pet interposition. It is not equivalent to player HP loss:
        // retained Block makes a zero-damage hit fully blocked, while a Block
        // prefix followed by Osty absorption leaves nonzero pre-pet damage.
        let was_fully_blocked = (blocked_int > 0 || state.block > 0) && unblocked_int == 0;
        let mut pet_actual_loss = 0_i32;
        let mut pet_died = false;
        // `_commit_monster_attack_hit` (frozen Python, deleted #2827) snapshots `had_ally` before the
        // interpose, so a pet that dies to this hit still routes Suck's
        // per-group predicate through the pet's loss rather than the player's.
        let had_ally = state.fanouts.pet().osty().is_some();
        if let Some(osty) = state.fanouts.pet().osty() {
            let pet_loss: i32 = unblocked_int
                .min(i64::from(osty.hp()))
                .try_into()
                .map_err(|_| overflow("Osty incoming attack"))?;
            state
                .fanouts
                .mutate_pet(|pet| {
                    pet_actual_loss = pet.lose_hp(pet_loss)?;
                    Ok(())
                })
                .map_err(|_| overflow("Osty incoming attack"))?;
            unblocked_int -= i64::from(pet_actual_loss);
            pet_died = pet_actual_loss > 0 && state.fanouts.pet().osty().is_none();
        }

        let hp_lost = commit_player_hp_loss(state, unblocked_int, blocked_int, events)?;
        // The player result retains pre-interposition WasFullyBlocked
        // (Damage0x3e96c8 IL069f–06d3). EmotionChip's history predicate
        // 0x930d5 reads that flag, not positive player spill: absorbed damage
        // must not become a fully-blocked result merely because some Block
        // was also consumed. Its Received callback only changes UI status.
        if pet_actual_loss > 0 && state.emotion_chip_owned() {
            state.set_emotion_damage_current_turn(true);
        }
        let player_was_target_killed = state.hp <= 0;
        if hp_lost > 0 {
            positive_results =
                positive_results
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow(
                        "monster attack positive results",
                    ))?;
        }
        if suck_owner
            && (if had_ally {
                pet_actual_loss > 0
            } else {
                hp_lost > 0
            })
            && state
                .monsters
                .get(actor)
                .is_some_and(|monster| monster.powers.value(PowerId::Suck) > 0)
        {
            suck_triggers = suck_triggers
                .checked_add(1)
                .ok_or(EngineRefusal::CounterOverflow("SuckPower result count"))?;
        }

        // CreatureCmd.Damage 0x3e96c8 commits both HP writes, then walks
        // [pet, player] results. Each result does HPChanged/Given before its
        // frozen-WasTargetKilled && live-IsDead enqueue decision. Actual Kill
        // is delayed until every result's Received walk has completed.
        necro_mastery_after_pet_hp_lost_with_catalog(state, catalog, pet_actual_loss, events)?;
        let pet_death_queued = pet_died && state.fanouts.pet().osty().is_none();

        // `_commit_monster_attack_hit` (frozen Python, deleted #2827): PaperCutsPower belongs
        // intrinsically to Scroll of Biting and fires after the powered hit
        // result, but before that result's lethal/player-power suffix. The
        // owner-live check is post-Thorns, so a retaliation kill suppresses
        // Paper Cuts without retracting the already committed attack.
        if state.monsters[actor].kind == MonsterKind::ScrollOfBiting
            && state.monsters[actor].hp > 0
            && hp_lost > 0
        {
            lose_player_max_hp_from_paper_cuts(state, catalog, 2, events)?;
        }
        // `CreatureCmd/<Damage>d__12::MoveNext` 0x3e96c8 IL0b07–0d86 runs each
        // result's owner-side `Hook.AfterDamageGiven` (IL0cc4) BEFORE that
        // result's lethal/live-dead enqueue decision and its
        // `AfterDamageReceived` walk (IL0d86).
        //
        // PaperCuts above is the other monster-ownable listener on this hook
        // and is hand-inlined here. One owner CAN carry both — `dealer ==
        // Owner` filters the dealer, not the power, so it would not stop them
        // co-firing. Their order is unobservable for a different reason: they
        // write disjoint state (PaperCuts loses the player max HP, Imbalanced
        // writes the dealer's own override) and neither reads the other's.
        // Besides, PaperCuts is intrinsic to Scroll of Biting and Imbalanced to
        // Bowlbug, so no admitted roster puts both on one creature.
        monsters_after_damage_given(state, dealer_uid, was_fully_blocked)?;
        let player_death_queued = player_was_target_killed && state.hp <= 0;
        // A lethal result skips Received even if the final Kill will be
        // prevented by Fairy/Lizard Tail. Revival by an earlier callback can
        // instead make the receiver live before this queue decision.
        if !player_death_queued && !state.fanouts.player_hooks_deactivated() {
            // Hook.AfterDamageReceived snapshots the complete player-power list,
            // then rechecks live membership before each callback. This closed trio
            // is the only admitted co-firing subset; owner-side Inferno/Rupture
            // predicates are false for the represented enemy hit.
            let order = state.after_damage_received_power_order().ok_or(
                EngineRefusal::PowerOrderNotModeled("after-damage-received power order"),
            )?;
            for token in order.iter().copied() {
                // A prior callback (including Gambit) can actually kill
                // the player and deactivate every owner hook. Raw HP is not
                // this latch: a lethal result can still await final Kill.
                if state.fanouts.player_hooks_deactivated() {
                    break;
                }
                let power = token.power();
                let amount = state.powers.value(power);
                if amount <= 0 {
                    continue;
                }
                match token {
                    AfterDamageReceivedPower::TheGambit if hp_lost > 0 => {
                        if !state.unregister_after_damage_received_power(power) {
                            return Err(EngineRefusal::PowerOrderNotModeled(
                                "after-damage-received power removal",
                            ));
                        }
                        state.powers.set(PowerId::TheGambit, SlotWire::Int, 0);
                        note_power(events, Subject::Player, power, 0);
                        let hp_before = state.hp;
                        state.hp = 0;
                        // `Kill` `0x3ebe90` IL_01ca-IL_0200: a living
                        // receiver loses its HP and raises
                        // `AfterCurrentHpChanged` before `ShouldDie`.
                        red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
                        // Power removal is ungated. Solo Kill still reaches the
                        // lethal resolver even when an earlier retaliation began
                        // the win transition; a later damage child is ending-gated.
                        resolve_player_lethal_with_catalog(state, catalog, events, true)?;
                    }
                    AfterDamageReceivedPower::Reflect if blocked_int > 0 && !state.history.over => {
                        damage_monster_with_optional_catalog(
                            state,
                            catalog,
                            actor,
                            DotNetDecimal::from_i64(i64::from(blocked_int)),
                            false,
                            true,
                            events,
                        )?;
                    }
                    AfterDamageReceivedPower::FlameBarrier if !state.history.over => {
                        damage_monster_with_optional_catalog(
                            state,
                            catalog,
                            actor,
                            DotNetDecimal::from_i64(i64::from(amount)),
                            false,
                            true,
                            events,
                        )?;
                    }
                    _ => {}
                }
            }
            // Beating Remnant is a relic listener in the same
            // AfterDamageReceived walk, after the represented player-power
            // listeners. A queued lethal result never reaches this suffix,
            // even when its later ShouldDie walk prevents actual death.
            // Unlike Demon Tongue, it is not gated on
            // the current side being the owner, so enemy multi-hit attacks must
            // publish every surviving hit before starting the next one.
            if !state.fanouts.player_hooks_deactivated() {
                after_owner_damage_received_relics(state, catalog, hp_lost, events)?;
            }
        }
        if pet_death_queued {
            finish_queued_osty_death(state, catalog, events)?;
        }
        if player_death_queued {
            // Kill acts on the queued retained identity, including a receiver
            // restored by a later callback. Ordinary solo preventers run now.
            state.hp = 0;
            if !resolve_player_lethal_with_catalog(state, catalog, events, true)? {
                return Ok(positive_results);
            }
        }
    }
    // `_finish_monster_attack_command` (frozen Python, deleted #2827): the attack-scoped Vigor
    // snapshot is consumed once per command, after all of its hits have
    // resolved, and Suck's `Hook.AfterAttack` listener runs after it. Painful
    // Stabs is the caller's business: it is Test Subject-private and its one
    // move body consumes the returned positive-result receipt directly.
    if let Some(monster) = state.monsters_mut().get_mut(actor) {
        let vigor = monster.powers.value(PowerId::Vigor);
        if vigor > 0 {
            let uid = monster.uid;
            monster.powers.set(PowerId::Vigor, SlotWire::Int, 0);
            note_power(events, Subject::Monster(uid), PowerId::Vigor, 0);
        }
    }
    // `SuckPower/<AfterAttack>d__6::MoveNext` RVA `0x346c58` IL_0160-IL_018e:
    // once the whole command's groups are counted, a positive count Flashes
    // and awaits one `Apply<StrengthPower>(Owner, Amount * count, Owner)`.
    // `PowerCmd::Apply` gates on combat-ending and `CanReceivePowers`, so a
    // terminal command or a dead owner grants nothing —
    // `_finish_monster_attack_command`'s `not s.over and m.hp > 0` (frozen Python, deleted #2827).
    // The single Apply is why this is `Amount * count` rather than `count`
    // separate additions: no intervening listener can observe the partial sums.
    if suck_triggers > 0 && !state.history.over {
        let Some(monster) = state.monsters.get(actor) else {
            return Ok(positive_results);
        };
        // `_validated_suck_amount` (frozen Python, deleted #2827): FossilStalker's
        // `AfterAddedToRoom` d__22 is the only writer and applies exactly
        // `SuckPower(3)`. Any other live projection is an unreviewed power
        // census and must refuse rather than approximate (I5).
        if monster.kind != MonsterKind::FossilStalker
            || (monster.hp > 0 && monster.powers.value(PowerId::Suck) != 3)
        {
            return Err(EngineRefusal::MalformedArgs("SuckPower owner/amount"));
        }
        let amount = monster.powers.value(PowerId::Suck);
        if amount > 0 && monster.hp > 0 {
            let gained: i32 = i64::from(amount)
                .checked_mul(suck_triggers)
                .and_then(|gained| i32::try_from(gained).ok())
                .ok_or(EngineRefusal::CounterOverflow("SuckPower Strength"))?;
            let updated =
                checked_monster_strength_successor(monster, gained, "SuckPower Strength")?;
            let uid = monster.uid;
            let upkeep = state.fanouts.misery_attachment_upkeep();
            // `SuckPower/<AfterAttack>d__6::MoveNext` IL_016d-IL_018e applies
            // to `Owner` with `Owner` as the applier.
            write_monster_strength(
                &mut state.monsters_mut()[actor],
                updated,
                crate::hot::Applier::Monster(uid),
                upkeep,
            );
            note_power(events, Subject::Monster(uid), PowerId::Strength, updated);
        }
    }
    Ok(positive_results)
}

pub fn monster_attack_player(
    state: &mut HotState,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.card_states.hopper().is_some()
        || state.card_states.dampen().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper)
        || super::cards::aeonglass_owner_reachable(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper public monster attack requires catalog",
        ));
    }
    monster_attack_player_positive_results(state, actor, base, hits, events).map(|_| ())
}

/// Monster attack after a catalog-bearing turn/move boundary authenticated
/// the exact Dampen snapshot. Kept separate from the catalogless public API.
pub(crate) fn monster_attack_player_dampen_transient(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !super::cards::dampen_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen internal monster attack entry",
        ));
    }
    monster_attack_player_with_catalog(state, catalog, actor, base, hits, events)
}

/// Monster attack below an already catalog-authenticated Aeonglass turn.
pub(crate) fn monster_attack_player_aeonglass_transient(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !super::cards::aeonglass_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Aeonglass internal monster attack entry",
        ));
    }
    monster_attack_player_with_catalog(state, catalog, actor, base, hits, events)
}

pub(crate) fn monster_attack_player_hopper_transient(
    state: &mut HotState,
    catalog: &Catalog,
    actor: usize,
    base: i64,
    hits: i64,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !super::monsters::thieving_hopper_internal_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper internal monster attack entry",
        ));
    }
    monster_attack_player_with_catalog(state, catalog, actor, base, hits, events)
}

/// Apply one native `CreatureCmd::LoseMaxHp` to the owner player.
///
/// Current v0.111.0 `CreatureCmd/<LoseMaxHp>d__23::MoveNext` **0x3ec9bc**
/// (DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`)
/// computes the raw new cap, awaits one null-dealer unpowered/unblockable
/// Damage command when current HP exceeds it, then awaits
/// `SetMaxHp(max(1, raw))`. `isFromCard` changes only the Damage props from 6
/// to 14: it does not supply the active CardModel as the damage source.
///
/// Python `lose_player_max_hp` (frozen, deleted #2827) preserves the same ordering. The
/// owner-side form reaches immediate-source Rupture and Inferno after a
/// surviving positive result; the enemy-side Paper Cuts form reaches neither.
/// Beating Remnant, Centennial Puzzle and Self-Forming Clay remain refused
/// before this hot path; Demon Tongue uses the exact owner-side suffix below.
fn lose_player_max_hp_apply(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    amount: i32,
    current_side_is_owner: bool,
    site: &'static str,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs(site));
    }
    let new_max = state
        .max_hp
        .checked_sub(amount)
        .ok_or_else(|| overflow(site))?;
    if new_max < state.hp {
        let nested = i64::from(state.hp) - i64::from(new_max);
        let hp_lost = commit_player_hp_loss(state, nested, 0, events)?;
        // LoseMaxHp continues to SetMaxHp even when the nested damage is
        // lethal. Keep the terminal flag, but never return before the cap is
        // published below.
        if resolve_player_lethal_with_catalog(state, catalog, events, true)?
            && current_side_is_owner
        {
            // Props 14 still carries a null card source, so Rupture applies
            // immediately instead of joining CardPlayFrame's source-keyed
            // batch. Inferno follows Rupture in the represented owner walk.
            // The owner-side walk keeps the command's catalog (#3172):
            // Centennial Puzzle's Draw needs it, as on every other owner
            // damage path.
            rupture_after_owner_hp_loss(state, hp_lost, events)?;
            inferno_after_player_hp_loss(state, catalog, hp_lost, events)?;
            after_owner_damage_received_relics(state, catalog, hp_lost, events)?;
        }
    }
    // `SetMaxHp` (`<SetMaxHp>d__24` `0x3ecf3c`) reaches
    // `Creature::SetMaxHpInternal` (`0x11d754`), whose IL_003a-IL_004c clamp
    // writes the HP property directly: no `AfterCurrentHpChanged`. A lower cap
    // can lift the player above Red Skull's threshold with its Strength still
    // held, and native then keeps it until the NEXT creature's HP changes —
    // a monster's included. That stale latch is not represented (#3044).
    let hp_before_cap = state.hp;
    let max_hp_before_cap = state.max_hp;
    state.max_hp = new_max.max(1);
    if state.hp > state.max_hp {
        state.hp = state.max_hp;
    }
    if state.fanouts.red_skull_owned()
        && !state.fanouts.player_hooks_deactivated()
        && !state.history.over
        && red_skull_threshold_met(hp_before_cap, max_hp_before_cap)
            != red_skull_threshold_met(state.hp, state.max_hp)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Red Skull Strength held across a max-HP loss",
        ));
    }
    Ok(())
}

/// Scroll of Biting's intrinsic enemy-side `PaperCutsPower` max-HP loss.
fn lose_player_max_hp_from_paper_cuts(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    lose_player_max_hp_apply(state, catalog, amount, false, "Paper Cuts max HP", events)
}

/// Brightest Flame's owner-side, card-originated max-HP loss.
///
/// The sole caller owns clone rehearsal around the surrounding Energy/Draw
/// transaction, so the nested damage/listener/monster-death suffix is atomic
/// without adding another state clone to the live hot path.
pub(crate) fn lose_player_max_hp_from_card(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    lose_player_max_hp_apply(state, catalog, amount, true, "card-sourced max HP", events)
}

/// Evaluate, truncate once, mutate HP, and record one player damage result
/// (`_commit_player_hp_loss` + `_record_player_unblocked_damage_result`, frozen
/// Python, deleted #2827).
///
/// The `ModifyHpLost` walk (Intangible, Beating Remnant, Tungsten Rod, Buffer,
/// The Boot) runs between the amount and the truncation in Python. Admitted
/// inputs are integral, so the represented walk is Intangible's cap-to-one
/// followed by Buffer's zeroing callback. Buffer consumes exactly one stack
/// only when that callback crosses a nonzero integer boundary.
fn commit_player_hp_loss(
    state: &mut HotState,
    amount: i64,
    blocked: i32,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let mut evaluated = amount.max(0);
    if state.powers.value(PowerId::Intangible) > 0 {
        evaluated = evaluated.min(1);
    }
    if state.fanouts.beating_remnant_owned() {
        if state.tungsten_rod_owned() {
            let remaining =
                20_i64.saturating_sub(i64::from(state.fanouts.beating_remnant_damage_received()));
            let beating_then_tungsten = evaluated.min(remaining).saturating_sub(1).max(0);
            let tungsten_then_beating = evaluated.saturating_sub(1).max(0).min(remaining);
            if beating_then_tungsten != tungsten_then_beating {
                return Err(EngineRefusal::MalformedArgs(
                    "Beating Remnant and Tungsten Rod listener order",
                ));
            }
        }
        evaluated = evaluated
            .min(20_i64.saturating_sub(i64::from(state.fanouts.beating_remnant_damage_received())));
        crate::coverage::record_relic(RelicId::RelicBeatingRemnant);
    }
    if state.tungsten_rod_owned() {
        evaluated = evaluated.saturating_sub(1).max(0);
        crate::coverage::record_relic(RelicId::RelicTungstenRod);
    }
    let buffer = state.powers.value(PowerId::Buffer);
    if buffer > 0 && evaluated > 0 {
        state.powers.set(PowerId::Buffer, SlotWire::Int, buffer - 1);
        note_power(events, Subject::Player, PowerId::Buffer, buffer - 1);
        evaluated = 0;
    }
    let hp_lost: i32 = evaluated
        .clamp(0, HP_LOSS_CLAMP)
        .try_into()
        .map_err(|_| overflow("player hp loss"))?;
    let hp_before = state.hp;
    state.hp -= hp_lost;
    if hp_lost > 0 {
        state.history.player_unblocked_damage_results_combat = state
            .history
            .player_unblocked_damage_results_combat
            .checked_add(1)
            .ok_or_else(|| overflow("player_unblocked_damage_results_combat"))?;
        state.history.player_unblocked_damage_this_turn = true;
    }
    let was_fully_blocked = blocked > 0 && amount == 0;
    if state.emotion_chip_owned() && !was_fully_blocked {
        state.set_emotion_damage_current_turn(true);
        crate::coverage::record_relic(RelicId::RelicEmotionChip);
    }
    events.push(Event::PlayerDamaged {
        blocked,
        hp_lost,
        hp: state.hp,
    });
    // `0x3e96c8` IL_0be4: the result's `AfterCurrentHpChanged`, raised only
    // for a positive `UnblockedDamage` (IL_0bb6-IL_0bbc). Every listener
    // between this commit and it (AfterBlockBroken) leaves Strength unread.
    if hp_lost > 0 {
        red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    }
    Ok(hp_lost)
}

/// Player-relic `AfterDamageReceived` suffix for a surviving owner-side
/// result. Demon Tongue fires at most once per owner turn and heals the exact
/// post-modifier HP loss; its body has no in-progress gate.
///
/// Demon Tongue heals only damage taken **while the owner's own side is
/// active** (#2808). Native authority: v0.111.0 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// `DemonTongue/<AfterDamageReceived>d__3::MoveNext` RVA `0x322adc`:
///
/// * `IL_0020`-`IL_0032`: a null `Owner.Creature.CombatState` returns;
/// * `IL_0037`-`IL_005e`: `CombatState.CurrentSide == Owner.Creature.Side`,
///   otherwise it returns without healing and without arming the latch;
/// * `IL_0063`-`IL_0076`: `target == Owner.Creature` (structural here: this
///   suffix runs only for player results);
/// * `IL_007b`-`IL_0089`: `result.UnblockedDamage > 0`;
/// * `IL_008e`-`IL_0096`: `!_triggeredThisTurn`;
/// * `IL_009b`-`IL_00c4`: set the latch, `Flash`, then
///   `CreatureCmd::Heal(owner, UnblockedDamage)`.
///
/// `CurrentSide` is written only by `CombatManager::SwitchSides` RVA
/// `0x136b08` (`IL_0053`-`IL_005a` sets Enemy after the player side's
/// complete end cascade; `IL_0061`-`IL_0068` sets Player), and
/// `CombatState::.ctor` RVA `0x136f30` `IL_0055`-`IL_0057` starts it at
/// Player. [`HotState::player_side_active`] is exactly that marker: cleared
/// at the monster-side switch after Disintegration and set again at
/// `begin_player_turn`. Enemy-side damage therefore neither heals nor spends
/// the once-per-turn latch. The latch reset is
/// `DemonTongue::BeforeSideTurnStart` RVA `0x927d7` (`IL_0001`-`IL_001c`:
/// only when the owner's creature is a participant, i.e. the owner's side
/// start), in `relics::before_side_turn_start`.
fn after_owner_damage_received_relics(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    hp_lost: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if hp_lost <= 0 {
        return Ok(());
    }
    if state.demon_tongue_owned() && state.player_side_active && !state.demon_tongue_triggered() {
        state.set_demon_tongue_triggered(true);
        let hp_before = state.hp;
        state.hp = state.hp.saturating_add(hp_lost).min(state.max_hp);
        // `CreatureCmd.Heal` `0x3eb4b0` raises `AfterCurrentHpChanged`.
        red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
        crate::coverage::record_relic(RelicId::RelicDemonTongue);
    }
    if state.fanouts.self_forming_clay_owned() && !state.history.over {
        apply_self_forming_clay_power(state, 3, events)?;
        crate::coverage::record_relic(RelicId::RelicSelfFormingClay);
    }
    beating_remnant_after_damage_received(state, hp_lost)?;
    if state.fanouts.puzzle_armed() && !state.history.over {
        let catalog = catalog.ok_or(EngineRefusal::MalformedArgs(
            "Centennial Puzzle damage requires catalog",
        ))?;
        state.fanouts.set_puzzle_armed(false);
        for _ in 0..3 {
            super::draw::centennial_puzzle_draw_one(state, catalog, events)?;
        }
        crate::coverage::record_relic(RelicId::RelicCentennialPuzzle);
    }
    Ok(())
}

/// Beating Remnant's side-agnostic `AfterDamageReceived` listener.
///
/// This is a Received hook, so a result queued for final Kill must skip it
/// even when Fairy/Lizard later prevents that Kill. The monster-hit path
/// enforces this using the frozen result and live queue decision; independent
/// legacy HP-loss callers retain their separately scoped ordering contracts.
fn beating_remnant_after_damage_received(
    state: &mut HotState,
    hp_lost: i32,
) -> Result<(), EngineRefusal> {
    if hp_lost <= 0 {
        return Ok(());
    }
    if state.fanouts.beating_remnant_owned() {
        let updated = state
            .fanouts
            .beating_remnant_damage_received()
            .checked_add(hp_lost)
            .ok_or_else(|| overflow("Beating Remnant damage received"))?;
        let written = state.fanouts.set_beating_remnant_damage_received(updated);
        debug_assert!(written);
        crate::coverage::record_relic(RelicId::RelicBeatingRemnant);
    }
    Ok(())
}

/// `_resolve_player_lethal` (frozen Python, deleted #2827): the death-prevention hook family.
///
/// Returns whether the player is still standing. Fairy in a Bottle is the
/// represented ordinary preventer and wins before the Lizard Tail late hook.
/// A forced Kill enters with `run_should_die = false` and skips both.
#[cfg(test)]
fn resolve_player_lethal(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    resolve_player_lethal_inner(state, events, true)
}

fn resolve_player_lethal_inner(
    state: &mut HotState,
    events: &mut Vec<Event>,
    run_should_die: bool,
) -> Result<bool, EngineRefusal> {
    resolve_player_lethal_with_catalog(state, None, events, run_should_die)
}

/// Whether an `IllusionPower` listener is in combat, so that it vetoes the
/// dying player's own power removal (#2646).
///
/// Native, sts2.dll v0.111.0 (sha256 `9cb4f1ad…`):
/// - `Creature::RemoveAllPowersAfterDeath` `0x11dbac` IL_0027-IL_002c keeps
///   every power the filter `<RemoveAllPowersAfterDeath>b__136_0` `0x3dad34`
///   rejects. That filter removes a power only when its own
///   `ShouldPowerBeRemovedAfterOwnerDeath` (IL_0002) and
///   `Hook::ShouldPowerBeRemovedOnDeath` (IL_000a) are both true.
/// - `Hook::ShouldPowerBeRemovedOnDeath` `0x106a34` IL_0021-IL_003e walks
///   `IterateHookListeners` and returns false on the first listener that
///   says false. `IllusionPower::ShouldPowerBeRemovedOnDeath` `0xa3a80` is
///   the only override of the `AbstractModel` default `0x7a341` in the DLL
///   (method-name census). It never tests the power's owner: it keeps every
///   non-debuff (IL_0002-IL_0008) and every `ITemporaryPower` debuff
///   (IL_000b-IL_0014).
/// - `Apply<IllusionPower>` has exactly two call sites in the DLL (a token
///   census over every MethodSpec instantiation):
///   `Parafright/<AfterAddedToRoom>d__15` `0x365ebc` IL_0097 and
///   `EyeWithTeeth/<AfterAddedToRoom>d__9` `0x35a068` IL_0097. Both apply it
///   to the monster itself. The Obscura and Fogmog only summon those
///   monsters ([`super::monsters::spawn_illusion`]), so they are not
///   listeners themselves.
/// - The holder is never removed from combat:
///   `IllusionPower::ShouldCreatureBeRemovedFromCombatAfterDeath` `0xa3b33`
///   returns false for its owner. A retained dead holder still vetoes:
///   `CombatState/<IterateHookListeners>d__69` `0x3f9720` IL_0092-IL_0097
///   adds every creature's powers with no HP filter, and
///   `CombatState::Contains` `0x137564` IL_00ad-IL_00c9 accepts a power
///   whose non-player owner still has a `CombatState`.
///
/// So the veto is live exactly while a Parafright or Eye With Teeth is on
/// the monster roster, whether alive or retained dead.
pub(crate) fn illusion_removal_veto_in_combat(state: &HotState) -> bool {
    state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Parafright | MonsterKind::EyeWithTeeth
        )
    })
}

/// The unmodeled Illusion-retained Necro interval of a player death (#2646).
///
/// Native `CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext`
/// `0x3ebe90` runs the player's `RemoveAllPowersAfterDeath` at IL_04f1. It
/// then clears the OrbQueue (IL_06a2-IL_06a7), tests `Player.IsOstyAlive`
/// (IL_06b2) and kills the live Osty (IL_06ca), all before
/// `Player.DeactivateHooks` at IL_072d. Necro Mastery is Type 1 (`0xa4c73`),
/// so an Illusion veto ([`illusion_removal_veto_in_combat`]) keeps it.
/// `NecroMasteryPower/<AfterCurrentHpChanged>d__4` `0x33e950` then sees the
/// pet's negative delta. It passes the delta test at IL_0021-IL_0030, the
/// Osty test at IL_0042 and the pet-owner test at IL_0054-IL_0064, and
/// damages every hittable enemy at IL_007b-IL_00a2. That damage runs while
/// the dead owner's debuffs and orbs are already gone.
///
/// Rust does not represent that partial cleanup, so it refuses the exact
/// entry shape. All three conditions are required:
/// - a live Osty, because a dead or absent pet never takes the IL_06ca Kill;
/// - a positive Necro Mastery, because an absent power has no listener;
/// - the veto, because without it IL_04f1 removes Necro first.
///
/// A prevented death returns before this point, so its Kill never runs.
pub(crate) fn illusion_retained_necro_cleanup_is_reached(
    state: &HotState,
    illusion_veto: bool,
) -> bool {
    illusion_veto
        && state.fanouts.pet().osty().is_some()
        && state.powers.value(PowerId::NecroMastery) > 0
}

/// Native solo KillWithoutCheckingWinCondition0x3ebe90 (v111 DLL
/// 9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4)
/// runs ShouldDie before actual-death observers, kills a still-live owned
/// Osty atIL06ac–06ca, then Player.DeactivateHooks atIL072d. Preserve this
/// independent keyed flag: HP0 can still await Kill, and combat may already
/// be ending because an earlier result killed the last enemy. Contains
/// 0x137564 excludes the inactive owner's powers, relics and cards.
fn resolve_player_lethal_with_catalog(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
    run_should_die: bool,
) -> Result<bool, EngineRefusal> {
    if state.fanouts.player_hooks_deactivated() {
        return Ok(false);
    }
    if state.hp > 0 {
        return Ok(true);
    }
    // Native LoseHpInternal clamps lethal HP to zero before the preventer
    // walk; the simulator normalises its signed arithmetic the same way.
    state.hp = 0;
    if run_should_die && super::potions::consume_first_fairy_after_lethal(state, events)? {
        return Ok(true);
    }
    // Lizard Tail is a `ShouldDieLate` preventer (`LizardTail::ShouldDieLate`
    // RVA `0x9650c`), walked by `Hook::ShouldDie` RVA `0x1063f8` IL_0048-IL_0060
    // after every `ShouldDie`. A forced Kill skips that whole hook
    // (`0x3ebe90` IL_02d4-IL_0308), so Sandpit leaves an unused tail unused
    // (#3331).
    if run_should_die && state.fanouts.lizard_tail_owned() && !state.fanouts.lizard_tail_used() {
        state.fanouts.set_lizard_tail_used(true);
        let healed = (state.max_hp / 2).max(1);
        let hp_before = state.hp;
        state.hp = healed.min(state.max_hp);
        // The revive is a `CreatureCmd.Heal` (`0x3eb4b0`); half of max HP
        // never leaves Red Skull's threshold except at max HP 1.
        red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
        crate::coverage::record_relic(RelicId::RelicLizardTail);
        return Ok(true);
    }
    // Illusion's global removal veto 0xa3a80 can preserve NecroMastery
    // while owner debuffs/orbs are already cleaned up. That wider nested
    // corpse-state composition is not yet represented, and this is its only
    // wall since #2646 retired the cold admission refusal: the check reads
    // the exact state at the instant native would enter the interval.
    let illusion_veto_reachable = illusion_removal_veto_in_combat(state);
    if illusion_retained_necro_cleanup_is_reached(state, illusion_veto_reachable) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "Illusion retained Necro owner-death cleanup",
        ));
    }
    // Every finite monster Shrink in this solo slice retains the player as
    // its concrete applier. ShrinkPower.AfterDeath removes those instances
    // before the fatal player's own ordinary power cleanup; the infinite
    // player-owned Shrink singleton is then removed with the rest of the
    // dead owner's Type-2 powers. Represented preventers return before this
    // suffix, so a prevented death deliberately preserves both families.
    for index in 0..state.monsters.len() {
        let shrink = state.monsters[index].powers.value(PowerId::Shrink);
        if shrink <= 0 {
            continue;
        }
        let monster = &mut state.monsters_mut()[index];
        monster.powers.set(PowerId::Shrink, SlotWire::Int, 0);
        monster.misery_debuff_order.remove_all(MiseryToken::Shrink);
        note_power(events, Subject::Monster(monster.uid), PowerId::Shrink, 0);
    }
    if state.powers.value(PowerId::PlayerShrink) > 0 {
        state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 0);
        note_power(events, Subject::Player, PowerId::PlayerShrink, 0);
    }
    // CoveredPower is ordinary owner-death-removed state. Intercept retains
    // only the surviving remote Player key, if any.
    let covered = state.fanouts.intercept_covered_mask() & !0b01;
    let written = state.fanouts.set_intercept_covered_mask(covered);
    debug_assert!(written);
    // A post-body Corrupted hit can kill the player after an enemy's
    // terminal transition. Actual-death listeners still run in that case.
    super::cards::physical_cards_after_actual_death(state)?;
    // RemoveAllPowersAfterDeath0x11dbac runs at Kill IL04f1 before
    // live-pet cleanup. Without Illusion's global veto, ordinary Type1
    // NecroMastery is gone before the pet's HPChanged callback.
    if !illusion_veto_reachable && state.powers.value(PowerId::NecroMastery) != 0 {
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 0);
        note_power(events, Subject::Player, PowerId::NecroMastery, 0);
    }
    // The same native RemoveAllPowersAfterDeath pass detaches each represented
    // Type1 reset listener before Player.DeactivateHooks. IllusionPower's
    // 0xa3a80 veto skips this detach globally; preserve the ledger in that
    // case even though the final deactivation makes every retained hook inert.
    if !illusion_veto_reachable {
        for power in [
            PowerId::Genesis,
            PowerId::StarNextTurn,
            PowerId::EnergyNextTurn,
            PowerId::LightningRod,
            PowerId::Spinner,
        ] {
            if state.powers.value(power) != 0 {
                state.powers.set(power, SlotWire::Int, 0);
                note_power(events, Subject::Player, power, 0);
            }
            match power {
                PowerId::Genesis | PowerId::StarNextTurn => {
                    state.fanouts.unregister_star_energy_reset(power);
                }
                PowerId::LightningRod => {
                    state
                        .orbs
                        .unregister_reset_power(crate::hot::OrbResetPower::LightningRod);
                    state.fanouts.unregister_after_energy_reset(
                        crate::hot::AfterEnergyResetPower::LightningRod,
                    );
                }
                PowerId::Spinner => {
                    state
                        .orbs
                        .unregister_reset_power(crate::hot::OrbResetPower::Spinner);
                    state
                        .fanouts
                        .unregister_after_energy_reset(crate::hot::AfterEnergyResetPower::Spinner);
                }
                PowerId::EnergyNextTurn => state.fanouts.unregister_after_energy_reset(
                    crate::hot::AfterEnergyResetPower::EnergyNextTurn,
                ),
                _ => unreachable!("owner-death reset listener is closed"),
            }
        }
        if state.fanouts.radiance() != 0 {
            state.fanouts.set_radiance(0);
        }
        state
            .fanouts
            .unregister_after_energy_reset(crate::hot::AfterEnergyResetPower::Radiance);
        state.normalize_after_energy_reset_order_if_unique();
    }
    // Native Kill0x3ebe90 IL06a2–06a7 clears OrbQueue before the pet. A
    // suspended Channel resumes against capacity0, so TryEnqueue fails;
    // preventers returned above and deliberately preserve the live queue.
    state.orbs.clear_on_owner_death();
    // Native Kill0x3ebe90 IL06ac–06ca kills still-live Osty before
    // Player.DeactivateHooks IL072d. An already-dead queued pet is absent
    // here and drains later, when owner callbacks are no longer eligible.
    if state.fanouts.pet().osty().is_some() {
        finish_queued_osty_death(state, catalog, events)?;
    }
    state.fanouts.set_player_hooks_deactivated(true);
    if !state.history.over {
        state.history.over = true;
        events.push(Event::CombatOver { player_won: false });
    }
    Ok(false)
}

/// DoomPower's direct player kill. This is not damage: Block and every damage
/// history/listener stay untouched, then the ordinary lethal boundary runs.
pub(crate) fn doom_kill_player(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hp_before = state.hp;
    state.hp = 0;
    // `Kill` `0x3ebe90` IL_01ca-IL_0200 raises `AfterCurrentHpChanged` for a
    // living receiver before `ShouldDie` and `DeactivateHooks`.
    red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    resolve_player_lethal_with_catalog(state, catalog, events, true).map(|_| ())
}

/// Sandpit's `CreatureCmd::Kill(..., forceKill: true)` bypasses ShouldDie
/// preventers, unlike Doom's ordinary Kill(false): `0x3ebe90` IL_02d4-IL_0308
/// takes `shouldDie = true` without calling `Hook::ShouldDie`, so neither
/// Fairy in a Bottle nor Lizard Tail's `ShouldDieLate` runs. The same `force`
/// reaches the still-live Osty's Kill at IL_06c5-IL_06ca. Osty's own preventers
/// are the empty set either way, because both of those hooks test for the
/// owner's creature (`FairyInABottle::ShouldDie` RVA `0xac413`,
/// `LizardTail::ShouldDieLate` RVA `0x9650c`, IL_0001-IL_000d).
pub(crate) fn force_kill_player(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let hp_before = state.hp;
    state.hp = 0;
    // The same `Kill` `0x3ebe90` IL_01ca-IL_0200 `AfterCurrentHpChanged`;
    // `forceKill` only skips the `ShouldDie` preventers after it.
    red_skull_after_player_hp_changed(state, hp_before, state.max_hp, events)?;
    resolve_player_lethal_inner(state, events, false).map(|_| ())
}

/// One unmodified owner-side damage result against the player.
///
/// Used by turn-end-in-hand Status bodies, monster-Thorns retaliation and
/// Inferno's self-damage tick. It shares Intangible/Buffer result modifiers
/// and the immediate-source Rupture/Inferno suffix without an attacker
/// snapshot. A card body's own loss with a registered Rupture entry takes
/// [`damage_player_from_card_batching_rupture`] instead (#3298).
fn damage_player_unpowered_owner(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    damage_player_unpowered_owner_with(state, catalog, amount, blockable, None, events)
}

/// The same owner damage command from a card whose Rupture `playedCards`
/// entry is registered (#3298): Rupture's `AfterDamageReceived` adds its
/// live Amount to the entry instead of applying Strength
/// (`RupturePower/<AfterDamageReceived>d__9::MoveNext` RVA `0x342fec`
/// IL_0063-IL_0081 `ContainsKey(cardSource)`, IL_00fc-IL_0122
/// `playedCards[cardSource] += Amount`). Every other step of the command is
/// the ordinary card path's. Returns the amount to add to the entry: zero
/// when the result was not a surviving positive loss.
pub(crate) fn damage_player_from_card_batching_rupture(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let mut batch = 0;
    damage_player_unpowered_owner_with(
        state,
        Some(catalog),
        amount,
        blockable,
        Some(&mut batch),
        events,
    )?;
    Ok(batch)
}

#[inline(always)]
fn damage_player_unpowered_owner_with(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    amount: i64,
    blockable: bool,
    rupture_batch: Option<&mut i32>,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if state.history.over {
        return Ok(false);
    }
    let blocked: i32 = if blockable {
        i64::from(state.block)
            .min(amount.max(0))
            .try_into()
            .map_err(|_| overflow("player blocked damage"))?
    } else {
        0
    };
    state.block -= blocked;
    let hp_lost =
        commit_player_hp_loss(state, (amount - i64::from(blocked)).max(0), blocked, events)?;
    if !resolve_player_lethal_with_catalog(state, catalog, events, true)? {
        return Ok(false);
    }
    match rupture_batch {
        None => rupture_after_owner_hp_loss(state, hp_lost, events)?,
        Some(batch) => {
            let rupture = state.powers.value(PowerId::Rupture);
            if hp_lost > 0 && rupture > 0 {
                *batch = rupture;
            }
        }
    }
    inferno_after_player_hp_loss(state, catalog, hp_lost, events)?;
    after_owner_damage_received_relics(state, catalog, hp_lost, events)?;
    Ok(!state.history.over)
}

pub fn damage_player_from_card(
    state: &mut HotState,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if state.fanouts.puzzle_armed() {
        return Err(EngineRefusal::MalformedArgs(
            "Centennial Puzzle card damage requires catalog",
        ));
    }
    let blocked = if blockable {
        i64::from(state.block.max(0)).min(amount.max(0))
    } else {
        0
    };
    let mut prospective_hp_lost = (amount - blocked).max(0);
    if state.powers.value(PowerId::Intangible) > 0 {
        prospective_hp_lost = prospective_hp_lost.min(1);
    }
    if state.tungsten_rod_owned() {
        prospective_hp_lost = prospective_hp_lost.saturating_sub(1);
    }
    if state.powers.value(PowerId::Buffer) > 0 {
        prospective_hp_lost = 0;
    }
    prospective_hp_lost = prospective_hp_lost.min(HP_LOSS_CLAMP);
    if !state.history.over
        && prospective_hp_lost > 0
        && i64::from(state.hp) > prospective_hp_lost
        && state.card_states.dampen().is_some()
        && state.powers.value(PowerId::Inferno) > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen card damage Inferno catalog",
        ));
    }
    damage_player_unpowered_owner(state, None, amount, blockable, events)
}

pub(crate) fn damage_player_from_card_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    damage_player_unpowered_owner(state, Some(catalog), amount, blockable, events)
}

/// One active-card sourced owner damage command.
///
/// Rupture records this exact CardPlay at BeforeCardPlayed and therefore
/// accumulates its live Amount into the persisted play-frame batch instead of
/// applying Strength immediately. The caller owns that frame update; this
/// primitive returns the positive surviving HP-loss receipt after running the
/// remaining Inferno suffix.
pub(crate) fn damage_player_from_active_card(
    state: &mut HotState,
    catalog: &Catalog,
    source_uid: u32,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<(bool, i32), EngineRefusal> {
    if state.history.over {
        return Ok((false, 0));
    }
    damage_player_from_active_card_even_if_ending(
        state, catalog, source_uid, amount, blockable, events,
    )
}

/// The same active-card owner damage command without the ending gate.
///
/// `combat_sim.player_hp_loss` (frozen Python, deleted #2827) has no `s.over` branch at all: the
/// ending gate above belongs to the `hp_loss` **step**, whose
/// `_run_steps_inner` loop-top guard refuses a body command reached
/// after the fight-ending kill. `Corrupted/<OnPlay>d__5::MoveNext` RVA
/// `0x3881bc` is not a body command — it runs from `EnchantmentModel.OnPlay`
/// after a complete native body, including one that killed the last enemy, so
/// it needs the ungated entry (Python `_enchantment_on_play`).
pub(crate) fn damage_player_from_active_card_even_if_ending(
    state: &mut HotState,
    catalog: &Catalog,
    _source_uid: u32,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<(bool, i32), EngineRefusal> {
    let blocked: i32 = if blockable {
        i64::from(state.block)
            .min(amount.max(0))
            .try_into()
            .map_err(|_| overflow("player blocked damage"))?
    } else {
        0
    };
    state.block -= blocked;
    let hp_lost =
        commit_player_hp_loss(state, (amount - i64::from(blocked)).max(0), blocked, events)?;
    if state.hp <= 0
        && state
            .fanouts
            .potion_slots()
            .contains(&Some(PotionId::FairyInABottle))
    {
        match super::potions::begin_fairy_wrapper_after_lethal(state, catalog, events)? {
            super::potions::FairyWrapperResult::Finished => {}
            // CheckForEmptyHand is suppressed while the active CardPlay is
            // below Fairy's effect frame, so this public callsite cannot park.
            super::potions::FairyWrapperResult::Suspended => {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            super::potions::FairyWrapperResult::Absent => {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
        }
    } else if !resolve_player_lethal_with_catalog(state, Some(catalog), events, true)? {
        return Ok((false, hp_lost));
    }
    inferno_after_player_hp_loss(state, Some(catalog), hp_lost, events)?;
    after_owner_damage_received_relics(state, Some(catalog), hp_lost, events)?;
    Ok((!state.history.over, hp_lost))
}

/// One power-sourced, unpowered owner damage command. Constrict uses this
/// entry after authenticating its exact-applier row; it deliberately shares
/// the card/status path's block, Intangible/Buffer, lethal, Rupture, and
/// Inferno ordering rather than approximating the native Damage props `4`.
#[cfg(test)]
pub(crate) fn damage_player_from_power(
    state: &mut HotState,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if state.fanouts.puzzle_armed() {
        return Err(EngineRefusal::MalformedArgs(
            "Centennial Puzzle power damage requires catalog",
        ));
    }
    damage_player_unpowered_owner(state, None, amount, blockable, events)
}

pub(crate) fn damage_player_from_power_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    amount: i64,
    blockable: bool,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    damage_player_unpowered_owner(state, Some(catalog), amount, blockable, events)
}

/// Rupture's immediate owner-side `AfterDamageReceived` consumer.
///
/// Null, finished-card and power sources apply Strength synchronously once
/// per positive surviving result (`RupturePower/<AfterDamageReceived>d__9`
/// RVA `0x342fec` IL_0063-IL_00a2: a null or unregistered `cardSource`). A
/// registered active card's own loss batches instead; see
/// [`damage_player_from_card_batching_rupture`] and
/// `play::card_body_hp_loss` (#3298).
fn rupture_after_owner_hp_loss(
    state: &mut HotState,
    hp_lost: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let rupture = state.powers.value(PowerId::Rupture);
    if hp_lost > 0 && rupture > 0 {
        apply_owner_strength(state, rupture, events)?;
    }
    Ok(())
}

/// `InfernoPower.AfterDamageReceived` after a surviving owner-side HP loss.
///
/// v0.111.0 DLL 9cb4f1ad: `InfernoPower/<AfterDamageReceived>d__8::MoveNext`
/// RVA `0x33d084` gates `target == Owner` (IL_0020-IL_002c),
/// `result.UnblockedDamage > 0` (IL_0033-IL_003f) and `CurrentSide ==
/// Owner.Side` (IL_0046-IL_0061), plays a VFX per hittable enemy
/// (IL_0068-IL_00ac), then awaits ONE `CreatureCmd::Damage(choiceContext,
/// CombatState.HittableEnemies, (decimal)Amount, props 4, Owner)` at
/// IL_00be-IL_00e1: a blockable, unpowered amount dealt by the player over
/// the whole enemy list.
///
/// One command means one batch (#3274). `CreatureCmd/<Damage>d__12::MoveNext`
/// RVA `0x3e96c8` snapshots the targets (IL_00c6), commits HP loss to every
/// one (loop closing IL_0aa4), dispatches the frozen results
/// (IL_0adf-IL_0e84), and only then awaits one `CreatureCmd::Kill` over the
/// killed list (IL_0eb4). So when Inferno kills the last enemies together,
/// Gremlin Horn's `<AfterDeath>d__6` (RVA `0x326170`) runs with the combat
/// already ending and its `PlayerCmd` energy and draw grant nothing, exactly as
/// for Explosive Ampoule (#3244). The old per-enemy walk killed the first
/// enemy before the second took its HP loss and paid the Horn. The batch
/// primitive is [`damage_monsters_with_optional_catalog`], which keeps the
/// parent command's catalog through the nested death path.
///
/// Position: this is one `AfterDamageReceived` listener of the player's own
/// damage result, awaited to completion (its Kill included) before the next
/// listener; the nested command opens its own batch on the stack, so an
/// enclosing batch keeps its receipts and closes the combat only when it
/// leaves. That is the same position the per-enemy walk had: batching changes
/// only what happens inside Inferno's command.
///
/// Lethal player damage and enemy-turn damage never fire it.
fn inferno_after_player_hp_loss(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    hp_lost: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let amount = state.powers.value(PowerId::Inferno);
    if hp_lost <= 0 || amount <= 0 || state.history.over {
        return Ok(());
    }
    let targets = alive_targets(state);
    let damage = DotNetDecimal::from_i64(i64::from(amount));
    damage_monsters_with_optional_catalog(state, catalog, &targets, damage, true, events)
}

/// Powered card `GainBlock` (`_gain_powered_card_block`, frozen Python, deleted #2827).
///
/// The admitted additive terms are Dexterity, temporary Dexterity, and Fasten
/// (only on Defend-tagged cards: `FastenPower::ModifyBlockAdditive` RVA
/// `0xa22c0` `IL_0029`-`IL_0041`, #3051); Unmovable supplies the admitted doubling
/// window and Frail multiplies powered card Block by 3/4. Enchantment block
/// and the remaining Vitruvian/Vambrace/Pael terms remain refused. NoBlock
/// supplies the powered owner-card zeroing term. Shadowmeld supplies the exact
/// owner-targeted `2^Amount` multiplier to powered and unpowered card block.
/// Only a positive final amount reaches `GainBlockInternal` and the block-gain
/// history, which is why Unmovable's prior-gain count advances only then.
/// Native stores at most 999,999,999 Block while the event retains the full
/// positive calculated amount (Python `_store_native_player_block` (frozen, deleted #2827)); Bulwark makes that shared storage edge directly reachable.
pub fn gain_powered_card_block(
    state: &mut HotState,
    catalog: &Catalog,
    source: &CardSpec,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    if state.card_states.dampen().is_some()
        && state.powers.value(PowerId::Juggernaut) > 0
        && !damage_combat_is_ending(state)
        && modified_card_block(state, source, raw, true)? > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen powered block Juggernaut catalog",
        ));
    }
    gain_card_block(state, catalog, source, raw, true, events)
}

/// Powered card `GainBlock` with the full post-modifier Decimal return value.
/// Native independently truncates that value for Block/history storage; Toric
/// Toughness retains this exact result after the awaited command completes.
pub(crate) fn gain_powered_card_block_retained_decimal(
    state: &mut HotState,
    catalog: &Catalog,
    source: &CardSpec,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<DotNetDecimal, EngineRefusal> {
    if state.card_states.dampen().is_some()
        && state.powers.value(PowerId::Juggernaut) > 0
        && !damage_combat_is_ending(state)
        && modified_card_block_decimal(state, source, raw, true)? > DotNetDecimal::zero()
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen powered block Juggernaut catalog",
        ));
    }
    gain_card_block_decimal(state, catalog, source, raw, true, events)
}

/// Pure `Hook.ModifyBlock` preview for a powered card block command.
///
/// This deliberately performs only the shared arithmetic: it does not inspect
/// combat-ending state, gain Block, advance Unmovable history, emit events, or
/// fire `AfterBlockGained`.
pub fn preview_powered_card_block(
    state: &HotState,
    source: &CardSpec,
    raw: i64,
) -> Result<i32, EngineRefusal> {
    modified_card_block(state, source, raw, true)
}

/// Apply the generic additive one-shot BlockNextTurn power.
pub fn apply_block_next_turn(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    apply_delayed_block_power(
        state,
        PowerId::BlockNextTurn,
        amount,
        "block next turn",
        events,
    )
}

/// Self-Forming Clay's own delayed-Block power (#3159), never BlockNextTurn.
///
/// v0.111.0 DLL `9cb4f1ad…`:
/// `SelfFormingClay/<AfterDamageReceived>d__7::MoveNext` RVA `0x330ae4`
/// gates on `CombatManager.IsInProgress` (IL_0020-IL_002a), `target ==
/// Owner.Creature` (IL_0031-IL_0042) and `result.UnblockedDamage > 0`
/// (IL_0049-IL_0055), then IL_005c-IL_008f awaits `PowerCmd.Apply<T>(ctx,
/// owner, DynamicVars["BlockNextTurn"].BaseValue (3, `get_CanonicalVars`
/// RVA `0x9b1de` IL_0001-IL_0007), owner, null, false)` through MethodSpec
/// `0x2b002689`, whose instantiation is TypeDef 1078 `SelfFormingClayPower`.
/// That single-target `PowerCmd/<Apply>d__1<T>::MoveNext` RVA `0x3ef988`
/// returns at `IsEnding` (IL_0020-IL_002a, which subsumes the relic's
/// in-progress gate and is `history.over` here) and otherwise stacks into
/// `FindExistingInstanceForStacking` (IL_006c) or appends a fresh instance
/// (IL_0084-IL_00b8). `SelfFormingClayPower::get_StackType` RVA `0xa72cc` is
/// Counter (IL_0001 `ldc.i4.1`, as BlockNextTurnPower), with no
/// `get_InstanceType` override, so one instance stacks additively and keeps
/// its `AfterBlockCleared` listener position; a separate BlockNextTurnPower
/// is a separate listener with its own GainBlock.
pub fn apply_self_forming_clay_power(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    apply_delayed_block_power(
        state,
        PowerId::SelfFormingClayPower,
        amount,
        "self forming clay power",
        events,
    )
}

/// One additive `AfterBlockCleared` delayed-Block power application: the
/// first positive stack registers the power's listener at the end of the
/// acquisition order, later stacks keep it. A full listener array refuses by
/// name rather than dropping or reordering a listener.
fn apply_delayed_block_power(
    state: &mut HotState,
    power: PowerId,
    amount: i32,
    label: &'static str,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount <= 0 {
        return Ok(());
    }
    let current = state.powers.value(power);
    let stacked = current.checked_add(amount).ok_or_else(|| overflow(label))?;
    if current == 0 && !state.fanouts.register_after_block_cleared(power) {
        return Err(EngineRefusal::PowerOrderNotModeled(
            "after-block-cleared power order",
        ));
    }
    state.powers.set(power, SlotWire::Int, stacked);
    note_power(events, Subject::Player, power, stacked);
    Ok(())
}

/// Card-or-monster-move, unpowered player Block. This shares Unmovable and
/// the positive-gain history window with powered card Block, but deliberately
/// skips Dexterity, temporary Dexterity, and Fasten.
pub fn gain_card_unpowered_block(
    state: &mut HotState,
    catalog: &Catalog,
    source: &CardSpec,
    raw: i64,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    if state.card_states.dampen().is_some()
        && state.powers.value(PowerId::Juggernaut) > 0
        && !damage_combat_is_ending(state)
        && modified_card_block(state, source, raw, false)? > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen unpowered block Juggernaut catalog",
        ));
    }
    gain_card_block(state, catalog, source, raw, false, events)
}

/// One props-4 unpowered, non-card player `GainBlock` command.
///
/// Python: `_gain_flat_player_block` (frozen, deleted #2827). Flat block skips every
/// card-only additive and history counter, but still folds the live
/// Shadowmeld multiplier, clamps stored Block at the native billion-minus-one
/// ceiling, and awaits the acquisition-ordered `AfterBlockGained` power walk.
/// Relics are rejected by admission, so the represented suffix is exactly
/// [`powers_after_block_gained`].
pub(crate) fn gain_flat_power_block_decimal(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    raw: DotNetDecimal,
    events: &mut Vec<Event>,
) -> Result<DotNetDecimal, EngineRefusal> {
    if damage_combat_is_ending(state) {
        return Ok(DotNetDecimal::zero());
    }
    let shadowmeld = state.powers.value(PowerId::Shadowmeld);
    if !(0..96).contains(&shadowmeld) {
        return Err(EngineRefusal::MalformedArgs("flat block shadowmeld amount"));
    }
    let mut modified = raw;
    for _ in 0..shadowmeld {
        modified = modified
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("flat player block"))?;
    }
    let gained = native_block_storage_amount(modified, "flat player block")?;
    if modified > DotNetDecimal::zero() {
        if !(0..=NATIVE_PLAYER_BLOCK_CAP).contains(&i64::from(state.block)) {
            return Err(EngineRefusal::MalformedArgs("player block"));
        }
        state.block =
            (i64::from(state.block) + i64::from(gained)).min(NATIVE_PLAYER_BLOCK_CAP) as i32;
        events.push(Event::PlayerBlockGained {
            amount: gained,
            block: state.block,
        });
    }
    powers_after_block_gained(state, catalog, modified, events)?;
    Ok(modified)
}

fn gain_flat_power_block(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    raw: i32,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    let modified = gain_flat_power_block_decimal(
        state,
        catalog,
        DotNetDecimal::from_i64(i64::from(raw)),
        events,
    )?;
    native_block_storage_amount(modified, "flat player block")
}

/// The player `CreatureCmd.GainBlock` body shared by powered and unpowered
/// card Block. `CreatureCmd/<GainBlock>d__18::MoveNext` (v0.111.0 RVA
/// `0x3eaec0`) returns zero on `CombatManager.IsOverOrEnding`
/// (IL_002d-0041) and on a dead recipient (IL_0046-005b), before any
/// modifier, history or `AfterBlockGained` listener. A dead player is a
/// pending loss, so [`damage_combat_is_ending`] is the whole gate; it also
/// covers the ending-but-not-over window `history.over` missed (#3502). The
/// flat-power and Decimal variants, and the Dampen/Juggernaut preflights in
/// front of them, read the same gate.
fn gain_card_block(
    state: &mut HotState,
    catalog: &Catalog,
    source: &CardSpec,
    raw: i64,
    powered: bool,
    events: &mut Vec<Event>,
) -> Result<i32, EngineRefusal> {
    if damage_combat_is_ending(state) {
        return Ok(0);
    }
    let gained = modified_card_block(state, source, raw, powered)?;
    if gained > 0
        && state.fanouts.vambrace_available()
        && let Some(uid) = super::play::active_play_current_uid()
        && state
            .fanouts
            .vambrace_trigger_uid()
            .is_none_or(|trigger| trigger == uid)
    {
        state.fanouts.set_vambrace_trigger_uid(Some(uid));
        crate::coverage::record_relic(RelicId::RelicVambrace);
    }
    if powered
        && gained > 0
        && state.fanouts.paels_legion_cooldown() == 0
        && let Some(uid) = super::play::active_play_current_uid()
        && state
            .fanouts
            .paels_legion_trigger_uid()
            .is_none_or(|trigger| trigger == uid)
    {
        state.fanouts.set_paels_legion_trigger_uid(Some(uid));
        crate::coverage::record_relic(RelicId::RelicPaelsLegion);
    }
    if gained > 0 {
        if !(0..=NATIVE_PLAYER_BLOCK_CAP).contains(&i64::from(state.block)) {
            return Err(EngineRefusal::MalformedArgs("player block"));
        }
        let card_block_gains = state
            .history
            .card_block_gains
            .checked_add(1)
            .ok_or_else(|| overflow("card_block_gains"))?;
        state.block =
            (i64::from(state.block) + i64::from(gained)).min(NATIVE_PLAYER_BLOCK_CAP) as i32;
        state.history.card_block_gains = card_block_gains;
        super::play::note_active_play_block_gain()?;
        events.push(Event::PlayerBlockGained {
            amount: gained,
            block: state.block,
        });
    }
    powers_after_block_gained(
        state,
        Some(catalog),
        DotNetDecimal::from_i64(i64::from(gained)),
        events,
    )?;
    Ok(gained)
}

fn gain_card_block_decimal(
    state: &mut HotState,
    catalog: &Catalog,
    source: &CardSpec,
    raw: i64,
    powered: bool,
    events: &mut Vec<Event>,
) -> Result<DotNetDecimal, EngineRefusal> {
    if damage_combat_is_ending(state) {
        return Ok(DotNetDecimal::zero());
    }
    let modified = modified_card_block_decimal(state, source, raw, powered)?;
    let gained = native_block_storage_amount(modified, "card block")?;
    if modified > DotNetDecimal::zero()
        && state.fanouts.vambrace_available()
        && let Some(uid) = super::play::active_play_current_uid()
        && state
            .fanouts
            .vambrace_trigger_uid()
            .is_none_or(|trigger| trigger == uid)
    {
        state.fanouts.set_vambrace_trigger_uid(Some(uid));
        crate::coverage::record_relic(RelicId::RelicVambrace);
    }
    if powered
        && modified > DotNetDecimal::zero()
        && state.fanouts.paels_legion_cooldown() == 0
        && let Some(uid) = super::play::active_play_current_uid()
        && state
            .fanouts
            .paels_legion_trigger_uid()
            .is_none_or(|trigger| trigger == uid)
    {
        state.fanouts.set_paels_legion_trigger_uid(Some(uid));
        crate::coverage::record_relic(RelicId::RelicPaelsLegion);
    }
    if modified > DotNetDecimal::zero() {
        if !(0..=NATIVE_PLAYER_BLOCK_CAP).contains(&i64::from(state.block)) {
            return Err(EngineRefusal::MalformedArgs("player block"));
        }
        let card_block_gains = state
            .history
            .card_block_gains
            .checked_add(1)
            .ok_or_else(|| overflow("card_block_gains"))?;
        state.block =
            (i64::from(state.block) + i64::from(gained)).min(NATIVE_PLAYER_BLOCK_CAP) as i32;
        state.history.card_block_gains = card_block_gains;
        super::play::note_active_play_block_gain()?;
        events.push(Event::PlayerBlockGained {
            amount: gained,
            block: state.block,
        });
    }
    // `AfterBlockGained` (Juggernaut and friends) fires even for a zero gain;
    // represented listeners gate zero internally.
    powers_after_block_gained(state, Some(catalog), modified, events)?;
    Ok(modified)
}

/// Unmovable's prior-gain count: this turn's card-Block gains by the owner,
/// excluding those recorded by the CardPlay now gaining Block.
///
/// `UnmovablePower::ModifyBlockMultiplicative` (RVA `0xaa578`, IL_005b–0096)
/// doubles while `Count(BlockGainedEntry, b__0) < Amount`. The filter
/// `<ModifyBlockMultiplicative>b__0` (RVA `0x34a534`) keeps an entry when it
/// `HappenedThisTurn` (IL_000c–001d; `history.card_block_gains` resets at the
/// owner's turn start), has a non-null `CardPlay` whose player is the owner
/// (IL_001f–0042; flat power/relic Block carries no CardPlay and never
/// advances the counter), has `IsCardOrMonsterMove` props (IL_0044–004f), and
/// — IL_0051–0062 — `entry.CardPlay != cardPlay`. Evil Eye's and Death's
/// Door's repeated `GainBlock` calls all pass one `cardPlay`
/// (`EvilEye/<OnPlay>d__9::MoveNext` RVA `0x39c590` IL_00e8–00ee inside the
/// IL_00b5–0164 loop; `DeathsDoor/<OnPlay>d__9::MoveNext` RVA `0x397230`
/// IL_00ed–00f3 inside the IL_00a5–0169 loop), so every gain of such a play
/// sees the same window. A replay body is a new CardPlay; see
/// [`super::play::active_play_own_block_gains`].
///
/// A body resumed mid-way from a persisted frame does not know its own
/// pre-suspension gains. The window is still exact when the whole turn's
/// count is below Amount (the window can only be smaller); otherwise it
/// refuses rather than guess.
fn unmovable_window(state: &HotState, unmovable: i32) -> Result<i16, EngineRefusal> {
    let total = state.history.card_block_gains;
    if unmovable <= 0 {
        return Ok(total);
    }
    match super::play::active_play_own_block_gains() {
        super::play::OwnBlockGains::NoPlay => Ok(total),
        super::play::OwnBlockGains::Known(own) => total
            .checked_sub(own)
            .filter(|window| *window >= 0)
            .ok_or(EngineRefusal::ContinuationNotModeled),
        super::play::OwnBlockGains::Unknown if i32::from(total) < unmovable => Ok(total),
        super::play::OwnBlockGains::Unknown => Err(EngineRefusal::ContinuationNotModeled),
    }
}

fn modified_card_block(
    state: &HotState,
    source: &CardSpec,
    raw: i64,
    powered: bool,
) -> Result<i32, EngineRefusal> {
    native_block_storage_amount(
        modified_card_block_decimal(state, source, raw, powered)?,
        "card block",
    )
}

fn modified_card_block_decimal(
    state: &HotState,
    source: &CardSpec,
    raw: i64,
    powered: bool,
) -> Result<DotNetDecimal, EngineRefusal> {
    let mut modified = raw;
    // `Hook::ModifyBlock` RVA `0x104c30` `IL_0014`-`IL_0036` adds the card
    // source's `EnchantBlockAdditive` before any listener, for powered and
    // unpowered card block alike; `Goopy::EnchantBlockAdditive` `0xd6092` is
    // `Amount - 1` with no props gate. Python adds `ench_block_bonus` in
    // both `_modify_powered_card_block_decimal` (frozen Python, deleted #2827) and
    // `_gain_card_unpowered_block`. The live amount is the slot-2
    // identity's.
    if let Some(amount) = crate::catalog::goopy_amount(source.identity) {
        modified = modified
            .checked_add(i64::from(amount) - 1)
            .ok_or_else(|| overflow("card block"))?;
    }
    if powered {
        if let Some(enchantment) = source
            .identity
            .enchantment
            .filter(|enchantment| matches!(enchantment.id, EnchantmentId::Nimble))
        {
            modified = modified
                .checked_add(i64::from(enchantment.amount))
                .ok_or_else(|| overflow("card block"))?;
        }
        modified = modified
            .checked_add(i64::from(state.powers.value(PowerId::Dexterity)))
            .and_then(|value| {
                value.checked_add(i64::from(state.powers.value(PowerId::TempDexterity)))
            })
            .ok_or_else(|| overflow("card block"))?;
        // `FastenPower::ModifyBlockAdditive` RVA `0xa22c0` (v0.111.0; offsets
        // as `dump_method.py` prints them, which count the 12-byte fat
        // header): `IL_000c`-`IL_001a` returns 0 unless the Block target is
        // the owner; `IL_001b`-`IL_0028` returns 0 unless
        // `ValueProps.IsPoweredCardOrMonsterMoveBlock` (RVA `0xd70b`: Move
        // set and Unpowered clear), which is this `powered` arm; and
        // `IL_0029`-`IL_0041` returns `Amount` when `cardSource` is null or
        // its `Tags` contain `CardTag.Defend` (constant 2), else 0. Fasten
        // therefore adds ONLY to Defend-tagged card Block (#3051).
        if source.row.tags.contains(&"Defend") {
            modified = modified
                .checked_add(i64::from(state.powers.value(PowerId::Fasten)))
                .ok_or_else(|| overflow("card block"))?;
        }
    }
    let unmovable = state.powers.value(PowerId::Unmovable);
    if i32::from(unmovable_window(state, unmovable)?) < unmovable {
        modified = modified
            .checked_mul(2)
            .ok_or_else(|| overflow("card block"))?;
    }
    let mut modified_decimal = DotNetDecimal::from_i64(modified);
    // Python validates the largest native Decimal intermediate before later
    // Frail/NoBlock factors can shrink or zero it. Keep that undamped path so
    // those factors cannot hide a Shadowmeld overflow.
    let mut undamped_decimal = modified_decimal;
    if powered && state.powers.value(PowerId::NoBlock) > 0 {
        modified_decimal = DotNetDecimal::zero();
    }
    if powered && state.powers.value(PowerId::PlayerFrail) > 0 {
        modified_decimal = modified_decimal
            .checked_mul(DotNetDecimal::ratio(3, 4).map_err(|_| overflow("frail multiplier"))?)
            .map_err(|_| overflow("card block"))?;
    }
    if state.fanouts.vambrace_available()
        && let Some(uid) = super::play::active_play_current_uid()
        && state
            .fanouts
            .vambrace_trigger_uid()
            .is_none_or(|trigger| trigger == uid)
    {
        undamped_decimal = undamped_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("Vambrace card block"))?;
        modified_decimal = modified_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("Vambrace card block"))?;
    }
    if powered && state.fanouts.paels_legion_cooldown() == 0 {
        undamped_decimal = undamped_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("Pael's Legion card block"))?;
        modified_decimal = modified_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("Pael's Legion card block"))?;
    }
    let shadowmeld = state.powers.value(PowerId::Shadowmeld);
    for _ in 0..shadowmeld {
        undamped_decimal = undamped_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("shadowmeld card block"))?;
        modified_decimal = modified_decimal
            .checked_mul(DotNetDecimal::from_i64(2))
            .map_err(|_| overflow("shadowmeld card block"))?;
    }
    native_block_storage_amount(modified_decimal, "card block")?;
    Ok(modified_decimal)
}

fn native_block_storage_amount(
    modified: DotNetDecimal,
    site: &'static str,
) -> Result<i32, EngineRefusal> {
    if modified <= DotNetDecimal::zero() {
        return Ok(0);
    }
    if modified >= DotNetDecimal::from_i64(2_147_483_648) {
        return Err(overflow(site));
    }
    modified
        .trunc_i64()
        .map_err(|_| overflow(site))?
        .try_into()
        .map_err(|_| overflow(site))
}

/// Acquisition-ordered player-power AfterBlockGained listeners.
pub(crate) fn powers_after_block_gained(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    gained: DotNetDecimal,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if gained <= DotNetDecimal::zero() || state.history.over {
        return Ok(());
    }
    let order = state.fanouts.after_block_gained_order().to_vec();
    for power in order {
        if state.history.over {
            break;
        }
        match power {
            PowerId::Juggernaut => {
                let amount = state.powers.value(PowerId::Juggernaut);
                if amount <= 0 {
                    continue;
                }
                if let Some(target) = roll_target(state)? {
                    // `JuggernautPower/<AfterBlockGained>d__4::MoveNext` RVA
                    // `0x33dab4` (v0.111.0 DLL 9cb4f1ad…) awaits one
                    // `CreatureCmd.Damage(choiceContext, target, Amount,
                    // ValueProp 4, Owner)` (IL_0091-IL_00a2) on a
                    // `CombatTargets` roll (IL_0064-IL_007e); a lethal hit
                    // runs `Hook.AfterDeath`, so the block command's catalog
                    // reaches Gremlin Horn's Draw (#3172).
                    damage_monster_with_optional_catalog(
                        state,
                        catalog,
                        target,
                        DotNetDecimal::from_i64(i64::from(amount)),
                        false,
                        true,
                        events,
                    )?;
                }
            }
            PowerId::BeaconOfHope => {
                // Admission excludes a live teammate while this callback is
                // active. In the remaining solo shape native enumerates an
                // empty teammate set; retaining the token preserves order.
                if state.powers.value(PowerId::BeaconOfHope) <= 0 || !state.player_side_active {
                    continue;
                }
            }
            _ => {
                return Err(EngineRefusal::PowerHookNotModeled {
                    power,
                    event: crate::hooks::HookEvent::AfterBlockGained,
                });
            }
        }
    }
    Ok(())
}

/// The scalar fields `_misery_active_debuff_counts` reconstructs
/// (frozen Python, deleted #2827), paired with their order vocabulary.
pub(crate) const MISERY_SCALAR_POWERS: [(PowerId, MiseryToken); 12] = [
    (PowerId::Weak, MiseryToken::Weak),
    (PowerId::Vuln, MiseryToken::Vuln),
    (PowerId::Hang, MiseryToken::Hang),
    (PowerId::Poison, MiseryToken::Poison),
    (PowerId::Doom, MiseryToken::Doom),
    (PowerId::Oblivion, MiseryToken::Oblivion),
    (PowerId::Strangle, MiseryToken::Strangle),
    (PowerId::Demise, MiseryToken::Demise),
    (PowerId::Shrink, MiseryToken::Shrink),
    (PowerId::Conqueror, MiseryToken::Conqueror),
    (PowerId::Debilitate, MiseryToken::Debilitate),
    (PowerId::SicEm, MiseryToken::SicEm),
];

/// One frozen, representable concrete Type-2 power cloned by Misery.
///
/// The native command clones concrete power instances. Rust's current hot
/// state represents twelve of those families as scalar singletons; every
/// other native family remains a typed admission refusal rather than being
/// flattened into this projection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct MiseryScalarSnapshot {
    pub power: PowerId,
    pub token: MiseryToken,
    pub amount: i32,
}

fn represented_misery_scalars() -> impl Iterator<Item = (PowerId, MiseryToken)> {
    MISERY_SCALAR_POWERS
        .into_iter()
        .filter(|(power, _)| super::admission::IMPLEMENTED_POWERS.contains(power))
}

/// Validate and freeze the complete Misery-readable projection of one
/// monster.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery::OnPlay` `0xe5bd0` runs in `Misery/<OnPlay>d__3::MoveNext`
/// `0x3ad358`, whose filter and clone lambdas are `Misery/<>c::<OnPlay>b__3_0`
/// `0x3ad30a` (`TypeForCurrentAmount == 2`, not `Type`) and
/// `Misery/<>c::<OnPlay>b__3_1` `0x3ad315` (the dictionary key is the clone,
/// the value the original's `Amount` at snapshot time). The temporary-wrapper
/// fold that runs between them and the attack is `MoveNext` IL_00a1-IL_0133,
/// whose `FirstOrDefault` predicate is
/// `Misery/<>c__DisplayClass3_0::<OnPlay>b__2` `0x3ad335`.
/// Python `_validated_misery_debuff_order` (frozen, deleted #2827).
/// Python `_misery_power_snapshot`.
///
/// Empty acquisition metadata is uniquely reconstructible only for zero or
/// one live scalar power. Scalar duplicates, missing/extra tokens, the four
/// scalar families not yet represented by the Rust engine, and the still-
/// absent TagTeam/Flanking instance families refuse. Knockdown is represented
/// as one positive amount for each repeated acquisition token. Negative
/// Strength, temporary Strength,
/// Shriek and Slow's Effigy owner are native Type-2 shapes whose concrete
/// applier/listener state is likewise absent.
///
/// **`ImbalancedPower` is no longer one of them (#2647 B2).** It is a
/// physical attachment the ledger carries in `Creature.Powers` order, and
/// [`misery_snapshot`] replays it in that position; see
/// [`crate::engine::monsters::imbalanced_amount`] for the two
/// representations of its amount.
///
/// **Nor is negative `StrengthPower`, once its provenance is recorded (#2693
/// S1).** `StrengthPower` has `get_StackType` `0xa8946` = 1 (Intensity) and
/// `get_AllowNegative` `0xa8949` = true, so `PowerModel::GetTypeForAmount`
/// `0x83a94` IL_0013-IL_003e reports type 2 for it exactly while its amount
/// is negative — its *sign*, not its presence, decides whether
/// `Misery/<>c::<OnPlay>b__3_0` `0x3ad30a` selects it. What the scalar alone
/// cannot say is **where** in `Creature.Powers` that one instance sits and
/// **who** applied it, and both are observable: the dictionary is one ordered
/// walk that a recipient's Artifact consumes against in order, and the
/// applier is what `PowerCmd::ModifyAmount`/`Apply` is handed for each copy.
/// A monster whose nonzero Strength carries no ledger row therefore has
/// *unrecorded provenance* and still refuses while it is negative, exactly as
/// the categorical `Strength < 0` disjunct did before this stage — see
/// [`write_monster_strength`] for when a row exists. A row that disagrees
/// with the scalar is malformed: the row adds position and applier, never a
/// second amount.
///
/// **Nor is a live temporary-Strength wrapper, once its provenance is
/// recorded (#2693 S3).** The aggregate `PowerId::TempStrength` scalar says
/// how much temporary Strength a monster has lost but not which concrete
/// instances took it, in what order they attached, or who applied them — and
/// `Misery` observes all three, because its fold (`Misery/<OnPlay>d__3::
/// MoveNext` `0x3ad358` IL_00a1-IL_0133) adds each selected wrapper's frozen
/// amount back onto the frozen `StrengthPower` value before any copy happens,
/// and then copies the wrapper separately through its own Artifact gate. S2's
/// rows carry that provenance; a nonzero scalar the rows do not account for
/// is *unrecorded* and still refuses, exactly as the categorical
/// `TempStrength != 0` disjunct did before this stage — see
/// [`super::monsters::temp_strength_provenance_is_recorded`] and
/// [`write_monster_temp_strength_wrapper`] for the give-up edges that produce
/// it.
pub(crate) fn misery_scalar_state_is_exact(monster: &HotMonster) -> Result<(), EngineRefusal> {
    let strength = monster.powers.value(PowerId::Strength);
    // Deliberately NOT `strength_provenance_is_recorded`: a nonnegative
    // Strength is Type 1, so `Misery`'s filter `0x3ad30a` leaves it out of the
    // dictionary entirely and its position is unobservable *to this read*.
    // What an unrecorded nonnegative scalar threatens is a LATER read, once
    // something drives it negative, and #2693 S4b refuses that at ADMISSION
    // rather than here — but only where the position is unplaceable AND some
    // reducer is reachable, so a Strength nothing can take below zero is never
    // selected and never needs a position. See
    // `"unplaceable entering Strength beside a reachable reducer"` in
    // `super::admission`.
    let strength_provenance_is_exact = match super::monsters::strength_attachment_record(monster) {
        None => strength >= 0,
        Some(record) => record.amount == strength,
    };
    // A live temporary-Strength wrapper, by contrast, is selected at EVERY
    // amount, so an unrecorded one is unreadable now.
    if !strength_provenance_is_exact
        || !super::monsters::temp_strength_provenance_is_recorded(monster)
        || monster.powers.value(PowerId::Shriek) != 0
        || monster.kind == MonsterKind::BygoneEffigy
    {
        return Err(EngineRefusal::MalformedArgs(
            "Misery concrete Type-2 power state",
        ));
    }

    misery_order_projection_is_exact(monster)
}

/// Would `Misery`'s filter select this physically attached instance?
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery/<>c::<OnPlay>b__3_0` `0x3ad30a` keeps a power iff
/// `PowerModel::get_TypeForCurrentAmount` `0x83a80` — that is
/// `GetTypeForAmount(Amount)` `0x83a94` — is 2.
///
/// * `ImbalancedPower`: `get_Type` `0xa3bd3` = 2 and `get_StackType`
///   `0xa3bd6` = 2, so `GetTypeForAmount` falls through at IL_0072 to `Type`
///   and it is **always** selected.
/// * `StrengthPower`: `get_StackType` `0xa8946` = 1 and `get_AllowNegative`
///   `0xa8949` = true, so IL_0013-IL_003e returns 2 iff the amount is
///   negative, and IL_0045's `brtrue` skips the `!AllowNegative` arm
///   entirely. A zero or positive Strength is type 1 and is **not** in the
///   dictionary at all — its ledger position is therefore unobservable while
///   it stays nonnegative.
/// * The concrete `TemporaryStrengthPower` loss wrappers (#2693 S2): the base
///   `get_AllowNegative` `0x83a7d` returns false and no subclass overrides
///   it, so `GetTypeForAmount` skips IL_0013's `AllowNegative` arm, and the
///   stored `Amount` is positive so IL_0063's negative test fails too —
///   IL_0072 falls through to `get_Type` `0xa9ac3`, which is 2 for a
///   non-positive wrapper. They are therefore **always** selected while they
///   are attached, whatever their amount. Reaching this predicate with one is
///   S3's business: [`misery_scalar_state_is_exact`] still refuses every
///   monster carrying a nonzero `TempStrength`, so no snapshot S2 can build
///   contains one. The truth is recorded here rather than approximated, and
///   the executor refuses rather than guessing what to do with it.
fn attachment_is_misery_selected(record: &crate::hot::AttachmentRecord) -> bool {
    match record.power {
        crate::hot::AttachedPowerModel::Imbalanced => true,
        crate::hot::AttachedPowerModel::Strength => record.amount < 0,
        model if model.is_temporary_strength_wrapper() => true,
        // Unreachable by the guard above; spelled so a new non-wrapper
        // variant is still a compile error here.
        crate::hot::AttachedPowerModel::CrushUnder
        | crate::hot::AttachedPowerModel::DarkShackles
        | crate::hot::AttachedPowerModel::DyingStar
        | crate::hot::AttachedPowerModel::EnfeeblingTouch
        | crate::hot::AttachedPowerModel::Mangle
        | crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown
        | crate::hot::AttachedPowerModel::PiercingWail
        | crate::hot::AttachedPowerModel::ShacklingPotion => true,
    }
}

/// Validate only the represented Type-2 multiset/acquisition-order witness.
///
/// Demise's owner-side listener uses this narrower authority even when an
/// unrelated represented power (temporary Strength, Shriek, Slow, or
/// Imbalanced) makes the broader Misery *clone* operation unavailable.
pub(crate) fn misery_order_projection_is_exact(monster: &HotMonster) -> Result<(), EngineRefusal> {
    for (power, _) in MISERY_SCALAR_POWERS {
        let amount = monster.powers.value(power);
        if amount < 0
            || (power == PowerId::Shrink && amount >= 999_999_999)
            || (amount > 0 && !super::admission::IMPLEMENTED_POWERS.contains(&power))
        {
            return Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state",
            ));
        }
    }
    if !monster.owner_latches_are_exact()
        || (monster.demise_after_intangible()
            && (monster.powers.value(PowerId::Demise) <= 0
                || monster.powers.value(PowerId::Intangible) <= 0))
        || (monster.demise_before_ritual()
            && (monster.powers.value(PowerId::Demise) <= 0
                || monster.powers.value(PowerId::Ritual) <= 0))
    {
        return Err(EngineRefusal::MalformedArgs(
            "Demise listener acquisition order",
        ));
    }

    if monster.powers.get(PowerId::Knockdown).is_some() {
        return Err(EngineRefusal::MalformedArgs(
            "Knockdown distinct-instance state",
        ));
    }
    let knockdown = monster.misery_debuff_order.knockdown();
    if knockdown.iter().any(|amount| *amount <= 0) {
        return Err(EngineRefusal::MalformedArgs(
            "Knockdown distinct-instance state",
        ));
    }

    let active = represented_misery_scalars()
        .filter(|(power, _)| monster.powers.value(*power) > 0)
        .count();
    let order = monster.misery_debuff_order.as_slice();
    if order.is_empty() {
        if active > 1 || !knockdown.is_empty() {
            return Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order",
            ));
        }
        return Ok(());
    }
    if order.len() != active + knockdown.len()
        || order
            .iter()
            .filter(|token| **token == MiseryToken::Knockdown)
            .count()
            != knockdown.len()
        || order.iter().any(|token| {
            *token != MiseryToken::Knockdown
                && represented_misery_scalars().all(|(_, candidate)| candidate != *token)
        })
        || represented_misery_scalars().any(|(power, token)| {
            order
                .iter()
                .filter(|candidate| **candidate == token)
                .count()
                != usize::from(monster.powers.value(power) > 0)
        })
    {
        return Err(EngineRefusal::MalformedArgs(
            "Misery power acquisition order",
        ));
    }
    Ok(())
}

pub(crate) fn misery_scalar_snapshot(
    monster: &HotMonster,
) -> Result<Vec<MiseryScalarSnapshot>, EngineRefusal> {
    misery_scalar_state_is_exact(monster)?;
    let knockdown = monster.misery_debuff_order.knockdown();
    let order = monster.misery_debuff_order.as_slice();
    if order.is_empty() {
        return Ok(represented_misery_scalars()
            .filter(|(power, _)| monster.powers.value(*power) > 0)
            .map(|(power, token)| MiseryScalarSnapshot {
                power,
                token,
                amount: monster.powers.value(power),
            })
            .collect());
    }

    let mut snapshot = Vec::with_capacity(order.len());
    let mut knockdown_index = 0;
    for token in order.iter().copied() {
        if token == MiseryToken::Knockdown {
            let Some(&amount) = knockdown.get(knockdown_index) else {
                return Err(EngineRefusal::MalformedArgs(
                    "Misery power acquisition order",
                ));
            };
            knockdown_index += 1;
            snapshot.push(MiseryScalarSnapshot {
                power: PowerId::Knockdown,
                token,
                amount,
            });
            continue;
        }
        let (power, _) = represented_misery_scalars()
            .find(|(_, candidate)| *candidate == token)
            .expect("validation authenticated scalar token");
        snapshot.push(MiseryScalarSnapshot {
            power,
            token,
            amount: monster.powers.value(power),
        });
    }
    debug_assert_eq!(knockdown_index, knockdown.len());
    Ok(snapshot)
}

/// One position of the frozen `Target.Powers` walk `Misery` replays.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum MiserySnapshotEntry {
    /// A represented scalar singleton, with its frozen amount.
    Scalar(MiseryScalarSnapshot),
    /// A physically attached instance, with its frozen amount and applier.
    Attachment(crate::hot::AttachmentRecord),
}

/// The frozen dictionary value of one snapshot position — what `0x3ad358`
/// IL_0268-IL_026f tests against zero and IL_02a5/IL_0343 pass to
/// `ModifyAmount`/`Apply`.
pub(crate) fn misery_snapshot_entry_amount(entry: &MiserySnapshotEntry) -> i32 {
    match entry {
        MiserySnapshotEntry::Scalar(frozen) => frozen.amount,
        MiserySnapshotEntry::Attachment(record) => record.amount,
    }
}

/// Freeze the complete ordered Type-2 projection `Misery` clones.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_003e-IL_009c is **one**
/// `Target.Powers.Where(filter).Select(clone).ToDictionary()`, so its replay
/// order is a single `Creature.Powers` sequence. Scalars and attachments are
/// therefore merged by ledger position here and never applied in two passes;
/// the recipient's Artifact consumes on whichever comes first
/// (`ArtifactPower::TryModifyPowerAmountReceived` `0x9f844`).
///
/// **The Bowlbug Rock's intrinsic instance leads the walk.** It carries no
/// ledger row while its amount is the applied 1
/// ([`crate::engine::monsters::imbalanced_amount`]), and position 0 is
/// *proven*, not assumed, for every roster that can hold a Rock:
/// `CombatManager/<StartCombatInternal>d__98::MoveNext` runs
/// `CombatManager::AfterCreatureAdded` for every creature at IL_011f-IL_0157
/// — hence `Creature::AfterAddedToRoom` `0x3dae14` and the Rock's
/// `PowerCmd.Apply<ImbalancedPower>` at `0x353e30` IL_007f-IL_0097 — before
/// it sets `IsInProgress` at IL_020f and fires `Hook::BeforeCombatStart` at
/// IL_023b, so no relic, power or ascension combat-start hook can precede it.
/// The only other creatures a Bowlbug roster holds are the player, whose
/// `Creature::AfterAddedToRoom` returns at IL_001b when `Monster` is null, and
/// one of `BowlbugEgg` (`<AfterAddedToRoom>d__15` `0x35398c` awaits the base
/// and applies nothing) or `BowlbugNectar` (no override at all). A census of
/// every `<AfterAddedToRoom>` body in the assembly finds each power `Apply`
/// targeting its own `MonsterModel::get_Creature`. Nothing ever attaches
/// ahead of it afterwards either: `Creature::ApplyPowerInternal` `0x11da0c`
/// IL_0063-IL_006f appends.
///
/// A legacy document whose acquisition order is empty carries at most one
/// reconstructible scalar and no positions at all
/// ([`misery_order_projection_is_exact`]). Placing an attachment relative to
/// that scalar would be a guess, so a ledger holding both refuses (I5) — and
/// only an attachment the filter would actually select can force that, since
/// an unselected one contributes no copy whose order could matter.
///
/// **Selection is per model, not per presence (#2693 S1).** Imbalanced is
/// always in the dictionary; a Strength row is in it exactly while its amount
/// is negative. See [`attachment_is_misery_selected`].
///
/// **The temporary-Strength fold runs here, before the attack (#2693 S3).**
/// See [`fold_temporary_strength_wrappers`]: it is `0x3ad358` IL_00a1-IL_0133,
/// which sits between the `ToDictionary` at IL_009c and the awaited
/// `DamageCmd` at IL_014c, so the adjusted values are frozen before anything
/// the attack does can move them.
pub(crate) fn misery_snapshot(
    monster: &HotMonster,
) -> Result<Vec<MiserySnapshotEntry>, EngineRefusal> {
    let scalars = misery_scalar_snapshot(monster)?;
    let mut snapshot = Vec::with_capacity(scalars.len().saturating_add(1));
    if monster.kind == MonsterKind::BowlbugRock
        && super::monsters::imbalanced_attachment(monster).is_none()
    {
        snapshot.push(MiserySnapshotEntry::Attachment(
            crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Imbalanced,
                applier: crate::hot::Applier::Monster(monster.uid),
                amount: 1,
            },
        ));
    }
    if monster.misery_debuff_order.as_slice().is_empty() {
        let mut selected = monster
            .misery_debuff_order
            .attachments()
            .copied()
            .filter(attachment_is_misery_selected)
            .peekable();
        if !scalars.is_empty() && selected.peek().is_some() {
            return Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order",
            ));
        }
        snapshot.extend(selected.map(MiserySnapshotEntry::Attachment));
        snapshot.extend(scalars.into_iter().map(MiserySnapshotEntry::Scalar));
        fold_temporary_strength_wrappers(&mut snapshot)?;
        return Ok(snapshot);
    }
    let mut scalars = scalars.into_iter();
    for entry in monster.misery_debuff_order.walk() {
        match entry {
            crate::hot::MiseryLedgerEntry::Token(_) => {
                let Some(frozen) = scalars.next() else {
                    return Err(EngineRefusal::MalformedArgs(
                        "Misery power acquisition order",
                    ));
                };
                snapshot.push(MiserySnapshotEntry::Scalar(frozen));
            }
            // An unselected attachment holds its ledger position but is not in
            // the frozen dictionary at all (`0x3ad30a`), so it contributes no
            // copy and consumes no recipient Artifact.
            crate::hot::MiseryLedgerEntry::Attachment(record) => {
                if attachment_is_misery_selected(&record) {
                    snapshot.push(MiserySnapshotEntry::Attachment(record));
                }
            }
        }
    }
    if scalars.next().is_some() {
        return Err(EngineRefusal::MalformedArgs(
            "Misery power acquisition order",
        ));
    }
    fold_temporary_strength_wrappers(&mut snapshot)?;
    Ok(snapshot)
}

/// Can any creature other than a Misery source ever be one of its recipients
/// in this fight? (#3311)
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` reads its frozen dictionary
/// `<debuffAmounts>5__2` in exactly two places: the temporary-Strength fold
/// at IL_00a1-IL_0133, which rewrites dictionary values and nothing else,
/// and the per-recipient walk that IL_0236 opens. That walk sits inside the
/// enumeration of `ICombatState::get_HittableEnemies` begun at IL_01ed
/// (`CombatState::get_HittableEnemies` `0x1373a5` is `get_Enemies` filtered
/// by `get_IsHittable`, predicate `0x3f91ba`), and IL_0220-IL_0231 skips the
/// original `cardPlay.Target` by reference. Building the dictionary has no
/// combat-state effect either: the filter `0x3ad30a` only reads
/// `TypeForCurrentAmount`, and the clone `0x3ad315` is
/// `AbstractModel::ClonePreservingMutability` `0x79ef7` ->
/// `MutableClone` `0x79f0c` (`MemberwiseClone`, then
/// `PowerModel::DeepCloneFields` `0x8406d` and `PowerModel::AfterCloned`
/// `0x84093`, which write only the fresh clone's own fields; every one of
/// the 36 `InitInternalData` overrides that `DeepCloneFields` calls is a bare
/// `newobj`/`ldnull; ret`). So when no enemy other than the target can be
/// hittable after the attack, the target's Type-2 contents are unobservable.
///
/// That holds for the whole fight when the roster is one monster and nothing
/// can add an enemy. A whole-assembly census of `CreatureCmd::Add` callers
/// finds exactly: the move bodies of `Fabricator` (`<SpawnBot>d__25`
/// `0x35a80c`), `Fogmog` (`<IllusionMove>d__15` `0x35bef4`), `LivingFog`
/// (`<BloatMove>d__26` `0x3620c0`), `Ovicopter` (`<LayEggsMove>d__26`
/// `0x3651e8`), `TheObscura` (`<IllusionMove>d__25` `0x370830`) and
/// `TwoTailedRat` (`<CallForBackup>d__41` `0x373d98`); the `AfterDeath`
/// bodies of `StockPower` (`0x346280`), `SurprisePower` (`0x347044`) and
/// `InfestedPower` (`0x33d3ec`); `PlayerCmd/<AddPet>d__15` `0x3ee7b0`, whose
/// pet joins the player's side through `PlayerCombatState::AddPetInternal`
/// (IL_003f) and so is never an enemy; `CreatureCmd`'s own overload
/// forwarders; and a test mock. A census of every IL reference to the three
/// power types finds their only appliers are `Axebot` (`0x352dcc`
/// IL_003f), `GremlinMerc` (`0x35d898` IL_009f) and `PhrogParasite`
/// (`0x3667dc` IL_0098), each on itself at room entry. A lone monster of any
/// other kind can therefore never be joined.
///
/// This is the admission-time question only; `misery_inner` still takes the
/// snapshot's refusal the moment a recipient does exist, so a roster this
/// predicate misjudged would refuse late rather than answer wrongly.
pub(crate) fn misery_roster_can_present_a_recipient(state: &crate::hot::HotState) -> bool {
    match state.monsters.as_slice() {
        [only] => matches!(
            only.kind,
            MonsterKind::Fabricator
                | MonsterKind::Fogmog
                | MonsterKind::LivingFog
                | MonsterKind::Ovicopter
                | MonsterKind::TheObscura
                | MonsterKind::TwoTailedRat
                | MonsterKind::Axebot
                | MonsterKind::GremlinMerc
                | MonsterKind::PhrogParasite
        ),
        _ => true,
    }
}

/// Fold each selected temporary-Strength wrapper's frozen amount into the
/// frozen `StrengthPower` value, in place (#2693 S3).
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_00a1-IL_0133 walks the frozen
/// dictionary once and, for each entry whose key `is ITemporaryPower`
/// (IL_00cb `isinst`, IL_00dc skips the rest), looks for the **first** entry
/// whose `Key.Id` equals that wrapper's `InternallyAppliedPower.Id`
/// (`FirstOrDefault` at IL_00f1, predicate
/// `Misery/<>c__DisplayClass3_0::<OnPlay>b__2` `0x3ad335`). Three facts that
/// an aggregate signed scalar cannot express, each read rather than assumed:
///
/// * **No synthesis.** A missing internally-applied entry is a default
///   `KeyValuePair` whose `Key` is null, and IL_00ff's `brfalse` skips the
///   whole adjustment — the wrapper still copies, but no `StrengthPower`
///   entry is invented. The target exists exactly while the source's Strength
///   is negative, because that is when `Misery`'s filter selects it
///   ([`attachment_is_misery_selected`]).
/// * **The adjustment is `+= the wrapper's own frozen value`** (IL_0101-
///   IL_0127: `dict[key] = dict[key] + entry.Value`, where `entry` is the
///   `KeyValuePair` the enumerator handed out). The stored wrapper `Amount`
///   is POSITIVE and its effect on Strength was negative
///   (`get_Sign` `0xa9add`, `<BeforeApplied>d__20::MoveNext` `0x348d20`
///   IL_0028-IL_004b), so the fold *removes* the wrapper's contribution from
///   the value that is about to be copied — the recipient then receives the
///   intrinsic part as Strength and the wrapper separately, each through its
///   own Artifact gate. Intrinsic `+2` with a loss wrapper of `5` is a source
///   at `-3` that copies `+2`.
/// * **Several wrappers accumulate, and the wrapper entries never move.**
///   The read at IL_011a is of the dictionary's current value while
///   `entry.Value` is the enumerator's frozen copy, and no wrapper is ever a
///   fold *target* (its own `Id` is its own model's), so two wrappers add
///   both of their amounts exactly once each and keep their own positions.
///
/// `InternallyAppliedPower` is `Power<StrengthPower>` for **all fourteen**
/// `TemporaryStrengthPower` subclasses — `get_InternallyAppliedPower`
/// `0xa9ad3` is declared once on the base and overridden by none of them
/// (#2693 S2 dumped every one) — so the target is always the Strength entry,
/// which is why this takes no per-model parameter.
///
/// The frozen value is a native `int32` (`0x3ad315` boxes `get_Amount` into a
/// `ValueTuple<PowerModel, int32>`), so an overflow is a representation limit
/// rather than a native state and refuses.
fn fold_temporary_strength_wrappers(
    snapshot: &mut [MiserySnapshotEntry],
) -> Result<(), EngineRefusal> {
    let strength = snapshot.iter().position(|entry| {
        matches!(
            entry,
            MiserySnapshotEntry::Attachment(record)
                if record.power == crate::hot::AttachedPowerModel::Strength
        )
    });
    let Some(strength) = strength else {
        return Ok(());
    };
    let wrappers = snapshot
        .iter()
        .filter_map(|entry| match entry {
            MiserySnapshotEntry::Attachment(record)
                if record.power.is_temporary_strength_wrapper() =>
            {
                Some(record.amount)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for amount in wrappers {
        let MiserySnapshotEntry::Attachment(record) = &mut snapshot[strength] else {
            unreachable!("the located entry is the Strength attachment");
        };
        record.amount = record
            .amount
            .checked_add(amount)
            .ok_or_else(|| overflow("Misery temporary Strength fold"))?;
    }
    Ok(())
}

/// Validate the complete represented distinct-instance Knockdown projection.
pub(crate) fn knockdown_state_is_exact(monster: &HotMonster) -> bool {
    let amounts = monster.misery_debuff_order.knockdown();
    monster.powers.get(PowerId::Knockdown).is_none()
        && amounts.iter().all(|amount| *amount > 0)
        && monster
            .misery_debuff_order
            .as_slice()
            .iter()
            .filter(|token| **token == MiseryToken::Knockdown)
            .count()
            == amounts.len()
}

fn require_live_knockdown_state(state: &HotState) -> Result<bool, EngineRefusal> {
    let mut reachable = false;
    for monster in state.monsters.iter() {
        let present = !monster.misery_debuff_order.knockdown().is_empty()
            || monster.powers.get(PowerId::Knockdown).is_some()
            || monster
                .misery_debuff_order
                .as_slice()
                .contains(&MiseryToken::Knockdown);
        reachable |= present;
        if present
            && ((monster.hp <= 0 && !state.fanouts.monster_death_is_pending(monster.uid))
                || !knockdown_state_is_exact(monster))
        {
            return Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state",
            ));
        }
    }
    Ok(reachable)
}

fn require_knockdown_death_entry(state: &HotState, target: usize) -> Result<bool, EngineRefusal> {
    let mut reachable = false;
    for (index, monster) in state.monsters.iter().enumerate() {
        let present = !monster.misery_debuff_order.knockdown().is_empty()
            || monster.powers.get(PowerId::Knockdown).is_some()
            || monster
                .misery_debuff_order
                .as_slice()
                .contains(&MiseryToken::Knockdown);
        reachable |= present;
        if present
            && (!knockdown_state_is_exact(monster)
                || (index != target
                    && monster.hp <= 0
                    && !state.fanouts.monster_death_is_pending(monster.uid)))
        {
            return Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state",
            ));
        }
    }
    Ok(reachable)
}

/// Maintain the exact scalar subset of Misery's acquisition-order witness.
///
/// Python deliberately treats this record as best-effort legacy metadata:
/// `_misery_active_debuff_counts` reconstructs the complete live multiset
/// (frozen Python, deleted #2827), then `_record_misery_debuff_application` catches a refused
/// reconstruction and returns without touching the order.
/// A malformed legacy negative sibling must not manufacture acquisition
/// evidence. Waterfall's retained old Poison model is not such a sibling: its
/// same-UID continuation decrements detached state and leaves the cleared
/// attached slot at zero.
fn record_misery_debuff_application(
    monster: &mut HotMonster,
    power: PowerId,
    token: MiseryToken,
    new_instance: bool,
) {
    if !MISERY_SCALAR_POWERS.contains(&(power, token)) {
        return;
    }
    let knockdown = monster.misery_debuff_order.knockdown();
    if knockdown.iter().any(|amount| *amount <= 0) {
        return;
    }
    let mut previous_count = knockdown.len();
    let mut inferred = None;
    for (power, candidate) in MISERY_SCALAR_POWERS {
        let amount = monster.powers.value(power);
        if amount < 0 {
            return;
        }
        if amount > 0 && !(new_instance && candidate == token) {
            previous_count += 1;
            inferred = Some(candidate);
        }
    }

    let order = monster.misery_debuff_order.as_slice();
    let matches_previous = order.len() == previous_count
        && MISERY_SCALAR_POWERS.iter().all(|(power, candidate)| {
            let expected =
                monster.powers.value(*power) > 0 && !(new_instance && *candidate == token);
            order.iter().filter(|entry| **entry == *candidate).count() == usize::from(expected)
        })
        && order
            .iter()
            .filter(|entry| **entry == MiseryToken::Knockdown)
            .count()
            == knockdown.len();
    if !matches_previous {
        if !order.is_empty() || previous_count > 1 {
            return;
        }
        if let Some(previous) = inferred {
            monster.misery_debuff_order.push(previous);
        }
    }
    if new_instance {
        monster.misery_debuff_order.push(token);
    }
}

/// Apply a player-originated debuff to a monster (`apply_monster_debuff`, frozen Python, deleted #2827), through the `ArtifactPower` gate.
///
/// This low-level scalar writer intentionally does not run Artifact or Lamp;
/// public card/power writers below own those command listeners. The
/// acquisition-order record (`_record_misery_debuff_application`, frozen Python, deleted #2827)
/// appends only when the power was not already active.
pub fn apply_monster_debuff(
    monster: &mut HotMonster,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
) {
    let was_active = monster.powers.value(power) > 0;
    let updated = monster.powers.value(power) + amount;
    monster.powers.set(power, SlotWire::Int, updated);
    if updated > 0 {
        record_misery_debuff_application(monster, power, token, !was_active);
    }
}

/// Apply a **card-sourced** debuff to a monster, through the given-side
/// amount modifiers (`_card_power_amount_given`, frozen Python, deleted #2827).
///
/// The represented multiplicative term is Unsettling Lamp. The first
/// non-Artifact-blocked permanent Type-2 application latches the physical
/// CardPlay and every later such application by that same play doubles. The
/// latch is consumed by `AfterCardPlayed`, before a replay body begins.
///
/// Poison additionally allocates its instance identity here: an application
/// that makes a previously-unpoisoned creature live takes the next
/// `next_poison_uid` (`_ensure_poison_instance_uid`, frozen Python, deleted #2827).
pub fn apply_card_monster_debuff(
    state: &mut HotState,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let live_target = state
        .monsters
        .get(target)
        .is_some_and(|monster| monster.hp > 0);
    let artifact_blocks = state
        .monsters
        .get(target)
        .is_some_and(|monster| monster.powers.value(PowerId::Artifact) > 0);
    if !state.history.over
        && live_target
        && amount != 0
        && !artifact_blocks
        && state.card_states.dampen().is_some()
        && state.powers.value(PowerId::SleightOfFlesh) > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen card debuff Sleight catalog",
        ));
    }
    // A Vicious callback must run the complete ordinary Draw command, whose
    // status/physical/listener readers require the catalog. Keep the old seam
    // total for every non-Vulnerable writer, but fail before mutation if a
    // future Vulnerable producer forgets to use the catalog-aware sibling.
    if power == PowerId::Vuln && vicious_listener_is_reachable(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Vicious Vulnerable producer catalog",
        ));
    }
    apply_player_monster_debuff(
        state,
        PlayerMonsterDebuffSource::Card(None),
        target,
        power,
        token,
        amount,
        events,
    )
}

/// Append one card-sourced local-player Knockdown instance through the shared
/// Artifact-before-Lamp and AfterPowerAmountChanged pipeline.
pub fn apply_card_knockdown(
    state: &mut HotState,
    target: usize,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if state.history.over || monster.hp <= 0 {
        return Ok(());
    }
    if amount > 0
        && monster.powers.value(PowerId::Artifact) <= 0
        && state.card_states.dampen().is_some()
        && state.powers.value(PowerId::SleightOfFlesh) > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen Knockdown Sleight catalog",
        ));
    }
    if amount <= 0 || !knockdown_state_is_exact(monster) {
        return Err(EngineRefusal::MalformedArgs(
            "Knockdown distinct-instance state",
        ));
    }
    let Some(amount) = card_monster_type_two_amount(state, target, amount, events)? else {
        return Ok(());
    };
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("Knockdown amount"));
    }
    // Artifact suppresses the application before Misery observes it.  Do not
    // reject an otherwise unreachable/ambiguous legacy scalar ordering when
    // Artifact consumes this Knockdown without creating an instance.
    let old_snapshot = misery_scalar_snapshot(&state.monsters[target])?;
    let inferred = state.monsters[target]
        .misery_debuff_order
        .as_slice()
        .is_empty()
        .then(|| old_snapshot.first().map(|entry| entry.token))
        .flatten();
    let uid = state.monsters[target].uid;
    let order = &mut state.monsters_mut()[target].misery_debuff_order;
    if let Some(token) = inferred {
        order.push(token);
    }
    order.push_knockdown(amount);
    note_power(events, Subject::Monster(uid), PowerId::Knockdown, amount);
    powers_after_power_amount_changed(
        state,
        None,
        target,
        Some(PowerId::Knockdown),
        amount,
        false,
        // `Knockdown/<OnPlay>d__5::MoveNext` `0x3a8adc` IL_0100-IL_010d passes
        // `card.Owner.Player.Creature`; a `Misery` copy of the scalar takes
        // the same path through this seam.
        crate::hot::Applier::Player,
        events,
    )
}

/// Apply a card-sourced monster debuff with the catalog needed by Vicious.
///
/// Vicious's awaited Draw is part of the originating Power command. Rehearse
/// that complete command before publishing Artifact, the changed debuff, RNG,
/// pile/history, or listener effects; a late unknown atom, reshuffle, or draw
/// listener refusal therefore leaves the caller's command untouched.
pub fn apply_card_monster_debuff_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let dampen_reachable = state.card_states.dampen().is_some();
    if dampen_reachable && !super::cards::dampen_state_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs("Dampen card debuff entry"));
    }
    if power == PowerId::Vuln && vicious_listener_is_reachable(state) || dampen_reachable {
        if !player_type_one_listener_order_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs(
                "after-power-amount-changed listener order",
            ));
        }
        super::play::rehearse_preserving_active_plays(|| {
            let mut probe = state.clone();
            apply_player_monster_debuff(
                &mut probe,
                PlayerMonsterDebuffSource::Card(Some(catalog)),
                target,
                power,
                token,
                amount,
                &mut Vec::new(),
            )
        })?;
    }
    apply_player_monster_debuff(
        state,
        PlayerMonsterDebuffSource::Card(Some(catalog)),
        target,
        power,
        token,
        amount,
        events,
    )
}

/// Apply one `Misery`-cloned `ImbalancedPower` instance to a recipient.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * **The clone is a real clone that keeps its applier.**
///   `Misery/<>c::<OnPlay>b__3_1` `0x3ad315` and `0x3ad358` IL_0320-IL_032c
///   both call `AbstractModel::ClonePreservingMutability` `0x79ef7`, which
///   returns `this` for a canonical model and `AbstractModel::MutableClone`
///   `0x79f0c` otherwise. An attached instance is always mutable, because
///   `PowerCmd/<Apply>d__1\`1::MoveNext` `0x3ef988` IL_0081-IL_0089 builds it
///   with `PowerModel::ToMutable` `0x83fe4` (`AssertCanonical` then
///   `MutableClone`). So the clone always routes through `MutableClone` ->
///   `PowerModel::AfterCloned` `0x84093`, which nulls the five events and
///   `_owner` and leaves `_applier` alone. Plan §8 open question 3 is closed
///   by that pair, not inferred.
/// * **Artifact blocks it and is consumed.**
///   `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` passes when
///   `target != Owner` (IL_000c-IL_001e), when
///   `GetTypeForAmount(delta) != 2` (IL_001f-IL_0032) or when `!IsVisible`
///   (IL_0033-IL_0044); otherwise it zeroes the delta and reports consumed,
///   and `ArtifactPower/<AfterModifyingPowerAmountReceived>d__7::MoveNext`
///   `0x334fd0` IL_001b-IL_001e decrements on that flag. Both remaining
///   conjuncts hold for this clone: `PowerModel::get_AllowNegative` `0x83a7d`
///   is false and `ImbalancedPower` does not override it, so
///   `GetTypeForAmount` `0x83a94` falls through at IL_0072 to `get_Type`
///   `0xa3bd3` = 2 for a positive delta; and `PowerModel::get_IsVisible`
///   `0x83754` returns `get_IsVisibleInternal` `0x83780` = true whenever
///   `Target` is null, which it is — `PowerModel::set_Target` is never called
///   for this family, and `PowerModel::DeepCloneFields` `0x8406d` /
///   `AfterCloned` leave `_target` alone. Plan §8 open question 5 is closed
///   the same way.
/// * **Stacking is a singleton `ModifyAmount`, not a new position.**
///   `PowerModel::get_InstanceType` `0x83751` is 0 and `ImbalancedPower` does
///   not override it, so `PowerCmd::FindExistingInstanceForStacking`
///   `0x1338d8` takes the `Creature::GetPower(Id)` arm at IL_0058 and
///   `0x3ad358` IL_029b-IL_02bd routes to `PowerCmd::ModifyAmount`
///   `0x3f032c`, which reaches `PowerModel::SetAmount` `0x83f8c` — a write to
///   `_amount` that never touches `Creature::_powers`. `ModifyAmount` also
///   never reaches `set_Applier` (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`
///   writes it at IL_013d on the fresh-attach fall-through only), so a
///   stacked instance keeps the applier it was attached with. `SetAmount`
///   clamps to +/-999999999 at IL_0013-IL_0022, which is the only bound
///   native puts on the amount.
///
/// **The Unsettling Lamp doubles it exactly when its applier is still in
/// combat (#3404, #2647).** The whole assembly has exactly two given-side
/// amount modifiers — `SneckoSkull::ModifyPowerAmountGivenAdditive`
/// `0x9bb8d`, which returns 0 for anything that is not a `PoisonPower`, and
/// `UnsettlingLamp::ModifyPowerAmountGivenMultiplicative` `0x9d5a8` — and
/// both commands gate them on `ICombatState.ContainsCreature(applier)`: the
/// fresh attach at `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`
/// IL_01d3-IL_01ec (hook call IL_0219) and the restack at
/// `PowerCmd/<ModifyAmount>d__6::MoveNext` `0x3f032c` IL_0106-IL_0119 (hook
/// call IL_0147). This clone's applier is the source row's retained monster
/// (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_02b6 / IL_0354), and its
/// `cardSource` is the Misery card itself (`ldloc.1`, IL_02bb / IL_0359).
///
/// * **The multiplier.** `0x9d5a8` returns 2 only when `TriggeringCard` is
///   set (IL_000c-IL_0012), equals this `cardSource` (IL_001a-IL_0022), has
///   not finished triggering (IL_002a-IL_0030), the power is not the
///   internal power of a doubled temporary power (IL_0038-IL_003f; the
///   `HasDoubledTemporaryPowerSource` `0x9d66c` predicate compares
///   `InternallyAppliedPower` types, never `ImbalancedPower`), and the amount
///   is Type 2 (IL_0047-IL_004f, true for this clone, above). While the
///   Misery play is active, "`TriggeringCard` is this card and unfinished" is
///   exactly [`super::play::active_card_lamp_triggered`] `== Some(true)`:
///   `UnsettlingLamp::AfterCardPlayed` `0x9d606` only finishes it after the
///   card's own play.
/// * **No latch.** `UnsettlingLamp::BeforePowerAmountChanged` `0x9d4fc`
///   IL_0032-IL_003f requires `applier == Owner.Player.Creature`, so a
///   monster-applied clone never sets `TriggeringCard`. An earlier
///   player-applied clone in the same Misery play can, which is what makes
///   the doubling reachable.
/// * **`ContainsCreature`** (`CombatState::ContainsCreature` `0x137286`) is
///   `_allies`/`_enemies` membership, answered for the applier monster by
///   [`super::monsters::in_native_enemies`]. The clone runs inside a player
///   card play, after the attack's deaths have finished, so no monster is
///   performing a move and a dead applier has already been evicted
///   (`CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext`
///   `0x3ebe90` IL_046a-IL_04d5).
pub(crate) fn apply_card_monster_imbalanced_clone(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    applier: crate::hot::Applier,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("Imbalanced clone amount"));
    }
    // `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_0039-IL_006c leaves on a
    // combat that is ending and on a target that cannot receive powers.
    if damage_combat_is_ending(state) || monster.hp <= 0 {
        return Ok(());
    }
    let applier_in_combat = match applier {
        crate::hot::Applier::Monster(uid) => {
            let applier = state
                .monsters
                .iter()
                .find(|monster| monster.uid == uid)
                .ok_or(EngineRefusal::MalformedArgs("Imbalanced clone applier"))?;
            super::monsters::in_native_enemies(applier)?
        }
        _ => return Err(EngineRefusal::MalformedArgs("Imbalanced clone applier")),
    };
    let Some(amount) =
        card_monster_applied_type_two_amount(state, target, amount, applier_in_combat, events)?
    else {
        // Artifact zeroed the delta and was consumed; `PowerModel::ApplyInternal`
        // `0x84012` IL_0001-IL_000e then returns before attaching.
        return Ok(());
    };
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("Imbalanced clone amount"));
    }
    let monster = &state.monsters[target];
    let existing = super::monsters::imbalanced_attachment(monster);
    let intrinsic = existing.is_none() && monster.kind == MonsterKind::BowlbugRock;
    let previous = super::monsters::imbalanced_amount(monster);
    match (existing, previous) {
        (Some(index), Some(current)) => {
            let updated = current
                .checked_add(amount)
                .ok_or_else(|| overflow("Imbalanced clone amount"))?
                .clamp(-999_999_999, 999_999_999);
            if !state.monsters_mut()[target]
                .misery_debuff_order
                .set_attachment_amount(index, updated)
            {
                return Err(EngineRefusal::MalformedArgs("Imbalanced clone amount"));
            }
        }
        (None, Some(current)) => {
            // The by-kind Rock: one native instance whose amount has left the
            // applied 1, so it needs the row the kind can no longer imply. It
            // occupies `Creature._powers[0]` (see [`misery_snapshot`]), and
            // `ModifyAmount` does not rewrite the applier, so the row keeps
            // the Rock's own intrinsic one from `0x353e30` IL_007f-IL_0097.
            debug_assert!(intrinsic, "a non-Rock cannot carry an amount without a row");
            let updated = current
                .checked_add(amount)
                .ok_or_else(|| overflow("Imbalanced clone amount"))?
                .clamp(-999_999_999, 999_999_999);
            let record = crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Imbalanced,
                applier: crate::hot::Applier::Monster(monster.uid),
                amount: updated,
            };
            let order = &mut state.monsters_mut()[target].misery_debuff_order;
            let mut placed = order.placed_attachments();
            placed.insert(0, (0, record));
            if !order.set_attachments(placed) {
                return Err(EngineRefusal::MalformedArgs("Imbalanced clone position"));
            }
        }
        (None, None) => {
            // A fresh attach appends (`Creature::ApplyPowerInternal`
            // `0x11da0c` IL_0063-IL_006f) and `set_Applier` `0x3efbac`
            // IL_013d writes the applier the command was given.
            state.monsters_mut()[target]
                .misery_debuff_order
                .push_attachment(crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Imbalanced,
                    applier,
                    amount,
                });
        }
        (Some(_), None) => {
            // A row whose amount could not be read. `imbalanced_amount`
            // resolves every row it finds, so this is unreachable — and it
            // refuses rather than falling through to the fresh-attach arm,
            // which would give one native instance a second position.
            return Err(EngineRefusal::MalformedArgs("Imbalanced clone amount"));
        }
    }
    // The catalog is threaded rather than dropped: this is a card-sourced
    // Type-2 application, so `Hook::AfterPowerAmountChanged` `0x1045ec` runs
    // and its damage would need the catalog wherever a Dampen entry is live —
    // which is exactly why `apply_card_knockdown`, the sibling with no catalog
    // to give, has to refuse that combination by name instead.
    //
    // #2727 — the clone's retained applier is threaded through. Four of
    // `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext`
    // `0x344dc4`'s five conditions hold for this clone (`amount != 0`
    // IL_0026, `GetTypeForAmount == 2` IL_0043, `Owner.IsEnemy` IL_0056, not
    // an `ITemporaryPower` IL_0080); the fifth, IL_0067-IL_0075
    // `applier == this.Owner`, does NOT — the Bowlbug Rock is the only applier
    // this family can carry (`0x353e30` IL_007f-IL_0097), so Sleight of Flesh
    // returns rather than firing. #2714 read IL_006e as a second owner test
    // and modeled it as firing; it is the applier test, and this is the
    // correction.
    powers_after_power_amount_changed(
        state,
        Some(catalog),
        target,
        None,
        amount,
        false,
        applier,
        events,
    )
}

/// One `Misery` copy of a frozen negative `StrengthPower`, recipient-side.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_0274-IL_028d looks the
/// recipient's instance up with `PowerCmd::FindExistingInstanceForStacking`
/// keyed by `entry.Key.Applier`, routes a hit to `PowerCmd::ModifyAmount`
/// `0x3f032c` at IL_029b-IL_02bd and a miss to `ClonePreservingMutability` +
/// `PowerCmd::Apply` at IL_0320-IL_035b. `StrengthPower` takes the base
/// `PowerModel::get_InstanceType` `0x83751` = 0, so that lookup is
/// `Creature::GetPower(Id)` (IL_0058) — by id alone — and both arms collapse
/// into one signed delta on the recipient's single instance, which
/// [`write_monster_strength`] resolves.
///
/// The frozen value is copied **without re-filtering by sign**: IL_0268-
/// IL_026f only skips an entry whose value is zero, which a selected Strength
/// never is ([`attachment_is_misery_selected`]).
///
/// **Artifact consumes on this copy.**
/// `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` IL_001f-IL_0032
/// tests `power.GetTypeForAmount(/* the incoming delta */ amount)`, and for
/// an `AllowNegative`, Intensity-stacking power `0x83a94` IL_0013-IL_003e
/// reports 2 for a negative delta — so a `-1` against an existing `+5`
/// consumes an Artifact and a `+1` restoring an existing `-5` does not. Only
/// the negative direction is reachable here.
///
/// **The Unsettling Lamp refuses beside a non-player applier**, for the
/// reason #2647 B2 established for the Imbalanced clone:
/// `card_monster_type_two_amount` models the Lamp for the always-present
/// player applier and has no applier concept, while native gates the
/// given-side modifiers on `ICombatState.ContainsCreature(applier)`
/// (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_01d3-IL_01ec). A Lamp
/// latched earlier in the same play would double a monster- or relic-applied
/// copy exactly when that applier is still in the roster. A `Player` applier
/// is the case the existing model *is*, so it does not refuse.
///
/// **The frozen value can be POSITIVE once the wrapper fold runs (#2693 S3)**,
/// and a positive delta is a different native command — not a Type-2 one at
/// all. `GetTypeForAmount` `0x83a94` IL_0013-IL_003e reports 2 for an
/// `AllowNegative` Intensity power only while the amount is negative, and
/// every amount-sensitive hook on the way tests exactly that value:
///
/// * `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` IL_001f-IL_0032
///   passes it through **without consuming**;
/// * `UnsettlingLamp::BeforePowerAmountChanged` `0x9d4fc` IL_0073-IL_007b
///   does not latch on it and `ModifyPowerAmountGivenMultiplicative`
///   `0x9d5a8` IL_0047-IL_004f returns `Decimal.One` for it;
/// * `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext` `0x344dc4`
///   IL_003d-IL_0049 returns before its damage
///   ([`powers_after_power_amount_changed`] carries that test).
///
/// So a positive copy lands its amount and nothing else, which is what makes
/// the `+2 / -5 / net -3` case end at `+2` on a recipient holding one
/// Artifact: the Artifact passes the Strength and spends itself on the
/// wrapper that follows it in the ledger.
pub(crate) fn apply_card_monster_strength_clone(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    applier: crate::hot::Applier,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if amount == 0 {
        // `0x3ad358` IL_0268-IL_026f skips a zero-valued entry before the
        // stacking lookup, so the executor never reaches this command with
        // one — see [`misery_snapshot_entry_amount`].
        return Err(EngineRefusal::MalformedArgs("Misery Strength clone amount"));
    }
    // `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_0039-IL_006c leaves on a
    // combat that is ending and on a target that cannot receive powers.
    if damage_combat_is_ending(state) || monster.hp <= 0 {
        return Ok(());
    }
    if applier != crate::hot::Applier::Player
        && (state.fanouts.unsettling_lamp_available()
            || super::play::active_card_lamp_owner_exists(state)
            || super::play::active_card_lamp_triggered() == Some(true))
    {
        return Err(EngineRefusal::MalformedArgs(
            "Misery Strength clone Unsettling Lamp applier",
        ));
    }
    let amount = if amount < 0 {
        let Some(gated) = card_monster_type_two_amount(state, target, amount, events)? else {
            // Artifact zeroed the delta and was consumed; `PowerModel::ApplyInternal`
            // `0x84012` IL_0001-IL_000e then returns before attaching.
            return Ok(());
        };
        gated
    } else {
        amount
    };
    apply_monster_strength_delta_after_type_two_gate(
        state,
        Some(catalog),
        target,
        amount,
        applier,
        events,
    )
}

/// Apply one card-sourced signed Strength delta through the represented
/// AfterPowerAmountChanged continuation.
///
/// Strength is not a Misery debuff token, so it cannot use
/// [`apply_card_monster_debuff`]. Malaise is the exact current-build caller:
/// its negative Strength application is permanent Type-2 state and therefore
/// independently fires Sleight of Flesh before Malaise proceeds to Weak.
pub fn apply_card_monster_strength_delta(
    state: &mut HotState,
    target: usize,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if state.history.over || monster.hp <= 0 {
        return Ok(());
    }
    if amount >= 0 {
        return Err(EngineRefusal::MalformedArgs("card monster Strength debuff"));
    }
    if monster.powers.value(PowerId::Artifact) <= 0
        && state.card_states.dampen().is_some()
        && state.powers.value(PowerId::SleightOfFlesh) > 0
        && !super::play::active_play_transaction_is_running()
    {
        return Err(EngineRefusal::MalformedArgs(
            "Dampen Strength debuff Sleight catalog",
        ));
    }
    let Some(amount) = card_monster_type_two_amount(state, target, amount, events)? else {
        return Ok(());
    };
    // `Malaise/<OnPlay>d__9::MoveNext` `0x3ab428` IL_00fc-IL_0109 passes
    // `card.Owner.Player.Creature` as the applier.
    apply_monster_strength_delta_after_type_two_gate(
        state,
        None,
        target,
        amount,
        crate::hot::Applier::Player,
        events,
    )
}

/// Publish one signed monster-Strength delta and run its represented
/// `AfterPowerAmountChanged` suffix.
///
/// `applier` is what the calling command hands
/// `PowerCmd.Apply<StrengthPower>`; it is recorded on a fresh attach and
/// ignored on a stacking application. See [`write_monster_strength`] for the
/// per-caller IL that decides it.
pub(crate) fn apply_monster_strength_delta_after_type_two_gate(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    amount: i32,
    applier: crate::hot::Applier,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let uid = state.monsters[target].uid;
    let updated = state.monsters[target]
        .powers
        .value(PowerId::Strength)
        .checked_add(amount)
        .ok_or_else(|| overflow("monster strength"))?;
    let upkeep = state.fanouts.misery_attachment_upkeep();
    write_monster_strength(&mut state.monsters_mut()[target], updated, applier, upkeep);
    note_power(events, Subject::Monster(uid), PowerId::Strength, updated);
    // The same applier the command was handed reaches the hook verbatim
    // (`0x3efbac` IL_05d4, `0x3f032c` IL_02e0), so the Sleight of Flesh arm
    // sees Malaise's player, a `Misery` copy's retained identity, or Brimstone's
    // null exactly as native does.
    powers_after_power_amount_changed(
        state,
        catalog,
        target,
        Some(PowerId::Strength),
        amount,
        false,
        applier,
        events,
    )
}

/// Publish one concrete temporary-Strength wrapper application and run its
/// represented `AfterPowerAmountChanged` suffix (#2693 S2).
///
/// [`apply_monster_strength_delta_after_type_two_gate`]'s sibling, and the
/// same contract: the caller has already run the Artifact/Lamp gate
/// (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_0248), so `amount` is the
/// **positive** native `Amount` the wrapper actually attaches with. The two
/// scalars and the wrapper row move together in
/// [`write_monster_temp_strength_wrapper`]; only the nested Strength
/// application notifies, which is why exactly one
/// `powers_after_power_amount_changed` runs here
/// (`<AfterPowerAmountChanged>d__21::MoveNext` `0x348a88` IL_0020-IL_0036
/// returns on the wrapper's own initial application).
pub(crate) fn apply_monster_temp_strength_wrapper_after_type_two_gate(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    model: crate::hot::AttachedPowerModel,
    amount: i32,
    applier: crate::hot::Applier,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // #3342: every card producer and `Misery`'s clone reach the same
    // `PowerCmd/<Apply>d__2` `0x3efbac` fresh-attach order as Monarch's Gaze;
    // see [`temp_strength_wrapper_on_retained_owner`] for the per-producer IL.
    if temp_strength_wrapper_order_is_observable(state, target)
        && temp_strength_wrapper_on_retained_owner(
            state, catalog, target, model, amount, applier, events,
        )?
    {
        return Ok(());
    }
    let uid = state.monsters[target].uid;
    let upkeep = state.fanouts.misery_attachment_upkeep();
    let (temporary, strength) = write_monster_temp_strength_wrapper(
        &mut state.monsters_mut()[target],
        model,
        amount,
        applier,
        upkeep,
    )?;
    note_power(
        events,
        Subject::Monster(uid),
        PowerId::TempStrength,
        temporary,
    );
    note_power(events, Subject::Monster(uid), PowerId::Strength, strength);
    // The wrapper's own Type-2 application is an `ITemporaryPower`, which
    // `0x344dc4` IL_007a-IL_0087 returns on; what notifies here is the nested
    // `Apply<StrengthPower>`, and `<BeforeApplied>d__20::MoveNext` `0x348d20`
    // IL_003e-IL_004b forwards the applier the wrapper application was handed.
    powers_after_power_amount_changed(
        state,
        catalog,
        target,
        Some(PowerId::Strength),
        -amount,
        false,
        applier,
        events,
    )
}

/// Can the order of a temporary-Strength wrapper application's two halves be
/// observed on this target (#3338, generalized by #3342)?
///
/// Only a death during the nested Strength notification can observe it, and
/// only on an owner whose corpse stays in combat. A dying enemy leaves
/// `CombatState.Enemies` unless one of six
/// `ShouldCreatureBeRemovedFromCombatAfterDeath` overrides vetoes it for its
/// own owner (see [`super::monsters::in_native_enemies`]). Four of them can
/// sit on a wrapper target here:
///
/// * `IllusionPower` `0xa3b33` (Parafright, Eye With Teeth) and
///   `SteamEruptionPower` `0xa85fe` (Waterfall Giant), where death cleanup
///   keeps (Illusion) or wipes (Waterfall) what the fresh attach wrote, so the
///   two orders differ;
/// * `AdaptablePower` `0x9f608` and `PainfulStabsPower` `0xa5564` (Test
///   Subject), whose death cleanup wipes what the fresh attach wrote;
/// * `ReattachPower` `0xa665b` (Decimillipede segment). A segment that
///   revives cannot receive the wrapper, but the last segment to fall does
///   not revive, and its corpse can.
///
/// `DieForYouPower` (`0xa1861`) retains only Osty, which is never a wrapper
/// target. The one listener on the notification that can kill is
/// `SleightOfFleshPower` (`<AfterPowerAmountChanged>d__6::MoveNext`
/// `0x344dc4`), so without it every order produces the same state.
#[inline]
fn temp_strength_wrapper_order_is_observable(state: &HotState, target: usize) -> bool {
    matches!(
        state.monsters[target].kind,
        MonsterKind::WaterfallGiant
            | MonsterKind::Parafright
            | MonsterKind::EyeWithTeeth
            | MonsterKind::TestSubject
            | MonsterKind::DecimillipedeSegment
    ) && state
        .fanouts
        .after_power_amount_changed_order()
        .contains(&PowerId::SleightOfFlesh)
}

/// One temporary-Strength wrapper application on a retained-death owner with
/// Sleight of Flesh live, in native order (#3338 for Monarch's Gaze, #3342 for
/// every other producer).
///
/// Returns `Ok(false)` when the application is a known **restack**, and
/// leaves the state untouched. The caller then runs its own shared-writer
/// path, whose order is native for a restack (below). `Ok(true)` means this
/// wrote the whole application.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// Every wrapper is a `TemporaryStrengthPower` subclass that overrides none of
/// its hooks: `CrushUnderPower`, `DarkShacklesPower`, `DyingStarPower`,
/// `EnfeeblingTouchPower`, `ManglePower`, `MonarchsGazeStrengthDownPower`,
/// `PiercingWailPower` and `ShacklingPotionPower` each define only
/// `get_OriginModel`, `get_IsPositive` and `.ctor`. Every producer reaches the
/// same command:
///
/// * the cards and the potion call `PowerCmd.Apply<T>`, either the
///   single-target `0x1337f0` (`<Apply>d__1`1::MoveNext` `0x3ef988`) or the
///   all-target `0x133780`, whose `<Apply>d__0`1::MoveNext` `0x3ef7dc`
///   IL_007a calls `0x1337f0` per target. Dark Shackles `0x396954` IL_00e6,
///   Dying Star `0x39ac44` IL_01af, Enfeebling Touch `0x39bb64` IL_00e6,
///   Mangle `0x3ab670` IL_013c and Piercing Wail `0x3b253c` IL_00f3 use the
///   single-target overload; Crush Under `0x395cac` IL_01d5 and Shackling
///   Potion `0x34ffb8` IL_00ae the all-target one. `0x3ef988` restacks
///   through `FindExistingInstanceForStacking` IL_006c and `ModifyAmount`
///   IL_013b, and otherwise creates the model (`ToMutable` IL_0084) and calls
///   the concrete `PowerCmd::Apply` `0x133860` (IL_00b8), which is
///   `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`;
/// * `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` restacks through
///   `FindExistingInstanceForStacking` IL_028d and `ModifyAmount` IL_02bd,
///   and otherwise applies `ClonePreservingMutability` (IL_0327) through the
///   concrete `Apply` at IL_035b, again `0x3efbac`;
/// * `MonarchsGazePower/<AfterDamageGiven>d__4::MoveNext` `0x33e544` is the
///   same single-target `Apply<T>`.
///
/// So each has two orders:
///
/// * **Restack.** `ModifyAmount` raises the one instance first. The
///   wrapper's own `<AfterPowerAmountChanged>d__21::MoveNext` `0x348a88` then
///   applies the Strength (IL_004b-IL_007a). Its Sleight kill therefore sees
///   the raised wrapper. That is [`write_monster_temp_strength_wrapper`]'s
///   order, so the restack keeps the shared writer.
/// * **Fresh attach.** `0x3efbac` IL_02d9 calls `BeforeApplied`, which is
///   `<BeforeApplied>d__20::MoveNext` `0x348d20` IL_001d-IL_004b, the nested
///   `Apply<StrengthPower>(target, Sign * amount, applier, cardSource)`. Its
///   Sleight kill runs to completion before the command re-tests
///   `Creature::get_CanReceivePowers` (IL_0336-IL_0343) and attaches through
///   `ApplyInternal` (IL_0360). `CanReceivePowers` `0x11d2b5` is
///   `CombatState != null && Hook.ShouldAllowHitting`, and it does not test
///   liveness. After a nested kill:
///   - a Parafright or Eye With Teeth corpse is reviving
///     (`IllusionPower/<AfterDeath>d__18::MoveNext` `0x33ca54` IL_00a7-IL_00ae
///     sets `isReviving`), and `IllusionPower::ShouldAllowHitting` `0xa3b1b`
///     vetoes its owner while reviving. The wrapper never attaches. The Strength
///     it applied is Type 1 (`StrengthPower::get_Type` `0xa8943`), so
///     `IllusionPower::ShouldPowerBeRemovedOnDeath` `0xa3a80` IL_0017 keeps
///     it. No side-end restoration follows.
///   - a Test Subject that still holds Adaptable is reviving the same way
///     (`AdaptablePower/<AfterDeath>d__9::MoveNext` `0x334990` IL_0053-IL_005a
///     sets `isReviving`; `KillWithoutCheckingWinCondition` `0x3ebe90` IL_03b9
///     passes `wasRemovalPrevented = false`, so IL_001d-IL_0025 does not
///     leave), and `AdaptablePower::ShouldAllowHitting` `0x9f5ef` vetoes its
///     reviving owner. The wrapper never attaches, and the Test Subject death
///     cleanup has already wiped the nested Strength.
///   - otherwise a Waterfall Giant keeps its creature (`SteamEruptionPower::
///     ShouldCreatureBeRemovedFromCombatAfterDeath` `0xa85fe`). Its
///     `<AfterDeath>d__4::MoveNext` `0x34618c` IL_004d calls
///     `TriggerAboutToBlowState`, whose `<TriggerAboutToBlowState>d__77::
///     MoveNext` `0x3762a8` IL_0039-IL_0043 sets its HP to 999999999. The
///     sentinel is therefore alive, and even `DieForYouPower::ShouldAllowHitting`
///     `0xa1859` (`creature.IsAlive`, `Creature::get_IsAlive` `0x11d0fb`,
///     which Osty's instance applies to every creature) lets it be hit. The wrapper
///     attaches to the ABOUT sentinel whose death cleanup already removed
///     that Strength. Its side-end unwind (`<AfterSideTurnEnd>d__22::MoveNext`
///     `0x348ba8` IL_009d-IL_00c4) then *adds* the amount as Strength.
///   - a Test Subject in its final form carries neither veto, so it leaves
///     combat detached: `KillWithoutCheckingWinCondition` `0x3ebe90`
///     IL_04c8-IL_04d5 calls `CombatState::RemoveCreature(creature, true)`
///     (the creature is not performing a move, since every producer here is
///     player-sourced), and `0x1371d0` IL_009a-IL_009f sets its
///     `CombatState` to null. `CanReceivePowers` fails, and the Test Subject
///     death cleanup has already wiped the nested Strength.
///   - a Decimillipede segment that will reattach is reviving
///     (`ReattachPower/<AfterDeath>d__11::MoveNext` `0x341d30` IL_005a-IL_008c
///     sets `isReviving` unless IL_0040-IL_0056 finds every other segment dead),
///     and `ReattachPower::ShouldAllowHitting` `0xa6643` vetoes it. The
///     wrapper never attaches, and the Rust segment death has already wiped
///     the nested Strength.
///   - a dead corpse with no veto of its own gets the wrapper, unless Osty's
///     Die For You (`0xa1859`) rejects every dead creature. This applies to
///     a Waterfall that did not become the sentinel, a Test Subject kept only
///     by Painful Stabs, and the last Decimillipede segment (kept by
///     Reattach, not reviving). Rust's death cleanup for those corpses does
///     not settle the nested Strength the way native `RemoveAllPowersAfterDeath`
///     does, so neither outcome is represented, and this refuses.
///
/// Existence is known when no wrapper is attached at all
/// (`TempStrength == 0`) or when the attachment ledger is recorded. Without
/// the ledger, an owner with other wrappers may or may not carry this model.
/// Both orders give the same state unless the target dies, so this refuses
/// only an unknown existence that meets a nested death. It also refuses a
/// Waterfall death that did not become the sentinel.
fn temp_strength_wrapper_on_retained_owner(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    model: crate::hot::AttachedPowerModel,
    amount: i32,
    applier: crate::hot::Applier,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    // #3338 named Monarch's refusal first; #3342 adds the other producers
    // under their own name so the census can tell them apart.
    let refusal = EngineRefusal::MalformedArgs(match model {
        crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown => {
            "Monarch Sleight retained temporary wrapper"
        }
        _ => "Sleight retained temporary wrapper",
    });
    if amount <= 0 {
        // The shared writer's own contract (`ShouldRemoveDueToAmount`
        // `0x83b0d` IL_0001-IL_0010, `ApplyInternal` `0x84012` IL_0001-IL_000e).
        return Err(EngineRefusal::MalformedArgs(
            "temporary monster Strength wrapper amount",
        ));
    }
    let upkeep = state.fanouts.misery_attachment_upkeep();
    let monster = &state.monsters[target];
    let (uid, kind) = (monster.uid, monster.kind);
    let restack = if monster.powers.value(PowerId::TempStrength) == 0 {
        Some(false)
    } else if upkeep && super::monsters::temp_strength_provenance_is_recorded(monster) {
        Some(super::monsters::temp_strength_wrapper_attachment(monster, model).is_some())
    } else {
        None
    };
    if restack == Some(true) {
        return Ok(false);
    }
    // `BeforeApplied`'s nested Strength, with the wrapper's applier
    // (`0x348d20` IL_003e-IL_004b).
    let strength = state.monsters[target]
        .powers
        .value(PowerId::Strength)
        .checked_sub(amount)
        .ok_or_else(|| overflow("monster strength"))?;
    write_monster_strength(&mut state.monsters_mut()[target], strength, applier, upkeep);
    note_power(events, Subject::Monster(uid), PowerId::Strength, strength);
    let mark = events.len();
    powers_after_power_amount_changed(
        state,
        catalog,
        target,
        Some(PowerId::Strength),
        -amount,
        false,
        applier,
        events,
    )?;
    let died = events[mark..]
        .iter()
        .any(|event| matches!(event, Event::MonsterDied { uid: dead } if *dead == uid));
    // None of these kinds is replaced in its slot (only Stock does that).
    debug_assert_eq!(state.monsters[target].uid, uid);
    if died {
        if restack.is_none() {
            return Err(refusal);
        }
        let corpse = &state.monsters[target];
        match kind {
            // Reviving: `CanReceivePowers` is false (`0xa3b1b`).
            MonsterKind::Parafright | MonsterKind::EyeWithTeeth => return Ok(true),
            // Reviving: Adaptable vetoes hitting its owner (`0x9f5ef`).
            MonsterKind::TestSubject if corpse.powers.value(PowerId::Adaptable) > 0 => {
                return Ok(true);
            }
            // The final form keeps no veto, so it is removed and detached
            // (`0x3ebe90` IL_04c8-IL_04d5, `0x1371d0` IL_009a-IL_009f).
            MonsterKind::TestSubject if corpse.powers.value(PowerId::PainfulStabs) == 0 => {
                return Ok(true);
            }
            // Reviving: Reattach vetoes hitting its owner (`0xa6643`).
            MonsterKind::DecimillipedeSegment
                if corpse.revive_stage == super::monsters::SEGMENT_REVIVE_DEAD =>
            {
                return Ok(true);
            }
            MonsterKind::WaterfallGiant if corpse.is_about_to_blow() => {}
            _ => return Err(refusal),
        }
    }
    // `ApplyInternal` (`0x3efbac` IL_0360): the wrapper alone, at the end of
    // the ledger. Its provenance is judged as it stands now, after the nested
    // Strength and any death cleanup.
    let monster = &mut state.monsters_mut()[target];
    let recorded = super::monsters::temp_strength_provenance_is_recorded(monster);
    let temp = monster
        .powers
        .value(PowerId::TempStrength)
        .checked_sub(amount)
        .ok_or_else(|| overflow("temporary monster strength"))?;
    monster
        .powers
        .set(PowerId::TempStrength, SlotWire::Int, temp);
    if upkeep {
        if restack == Some(false) && recorded && amount <= 999_999_999 {
            monster
                .misery_debuff_order
                .push_attachment(crate::hot::AttachmentRecord {
                    power: model,
                    applier,
                    amount,
                });
        } else {
            abandon_monster_temp_strength_provenance(monster);
        }
    }
    note_power(events, Subject::Monster(uid), PowerId::TempStrength, temp);
    Ok(true)
}

/// Apply one `Misery` copy of a frozen temporary-Strength wrapper to a
/// recipient (#2693 S3).
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`. The
/// copy is an ordinary `PowerCmd::Apply`/`ModifyAmount` of the cloned wrapper
/// instance (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_0274-IL_035b), so
/// **on the recipient it is a real wrapper**, not an amount of temporary
/// Strength:
///
/// * its own `BeforeApplied` (`<BeforeApplied>d__20::MoveNext` `0x348d20`
///   IL_001d-IL_004b) applies `Sign * amount` `StrengthPower` to the
///   *recipient*, with the applier the copy was handed and the card as
///   source — which is the clone's retained `_applier`
///   (`PowerModel::AfterCloned` `0x84093` leaves `_applier` alone) and is the
///   player at all eight native writers;
/// * it stacks onto an existing instance **of the same model** rather than
///   attaching a second (`get_InstanceType` `0x83751` = 0,
///   `FindExistingInstanceForStacking` `0x1338d8` IL_0058), and the restack's
///   nested Strength comes from `<AfterPowerAmountChanged>d__21::MoveNext`
///   `0x348a88` IL_004b-IL_007a;
/// * it takes part in the recipient's own side-end unwind from then on
///   (`<AfterSideTurnEnd>d__22::MoveNext` `0x348ba8`), with the **recipient**
///   as the restoring applier.
///
/// All three are [`write_monster_temp_strength_wrapper`]'s and
/// [`unwind_monster_temp_strength_wrappers`]'s contracts already, which is
/// why this routes through them rather than writing either scalar itself.
///
/// **Artifact blocks the wrapper and is consumed.** The family is Type 2 at
/// every amount ([`crate::hot::AttachedPowerModel::is_temporary_strength_wrapper`]),
/// so `ArtifactPower::TryModifyPowerAmountReceived` `0x9f844` zeroes the
/// delta and reports consumed. That gate is `PowerCmd/<Apply>d__2::MoveNext`
/// `0x3efbac` IL_0248, which precedes `BeforeApplied` at IL_02d9, so a
/// blocked wrapper's nested `Apply<StrengthPower>(Sign * 0)` returns at
/// IL_0050-IL_005c before its own Artifact gate and `ApplyInternal`
/// `0x84012` IL_0001-IL_000e declines to attach: exactly **one** Artifact per
/// application, and "wrapper blocked but its Strength not" is unreachable.
///
/// **A reachable Unsettling Lamp refuses**, and this one is about ORDER
/// rather than the applier. `UnsettlingLamp::ModifyPowerAmountGivenMultiplicative`
/// `0x9d5a8` IL_0038-IL_003f declines to double a power when
/// `HasDoubledTemporaryPowerSource` `0x9d66c` holds — i.e. when the Lamp has
/// already doubled an `ITemporaryPower` whose `InternallyAppliedPower` is of
/// the same type (`<HasDoubledTemporaryPowerSource>b__0` `0x333a9e`). So a
/// Lamp that latches on a *wrapper* copy must not double a `StrengthPower`
/// copy later in the same play, while one that latches on a scalar debuff
/// must. `card_monster_type_two_amount` models the Lamp as a latched flag
/// with no notion of which model it latched on, and both ledger orders are
/// reachable, so the distinction is not representable here. Refusing is the
/// I5 answer; the root-level rule that keeps it out of an admitted root is in
/// [`crate::engine::admission`].
pub(crate) fn apply_card_monster_temp_strength_wrapper_clone(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    model: crate::hot::AttachedPowerModel,
    applier: crate::hot::Applier,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !model.is_temporary_strength_wrapper() {
        return Err(EngineRefusal::MalformedArgs(
            "Misery temporary Strength clone model",
        ));
    }
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if amount <= 0 {
        // A wrapper's stored `Amount` is positive and `ShouldRemoveDueToAmount`
        // `0x83b0d` IL_0001-IL_0010 removes it at anything else, so a
        // non-positive frozen value is not a native state.
        return Err(EngineRefusal::MalformedArgs(
            "Misery temporary Strength clone amount",
        ));
    }
    // `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_0039-IL_006c leaves on a
    // combat that is ending and on a target that cannot receive powers.
    if damage_combat_is_ending(state) || monster.hp <= 0 {
        return Ok(());
    }
    if state.fanouts.unsettling_lamp_available()
        || super::play::active_card_lamp_owner_exists(state)
        || super::play::active_card_lamp_triggered() == Some(true)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Misery temporary Strength clone Unsettling Lamp order",
        ));
    }
    let Some(amount) = card_monster_type_two_amount(state, target, amount, events)? else {
        return Ok(());
    };
    apply_monster_temp_strength_wrapper_after_type_two_gate(
        state,
        Some(catalog),
        target,
        model,
        amount,
        applier,
        events,
    )
}

/// Apply a player-power-sourced debuff. Relic-only amount modifiers do not
/// participate, but the Artifact gate, Poison instance identity, Misery
/// acquisition order, and visible power event are the same shared command.
pub(crate) fn apply_power_monster_debuff(
    state: &mut HotState,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if power == PowerId::Vuln && vicious_listener_is_reachable(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Vicious Vulnerable producer catalog",
        ));
    }
    apply_player_monster_debuff(
        state,
        PlayerMonsterDebuffSource::Power(None),
        target,
        power,
        token,
        amount,
        events,
    )
}

/// Apply a **relic-sourced** debuff to one monster.
///
/// The one current family is the turn-1 all-enemy debuff relics
/// (`engine::relics::turn_one_all_enemy_debuffs`). Each of their bodies calls
/// `PowerCmd.Apply<T>` with a **null card** and the relic
/// `Owner.get_Creature` as applier —
/// `RedMask/<BeforeSideTurnStart>d__6::MoveNext` `0x32f4b8` IL_0081-IL_008d
/// pushes `Owner.Creature`, `ldnull`, `ldc.i4.0` and then
/// `spec:Apply<WeakPower>` at IL_008e;
/// `BagOfMarbles/<BeforeSideTurnStart>d__6::MoveNext` `0x31ee48` does the same
/// at IL_007c-IL_0089. A null card means no `CardPlay` is active, so the
/// card-sourced Unsettling Lamp term cannot participate; the applier being the
/// owner's Creature is `crate::hot::Applier::Player`, which is what the shared
/// seam records.
///
/// Everything else — the `ArtifactPower` received-side gate, the Misery
/// acquisition-order token, Poison's instance identity and the visible power
/// event — is the same shared command as the card and power sources, which is
/// the point of routing through it rather than writing the scalar directly.
pub(crate) fn apply_relic_monster_debuff(
    state: &mut HotState,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if power == PowerId::Vuln && vicious_listener_is_reachable(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Vicious Vulnerable producer catalog",
        ));
    }
    apply_player_monster_debuff(
        state,
        PlayerMonsterDebuffSource::Relic,
        target,
        power,
        token,
        amount,
        events,
    )
}

/// Catalog-aware player-power-sourced debuff seam for Vicious-observable
/// Vulnerable. Expose is the complete current caller census.
pub(crate) fn apply_power_monster_debuff_with_catalog(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if power == PowerId::Vuln && vicious_listener_is_reachable(state) {
        if !player_type_one_listener_order_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs(
                "after-power-amount-changed listener order",
            ));
        }
        super::play::rehearse_preserving_active_plays(|| {
            let mut probe = state.clone();
            apply_player_monster_debuff(
                &mut probe,
                PlayerMonsterDebuffSource::Power(Some(catalog)),
                target,
                power,
                token,
                amount,
                &mut Vec::new(),
            )
        })?;
    }
    apply_player_monster_debuff(
        state,
        PlayerMonsterDebuffSource::Power(Some(catalog)),
        target,
        power,
        token,
        amount,
        events,
    )
}

/// Apply one duration-bearing affliction to the player.
///
/// The amount and its `SkipNextDurationTick` latch are distinct canonical
/// slots so stacking preserves the already-live duration without teaching
/// the generic slot table about power-specific lifecycle rules.
///
/// The latch is written only when the power was ABSENT (#3028). v0.111.0
/// `PowerCmd.<Apply>d__2::MoveNext` (RVA 0x3efbac) calls
/// `FindExistingInstanceForStacking` at IL_00a3; an existing instance takes
/// `ModifyAmount` (IL_00c6) and leaves through IL_0121, never reaching the
/// marker. Only the new-instance path reaches IL_0429-044c, which sets
/// `SkipNextDurationTick = true` when `target.Side == 1` (Player, IL_0434)
/// and `power.Type == 2` (Debuff, IL_0442). All four pairs here are
/// player-side and Debuff-typed (`VulnerablePower`/`WeakPower`/`FrailPower`/
/// `SmoggyPower::get_Type` each return 2), so absence is the whole native
/// condition. Stacking keeps whatever latch the live instance already
/// carries. The latch is consumed by `PowerCmd.<TickDownDuration>d__5::
/// MoveNext` (RVA 0x3f0b18, IL_001c-002a) instead of one decrement.
///
/// Every enemy writer passes `mark_fresh = true` and lets this helper apply
/// the absence gate, Terror Eel's TERROR included (#3158: `<TerrorMove>d__30`
/// RVA 0x36d22c IL_018b-01a5 is a plain `PowerCmd.Apply<VulnerablePower>`).
/// `mark_fresh = false` suppresses the latch for a caller whose native write
/// does not go through the new-instance path.
///
/// Smoggy's latch is written natively too, but `SmoggyPower` never ticks
/// (#3161, `SmoggyPower::AfterSideTurnEnd` RVA 0xa804c has no
/// `TickDownDuration`), so the side-end loop never consumes it.
///
/// The represented `AfterPowerAmountChanged` listeners are all inert for
/// these enemy/player-owned writes: Vicious needs player-applied Vulnerable,
/// Shroud needs Doom, and Sleight of Flesh needs a monster-owned permanent
/// Type-2 change. Inky is the one player-applied Weak source and remains an
/// explicitly refused per-card enchantment; when that source lands it must
/// call this seam with `mark_fresh = false`.
pub(crate) fn apply_player_duration_affliction(
    state: &mut HotState,
    power: PowerId,
    fresh: PowerId,
    amount: i32,
    mark_fresh: bool,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let valid_pair = matches!(
        (power, fresh),
        (PowerId::PlayerVuln, PowerId::PlayerVulnFresh)
            | (PowerId::PlayerWeak, PowerId::PlayerWeakFresh)
            | (PowerId::PlayerFrail, PowerId::PlayerFrailFresh)
            | (PowerId::Smoggy, PowerId::SmoggyFresh)
    );
    if !valid_pair {
        return Err(EngineRefusal::MalformedArgs(
            "player duration affliction pair",
        ));
    }
    if state.history.over || amount == 0 {
        return Ok(());
    }
    let current = state.powers.value(power);
    // IL_00a3: an existing instance stacks through ModifyAmount and never
    // reaches the new-instance SkipNextDurationTick write at IL_044c.
    let new_instance = current <= 0;
    let updated = current
        .checked_add(amount)
        .ok_or_else(|| overflow("player duration affliction"))?;
    super::play::prepare_after_card_played_scalar_write(state, power, current, updated)?;
    state.powers.set(power, SlotWire::Int, updated);
    note_power(events, Subject::Player, power, updated);
    if mark_fresh && new_instance {
        state.powers.set(fresh, SlotWire::Bool, 1);
        note_power(events, Subject::Player, fresh, 1);
    }
    Ok(())
}

#[derive(Copy, Clone)]
enum PlayerMonsterDebuffSource<'a> {
    Card(Option<&'a Catalog>),
    Power(Option<&'a Catalog>),
    // A relic body's own `PowerCmd.Apply`, with a null card and the owner's
    // Creature as applier. Distinct from `Power` only as vocabulary: relic
    // bodies are not card-sourced, so no Unsettling Lamp term participates.
    Relic,
    // Only Damage's frozen result can authorize a still-attached lethal target.
    PowerBeforeKill(Option<&'a Catalog>, u32),
}

impl<'a> PlayerMonsterDebuffSource<'a> {
    fn catalog(self) -> Option<&'a Catalog> {
        match self {
            Self::Card(catalog) | Self::Power(catalog) | Self::PowerBeforeKill(catalog, _) => {
                catalog
            }
            Self::Relic => None,
        }
    }

    fn card_sourced(self) -> bool {
        matches!(self, Self::Card(_))
    }
}

fn apply_player_monster_debuff(
    state: &mut HotState,
    source: PlayerMonsterDebuffSource<'_>,
    target: usize,
    power: PowerId,
    token: MiseryToken,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    let before_kill = matches!(source,
        PlayerMonsterDebuffSource::PowerBeforeKill(_, uid)
            if uid == monster.uid && !state.fanouts.pet().has_die_for_you());
    // PowerCmd.Apply gates native IsOverOrEnding before Artifact and amount
    // listeners. An outer Damage batch may have committed the last primary
    // enemy already, while its deferred Kill list is still being drained.
    if damage_combat_is_ending(state) || (monster.hp <= 0 && !before_kill) || amount == 0 {
        return Ok(());
    }
    let amount = if power == PowerId::Poison && amount > 0 && state.snecko_skull_owned() {
        crate::coverage::record_relic(RelicId::RelicSneckoSkull);
        amount
            .checked_add(1)
            .ok_or_else(|| overflow("Snecko Skull poison"))?
    } else {
        amount
    };
    let Some(amount) =
        player_monster_debuff_amount(state, target, amount, source.card_sourced(), events)?
    else {
        return Ok(());
    };
    let fresh_poison = {
        let monster = &state.monsters[target];
        power == PowerId::Poison
            && monster.powers.value(power) <= 0
            && monster
                .powers
                .value(power)
                .checked_add(amount)
                .is_some_and(|updated| updated > 0)
    };
    let was_active = state.monsters[target].powers.value(power) > 0;
    let updated = state.monsters[target]
        .powers
        .value(power)
        .checked_add(amount)
        .ok_or_else(|| overflow("monster debuff"))?;
    let allocated = if fresh_poison {
        let uid = state.next_poison_uid;
        state.next_poison_uid = state
            .next_poison_uid
            .checked_add(1)
            .ok_or_else(|| overflow("next_poison_uid"))?;
        Some(uid)
    } else {
        None
    };
    let monster = &mut state.monsters_mut()[target];
    monster.powers.set(power, SlotWire::Int, updated);
    if power == PowerId::Demise && !was_active {
        // A newly-created singleton appends after the already-live
        // Intangible and Ritual listeners. Intensity re-stacks preserve both
        // relative-order bits; a future Ritual application flips its own bit.
        monster.set_demise_after_intangible(monster.powers.value(PowerId::Intangible) > 0);
        monster.set_demise_before_ritual(false);
    }
    if updated > 0 {
        record_misery_debuff_application(monster, power, token, !was_active);
    }
    if let Some(uid) = allocated {
        monster.poison_uid = uid;
    }
    let updated = monster.powers.value(power);
    let uid = monster.uid;
    if power == PowerId::Doom && amount != 0 {
        state.history.doom_applied_by_player_this_turn = true;
    }
    note_power(events, Subject::Monster(uid), power, updated);
    // Every source this seam takes is the local player: a card passes
    // `card.Owner.Player.Creature` (`Malaise/<OnPlay>d__9::MoveNext`
    // `0x3ab428` IL_00fc-IL_0109 and its siblings), a player-owned power
    // passes its own `PowerModel::get_Owner`
    // (`EnvenomPower/<AfterDamageGiven>d__6::MoveNext` `0x33a04c`
    // IL_006f-IL_0077), and a relic body passes `RelicModel::get_Owner`'s
    // `Player::get_Creature` (`RedMask/<BeforeSideTurnStart>d__6::MoveNext`
    // `0x32f4b8` IL_0081-IL_0087).
    // `PlayerMonsterDebuffSource` is the complete caller vocabulary.
    powers_after_power_amount_changed(
        state,
        source.catalog(),
        target,
        Some(power),
        amount,
        false,
        crate::hot::Applier::Player,
        events,
    )?;
    Ok(())
}

/// `UnsettlingLamp.BeforePowerAmountChanged` (`0x9d4fc`) observes Artifact
/// before its amount listener (`0x9d5a8`); the received-side Artifact gate is
/// `ArtifactPower.TryModifyPowerAmountReceived` (`0x9f844`). A fresh blocked
/// Type-2 application therefore consumes one Artifact and cannot latch Lamp.
/// The Lamp's card identity is the innermost native CardPlay represented by
/// `engine::play::ACTIVE_PLAYS`; absence of that identity refuses rather than
/// doubling an unrelated nested application.
fn player_monster_debuff_amount(
    state: &mut HotState,
    target: usize,
    amount: i32,
    card_sourced: bool,
    events: &mut Vec<Event>,
) -> Result<Option<i32>, EngineRefusal> {
    if card_sourced {
        return card_monster_type_two_amount(state, target, amount, events);
    }
    let artifact = state.monsters[target].powers.value(PowerId::Artifact);
    if artifact > 0 {
        let uid = state.monsters[target].uid;
        state.monsters_mut()[target]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, artifact - 1);
        note_power(
            events,
            Subject::Monster(uid),
            PowerId::Artifact,
            artifact - 1,
        );
        return Ok(None);
    }
    Ok(Some(amount))
}

/// Apply Shackling Potion's null-card temporary Strength wrapper.
///
/// Current v0.111.0 authority (`sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
/// `ShacklingPotion/<OnUse>d__10::MoveNext` RVA `0x34ffb8` applies
/// `ShacklingPotionPower(7)` to one frozen hittable-enemy snapshot. The
/// wrapper is Type 2, so Artifact consumes the whole application; otherwise
/// it applies Strength -7 and retains TempStrength -7 for the native
/// side-end restoration.
pub(crate) fn apply_potion_temporary_strength(
    state: &mut HotState,
    target: usize,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over
        || state
            .monsters
            .get(target)
            .is_none_or(|monster| monster.hp <= 0)
    {
        return Ok(());
    }
    let Some(delta) = player_monster_debuff_amount(state, target, -amount, false, events)? else {
        return Ok(());
    };
    let uid = state.monsters[target].uid;
    let applied = delta
        .checked_neg()
        .ok_or_else(|| overflow("temporary monster strength"))?;
    // #3342: `0x34ffb8` IL_00ae's all-target `Apply<ShacklingPotionPower>`
    // reaches `PowerCmd/<Apply>d__2` `0x3efbac` per target, so a fresh attach
    // lands its nested Strength (and any Sleight kill) before the wrapper.
    if temp_strength_wrapper_order_is_observable(state, target)
        && temp_strength_wrapper_on_retained_owner(
            state,
            None,
            target,
            crate::hot::AttachedPowerModel::ShacklingPotion,
            applied,
            crate::hot::Applier::Player,
            events,
        )?
    {
        return Ok(());
    }
    let upkeep = state.fanouts.misery_attachment_upkeep();
    let monster = &mut state.monsters_mut()[target];
    // The potion's temporary wrapper forwards its own applier to the nested
    // `Apply<StrengthPower>` (`0x348d20` IL_003e-IL_004b), and
    // `ShacklingPotion/<OnUse>d__10::MoveNext` `0x34ffb8` IL_00a1-IL_00a7
    // passes `potion.Owner.Player.Creature`.
    let (temporary, strength) = write_monster_temp_strength_wrapper(
        monster,
        crate::hot::AttachedPowerModel::ShacklingPotion,
        applied,
        crate::hot::Applier::Player,
        upkeep,
    )?;
    note_power(events, Subject::Monster(uid), PowerId::Strength, strength);
    note_power(
        events,
        Subject::Monster(uid),
        PowerId::TempStrength,
        temporary,
    );
    powers_after_power_amount_changed(
        state,
        None,
        target,
        Some(PowerId::Strength),
        delta,
        false,
        // `ShacklingPotion/<OnUse>d__10::MoveNext` `0x34ffb8` IL_00a1-IL_00a7
        // passes `potion.Owner.Player.Creature`, which `0x348d20`
        // IL_003e-IL_004b forwards to the nested Strength application.
        crate::hot::Applier::Player,
        events,
    )
}

/// Card-sourced Type-2 given/received listener walk shared by scalar debuffs,
/// signed Strength, and temporary-Strength wrappers.
pub(crate) fn card_monster_type_two_amount(
    state: &mut HotState,
    target: usize,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<Option<i32>, EngineRefusal> {
    let latched = super::play::active_card_lamp_triggered();
    let any_lamp_owner = super::play::active_card_lamp_owner_exists(state);
    let amount = if latched == Some(true) {
        amount
            .checked_mul(2)
            .ok_or_else(|| overflow("Unsettling Lamp power amount"))?
    } else {
        amount
    };
    let artifact = state.monsters[target].powers.value(PowerId::Artifact);
    if artifact > 0 {
        let uid = state.monsters[target].uid;
        state.monsters_mut()[target]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, artifact - 1);
        note_power(
            events,
            Subject::Monster(uid),
            PowerId::Artifact,
            artifact - 1,
        );
        return Ok(None);
    }
    if latched != Some(true) && !any_lamp_owner && state.fanouts.unsettling_lamp_available() {
        if latched.is_none() {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        super::play::trigger_active_card_lamp()?;
        return amount
            .checked_mul(2)
            .map(Some)
            .ok_or_else(|| overflow("Unsettling Lamp power amount"));
    }
    Ok(Some(amount))
}

/// [`card_monster_type_two_amount`] for a **monster** applier (#3404).
///
/// Same command, two differences, both cited on
/// [`apply_card_monster_imbalanced_clone`]: the Lamp's given-side doubling is
/// additionally gated on `ContainsCreature(applier)` (`applier_in_combat`),
/// and the Lamp can never latch here, because its
/// `BeforePowerAmountChanged` (`0x9d4fc` IL_0032-IL_003f) requires the player
/// as applier. The doubled amount then meets the recipient's Artifact exactly
/// as the player-applied path's does.
fn card_monster_applied_type_two_amount(
    state: &mut HotState,
    target: usize,
    amount: i32,
    applier_in_combat: bool,
    events: &mut Vec<Event>,
) -> Result<Option<i32>, EngineRefusal> {
    let latched = super::play::active_card_lamp_triggered();
    if latched.is_none()
        && (state.fanouts.unsettling_lamp_available()
            || super::play::active_card_lamp_owner_exists(state))
    {
        // No active CardPlay to compare with the Lamp's `TriggeringCard`.
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let amount = if applier_in_combat && latched == Some(true) {
        amount
            .checked_mul(2)
            .ok_or_else(|| overflow("Unsettling Lamp power amount"))?
    } else {
        amount
    };
    let artifact = state.monsters[target].powers.value(PowerId::Artifact);
    if artifact > 0 {
        let uid = state.monsters[target].uid;
        state.monsters_mut()[target]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, artifact - 1);
        note_power(
            events,
            Subject::Monster(uid),
            PowerId::Artifact,
            artifact - 1,
        );
        return Ok(None);
    }
    Ok(Some(amount))
}

/// The represented AfterPowerAmountChanged suffix for one permanent Type-2
/// monster power amount change.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// **`applier` is the hook's own argument, not the power's stored `_applier`**
/// (#2727). `Hook::AfterPowerAmountChanged` `0x1045ec` has exactly two callers
/// in the assembly (`scan_calls.py AfterPowerAmountChanged`), and each passes
/// the applier its own command was handed verbatim:
/// `PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_05d4-IL_05df and
/// `PowerCmd/<ModifyAmount>d__6::MoveNext` `0x3f032c` IL_02e0-IL_02eb.
/// `Hook/<AfterPowerAmountChanged>d__64::MoveNext` `0x3d06d0` IL_005f then
/// forwards it unchanged to every listener. The two therefore coincide for an
/// ordinary application — `set_Applier` `0x3efbac` IL_013d writes the same
/// value on a fresh attach — and part ways for a `Misery` copy, which hands
/// each command the *source instance's* retained `_applier`
/// (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_0288/IL_02b6 for the
/// stacking lookup and `ModifyAmount`, IL_0354 for `Apply`).
///
/// **All three live listeners require the player as applier.** Each is a
/// player-owned power and each tests `applier == this.Owner` by reference:
/// `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext` `0x344dc4`
/// IL_0067-IL_0075, `ViciousPower/<AfterPowerAmountChanged>d__6::MoveNext`
/// `0x34a6f8` IL_0037-IL_0045 and
/// `ShroudPower/<AfterPowerAmountChanged>d__6::MoveNext` `0x3446a4`
/// IL_001d-IL_002b. So a monster-, relic- or unknown-applied change walks the
/// frozen acquisition order and every represented callback is inert.
/// `SwordSagePower::AfterPowerAmountChanged` `0xa902c` reads no applier at
/// all; it is inert here for its own reason (IL_000c-IL_0019 requires the
/// changed power to be a `SwordSagePower`).
///
/// **`SandpitPower` is deliberately not in the order** (#2733). Its
/// `<AfterPowerAmountChanged>d__13::MoveNext` RVA `0x343310` returns under
/// `TestMode.IsOn` (IL_001d-IL_0024) and for any power other than itself
/// (IL_0029-IL_0032). For its own amount it awaits `UpdateCreaturePositions`
/// (IL_0038; `<UpdateCreaturePositions>d__18` only tweens node positions) and,
/// when the Target is the local player (IL_0092-IL_009d), sets The
/// Insatiable's music parameter (IL_009f-IL_00c1). It writes no combat state
/// for any argument, so there is no gameplay walk to represent.
// Spelled out rather than bundled: these are the native hook's own arguments
// (`Hook::AfterPowerAmountChanged` `0x1045ec` takes combat state, choice
// context, power, amount, applier and card source), and a struct would put a
// name of ours between the call sites and the IL they mirror.
#[allow(clippy::too_many_arguments)]
fn powers_after_power_amount_changed(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    target: usize,
    changed_power: Option<PowerId>,
    amount: i32,
    temporary: bool,
    applier: crate::hot::Applier,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount == 0 || temporary || state.history.over {
        return Ok(());
    }
    // Only a player applier can reach the awaited Draw: Vicious returns at
    // `0x34a6f8` IL_0045 otherwise, so the native enumerator completes
    // synchronously and there is no continuation to freeze.
    if changed_power == Some(PowerId::Vuln)
        && amount > 0
        && applier == crate::hot::Applier::Player
        && catalog.is_some_and(|catalog| {
            super::play::cardplay_vulnerable_step_can_suspend(state, catalog)
        })
    {
        let catalog = catalog.expect("checked above");
        return start_after_power_amount_changed_vulnerable(state, catalog, target, amount, events);
    }
    let order = state.fanouts.after_power_amount_changed_order().to_vec();
    for power in order {
        if state.history.over {
            break;
        }
        match power {
            PowerId::Vicious => {
                // `0x34a6f8` IL_0020-IL_0032 wants a positive delta,
                // IL_0037-IL_0045 the listener's own owner as applier and
                // IL_004a-IL_0057 a `VulnerablePower`.
                if changed_power != Some(PowerId::Vuln)
                    || amount <= 0
                    || applier != crate::hot::Applier::Player
                {
                    continue;
                }
                let draws = state.powers.value(power);
                if draws <= 0 {
                    return Err(EngineRefusal::MalformedArgs(
                        "after-power-amount-changed vicious",
                    ));
                }
                let catalog = catalog.ok_or(EngineRefusal::MalformedArgs(
                    "Vicious Vulnerable producer catalog",
                ))?;
                super::draw::draw_cards(
                    state,
                    catalog,
                    draws as usize,
                    super::draw::DrawSource::Command,
                    events,
                )?;
            }
            PowerId::SleightOfFlesh => {
                // `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext`
                // `0x344dc4` IL_003d-IL_0049 tests
                // `power.GetTypeForAmount(/* the incoming delta */ amount)`
                // and returns unless it is 2. `StrengthPower` is
                // `AllowNegative` `0xa8949` and Intensity-stacking `0xa8946`,
                // so `GetTypeForAmount` `0x83a94` IL_0013-IL_003e reports 2
                // for it exactly while the delta is negative — a POSITIVE
                // Strength delta is Type 1 and this listener does not fire.
                // Both producers of one are reachable: Brimstone's
                // `+1`-to-every-enemy (`0x32098c` IL_012d-IL_0130) and, since
                // #2693 S3, a `Misery` copy whose frozen Strength value the
                // temporary-wrapper fold has raised above zero. Every other
                // change this fanout carries is Type 2 for a positive delta,
                // which is why only Strength needs the test.
                if changed_power == Some(PowerId::Strength) && amount > 0 {
                    continue;
                }
                // #2727 — `0x344dc4` IL_0067-IL_0075 loads the hook's
                // `applier` argument and the listener's own
                // `PowerModel::get_Owner`, and `beq` is *reference* identity.
                // Sleight of Flesh is a player-owned power, so the applier
                // must be the player. A `Misery` copy of a monster-, relic-
                // or unknown-applied carrier fails it (`0x3ad358`
                // IL_02b6/IL_0354 hand the command the source instance's
                // retained `_applier`), and so would any future writer that
                // is not the local player.
                if applier != crate::hot::Applier::Player {
                    continue;
                }
                if state.monsters[target].hp <= 0 {
                    continue;
                }
                let damage = state.powers.value(power);
                damage_monster_with_optional_catalog(
                    state,
                    catalog,
                    target,
                    DotNetDecimal::from_i64(i64::from(damage)),
                    false,
                    true,
                    events,
                )?;
            }
            PowerId::Shroud => {
                // `0x3446a4` IL_001d-IL_002b requires the listener's own
                // owner as applier and IL_0030-IL_003d a `DoomPower`.
                if changed_power != Some(PowerId::Doom) || applier != crate::hot::Applier::Player {
                    continue;
                }
                let block = state.powers.value(power);
                if block <= 0 {
                    return Err(EngineRefusal::MalformedArgs(
                        "after-power-amount-changed shroud",
                    ));
                }
                gain_flat_power_block(state, catalog, block, events)?;
            }
            // Sword Sage observes only its own owner-power amount changes;
            // this fanout carries monster Type-2 changes.
            PowerId::SwordSage => {}
            _ => {
                return Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order",
                ));
            }
        }
    }
    Ok(())
}

fn exact_power_amount_target(
    state: &HotState,
    target: usize,
) -> Result<(i32, u32, Option<i32>), EngineRefusal> {
    let monster = state
        .monsters
        .get(target)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    let identity = (monster.slot, monster.uid);
    let duplicate_count = state
        .monsters
        .iter()
        .filter(|candidate| (candidate.slot, candidate.uid) == identity)
        .count();
    let fallback = if duplicate_count > 1 {
        Some(i32::try_from(target).map_err(|_| EngineRefusal::ContinuationNotModeled)?)
    } else {
        None
    };
    Ok((identity.0, identity.1, fallback))
}

fn power_amount_target_index(
    state: &HotState,
    target: (i32, u32, Option<i32>),
) -> Result<Option<usize>, EngineRefusal> {
    let (slot, uid, fallback) = target;
    let matches = state
        .monsters
        .iter()
        .enumerate()
        .filter_map(|(index, monster)| {
            ((monster.slot, monster.uid) == (slot, uid)).then_some(index)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [index] => Ok(Some(*index)),
        _ => {
            let index = fallback
                .and_then(|index| usize::try_from(index).ok())
                .filter(|index| matches.contains(index))
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            Ok(Some(index))
        }
    }
}

fn start_after_power_amount_changed_vulnerable(
    state: &mut HotState,
    catalog: &Catalog,
    target: usize,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !player_type_one_listener_order_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "after-power-amount-changed listener order",
        ));
    }
    let listeners = state.fanouts.after_power_amount_changed_order().to_vec();
    if listeners.is_empty() {
        return Ok(());
    }
    let record = AfterPowerAmountChangedRecord {
        listeners,
        cursor: 0,
        amount,
        target: exact_power_amount_target(state, target)?,
    };
    state
        .frames
        .push_after_power_amount_changed(&record)
        .ok_or(EngineRefusal::ContinuationNotModeled)?;
    advance_top_after_power_amount_changed(state, catalog, events)
}

/// Resume the frozen native AfterPowerAmountChanged enumerator. The cursor is
/// committed before each awaited listener; Vicious owns its Draw and the
/// remaining Sleight/Shroud/Sword Sage suffix runs exactly once afterward.
///
/// **The record carries no applier because only one value can reach it.**
/// [`start_after_power_amount_changed_vulnerable`] is the sole producer and
/// [`powers_after_power_amount_changed`] calls it only for a positive
/// `Vulnerable` delta applied by `crate::hot::Applier::Player` — a non-player
/// applier cannot suspend, because `ViciousPower/<AfterPowerAmountChanged>
/// d__6::MoveNext` `0x34a6f8` IL_0037-IL_0045 returns before its awaited Draw
/// and the native enumerator then completes synchronously. So the Sleight arm
/// below is reached with a player applier by construction, which is what
/// `0x344dc4` IL_0067-IL_0075 requires (#2727).
pub(crate) fn advance_top_after_power_amount_changed(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    loop {
        let mut record = match state.frames.top() {
            Some(crate::frame::Frame::AfterPowerAmountChanged { record }) => state
                .frames
                .after_power_amount_changed(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?
                .to_owned(),
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        if record.listeners.as_slice() != &*state.fanouts.after_power_amount_changed_order()
            || !player_type_one_listener_order_is_exact(state)
            || record.cursor == 0 && power_amount_target_index(state, record.target)?.is_none()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        if state.history.over {
            state
                .frames
                .pop_top_after_power_amount_changed()
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            return Ok(());
        }
        let cursor =
            usize::try_from(record.cursor).map_err(|_| EngineRefusal::ContinuationNotModeled)?;
        let Some(listener) = record.listeners.get(cursor).copied() else {
            state
                .frames
                .pop_top_after_power_amount_changed()
                .ok_or(EngineRefusal::ContinuationNotModeled)?;
            return Ok(());
        };
        record.cursor = record
            .cursor
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "AfterPowerAmountChanged cursor",
            ))?;
        state
            .frames
            .replace_top_after_power_amount_changed(&record)
            .ok_or(EngineRefusal::ContinuationNotModeled)?;
        match listener {
            PowerId::Vicious => {
                let draws = state.powers.value(PowerId::Vicious);
                if draws <= 0 {
                    return Err(EngineRefusal::MalformedArgs(
                        "after-power-amount-changed vicious",
                    ));
                }
                if super::draw::draw_cards_for_potion(
                    state,
                    catalog,
                    draws as usize,
                    DrawCaller::AfterPowerAmountChanged,
                    events,
                )? == super::draw::PotionDrawResult::Suspended
                {
                    return Ok(());
                }
            }
            PowerId::SleightOfFlesh => {
                let target = match power_amount_target_index(state, record.target)? {
                    Some(target) => target,
                    None => continue,
                };
                if state.history.over || state.monsters[target].hp <= 0 {
                    continue;
                }
                let damage = state.powers.value(PowerId::SleightOfFlesh);
                if damage <= 0 {
                    return Err(EngineRefusal::MalformedArgs(
                        "after-power-amount-changed sleight",
                    ));
                }
                damage_monster_with_optional_catalog(
                    state,
                    Some(catalog),
                    target,
                    DotNetDecimal::from_i64(i64::from(damage)),
                    false,
                    true,
                    events,
                )?;
            }
            // Both callbacks are inert for a player-applied Vulnerable event.
            PowerId::Shroud | PowerId::SwordSage => {}
            _ => {
                return Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order",
                ));
            }
        }
        if state.pending.is_some() {
            return Ok(());
        }
    }
}

/// Authenticate the represented AfterPowerAmountChanged walk for a native
/// PowerCmd whose applier is null. Conqueror's owner-side duration decrement
/// takes this path: every represented callback is inert for a null applier,
/// but malformed acquisition order must still refuse before the scalar write.
pub(crate) fn null_applier_power_amount_changed_is_exact(
    state: &HotState,
) -> Result<(), EngineRefusal> {
    if player_type_one_listener_order_is_exact(state) {
        Ok(())
    } else {
        Err(EngineRefusal::MalformedArgs(
            "after-power-amount-changed listener order",
        ))
    }
}

/// Apply one card-sourced positive Intensity stack to Energy Next Turn.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `RefineBlade/<OnPlay>d__5::MoveNext` RVA `0x3b65ac` awaits
/// `PowerCmd::Apply<EnergyNextTurnPower>` after Forge. The command is ending-
/// gated, stores the stacked Type-1 amount, publishes the amount change, then
/// serially walks the frozen `AfterPowerAmountChanged` listener order. Every
/// represented listener is inert for this player-owned application, but the
/// complete acquisition order must still be authenticated rather than
/// silently skipping a malformed callback set.
pub(crate) fn apply_card_energy_next_turn(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if amount <= 0 {
        return Err(EngineRefusal::MalformedArgs("card EnergyNextTurn amount"));
    }
    if state.history.over {
        return Ok(());
    }
    if !player_type_one_listener_order_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "after-power-amount-changed listener order",
        ));
    }
    let current = match state.powers.get(PowerId::EnergyNextTurn) {
        None => 0,
        Some(slot) if slot.wire == SlotWire::Int && slot.value >= 0 => slot.value,
        Some(_) => {
            return Err(EngineRefusal::MalformedArgs("EnergyNextTurn power state"));
        }
    };
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("energy next turn"))?;
    state
        .powers
        .set(PowerId::EnergyNextTurn, SlotWire::Int, updated);
    if current == 0
        && !state
            .fanouts
            .register_after_energy_reset(crate::hot::AfterEnergyResetPower::EnergyNextTurn)
    {
        return Err(EngineRefusal::CounterOverflow(
            "after-energy-reset listener order",
        ));
    }
    state.normalize_after_energy_reset_order_if_unique();
    note_power(events, Subject::Player, PowerId::EnergyNextTurn, updated);
    Ok(())
}

pub(crate) fn vicious_listener_is_reachable(state: &HotState) -> bool {
    state.powers.value(PowerId::Vicious) != 0
        || state
            .fanouts
            .after_power_amount_changed_order()
            .contains(&PowerId::Vicious)
}

/// Authenticate the complete represented AfterPowerAmountChanged listener set.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `ShroudPower/<AfterPowerAmountChanged>d__6::MoveNext` RVA `0x3446a4`
/// requires its owner to be the applier and the changed power to be Doom.
/// `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext` RVA
/// `0x344dc4` requires a Type-2 change to an enemy-owned non-temporary power.
/// Vicious's callback body RVA `0x34a6f8` additionally requires a positive
/// delta, its owner as applier, and Vulnerable; it is therefore also a
/// synchronous no-op for player-owned Type-1 changes such as Shadow Step,
/// Double Damage, and The Hunt. Native still walks the frozen acquisition
/// order. Require one occurrence of every live represented listener before
/// callers elide that inert walk.
pub(crate) fn player_type_one_listener_order_is_exact(state: &HotState) -> bool {
    let exact_amount = |power| match state.powers.get(power) {
        None => Some(0),
        Some(slot) if slot.wire == SlotWire::Int => Some(slot.value),
        Some(_) => None,
    };
    let (Some(vicious_amount), Some(shroud_amount), Some(sleight_amount), Some(sword_sage_amount)) = (
        exact_amount(PowerId::Vicious),
        exact_amount(PowerId::Shroud),
        exact_amount(PowerId::SleightOfFlesh),
        exact_amount(PowerId::SwordSage),
    ) else {
        return false;
    };
    if vicious_amount < 0 || shroud_amount < 0 || sleight_amount < 0 || sword_sage_amount < 0 {
        return false;
    }
    let vicious_active = vicious_amount > 0;
    let shroud_active = shroud_amount > 0;
    let sleight_active = sleight_amount > 0;
    let sword_sage_active = sword_sage_amount > 0;
    let active_len = usize::from(vicious_active)
        + usize::from(shroud_active)
        + usize::from(sleight_active)
        + usize::from(sword_sage_active);
    let order = state.fanouts.after_power_amount_changed_order();
    order.len() == active_len
        && (!vicious_active || order.contains(&PowerId::Vicious))
        && (!shroud_active || order.contains(&PowerId::Shroud))
        && (!sleight_active || order.contains(&PowerId::SleightOfFlesh))
        && (!sword_sage_active || order.contains(&PowerId::SwordSage))
        && order.iter().all(|listener| match listener {
            PowerId::Vicious => vicious_active,
            PowerId::Shroud => shroud_active,
            PowerId::SleightOfFlesh => sleight_active,
            PowerId::SwordSage => sword_sage_active,
            _ => false,
        })
}

/// Apply one exact Type-1 Shadow Step-family player-power amount mutation.
///
/// Current v0.111.0 authority is ARM64 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// `ShadowStepPower/<AfterSideTurnStart>d__4::MoveNext` RVA `0x3441ec`
/// applies its complete Amount to Double Damage; the latter's owner-side-end
/// body RVA `0x33983c` decrements exactly one through `ModifyAmount(-1)`.
/// Apply, stack, and decrement all publish AfterPowerAmountChanged after the
/// amount write. The represented Shroud and Sleight of Flesh listeners ignore
/// these player-owned Type-1 events, but their complete recorded order is
/// still authenticated before mutation.
pub(crate) fn modify_shadow_step_power_amount(
    state: &mut HotState,
    power: PowerId,
    delta: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !matches!(power, PowerId::ShadowStep | PowerId::DoubleDamage) || delta == 0 {
        return Err(EngineRefusal::MalformedArgs("Shadow Step power mutation"));
    }
    if !player_type_one_listener_order_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "after-power-amount-changed listener order",
        ));
    }
    let current = state.powers.value(power);
    if current < 0 {
        return Err(EngineRefusal::MalformedArgs("Shadow Step power amount"));
    }
    let updated = current
        .checked_add(delta)
        .ok_or(EngineRefusal::CounterOverflow("Shadow Step power amount"))?;
    if updated < 0 {
        return Err(EngineRefusal::MalformedArgs("Shadow Step power decrement"));
    }
    if power == PowerId::ShadowStep
        && current == 0
        && updated > 0
        && !state
            .fanouts
            .register_after_side_turn_start(PowerId::ShadowStep)
    {
        return Err(EngineRefusal::PowerOrderNotModeled("AfterSideTurnStart"));
    }
    super::turn::prepare_after_side_turn_end_scalar_write(state, power, current, updated)?;
    state.powers.set(power, SlotWire::Int, updated);
    note_power(events, Subject::Player, power, updated);
    Ok(())
}

/// `PoisonPower.AfterSideTurnStart` (IL 0x3874ec), at the monster side's
/// start (`_finish_side_switch_after_disintegration`, frozen Python, deleted #2827).
///
/// Per poisoned living creature: the iteration count is
/// `min(amount, 1 + accelerant)` computed **once**, each iteration deals the
/// *current retained PowerModel* amount as props-6 damage (unblockable,
/// unpowered — no modifiers, though the HP-loss hooks still apply). The
/// coroutine retains its Owner and PowerModel: while the same PoisonPower is
/// attached it re-reads and publishes its amount; after owner death removes
/// that slot it keeps decrementing the detached model and can damage a revived
/// same-UID owner without rewriting the cleared slot. A replacement UID never
/// inherits those commands. The represented single opposing Player supplies
/// the complete alive-opponent Accelerant sum. The widened addition preserves
/// native Decimal's `1 + amount` at the frozen non-negative `i32` boundary.
///
/// Returns whether combat is still running: a poison kill that ends the fight
/// stops the whole side start, so the enemy phase never acts.
fn tick_monster_poison_inner(
    state: &mut HotState,
    catalog: Option<&Catalog>,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    if !poison_identity_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Poison identity universe"));
    }
    null_applier_power_amount_changed_is_exact(state)?;
    // Native's opposing-player query includes only living opponents. A remote
    // teammate can keep combat live after the represented owner dies, but the
    // dead owner's Accelerant has already been removed and contributes zero.
    let accelerant = if state.hp > 0 {
        state.powers.value(PowerId::Accelerant)
    } else {
        0
    };
    if accelerant < 0 {
        return Err(EngineRefusal::MalformedArgs("Accelerant power amount"));
    }
    let trigger_bound = i64::from(accelerant)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Poison trigger count"))?;
    for index in 0..state.monsters.len() {
        let (owner_uid, owner_poison_uid, hp, poison) = {
            let monster = &state.monsters[index];
            (
                monster.uid,
                monster.poison_uid,
                monster.hp,
                monster.powers.value(PowerId::Poison),
            )
        };
        if hp <= 0 || poison <= 0 {
            continue;
        }
        let iterations: i32 = i64::from(poison)
            .min(trigger_bound)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("Poison trigger count"))?;
        let mut retained_amount = poison;
        let mut attached = true;
        for _ in 0..iterations {
            if state.history.over {
                break;
            }
            let owner = &state.monsters[index];
            attached = attached
                && owner.uid == owner_uid
                && owner.poison_uid == owner_poison_uid
                && owner.powers.value(PowerId::Poison) > 0;
            if attached {
                retained_amount = owner.powers.value(PowerId::Poison);
            }
            if retained_amount <= 0 {
                return Err(EngineRefusal::MalformedArgs("retained Poison power amount"));
            }
            if owner.uid == owner_uid && owner.hp > 0 {
                // Each Poison trigger's Damage death runs `Hook.AfterDeath`
                // with the side start's catalog (Gremlin Horn's Draw, #3172).
                damage_monster_with_optional_catalog(
                    state,
                    catalog,
                    index,
                    DotNetDecimal::from_i64(i64::from(retained_amount)),
                    false,
                    false,
                    events,
                )?;
            }
            let owner = &state.monsters[index];
            attached = attached
                && owner.uid == owner_uid
                && owner.poison_uid == owner_poison_uid
                && owner.powers.value(PowerId::Poison) > 0;
            if owner.uid == owner_uid && owner.hp > 0 && !state.history.over {
                retained_amount -= 1;
                if attached {
                    let owner = &mut state.monsters_mut()[index];
                    owner
                        .powers
                        .set(PowerId::Poison, SlotWire::Int, retained_amount);
                    if retained_amount == 0 {
                        owner.poison_uid = -1;
                        owner.misery_debuff_order.remove_all(MiseryToken::Poison);
                    }
                }
                note_power(
                    events,
                    Subject::Monster(owner_uid),
                    PowerId::Poison,
                    retained_amount,
                );
                // `PoisonPower.Trigger` decrements with a null applier. The
                // native hook walk is still authenticated, but Vicious,
                // Shroud, and Sleight of Flesh all require a matching owner
                // applier and are therefore inert here.
            }
        }
        if state.history.over {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Validate the exact attached PoisonPower identity universe.
///
/// Positive entry-projected Poison may still carry Python's legacy `-1`
/// sentinel until an identity-sensitive consumer (currently Outbreak) first
/// observes it. Allocated identities are non-negative, strictly below the
/// cursor, and never alias. Absent Poison owns `-1`; Misery's acquisition
/// order must describe the same scalar power set.
pub(crate) fn poison_identity_state_is_exact(state: &HotState) -> bool {
    if state.next_poison_uid < 0 {
        return false;
    }
    for (index, monster) in state.monsters.iter().enumerate() {
        let poison_tokens = monster
            .misery_debuff_order
            .as_slice()
            .iter()
            .filter(|token| matches!(token, MiseryToken::Poison))
            .count();
        match monster.powers.get(PowerId::Poison) {
            None if monster.poison_uid == -1 && poison_tokens == 0 => {}
            Some(slot)
                if slot.wire == SlotWire::Int
                    && slot.value > 0
                    && monster.hp > 0
                    && (monster.poison_uid == -1
                        || (monster.poison_uid >= 0
                            && monster.poison_uid < state.next_poison_uid)) =>
            {
                if poison_tokens != 1 {
                    return false;
                }
                if monster.poison_uid >= 0
                    && state.monsters[..index]
                        .iter()
                        .any(|previous| previous.poison_uid == monster.poison_uid)
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Lazily give one legacy entry-projected Poison instance the keyed identity
/// Python assigns when Outbreak first needs to retain that concrete power.
pub(crate) fn ensure_poison_instance_uid(
    state: &mut HotState,
    target: usize,
) -> Result<i32, EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Err(EngineRefusal::TargetMismatch { required: true });
    };
    if monster.hp <= 0 || monster.powers.value(PowerId::Poison) <= 0 {
        return Err(EngineRefusal::MalformedArgs("absent Poison identity"));
    }
    if monster.poison_uid >= 0 {
        return Ok(monster.poison_uid);
    }
    let uid = state.next_poison_uid;
    state.next_poison_uid = state
        .next_poison_uid
        .checked_add(1)
        .ok_or_else(|| overflow("next_poison_uid"))?;
    state.monsters_mut()[target].poison_uid = uid;
    Ok(uid)
}

/// Trigger one frozen `(slot, uid)` PoisonPower through its retained model.
///
/// Outbreak uses this after taking its fresh second-wave snapshot. The
/// attached power identity and initial trigger count are frozen, while each
/// iteration re-reads the old attached amount or its detached retained copy.
/// Replacement creatures never inherit work. Every decrement has a null
/// applier, so the complete amount-change order is authenticated but its
/// player-owner callbacks remain inert.
pub(crate) fn trigger_poison_identity_null_applier(
    state: &mut HotState,
    catalog: &Catalog,
    identity: (i32, u32),
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !poison_identity_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Poison identity universe"));
    }
    null_applier_power_amount_changed_is_exact(state)?;
    let Some(initial_index) = state
        .monsters
        .iter()
        .position(|monster| (monster.slot, monster.uid) == identity)
    else {
        return Ok(());
    };
    let initial = &state.monsters[initial_index];
    let poison_uid = initial.poison_uid;
    let poison = initial.powers.value(PowerId::Poison);
    if initial.hp <= 0 || poison <= 0 || poison_uid < 0 {
        return Ok(());
    }
    let accelerant = if state.hp > 0 {
        state.powers.value(PowerId::Accelerant)
    } else {
        0
    };
    if accelerant < 0 {
        return Err(EngineRefusal::MalformedArgs("Accelerant power amount"));
    }
    let trigger_bound = i64::from(accelerant)
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Poison trigger count"))?;
    let iterations: i32 = i64::from(poison)
        .min(trigger_bound)
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("Poison trigger count"))?;
    let mut retained_amount = poison;
    let mut attached = true;
    for _ in 0..iterations {
        if state.history.over {
            break;
        }
        let current_index = state
            .monsters
            .iter()
            .position(|monster| (monster.slot, monster.uid) == identity);
        if let Some(index) = current_index {
            let owner = &state.monsters[index];
            attached = attached
                && owner.poison_uid == poison_uid
                && owner.powers.value(PowerId::Poison) > 0;
            if attached {
                retained_amount = owner.powers.value(PowerId::Poison);
            }
            if retained_amount <= 0 {
                return Err(EngineRefusal::MalformedArgs("retained Poison power amount"));
            }
            if owner.hp > 0 {
                damage_monster_after_catalog_auth(
                    state,
                    catalog,
                    index,
                    DotNetDecimal::from_i64(i64::from(retained_amount)),
                    false,
                    false,
                    events,
                )?;
            }
        }
        let current_index = state
            .monsters
            .iter()
            .position(|monster| (monster.slot, monster.uid) == identity);
        let Some(index) = current_index else {
            continue;
        };
        let owner = &state.monsters[index];
        attached =
            attached && owner.poison_uid == poison_uid && owner.powers.value(PowerId::Poison) > 0;
        if owner.hp > 0 && !state.history.over {
            retained_amount = retained_amount
                .checked_sub(1)
                .ok_or(EngineRefusal::CounterOverflow("Poison decrement"))?;
            if attached {
                let owner = &mut state.monsters_mut()[index];
                owner
                    .powers
                    .set(PowerId::Poison, SlotWire::Int, retained_amount);
                if retained_amount == 0 {
                    owner.poison_uid = -1;
                    owner.misery_debuff_order.remove_all(MiseryToken::Poison);
                }
            }
            note_power(
                events,
                Subject::Monster(identity.1),
                PowerId::Poison,
                retained_amount,
            );
        }
    }
    Ok(())
}

/// Public catalogless Poison tick.
///
/// Hopper's cold DeckVersion payload cannot be authenticated at this seam, so
/// callers must enter through the catalog-bearing end-turn path instead.
pub fn tick_monster_poison(
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    if hopper_reachable
        || state.card_states.dampen().is_some()
        || super::cards::aeonglass_owner_reachable(state)
    {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper Poison catalog",
        ));
    }
    let mut successor = state.clone();
    let mut suffix = Vec::new();
    let running = tick_monster_poison_inner(&mut successor, None, &mut suffix)?;
    *state = successor;
    events.extend(suffix);
    Ok(running)
}

/// Poison tick after the catalog-bearing end-turn boundary authenticated the
/// complete Hopper sidecar and selected the whole-turn transaction.
///
/// It keeps that boundary's catalog (it was
/// `tick_monster_poison_after_hopper_auth`, which dropped it, #3172): each
/// trigger is one `CreatureCmd.Damage` (`PoisonPower.AfterSideTurnStart`,
/// IL 0x3874ec), and a lethal one awaits `CreatureCmd.Kill` and its
/// `Hook.AfterDeath` walk, where `GremlinHorn/<AfterDeath>d__6::MoveNext` RVA
/// `0x326170` (v0.111.0 DLL 9cb4f1ad…) gates only on the dead creature's
/// side (IL_0024-IL_003f) before `PlayerCmd.GainEnergy` (IL_0062) and
/// `CardPileCmd.Draw` (IL_00d9).
pub(crate) fn tick_monster_poison_after_catalog_auth(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let hopper_reachable = state.card_states.hopper().is_some()
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind == MonsterKind::ThievingHopper);
    let dampen_reachable = state.card_states.dampen().is_some();
    let aeonglass_reachable = super::cards::aeonglass_owner_reachable(state);
    if hopper_reachable && !super::monsters::thieving_hopper_internal_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper internal Poison entry",
        ));
    }
    if dampen_reachable && !super::cards::dampen_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("Dampen internal Poison entry"));
    }
    if aeonglass_reachable && !super::cards::aeonglass_internal_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Aeonglass internal Poison entry",
        ));
    }
    tick_monster_poison_inner(state, Some(catalog), events)
}

/// One owner-targeted `StrengthPower` command (`_apply_owner_strength`, frozen Python, deleted #2827).
///
/// A zero amount and an ended combat are both `PowerCmd` no-ops — which is
/// load-bearing rather than an optimisation: Ruined Helmet's first-positive
/// latch is *not* consumed by one, so the two cases must not fall through to
/// the write. Ruined Helmet doubles the first positive application and
/// latches only after the successful write. Temporary-Strength wrappers keep
/// the original amount in their expiry ledger, leaving the excess permanent.
pub fn apply_owner_strength(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    let ruined_helmet =
        amount > 0 && state.fanouts.ruined_helmet_owned() && !state.fanouts.ruined_helmet_used();
    let modified = if ruined_helmet {
        amount
            .checked_mul(2)
            .ok_or_else(|| overflow("Ruined Helmet strength"))?
    } else {
        amount
    };
    let updated = state
        .powers
        .value(PowerId::Strength)
        .checked_add(modified)
        .ok_or_else(|| overflow("player strength"))?;
    state.powers.set(PowerId::Strength, SlotWire::Int, updated);
    note_power(events, Subject::Player, PowerId::Strength, updated);
    if ruined_helmet {
        state.fanouts.set_ruined_helmet_used();
        crate::coverage::record_relic(RelicId::RelicRuinedHelmet);
    }
    Ok(())
}

/// Red Skull's Strength amount: `RedSkull::get_CanonicalVars` (v0.111.0 RVA
/// `0x9a2c7`) builds its `StrengthVar` from `ldc.i4.3` at IL_001d.
pub(crate) const RED_SKULL_STRENGTH: i32 = 3;

/// Red Skull's HP test: `<ModifyStrengthIfNecessary>d__14::MoveNext` (v0.111.0
/// RVA `0x32f790`) IL_0033-IL_006f computes "above" as the decimal
/// `CurrentHp > MaxHp * (HpThreshold / 100)`, with `HpThreshold` 50
/// (`get_CanonicalVars` `0x9a2c7` IL_0009-IL_0010). This is its negation — the
/// state in which the relic's Strength is held — as exact integer arithmetic.
pub(crate) fn red_skull_threshold_met(hp: i32, max_hp: i32) -> bool {
    i64::from(hp) * 2 <= i64::from(max_hp)
}

/// Red Skull's `AfterCurrentHpChanged` for one player HP write (#3044).
///
/// Red Skull's +3 is a real `StrengthPower`, not a damage term. v0.111.0 DLL
/// `9cb4f1ad…`, `RedSkull`:
///
/// * `<AfterCurrentHpChanged>d__13::MoveNext` RVA `0x32f5ec` IL_001d-IL_0029
///   returns unless `CombatManager.Instance.IsInProgress`, then awaits
///   `ModifyStrengthIfNecessary` (IL_002c). It never tests which creature
///   changed, and `Hook.AfterCurrentHpChanged` is raised by
///   `CreatureCmd.Damage` (`0x3e96c8` IL_0be4, only for a positive
///   `UnblockedDamage`), `Heal` (`0x3eb4b0`, any positive amount), `Kill`
///   (`0x3ebe90` IL_0200, before `ShouldDie` and `DeactivateHooks`) and
///   `SetCurrentHp` (`scan_calls.py AfterCurrentHpChanged`).
/// * `<ModifyStrengthIfNecessary>d__14::MoveNext` RVA `0x32f790`: "above" is
///   [`red_skull_threshold_met`]'s negation (IL_0033-IL_006f); the amount is
///   `DynamicVars.Strength` (IL_0088). Above with `StrengthApplied` set, it
///   awaits `PowerCmd.Apply<StrengthPower>(owner, -Strength, owner, null)`
///   (IL_00a8-IL_00b8) and clears the latch (IL_0116); at or below with the
///   latch clear, it awaits `Apply<StrengthPower>(owner, +Strength, owner,
///   null)` (IL_0130-IL_013b) and sets it (IL_0196). Otherwise nothing.
/// * `AfterCombatEnd` `0x9a37b` clears the latch; `AfterRoomEntered`
///   (`<AfterRoomEntered>d__11` `0x32f6bc`) runs the same body in a
///   `CombatRoom` with no `IsInProgress` test (the opening's
///   `apply_room_entry_strength`).
///
/// **The latch is derived, not stored.** Every in-combat change of the
/// player's HP runs this body, so on entry to any HP write the latch equals
/// [`red_skull_threshold_met`] of the HP it had before the write; a monster's
/// HP change therefore re-runs it as a no-op. Two native paths move the
/// quotient without an in-progress `AfterCurrentHpChanged`:
///
/// * `SetMaxHp` after `LoseMaxHp`'s nested Damage, which
///   [`lose_player_max_hp_apply`] refuses by name when it would cross;
/// * Planisphere's room-entry heal after Red Skull's own `AfterRoomEntered`
///   (combat not yet in progress). The opening admits that only when a
///   turn-one Blood Vial heal re-runs this body before the player's first
///   action, and marks the latch stale until then
///   (`HotFanouts::red_skull_latch_stale`); this body flips its derived
///   answer once and clears the mark.
///
/// So each writer passes the HP and max HP it had before, and a crossing is
/// exactly one latch transition.
///
/// The application goes through [`apply_owner_strength`]: its `history.over`
/// no-op is `PowerCmd.Apply`'s ending gate (the latch still moves natively,
/// but nothing after the combat ends reads it), and Ruined Helmet doubles a
/// first positive application whatever its source. The applier is the owner
/// with a null card source, so the represented given-side and
/// amount-changed listeners are inert: Unsettling Lamp needs a card source
/// (`BeforePowerAmountChanged` `0x9d4fc` IL_0028,
/// `ModifyPowerAmountGivenMultiplicative` `0x9d5a8` IL_000d-IL_0022), Sleight
/// of Flesh an enemy-owned power (`0x344dc4` IL_0050-IL_0060), Vicious
/// Vulnerable and Shroud Doom, and player Artifact is never admitted.
/// A player whose hooks were deactivated by `Kill` is no listener.
pub(crate) fn red_skull_after_player_hp_changed(
    state: &mut HotState,
    before_hp: i32,
    before_max_hp: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !state.fanouts.red_skull_owned() || state.fanouts.player_hooks_deactivated() {
        return Ok(());
    }
    let stale = state.fanouts.red_skull_latch_stale();
    state.fanouts.set_red_skull_latch_stale(false);
    let held = red_skull_threshold_met(before_hp, before_max_hp) != stale;
    let wanted = red_skull_threshold_met(state.hp, state.max_hp);
    if held == wanted {
        return Ok(());
    }
    crate::coverage::record_relic(RelicId::RelicRedSkull);
    let amount = if wanted {
        RED_SKULL_STRENGTH
    } else {
        -RED_SKULL_STRENGTH
    };
    apply_owner_strength(state, amount, events)
}

/// `CreatureCmd::GainMaxHp(player, amount)` for a non-negative `amount`.
///
/// v0.111.0 (DLL `9cb4f1ad…`) `CreatureCmd/<GainMaxHp>d__22::MoveNext` RVA
/// `0x3eb2f0`: `SetMaxHp(creature, MaxHp + amount)` at IL_005b, whose result
/// is the accepted delta, then `Heal(creature, delta, true)` at IL_010b, whose
/// `AfterCurrentHpChanged` reaches Red Skull with both new values (#3044).
/// `<SetMaxHp>d__24` (`0x3ecf3c`) runs no hook on a raise (its only call
/// besides `SetMaxHpInternal` is the `Kill` of a max-HP drop to zero). The
/// player heal has no IsEnding skip (`<Heal>d__20` `0x3eb4b0` IL_0041-IL_005f
/// skips only a non-player creature), so this runs after a lethal too.
///
/// Feed's fatal reward and Dragon Fruit (#3320) share this body; the
/// arithmetic is Feed's, moved here verbatim.
pub(crate) fn gain_player_max_hp(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let old_max_hp = state.max_hp;
    let new_max_hp = (i64::from(old_max_hp) + i64::from(amount)).min(999_999_999) as i32;
    let accepted = new_max_hp - old_max_hp;
    let hp_before = state.hp;
    state.max_hp = new_max_hp;
    state.hp = state.hp.saturating_add(accepted).min(new_max_hp);
    red_skull_after_player_hp_changed(state, hp_before, old_max_hp, events)
}

/// One positive owner `TemporaryStrengthPower` command. The wrapper records
/// only the original amount for side-end expiry; Ruined Helmet's first-use
/// excess is applied to permanent Strength and therefore survives that
/// cleanup, matching `_apply_owner_strength(..., temporary=True)`.
pub(crate) fn apply_owner_temporary_strength(
    state: &mut HotState,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    if amount < 0 {
        return Err(EngineRefusal::MalformedArgs("temporary player strength"));
    }
    let current = state.temp_strength;
    let updated = current
        .checked_add(amount)
        .ok_or(EngineRefusal::CounterOverflow("temporary player strength"))?;
    let ruined_helmet = state.fanouts.ruined_helmet_owned() && !state.fanouts.ruined_helmet_used();
    let permanent = ruined_helmet
        .then(|| {
            state
                .powers
                .value(PowerId::Strength)
                .checked_add(amount)
                .ok_or_else(|| overflow("Ruined Helmet temporary strength"))
        })
        .transpose()?;
    super::turn::prepare_after_side_turn_end_singleton_write(
        state,
        crate::hot::AfterSideTurnEndPowerToken::TemporaryStrength,
        current != 0,
        updated != 0,
    )?;
    state.temp_strength = updated;
    if let Some(permanent) = permanent {
        state
            .powers
            .set(PowerId::Strength, SlotWire::Int, permanent);
        note_power(events, Subject::Player, PowerId::Strength, permanent);
        state.fanouts.set_ruined_helmet_used();
        crate::coverage::record_relic(RelicId::RelicRuinedHelmet);
    }
    Ok(())
}

/// Apply one signed owner Strength/Dexterity command from a retained power
/// applier. Both native powers allow negative amounts. The PowerCmd ending
/// gate precedes every amount listener and write; positive Strength reuses
/// the owner received-amount seam, while Dexterity is a plain signed add on
/// the currently represented no-player-Artifact surface.
pub(crate) fn apply_signed_player_stat(
    state: &mut HotState,
    power: PowerId,
    amount: i32,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if state.history.over || amount == 0 {
        return Ok(());
    }
    match power {
        PowerId::Strength => apply_owner_strength(state, amount, events),
        PowerId::Dexterity => {
            let updated = state
                .powers
                .value(PowerId::Dexterity)
                .checked_add(amount)
                .ok_or_else(|| overflow("player dexterity"))?;
            state.powers.set(PowerId::Dexterity, SlotWire::Int, updated);
            note_power(events, Subject::Player, PowerId::Dexterity, updated);
            Ok(())
        }
        _ => Err(EngineRefusal::MalformedArgs("signed player stat")),
    }
}

/// Emit a power-change event for the given subject.
pub fn note_power(events: &mut Vec<Event>, subject: Subject, power: PowerId, amount: i32) {
    events.push(Event::PowerChanged {
        subject,
        power,
        amount,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CardAtom, CardEnchantment, CardIdentity, Catalog, CatalogBuilder};
    use crate::engine::cards::inject_legacy_bottom;
    use crate::engine::play::play_card;
    use crate::hot::{
        CARD_FLAG_LEGACY, CardInstanceState, HopperDeckRow, HopperDeckState, HopperLootEvent,
        HopperLootKind, HotCard, PileId,
    };
    use crate::ids::RelicId;

    fn demon_tongue_state(side_active: bool) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 100;
        state.set_batch_six_deep_relic_ownership(true, false, false, false);
        state.player_side_active = side_active;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state
    }

    /// #2808, `DemonTongue/<AfterDamageReceived>d__3` RVA `0x322adc`
    /// `IL_0037`-`IL_005e`: on the owner's own side the first unblocked result
    /// heals back in full and arms the latch; a second result that turn stays.
    #[test]
    fn demon_tongue_heals_the_first_owner_side_result_once() {
        let mut state = demon_tongue_state(true);
        assert!(damage_player_from_card(&mut state, 7, true, &mut Vec::new()).unwrap());
        assert_eq!(state.hp, 80, "the 7 lost are healed back");
        assert!(state.demon_tongue_triggered());
        assert!(damage_player_from_card(&mut state, 5, true, &mut Vec::new()).unwrap());
        assert_eq!(state.hp, 75, "the latch holds for the rest of the turn");

        // A monster hit taken while the player's side is still current
        // (e.g. a player-side retaliation) is owner-side too.
        let mut attacked = demon_tongue_state(true);
        monster_attack_player(&mut attacked, 0, 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(attacked.hp, 80);
        assert!(attacked.demon_tongue_triggered());
    }

    /// #2808: `CurrentSide != Owner.Side` returns before the heal and before
    /// the latch write, so enemy-side damage is neither healed nor spends the
    /// once-per-turn heal. Floor 31 (Infested Prisms) of `YLVVPKPH1MTW` is the
    /// native witness: 77 -> 75 across the turn-1 enemy side.
    #[test]
    fn demon_tongue_ignores_enemy_side_damage_and_keeps_its_latch() {
        let mut state = demon_tongue_state(false);
        monster_attack_player(&mut state, 0, 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.hp, 70, "enemy-side damage is not healed");
        assert!(!state.demon_tongue_triggered(), "the latch is not armed");
        assert!(damage_player_from_card(&mut state, 4, true, &mut Vec::new()).unwrap());
        assert_eq!(state.hp, 66, "still enemy side: no heal from any source");

        // Back on the owner's side, the untouched latch still heals once.
        state.player_side_active = true;
        assert!(damage_player_from_card(&mut state, 6, true, &mut Vec::new()).unwrap());
        assert_eq!(state.hp, 66);
        assert!(state.demon_tongue_triggered());
    }

    #[test]
    fn batch_two_attack_relics_share_the_python_additive_then_multiplier_fold() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .set_relics(&[
                RelicId::RelicStrikeDummy,
                RelicId::RelicMiniatureCannon,
                RelicId::RelicPaperPhrog,
            ])
            .unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 1);
        state.piles.get_mut(PileId::Play).make_mut().push(card);

        player_attack_from_card(
            &mut state,
            (&catalog, &source, card.uid),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 79); // floor((6 + 3 + 3) * 7/4)
    }

    #[test]
    fn batch_four_deep_damage_relics_match_python_modifier_positions() {
        let mut boot = HotState::at_defaults();
        boot.hp = 20;
        boot.max_hp = 20;
        boot.set_deep_relic_ownership(false, false, true, false);
        boot.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        damage_monster(
            &mut boot,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(boot.monsters[0].hp, 15);

        let mut drill = HotState::at_defaults();
        drill.hp = 20;
        drill.max_hp = 20;
        drill.set_deep_relic_ownership(true, false, false, false);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.block = 3;
        drill.monsters_mut().push(target);
        damage_monster(
            &mut drill,
            0,
            DotNetDecimal::from_i64(4),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((drill.monsters[0].hp, drill.monsters[0].block), (19, 0));
        assert_eq!(drill.monsters[0].powers.value(PowerId::Vuln), 2);

        let mut skull = HotState::at_defaults();
        skull.hp = 20;
        skull.max_hp = 20;
        skull.set_deep_relic_ownership(false, true, false, false);
        skull
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        apply_player_monster_debuff(
            &mut skull,
            PlayerMonsterDebuffSource::Card(None),
            0,
            PowerId::Poison,
            MiseryToken::Poison,
            2,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(skull.monsters[0].powers.value(PowerId::Poison), 3);

        let mut rod = HotState::at_defaults();
        rod.hp = 20;
        rod.max_hp = 20;
        rod.set_deep_relic_ownership(false, false, false, true);
        damage_player_from_card(&mut rod, 3, false, &mut Vec::new()).unwrap();
        assert_eq!(rod.hp, 18);
        rod.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        damage_player_from_card(&mut rod, 10, false, &mut Vec::new()).unwrap();
        assert_eq!(rod.hp, 18);
    }

    #[test]
    fn batch_three_attack_relics_share_one_additive_and_multiplier_floor() {
        let identity = CardIdentity {
            id: CardId::MinionStrike,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .set_relics(&[
                RelicId::RelicFakeStrikeDummy,
                RelicId::RelicMysticLighter,
                RelicId::RelicRedSkull,
                RelicId::RelicUndyingSigil,
                RelicId::RelicVitruvianMinion,
            ])
            .unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 100;
        // At 50/100 Red Skull holds its real StrengthPower (#3044): the 3 is
        // in the Strength scalar, not a term of the fold.
        state.fanouts.set_red_skull_owned(true);
        state.powers.set(PowerId::Strength, SlotWire::Int, 3);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Play).make_mut().push(card);

        player_attack_from_card(
            &mut state,
            (&catalog, &source, card.uid),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 62); // (6 + 1 + 9 + 3) * 2

        // Undying Sigil (#3035, `0x9d400` IL_0024-IL_0030) only listens when
        // the PLAYER is the target: a doomed target of the player's own attack
        // takes the full (6 + 1 + 9 + 3) * 2.
        state.monsters_mut()[0].hp = 50;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 50);
        player_attack_from_card(
            &mut state,
            (&catalog, &source, card.uid),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 12);
    }

    fn aeonglass_damage_fixture() -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Aeonglass).unwrap();
        builder
            .intern_reachable(CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        state.exact_piles = true;
        state.rng.set(
            crate::hot::RngStream::Niche,
            crate::hot::RngStreamState {
                words: [11, 22, 33, 44],
                counter: 0,
            },
        );
        super::super::monsters::construct_aeonglass_boss(&mut state, &catalog, &mut Vec::new())
            .unwrap();
        (state, catalog)
    }

    #[test]
    fn aeonglass_public_damage_roots_fail_closed_and_catalog_damage_is_atomic() {
        let (state, catalog) = aeonglass_damage_fixture();
        let mut direct = state.clone();
        let before = direct.clone();
        let mut events = vec![Event::TurnBegan { turn: 48 }];
        let events_before = events.clone();
        assert!(matches!(
            damage_monster(
                &mut direct,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Aeonglass damage catalog"))
        ));
        assert_eq!(direct, before);
        assert_eq!(events, events_before);

        let mut attack = state.clone();
        let attack_before = attack.clone();
        assert!(monster_attack_player(&mut attack, 0, 1, 1, &mut events).is_err());
        assert_eq!(attack, attack_before);
        assert_eq!(events, events_before);

        let mut malformed = state.clone();
        assert!(malformed.monsters_mut()[0].set_aeonglass_additional_strength(1));
        let malformed_before = malformed.clone();
        assert!(
            damage_monster_with_catalog(
                &mut malformed,
                &catalog,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut events,
            )
            .is_err()
        );
        assert_eq!(malformed, malformed_before);
        assert_eq!(events, events_before);

        let mut lethal = state;
        lethal.monsters_mut()[0].hp = 3;
        damage_monster_with_catalog(
            &mut lethal,
            &catalog,
            0,
            DotNetDecimal::from_i64(3),
            true,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.history.over);
        assert_eq!(lethal.monsters[0].hp, 0);
        assert_eq!(lethal.fanouts.withering_cards_left(), 6);
        assert!(super::super::cards::aeonglass_state_is_exact(
            &lethal, &catalog
        ));
    }

    fn dying_hopper_with_swipe() -> HotState {
        let mut state = HotState::at_defaults();
        state.exact_piles = true;
        state.next_card_uid = 2;
        let row = HopperDeckRow {
            card: HotCard {
                uid: 1,
                atom: 0,
                flags: 0,
            },
            state: CardInstanceState::default(),
        };
        state.card_states.set_hopper(Some(HopperDeckState {
            master: Vec::new(),
            history: vec![HopperLootEvent {
                kind: HopperLootKind::Stolen,
                row,
            }],
        }));
        let mut hopper = HotMonster::new(MonsterKind::ThievingHopper, 0);
        hopper.hp = 0;
        hopper.max_hp = super::super::monsters::THIEVING_HOPPER_HP;
        hopper.loop_pos = 1;
        hopper.powers.set(
            PowerId::EscapeArtist,
            SlotWire::Int,
            super::super::monsters::THIEVING_HOPPER_ESCAPE_ARTIST - 1,
        );
        assert!(hopper.set_hopper_swipe_uid(Some(1)));
        state.monsters_mut().push(hopper);
        assert!(super::super::monsters::hopper_dying_entry_is_exact(
            &state, 0
        ));
        state
    }

    fn hopper_test_catalog() -> Catalog {
        let mut builder = CatalogBuilder::new();
        builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder.build()
    }

    #[test]
    fn hopper_swipe_return_precedes_power_cleanup_and_monster_died_publication() {
        let mut state = dying_hopper_with_swipe();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -1);
        let mut events = Vec::new();

        assert_eq!(
            finish_monster_death_inner(&mut state, 0, None, &mut events),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        let deck = state.card_states.hopper().unwrap();
        assert_eq!(deck.master.len(), 1);
        assert!(matches!(
            deck.history.as_slice(),
            [
                HopperLootEvent {
                    kind: HopperLootKind::Stolen,
                    ..
                },
                HopperLootEvent {
                    kind: HopperLootKind::Returned,
                    ..
                }
            ]
        ));
        assert!(state.monsters[0].hopper_swipe_uid().is_none());
        assert_eq!(
            state.monsters[0].powers.value(PowerId::EscapeArtist),
            super::super::monsters::THIEVING_HOPPER_ESCAPE_ARTIST - 1
        );
        assert!(
            events.is_empty(),
            "MonsterDied is later than Swipe BeforeDeath"
        );
    }

    #[test]
    fn hopper_swipe_forgery_and_late_overflow_roll_back_public_death() {
        let catalog = hopper_test_catalog();
        let mut forged = dying_hopper_with_swipe();
        let stolen = forged.card_states.hopper().unwrap().history[0].row.clone();
        let mut forged_deck = forged.card_states.hopper().unwrap().clone();
        forged_deck.master.push(stolen);
        forged.card_states.set_hopper(Some(forged_deck));
        let forged_before = forged.clone();
        let mut forged_events = vec![Event::CombatOver { player_won: false }];
        let forged_events_before = forged_events.clone();
        assert!(
            finish_monster_death_with_catalog(&mut forged, &catalog, 0, &mut forged_events,)
                .is_err()
        );
        assert_eq!(forged, forged_before);
        assert_eq!(forged_events, forged_events_before);

        let mut overflow = dying_hopper_with_swipe();
        overflow.monsters_mut()[0]
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX);
        overflow.monsters_mut()[0]
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -1);
        let overflow_before = overflow.clone();
        let mut overflow_events = vec![Event::CombatOver { player_won: false }];
        let overflow_events_before = overflow_events.clone();
        assert_eq!(
            finish_monster_death_with_catalog(&mut overflow, &catalog, 0, &mut overflow_events,),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(overflow, overflow_before);
        assert_eq!(overflow_events, overflow_events_before);

        let mut knockdown = dying_hopper_with_swipe();
        knockdown.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Knockdown);
        let before = knockdown.clone();
        let mut events = vec![Event::CombatOver { player_won: false }];
        let events_before = events.clone();
        assert_eq!(
            finish_monster_death_with_catalog(&mut knockdown, &catalog, 0, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state"
            ))
        );
        assert_eq!(knockdown, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn hopper_public_attacks_reject_missing_or_orphan_sidecars_before_mutation() {
        let mut missing = HotState::at_defaults();
        missing.hp = 50;
        missing.block = 7;
        let mut hopper = HotMonster::new(
            MonsterKind::ThievingHopper,
            super::super::monsters::THIEVING_HOPPER_HP,
        );
        hopper.max_hp = super::super::monsters::THIEVING_HOPPER_HP;
        hopper.powers.set(
            PowerId::EscapeArtist,
            SlotWire::Int,
            super::super::monsters::THIEVING_HOPPER_ESCAPE_ARTIST,
        );
        missing.monsters_mut().push(hopper);
        let before = missing.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let events_before = events.clone();
        assert!(monster_attack_player(&mut missing, 0, 9, 1, &mut events).is_err());
        assert_eq!(missing, before);
        assert_eq!(events, events_before);

        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let spec = *catalog.spec(atom).unwrap();
        let mut orphan = HotState::at_defaults();
        orphan.hp = 50;
        orphan.exact_piles = true;
        orphan
            .card_states
            .set_hopper(Some(HopperDeckState::default()));
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.max_hp = 20;
        target.block = 5;
        orphan.monsters_mut().push(target);
        let before = orphan.clone();
        let mut events = vec![Event::TurnBegan { turn: 98 }];
        let events_before = events.clone();
        assert!(player_attack(&mut orphan, &spec, &[0], 6, 1, &mut events).is_err());
        assert_eq!(orphan, before);
        assert_eq!(events, events_before);

        assert!(
            damage_monster(
                &mut orphan,
                0,
                DotNetDecimal::from_i64(6),
                true,
                true,
                &mut events,
            )
            .is_err()
        );
        assert_eq!(orphan, before);
        assert_eq!(events, events_before);
    }

    fn gremlin_merc_lethal_failure_state() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 1);
        merc.max_hp = 51;
        state.monsters_mut().push(merc);
        // Surprise requires two Niche draws; an all-zero xoshiro row is not a
        // native stream and must fail before the lethal command publishes.
        state.rng.set(
            crate::hot::RngStream::Niche,
            crate::hot::RngStreamState {
                words: [0; 4],
                counter: 0,
            },
        );
        assert!(super::super::monsters::gremlin_merc_state_is_valid(&state));
        state
    }

    #[test]
    fn gremlin_merc_surprise_refusal_rolls_back_public_damage_and_direct_death() {
        let mut state = gremlin_merc_lethal_failure_state();
        let before = state.clone();
        let mut events = vec![Event::CombatOver { player_won: false }];
        let events_before = events.clone();
        assert!(matches!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Gremlin Merc Niche stream"))
        ));
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        let mut dead = before;
        dead.monsters_mut()[0].hp = 0;
        let dead_before = dead.clone();
        let mut death_events = vec![Event::CombatOver { player_won: false }];
        let death_events_before = death_events.clone();
        assert!(matches!(
            finish_monster_death(&mut dead, 0, &mut death_events),
            Err(EngineRefusal::MalformedArgs("Gremlin Merc Niche stream"))
        ));
        assert_eq!(dead, dead_before);
        assert_eq!(death_events, death_events_before);
    }

    #[test]
    fn debilitate_folds_after_vulnerable_peers_and_after_weak_base_only() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();

        let mut outgoing = HotState::at_defaults();
        outgoing.hp = 100;
        outgoing
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        outgoing.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 1);
        outgoing.monsters_mut()[0]
            .powers
            .set(PowerId::Debilitate, SlotWire::Int, 2);
        outgoing.powers.set(PowerId::Cruelty, SlotWire::Int, 20);
        player_attack(&mut outgoing, &source, &[0], 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(outgoing.monsters[0].hp, 76, "(1.5 + .2) -> 2.4");

        let mut incoming = HotState::at_defaults();
        incoming.hp = 100;
        let mut dealer = HotMonster::new(MonsterKind::Toadpole, 100);
        dealer.powers.set(PowerId::Weak, SlotWire::Int, 1);
        dealer.powers.set(PowerId::Debilitate, SlotWire::Int, 2);
        incoming.monsters_mut().push(dealer);
        monster_attack_player(&mut incoming, 0, 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(incoming.hp, 95, ".75 -> .5");

        let mut krane = HotState::at_defaults();
        krane.hp = 100;
        krane.set_paper_krane_owned(true);
        let mut weak_dealer = HotMonster::new(MonsterKind::Toadpole, 100);
        weak_dealer.powers.set(PowerId::Weak, SlotWire::Int, 1);
        krane.monsters_mut().push(weak_dealer);
        monster_attack_player(&mut krane, 0, 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(krane.hp, 94, "Paper Krane changes .75 to .6");

        damage_monster(
            &mut incoming,
            0,
            DotNetDecimal::from_i64(7),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            incoming.monsters[0].hp, 93,
            "unpowered damage bypasses both folds"
        );
    }

    #[test]
    fn knockdown_prevalidates_all_targets_and_multiplies_only_osty_attacks() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();

        for (amounts, expected) in [(&[2][..], 20), (&[3][..], 30), (&[2, 3][..], 60)] {
            let mut pet = HotState::at_defaults();
            pet.hp = 50;
            // A live Osty: a dead or absent attacker lands no hit (#3495).
            pet.fanouts.set_osty(Some((5, 5))).unwrap();
            let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
            for amount in amounts {
                target.misery_debuff_order.push_knockdown(*amount);
            }
            pet.monsters_mut().push(target);
            player_pet_attack_from_card(
                &mut pet,
                (&catalog, &source, 7),
                &[0],
                10,
                1,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(pet.monsters[0].hp, 100 - expected);
        }

        let mut owner = HotState::at_defaults();
        owner.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.misery_debuff_order.push_knockdown(2);
        target.misery_debuff_order.push_knockdown(3);
        owner.monsters_mut().push(target);
        player_attack(&mut owner, &source, &[0], 10, 1, &mut Vec::new()).unwrap();
        assert_eq!(owner.monsters[0].hp, 90, "the local applier receives x1");
        damage_monster(
            &mut owner,
            0,
            DotNetDecimal::from_i64(5),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            owner.monsters[0].hp, 85,
            "unpowered damage bypasses Knockdown"
        );

        let mut fractional = HotState::at_defaults();
        fractional.hp = 50;
        fractional.fanouts.set_osty(Some((5, 5))).unwrap();
        fractional.powers.set(PowerId::Tracking, SlotWire::Int, 50);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.powers.set(PowerId::Weak, SlotWire::Int, 1);
        target.misery_debuff_order.push_knockdown(2);
        fractional.monsters_mut().push(target);
        player_pet_attack_from_card(
            &mut fractional,
            (&catalog, &source, 7),
            &[0],
            1,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            fractional.monsters[0].hp, 97,
            "Tracking 3/2 and Knockdown x2 share one final truncation"
        );

        let mut overflowed = HotState::at_defaults();
        overflowed.hp = 50;
        overflowed.fanouts.set_osty(Some((5, 5))).unwrap();
        let mut target = HotMonster::new(MonsterKind::Toadpole, i32::MAX);
        for _ in 0..3 {
            target.misery_debuff_order.push_knockdown(i32::MAX);
        }
        overflowed.monsters_mut().push(target);
        let before = overflowed.clone();
        let mut events = vec![Event::CombatOver { player_won: false }];
        let events_before = events.clone();
        let result = player_pet_attack_from_card(
            &mut overflowed,
            (&catalog, &source, 7),
            &[0],
            i64::from(i32::MAX),
            1,
            &mut events,
        );
        assert!(
            matches!(result, Err(EngineRefusal::CounterOverflow(_))),
            "{result:?}"
        );
        assert_eq!(overflowed, before);
        assert_eq!(events, events_before);

        let mut malformed = HotState::at_defaults();
        malformed.hp = 50;
        malformed
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
        second.misery_debuff_order.push(MiseryToken::Knockdown);
        malformed.monsters_mut().push(second);
        let before = malformed.clone();
        let mut events = vec![Event::CombatOver { player_won: false }];
        let events_before = events.clone();
        assert_eq!(
            player_attack(&mut malformed, &source, &[0, 1], 10, 1, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state"
            ))
        );
        assert_eq!(malformed, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn knockdown_public_damage_and_attack_reject_scalar_or_dead_and_roll_back_late_death() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(catalog.atom(&identity).unwrap()).unwrap();

        for mut monster in {
            let mut scalar = HotMonster::new(MonsterKind::Toadpole, 20);
            scalar.powers.set(PowerId::Knockdown, SlotWire::Int, 2);
            let mut dead = HotMonster::new(MonsterKind::Toadpole, 0);
            dead.misery_debuff_order.push_knockdown(2);
            [scalar, dead]
        } {
            monster.uid = 4;
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.monsters_mut().push(monster);
            // A live bystander keeps the combat from ending, so the command
            // passes its `IsOverOrEnding` entry return (#3483) and reaches
            // the Knockdown validation.
            let mut bystander = HotMonster::new(MonsterKind::Toadpole, 20);
            bystander.uid = 5;
            bystander.slot = 1;
            state.monsters_mut().push(bystander);
            let before = state.clone();
            let mut events = vec![Event::CombatOver { player_won: false }];
            let events_before = events.clone();
            assert_eq!(
                player_attack(&mut state, &source, &[0], 1, 1, &mut events),
                Err(EngineRefusal::MalformedArgs(
                    "Knockdown distinct-instance state"
                ))
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);
            assert_eq!(
                damage_monster(
                    &mut state,
                    0,
                    DotNetDecimal::from_i64(1),
                    false,
                    false,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "Knockdown distinct-instance state"
                ))
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);
        }

        let mut lethal = HotState::at_defaults();
        lethal.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 1);
        target.misery_debuff_order.push_knockdown(2);
        lethal.monsters_mut().push(target);
        inject_legacy_bottom(&mut lethal, &catalog, identity, 1, PileId::Discard).unwrap();
        lethal.next_card_uid = u32::MAX;
        let before = lethal.clone();
        let mut events = vec![Event::CombatOver { player_won: false }];
        let events_before = events.clone();
        assert_eq!(
            damage_monster(
                &mut lethal,
                0,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(lethal, before);
        assert_eq!(events, events_before);

        let mut dying = HotState::at_defaults();
        dying.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 0);
        target.misery_debuff_order.push(MiseryToken::Weak);
        target.misery_debuff_order.push_knockdown(2);
        target.misery_debuff_order.push_knockdown(3);
        dying.monsters_mut().push(target);
        finish_monster_death(&mut dying, 0, &mut Vec::new()).unwrap();
        assert!(dying.monsters[0].misery_debuff_order.is_empty());
    }

    fn pet_attack_catalog() -> (Catalog, CardSpec, CardAtom) {
        let poke = CardIdentity {
            id: CardId::Poke,
            upgrade: 0,
            enchantment: None,
        };
        let flatten = CardIdentity {
            id: CardId::Flatten,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let poke_atom = builder.intern(poke).unwrap();
        let flatten_atom = builder.intern(flatten).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(poke_atom).unwrap();
        (catalog, source, flatten_atom)
    }

    #[test]
    fn double_damage_is_one_x2_for_owner_and_pet_and_joins_shared_floor() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut owner = HotState::at_defaults();
        owner.hp = 50;
        owner
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        owner.powers.set(PowerId::DoubleDamage, SlotWire::Int, 3);
        player_attack(&mut owner, &source, &[0], 5, 1, &mut Vec::new()).unwrap();
        assert_eq!(
            owner.monsters[0].hp, 90,
            "Amount is duration, not x2^Amount"
        );

        let mut pet = HotState::at_defaults();
        pet.hp = 50;
        // A live Osty: a dead or absent attacker lands no hit (#3495).
        pet.fanouts.set_osty(Some((5, 5))).unwrap();
        pet.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        pet.powers.set(PowerId::DoubleDamage, SlotWire::Int, 1);
        player_pet_attack_from_card(
            &mut pet,
            (&catalog, &source, 7),
            &[0],
            5,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(pet.monsters[0].hp, 90);

        let mut composed = HotState::at_defaults();
        composed.hp = 50;
        composed
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        composed.powers.set(PowerId::DoubleDamage, SlotWire::Int, 1);
        composed.powers.set(PowerId::PlayerWeak, SlotWire::Int, 1);
        player_attack(&mut composed, &source, &[0], 5, 1, &mut Vec::new()).unwrap();
        assert_eq!(composed.monsters[0].hp, 93, "5 * 2 * 3/4 floors once");
    }

    /// #3438: `PenNib::ModifyDamageMultiplicative` (`0x9917c` IL_0033-IL_0040)
    /// admits the owner's Osty as a dealer, so the doubled card's Osty body is
    /// x2 — composed with the receiver's Vulnerable, Poke's 6 is 18 (the
    /// Aeonglass `f0d7a23c7f949351` step-42 hit). Without the double the same
    /// hit is 9, and the pet's hit is never doubled by a play Pen Nib did not
    /// mark.
    #[test]
    fn pen_nib_doubles_the_marked_cards_osty_body() {
        let (catalog, source, _) = pet_attack_catalog();
        for (pen_double, vulnerable, expected_loss) in [
            (true, false, 12),
            (false, false, 6),
            (true, true, 18),
            (false, true, 9),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.fanouts.set_osty(Some((5, 5))).unwrap();
            let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
            if vulnerable {
                target.powers.set(PowerId::Vuln, SlotWire::Int, 2);
            }
            state.monsters_mut().push(target);
            crate::engine::play::with_test_active_play_pen_double(7, pen_double, || {
                player_pet_attack_from_card(
                    &mut state,
                    (&catalog, &source, 7),
                    &[0],
                    6,
                    1,
                    &mut Vec::new(),
                )
                .unwrap();
            });
            assert_eq!(
                100 - state.monsters[0].hp,
                expected_loss,
                "pen_double={pen_double} vulnerable={vulnerable}"
            );
        }
    }

    #[test]
    fn gigantification_binds_owner_attack_cards_issued_by_osty_only() {
        let mut builder = CatalogBuilder::new();
        let attack_atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let skill_atom = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();

        for (atom, uid, expected_hp, expected_stacks) in
            [(attack_atom, 7, 70, 0), (skill_atom, 8, 90, 1)]
        {
            let source = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.fanouts.set_osty(Some((5, 5))).unwrap();
            assert!(state.fanouts.set_gigantification(1));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });

            player_pet_attack_from_card(
                &mut state,
                (&catalog, &source, uid),
                &[0],
                10,
                1,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(state.monsters[0].hp, expected_hp, "{}", source.row.name);
            assert_eq!(
                state.fanouts.gigantification(),
                expected_stacks,
                "{}",
                source.row.name
            );
            assert!(!state.fanouts.gigantification_bound());
        }
    }

    #[test]
    fn pet_attack_uses_pet_modifiers_records_one_command_and_rewrites_flatten() {
        let (catalog, source, flatten_atom) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        state.powers.set(PowerId::Strength, SlotWire::Int, 20);
        state.powers.set(PowerId::Calcify, SlotWire::Int, 4);
        state.powers.set(PowerId::Accuracy, SlotWire::Int, 30);
        state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 1);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 91,
            atom: flatten_atom,
            flags: 0,
        });

        player_pet_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 40, "only Calcify adds to Osty");
        assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 0);
        assert_eq!(
            state.monsters[0].nonowner_same_side_powered_damage_results_this_turn, 1,
            "the complete pet command records PLAYER_PET rather than PLAYER history"
        );
        assert_eq!(state.fanouts.pet().attacks_this_turn(), 1);
        assert!(
            state.exact_piles,
            "Flatten's native mapper forces exact mode"
        );
        assert_eq!(
            state
                .card_states
                .get_ref(91)
                .unwrap()
                .local_cost_modifiers
                .as_slice(),
            &[LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisTurn,
                reduce_only: false,
            }]
        );
    }

    #[test]
    fn lethality_requires_both_first_play_index_and_owner_attack_threshold() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 7,
            atom,
            flags: 0,
        };

        for (play_index, started, expected_hp) in [(0, 1, 187), (1, 1, 193), (0, 2, 193)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 200));
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Vuln, SlotWire::Int, 1);
            state.powers.set(PowerId::Strength, SlotWire::Int, 1);
            state.powers.set(PowerId::PlayerWeak, SlotWire::Int, 1);
            state.powers.set(PowerId::Lethality, SlotWire::Int, 75);
            state.history.owner_attack_plays_started_this_turn = started;
            state.piles.get_mut(PileId::Play).make_mut().push(card);
            let active = crate::engine::play::ActivePlayGuard::enter(card.uid).unwrap();
            active.set_play_index(play_index).unwrap();

            player_attack_from_card(
                &mut state,
                (&catalog, &source, card.uid),
                &[0],
                6,
                1,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                state.monsters[0].hp, expected_hp,
                "both native CurrentPlayIndex zero and the owner Attack-start threshold are required before the shared Weak/Vulnerable floor"
            );
        }
    }

    #[test]
    fn one_for_all_uses_authenticated_spent_not_card_type_or_printed_cost() {
        fn physical_case(id: CardId, spent: i16) -> HotState {
            let mut builder = CatalogBuilder::new();
            let atom = builder
                .intern(CardIdentity {
                    id,
                    upgrade: 0,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            let source = *catalog.spec(atom).unwrap();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::OneForAll, SlotWire::Int, 3);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 20));
            state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
            crate::engine::play::with_test_active_play_spent(7, spent, || {
                player_attack_from_card(
                    &mut state,
                    (&catalog, &source, 7),
                    &[0],
                    5,
                    1,
                    &mut Vec::new(),
                )
                .unwrap();
            });
            state
        }

        let free_skill = physical_case(CardId::DefendIronclad, 0);
        assert_eq!(
            free_skill.monsters[0].hp, 12,
            "native does not gate One For All on the source CardModel.Type"
        );
        assert_eq!(
            physical_case(CardId::DefendIronclad, 1).monsters[0].hp,
            15,
            "a printed-cost Skill contributes only when this CardPlay spent zero"
        );
        assert_eq!(
            physical_case(CardId::Whirlwind, 0).monsters[0].hp,
            15,
            "CostsX is excluded even when its authenticated spend is zero"
        );
    }

    #[test]
    fn one_for_all_bare_source_cost_and_missing_play_context_fail_closed() {
        let free = source_spec(CardId::Claw);
        let paid = source_spec(CardId::StrikeIronclad);
        let mut direct = HotState::at_defaults();
        direct.hp = 50;
        direct.powers.set(PowerId::OneForAll, SlotWire::Int, 3);
        direct
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        player_attack(&mut direct, &free, &[0], 5, 1, &mut Vec::new()).unwrap();
        player_attack(&mut direct, &paid, &[0], 5, 1, &mut Vec::new()).unwrap();
        assert_eq!(direct.monsters[0].hp, 17);

        let (catalog, atom) = source_catalog(CardId::Claw);
        let source = *catalog.spec(atom).unwrap();
        let mut missing = HotState::at_defaults();
        missing.hp = 50;
        missing.powers.set(PowerId::OneForAll, SlotWire::Int, 3);
        missing
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        missing
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(HotCard {
                uid: 7,
                atom,
                flags: 0,
            });
        let before = missing.clone();
        let refusal = crate::engine::play::with_test_active_play(7, || {
            player_attack_from_card(
                &mut missing,
                (&catalog, &source, 7),
                &[0],
                5,
                1,
                &mut Vec::new(),
            )
        });
        assert_eq!(
            refusal,
            Err(EngineRefusal::MalformedArgs(
                "OneForAll active play context"
            ))
        );
        assert_eq!(missing, before);
    }

    #[test]
    fn lethality_rejects_the_later_attack_after_a_zero_hit_attack_prefix() {
        let helix = CardIdentity {
            id: CardId::HelixDrill,
            upgrade: 0,
            enchantment: None,
        };
        let lethality = CardIdentity {
            id: CardId::Lethality,
            upgrade: 0,
            enchantment: None,
        };
        let strike = CardIdentity {
            id: CardId::StrikeRegent,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let helix_atom = builder.intern(helix).unwrap();
        let lethality_atom = builder.intern(lethality).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.next_card_uid = 4;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: helix_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: lethality_atom,
                flags: 0,
            },
            HotCard {
                uid: 3,
                atom: strike_atom,
                flags: 0,
            },
        ]);

        play_card(&mut state, &catalog, 1, Some(0), None, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].hp, 100, "Helix Drill executes zero hits");
        assert_eq!(state.history.owner_attack_plays_started_this_turn, 1);
        play_card(&mut state, &catalog, 2, None, None, &mut Vec::new()).unwrap();
        assert_eq!(state.powers.value(PowerId::Lethality), 50);
        play_card(&mut state, &catalog, 3, Some(0), None, &mut Vec::new()).unwrap();

        assert_eq!(state.history.owner_attack_plays_started_this_turn, 2);
        assert_eq!(
            state.monsters[0].hp, 94,
            "the later Strike receives ordinary 6 damage, not Lethality's 9"
        );
    }

    #[test]
    fn lethality_nonattack_source_uses_play_index_not_attack_history() {
        let identity = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        assert!(source.is_skill && !source.is_attack);
        let card = HotCard {
            uid: 8,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state.powers.set(PowerId::Lethality, SlotWire::Int, 50);
        state.history.owner_attack_plays_started_this_turn = 1;
        state.history.attack_skill_plays_started_this_turn = i16::MAX;
        state.history.zero_energy_attack_plays_started_this_turn = i16::MAX;
        state.piles.get_mut(PileId::Play).make_mut().push(card);
        let active = crate::engine::play::ActivePlayGuard::enter(card.uid).unwrap();

        active.set_play_index(0).unwrap();
        player_attack_from_card(
            &mut state,
            (&catalog, &source, card.uid),
            &[0],
            7,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        active.set_play_index(1).unwrap();
        player_attack_from_card(
            &mut state,
            (&catalog, &source, card.uid),
            &[0],
            7,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 183, "10 then 7 damage");
        assert_eq!(
            state.history.owner_attack_plays_started_this_turn, 1,
            "the non-Attack source adds no owner Attack entry, so authenticated play index alone distinguishes its replay"
        );
    }

    #[test]
    fn lethality_has_no_dealer_or_card_type_gate_but_nonplay_history_is_exact() {
        let identity = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 8,
            atom,
            flags: 0,
        };

        for (started, expected_hp) in [(0, 190), (1, 193)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 200));
            state.fanouts.set_osty(Some((5, 5))).unwrap();
            state.powers.set(PowerId::Lethality, SlotWire::Int, 50);
            state.history.owner_attack_plays_started_this_turn = started;
            state.piles.get_mut(PileId::Discard).make_mut().push(card);

            player_pet_attack_from_card(
                &mut state,
                (&catalog, &source, card.uid),
                &[0],
                7,
                1,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.monsters[0].hp, expected_hp);
        }
    }

    #[test]
    fn lethality_refuses_invalid_amount_or_ambiguous_physical_source_atomically() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 9,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state.powers.set(PowerId::Lethality, SlotWire::Int, 50);
        state.history.owner_attack_plays_started_this_turn = 1;
        state.piles.get_mut(PileId::Play).make_mut().push(card);
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            player_attack_from_card(
                &mut state,
                (&catalog, &source, card.uid),
                &[0],
                6,
                1,
                &mut events,
            ),
            Err(EngineRefusal::ActiveCardNotUnique {
                uid: card.uid,
                matches: 2,
            })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        state = before;
        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.powers.set(PowerId::Lethality, SlotWire::Int, -1);
        let before = state.clone();
        assert_eq!(
            player_attack_from_card(
                &mut state,
                (&catalog, &source, card.uid),
                &[0],
                6,
                1,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("LethalityPower amount"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn lethality_play_source_refuses_missing_or_wrong_active_context_atomically() {
        let identity = CardIdentity {
            id: CardId::DefendIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let card = HotCard {
            uid: 9,
            atom,
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 200));
        state.powers.set(PowerId::Lethality, SlotWire::Int, 50);
        state.piles.get_mut(PileId::Play).make_mut().push(card);

        for wrong_uid in [None, Some(77)] {
            let active = wrong_uid
                .map(crate::engine::play::ActivePlayGuard::enter)
                .transpose()
                .unwrap();
            if let Some(active) = active.as_ref() {
                active.set_play_index(0).unwrap();
            }
            let before = state.clone();
            let mut events = Vec::new();
            assert_eq!(
                player_attack_from_card(
                    &mut state,
                    (&catalog, &source, card.uid),
                    &[0],
                    7,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "Lethality active play context"
                ))
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }

        let unset = crate::engine::play::ActivePlayGuard::enter(card.uid).unwrap();
        let before = state.clone();
        assert_eq!(
            player_attack_from_card(
                &mut state,
                (&catalog, &source, card.uid),
                &[0],
                7,
                1,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs(
                "Lethality active play context"
            ))
        );
        assert_eq!(state, before);
        drop(unset);

        let active = crate::engine::play::ActivePlayGuard::enter(card.uid).unwrap();
        assert!(matches!(
            crate::engine::play::ActivePlayGuard::enter(card.uid),
            Err(EngineRefusal::ContinuationNotModeled)
        ));
        active.set_play_index(0).unwrap();
    }

    #[test]
    fn lethal_pet_thorns_keeps_the_built_hit_and_sic_em_revives_before_target_death() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 6);
        target.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        target.powers.set(PowerId::SicEm, SlotWire::Int, 3);
        state.monsters_mut().push(target);
        // A live peer keeps the combat from ending, so the revive's Heal runs
        // (#3246; the ending variants are the two tests below).
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 20);
        peer.uid = 9;
        state.monsters_mut().push(peer);
        state.fanouts.set_osty(Some((2, 2))).unwrap();

        player_pet_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 0);
        assert_eq!(
            state.monsters[0].nonowner_same_side_powered_damage_results_this_turn, 1,
            "the hit dealer remains PLAYER_PET even when Thorns kills Osty before Sic Em"
        );
        assert_eq!(
            state
                .fanouts
                .pet()
                .osty()
                .map(|osty| (osty.hp(), osty.max_hp())),
            Some((3, 3)),
            "Sic Em observes the retained PLAYER_PET dealer before Kill"
        );
        assert_eq!(state.fanouts.pet().attacks_this_turn(), 1);
    }

    /// #3246 (BR2R60965GJ1 n13, fixture `fed545959329a810`): Osty's killing
    /// hit on the last enemy re-summons through Sic Em while the combat is
    /// ending. `GainMaxHp` (`0x3eb2f0`) raises MaxHp (IL_005b) and its Heal
    /// (IL_010b) returns early for the non-player Osty (`<Heal>d__20`
    /// `0x3eb4b0` IL_0041-IL_005f): native 16/23 -> 16/26. With a live peer
    /// the combat is not ending and the same hit grows both values.
    #[test]
    fn sic_em_on_the_last_enemy_raises_osty_max_hp_without_healing() {
        let (catalog, source, _) = pet_attack_catalog();
        for (live_peer, expected) in [(false, (16, 26)), (true, (19, 26))] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            let mut target = HotMonster::new(MonsterKind::Toadpole, 6);
            target.powers.set(PowerId::SicEm, SlotWire::Int, 3);
            state.monsters_mut().push(target);
            if live_peer {
                let mut peer = HotMonster::new(MonsterKind::Toadpole, 20);
                peer.uid = 9;
                state.monsters_mut().push(peer);
            }
            state.fanouts.set_osty(Some((16, 23))).unwrap();
            player_pet_attack_from_card(
                &mut state,
                (&catalog, &source, 7),
                &[0],
                6,
                1,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.monsters[0].hp, 0);
            assert_eq!(
                state
                    .fanouts
                    .pet()
                    .osty()
                    .map(|osty| (osty.hp(), osty.max_hp())),
                Some(expected),
                "live peer: {live_peer}"
            );
        }
    }

    /// #3246: Thorns kills Osty and the same hit kills the last enemy, so
    /// Sic Em reaches the revive branch of `<Summon>d__0` (`0x3ee040`) while
    /// the combat is ending: `SetMaxHp` (IL_0387) on the corpse, a Heal that
    /// returns early (IL_03f7), then `AfterOstyRevived` (IL_0468) over a
    /// still-dead pet. That branch is not represented and refuses by name.
    #[test]
    fn sic_em_revive_of_a_thorns_killed_osty_while_ending_refuses_by_name() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 6);
        target.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        target.powers.set(PowerId::SicEm, SlotWire::Int, 3);
        state.monsters_mut().push(target);
        state.fanouts.set_osty(Some((2, 2))).unwrap();
        assert_eq!(
            player_pet_attack_from_card(
                &mut state,
                (&catalog, &source, 7),
                &[0],
                6,
                1,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::EndingSummonNotModeled("Sic Em summon"))
        );
    }

    #[test]
    fn pet_history_overflow_refuses_the_complete_command_atomically() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.nonowner_same_side_powered_damage_results_this_turn = i32::MAX;
        state.monsters_mut().push(target);
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 7 }];
        let before_events = events.clone();

        assert_eq!(
            player_pet_attack_from_card(
                &mut state,
                (&catalog, &source, 7),
                &[0],
                6,
                1,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "nonowner_same_side_powered_damage_results_this_turn"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn necro_mastery_pet_thorns_uses_actual_loss_and_ordered_unpowered_damage() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.block = 1;
        let mut first = HotMonster::new(MonsterKind::Toadpole, 100);
        first.uid = 1;
        first.powers.set(PowerId::Thorns, SlotWire::Int, 3);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 100);
        second.uid = 2;
        second.block = 30;
        state.monsters_mut().extend([first, second]);
        state.fanouts.set_osty(Some((2, 2))).unwrap();
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 2);

        player_pet_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0],
            1,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.fanouts.pet().osty().is_none());
        assert_eq!(state.block, 0);
        assert_eq!((state.monsters[0].hp, state.monsters[1].hp), (95, 96));
        assert_eq!((state.monsters[0].block, state.monsters[1].block), (0, 30));
        assert_eq!(
            state.monsters[0].nonowner_same_side_powered_damage_results_this_turn, 1,
            "only the built pet hit is powered"
        );
        assert_eq!(
            state.monsters[1].nonowner_same_side_powered_damage_results_this_turn, 0,
            "Necro retaliation is unpowered"
        );
    }

    #[test]
    fn necro_mastery_interpose_runs_after_spill_and_before_death_and_paper_cuts() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        let mut scroll = HotMonster::new(MonsterKind::ScrollOfBiting, 3);
        scroll.uid = 1;
        let mut witness = HotMonster::new(MonsterKind::Toadpole, 100);
        witness.uid = 2;
        state.monsters_mut().extend([scroll, witness]);
        state.fanouts.set_osty(Some((5, 5))).unwrap();
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 1);

        assert_eq!(
            monster_attack_player_positive_results(&mut state, 0, 7, 1, &mut Vec::new()).unwrap(),
            1
        );
        assert_eq!(state.hp, 48, "the two-point spill commits first");
        assert_eq!(state.max_hp, 50, "Necro kills the Scroll before Paper Cuts");
        assert!(state.fanouts.pet().osty().is_none());
        assert_eq!((state.monsters[0].hp, state.monsters[1].hp), (-2, 95));
    }

    /// #2927: Horn, player Thorns, a 2 HP attacker and a live bystander,
    /// with one Strike to draw.
    fn horn_thorns_attack_state(
        atom: crate::catalog::CardAtom,
        horn: bool,
        thorns: i32,
        bystander: bool,
    ) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.fanouts.set_gremlin_horn_owned(horn);
        if thorns != 0 {
            state.powers.set(PowerId::Thorns, SlotWire::Int, thorns);
        }
        state.next_card_uid = 1;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 0,
            atom,
            flags: 0,
        });
        let mut dealer = HotMonster::new(MonsterKind::Toadpole, 2);
        dealer.uid = 1;
        state.monsters_mut().push(dealer);
        if bystander {
            let mut witness = HotMonster::new(MonsterKind::Toadpole, 100);
            witness.uid = 2;
            state.monsters_mut().push(witness);
        }
        state
    }

    /// #2927 (run LYE3ZK9FYKKV floors 43 and 48): a player-Thorns kill of the
    /// attacker reaches Gremlin Horn's AfterDeath, which needs the catalog to
    /// draw. `GremlinHorn/<AfterDeath>d__6::MoveNext` RVA `0x326170` (v111
    /// DLL 9cb4f1ad…) gates only on `target.Side != Owner.Creature.Side`
    /// (IL_0024–IL_003f), then `PlayerCmd::GainEnergy` (IL_0062) and
    /// `CardPileCmd::Draw` (IL_00d9). The catalogless entry refuses; the
    /// catalog-bearing one grants both exactly once and the dead attacker's
    /// later hits are cancelled (0x3ef17c IL_0156-0161, #799).
    /// #3172: a Juggernaut kill with Gremlin Horn owned. Card Block now
    /// carries the card's catalog into `AfterBlockGained`;
    /// `JuggernautPower/<AfterBlockGained>d__4` (`0x33dab4`) awaits one
    /// `CreatureCmd.Damage` on a `CombatTargets` roll (IL_0064-IL_00a2), and
    /// that death's AfterDeath walk reaches the Horn's GainEnergy + Draw
    /// instead of refusing "Gremlin Horn death catalog". Whichever Toadpole
    /// the roll picks dies and the other keeps combat running.
    #[test]
    fn juggernaut_kill_runs_gremlin_horn_with_the_card_catalog() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let mut state = horn_thorns_attack_state(atom, true, 0, true);
        state.monsters_mut()[1].hp = 2;
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let energy = state.energy;
        let defend = source_spec(CardId::DefendIronclad);
        assert_eq!(
            gain_powered_card_block(&mut state, &catalog, &defend, 5, &mut Vec::new()),
            Ok(5)
        );
        assert_eq!(
            state
                .monsters
                .iter()
                .filter(|monster| monster.hp <= 0)
                .count(),
            1
        );
        assert!(!state.history.over);
        assert_eq!(state.energy, energy + 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert!(state.piles.get(PileId::Draw).is_empty());
    }

    #[test]
    fn player_thorns_kill_with_gremlin_horn_needs_the_catalog_bearing_attack() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);

        let mut catalogless = horn_thorns_attack_state(atom, true, 3, true);
        assert_eq!(
            monster_attack_player_positive_results(&mut catalogless, 0, 5, 3, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("Gremlin Horn death catalog"))
        );

        for hits in [1, 3] {
            let mut state = horn_thorns_attack_state(atom, true, 3, true);
            let energy = state.energy;
            let mut events = Vec::new();
            assert_eq!(
                monster_attack_player_positive_results_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    5,
                    hits,
                    &mut events,
                ),
                Ok(1),
                "only the in-flight hit lands ({hits} hits)"
            );
            assert_eq!(state.hp, 45);
            assert!(state.monsters[0].hp <= 0);
            assert_eq!(state.monsters[1].hp, 100);
            assert_eq!(state.energy, energy + 1, "Horn energy exactly once");
            assert_eq!(state.piles.get(PileId::Hand).len(), 1, "Horn draws once");
            assert!(state.piles.get(PileId::Draw).is_empty());
            assert!(!state.history.over);
            assert_eq!(
                events
                    .iter()
                    .filter(|e| matches!(e, Event::PlayerDamaged { .. }))
                    .count(),
                1
            );
        }

        // Last enemy: the kill ends combat, so neither Horn command runs.
        let mut terminal = horn_thorns_attack_state(atom, true, 3, false);
        let energy = terminal.energy;
        monster_attack_player_positive_results_with_catalog(
            &mut terminal,
            &catalog,
            0,
            5,
            3,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(terminal.history.over);
        assert_eq!(terminal.energy, energy);
        assert!(terminal.piles.get(PileId::Hand).is_empty());
    }

    /// #2927 moved fifteen move bodies from the catalogless attack entry to
    /// the catalog-bearing one. Where no catalog reader is live (no Horn,
    /// Puzzle, Hopper, Dampen or Aeonglass), the two must be the same
    /// command: identical result, state and events over plain, multi-hit,
    /// Thorns-retaliation, Thorns-kill, terminal and player-lethal shapes.
    #[test]
    fn catalog_and_catalogless_monster_attacks_agree_without_a_catalog_reader() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let shapes: [(i32, bool, i64, i64, i32); 7] = [
            // (thorns, bystander, base, hits, player hp)
            (0, true, 5, 1, 50),
            (0, true, 4, 3, 50),
            (1, true, 4, 3, 50),
            (3, true, 5, 3, 50),
            (3, false, 5, 3, 50),
            (0, true, 9, 2, 10),
            (1, false, 9, 2, 5),
        ];
        for (thorns, bystander, base, hits, hp) in shapes {
            let mut entry = horn_thorns_attack_state(atom, false, thorns, bystander);
            entry.hp = hp;
            let mut plain = entry.clone();
            let mut plain_events = Vec::new();
            let plain_result = monster_attack_player_positive_results(
                &mut plain,
                0,
                base,
                hits,
                &mut plain_events,
            );
            let mut with = entry.clone();
            let mut with_events = Vec::new();
            let with_result = monster_attack_player_positive_results_with_catalog(
                &mut with,
                &catalog,
                0,
                base,
                hits,
                &mut with_events,
            );
            let shape = (thorns, bystander, base, hits, hp);
            assert!(plain_result.is_ok(), "{shape:?}");
            assert_eq!(plain_result, with_result, "{shape:?}");
            assert_eq!(plain, with, "{shape:?}");
            assert_eq!(plain_events, with_events, "{shape:?}");
        }
    }

    #[test]
    fn split_necro_mastery_keeps_catalog_through_horn_and_cancels_later_attacks() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.fanouts.set_osty(Some((3, 3))).unwrap();
        state.fanouts.set_gremlin_horn_owned(true);
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 1);
        state.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
        state.next_card_uid = 1;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 0,
            atom,
            flags: 0,
        });
        let mut dealer = HotMonster::new(MonsterKind::Toadpole, 2);
        dealer.uid = 1;
        let mut witness = HotMonster::new(MonsterKind::Toadpole, 100);
        witness.uid = 2;
        state.monsters_mut().extend([dealer, witness]);
        let mut events = Vec::new();
        monster_attack_player_with_catalog(&mut state, &catalog, 0, 5, 3, &mut events).unwrap();
        assert_eq!(
            state.hp, 48,
            "in-flight spill only, then dead attacker stops later hits"
        );
        assert_eq!(state.monsters[1].hp, 97);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::PlayerDamaged { .. }))
                .count(),
            1
        );
        assert!(state.fanouts.pet().corpse());
    }

    #[test]
    fn necro_mastery_late_refusal_rolls_back_the_complete_monster_attack() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut dealer = HotMonster::new(MonsterKind::Toadpole, 100);
        dealer.uid = 1;
        state.monsters_mut().push(dealer);
        state.fanouts.set_osty(Some((3, 3))).unwrap();
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, -1);
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 4 }];
        let before_events = events.clone();

        assert_eq!(
            monster_attack_player_positive_results(&mut state, 0, 5, 1, &mut events),
            Err(EngineRefusal::MalformedArgs("necro mastery amount"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn osty_split_result_runs_all_player_order_permutations_once_even_at_zero_spill() {
        use crate::hot::AfterDamageReceivedPower::{FlameBarrier, Reflect, TheGambit};
        for order in [
            [FlameBarrier, Reflect, TheGambit],
            [FlameBarrier, TheGambit, Reflect],
            [Reflect, FlameBarrier, TheGambit],
            [Reflect, TheGambit, FlameBarrier],
            [TheGambit, FlameBarrier, Reflect],
            [TheGambit, Reflect, FlameBarrier],
        ] {
            for (pet_hp, attack, expected_hp, expected_pet) in
                [(8, 5, 50, Some(5)), (3, 5, 50, None), (3, 7, 30, None)]
            {
                let mut state = HotState::at_defaults();
                state.hp = 50;
                state.max_hp = 100;
                state.block = 2;
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                state.fanouts.set_osty(Some((pet_hp, pet_hp))).unwrap();
                state.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
                state.powers.set(PowerId::Reflect, SlotWire::Int, 1);
                state.powers.set(PowerId::TheGambit, SlotWire::Int, 1);
                assert!(state.fanouts.set_potion_belt(
                    vec![Some(PotionId::FairyInABottle)],
                    false,
                    false,
                    false,
                    false,
                    true
                ));
                assert!(state.set_after_damage_received_power_order(&order));
                let mut events = Vec::new();
                monster_attack_player(&mut state, 0, attack, 1, &mut events).unwrap();
                assert_eq!(state.hp, expected_hp, "{order:?}/{pet_hp}/{attack}");
                assert_eq!(state.monsters[0].hp, 94, "each player retaliation once");
                assert_eq!(state.fanouts.pet().osty().map(|p| p.hp()), expected_pet);
                assert_eq!(
                    events
                        .iter()
                        .filter(|e| matches!(e, Event::PlayerDamaged { .. }))
                        .count(),
                    1
                );
                assert_eq!(
                    state.powers.value(PowerId::TheGambit),
                    if attack == 7 { 0 } else { 1 }
                );
            }
        }
    }

    #[test]
    fn emotion_history_uses_original_block_result_when_osty_absorbs_the_spill() {
        for (block, expected) in [(2, true), (5, false)] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.block = block;
            state.set_batch_six_deep_relic_ownership(false, true, false, false);
            state.fanouts.set_osty(Some((10, 10))).unwrap();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 30));
            monster_attack_player(&mut state, 0, 5, 1, &mut Vec::new()).unwrap();
            assert_eq!(state.hp, 50);
            assert_eq!(state.emotion_damage_current_turn(), expected);
        }
    }

    #[test]
    fn lethal_split_result_skips_received_even_when_final_kill_is_prevented() {
        for pet in [false, true] {
            for fairy in [false, true] {
                let mut state = HotState::at_defaults();
                state.hp = 2;
                state.max_hp = 100;
                if pet {
                    state.fanouts.set_osty(Some((3, 3))).unwrap();
                }
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                state.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
                state
                    .fanouts
                    .set_batch_eight_deep_relic_ownership(true, false, false, !fairy, false);
                if fairy {
                    assert!(state.fanouts.set_potion_belt(
                        vec![Some(PotionId::FairyInABottle)],
                        false,
                        false,
                        false,
                        false,
                        true
                    ));
                }
                monster_attack_player(&mut state, 0, 8, 1, &mut Vec::new()).unwrap();
                assert_eq!(state.hp, if fairy { 30 } else { 50 });
                assert_eq!(
                    state.monsters[0].hp, 100,
                    "lethal result never reaches Flame Barrier"
                );
                assert_eq!(
                    state.fanouts.beating_remnant_damage_received(),
                    0,
                    "lethal result never reaches Beating Remnant"
                );
                assert!(!state.history.over);
                assert!(!state.fanouts.player_hooks_deactivated());
            }
        }
    }

    #[test]
    fn queued_retained_osty_kill_removes_revived_hp_and_dispatches_a_fresh_loss() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.fanouts.set_osty(Some((3, 3))).unwrap();
        state
            .fanouts
            .mutate_pet(|p| p.lose_hp(3).map(|_| ()))
            .unwrap();
        assert!(state.fanouts.pet().corpse());
        // Native queue retains this one object's identity across Summon.
        state.fanouts.mutate_pet(|p| p.summon(5)).unwrap();
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 2);
        let mut events = Vec::new();
        finish_queued_osty_death(&mut state, None, &mut events).unwrap();
        assert!(state.fanouts.pet().corpse());
        assert!(state.fanouts.pet().has_die_for_you());
        assert_eq!(state.monsters[0].hp, 90);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::MonsterDamaged { unblocked: 10, .. }))
                .count(),
            1
        );
    }

    #[test]
    fn pet_interpose_uses_owner_block_and_intangible_and_player_result_listeners() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.block = 2;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.fanouts.set_osty(Some((3, 3))).unwrap();

        assert_eq!(
            monster_attack_player_positive_results(&mut state, 0, 10, 1, &mut Vec::new()).unwrap(),
            1
        );
        assert_eq!((state.block, state.hp), (0, 45));
        assert!(state.fanouts.pet().osty().is_none());

        let mut intangible = HotState::at_defaults();
        intangible.hp = 50;
        intangible
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        intangible.fanouts.set_osty(Some((3, 3))).unwrap();
        intangible.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        assert_eq!(
            monster_attack_player_positive_results(&mut intangible, 0, 10, 1, &mut Vec::new(),)
                .unwrap(),
            0
        );
        assert_eq!(intangible.hp, 50);
        assert_eq!(intangible.fanouts.pet().osty().unwrap().hp(), 2);

        for power in [PowerId::FlameBarrier, PowerId::Reflect, PowerId::TheGambit] {
            let mut composed = intangible.clone();
            composed.powers.set(power, SlotWire::Int, 1);
            monster_attack_player_positive_results(&mut composed, 0, 10, 1, &mut Vec::new())
                .unwrap();
            assert_eq!(composed.hp, 50, "pet absorbs the capped result");
            assert_eq!(composed.fanouts.pet().osty().unwrap().hp(), 1);
            assert_eq!(
                composed.monsters[0].hp,
                if power == PowerId::FlameBarrier {
                    19
                } else {
                    20
                }
            );
            assert_eq!(
                composed.powers.value(power),
                1,
                "zero-spill Gambit does not trigger"
            );
        }
    }

    fn ceremonial_beast_for_plow(hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        let mut beast = HotMonster::new(MonsterKind::CeremonialBeast, hp);
        beast.max_hp = 262;
        beast.loop_pos = 1;
        beast.powers.set(PowerId::PlowThreshold, SlotWire::Int, 160);
        beast.powers.set(PowerId::Strength, SlotWire::Int, 7);
        beast.powers.set(PowerId::TempStrength, SlotWire::Int, -2);
        state.monsters_mut().push(beast);
        state
    }

    fn intangible_soul_fysh() -> HotState {
        let mut state = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::SoulFysh, 20);
        owner.max_hp = crate::engine::monsters::SOUL_FYSH_HP;
        owner.loop_pos = 4;
        owner.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        state.monsters_mut().push(owner);
        state
    }

    #[test]
    fn monster_intangible_caps_before_block_for_blockable_and_unblockable_damage() {
        let mut blockable = intangible_soul_fysh();
        blockable.monsters_mut()[0].block = 3;
        assert_eq!(
            damage_monster(
                &mut blockable,
                0,
                DotNetDecimal::from_i64(9),
                false,
                true,
                &mut Vec::new(),
            )
            .unwrap(),
            0
        );
        assert_eq!(
            (blockable.monsters[0].hp, blockable.monsters[0].block),
            (20, 2)
        );

        let mut unblockable = intangible_soul_fysh();
        unblockable.monsters_mut()[0].block = 3;
        assert_eq!(
            damage_monster(
                &mut unblockable,
                0,
                DotNetDecimal::from_i64(9),
                false,
                false,
                &mut Vec::new(),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            (unblockable.monsters[0].hp, unblockable.monsters[0].block),
            (19, 3)
        );
    }

    #[test]
    fn lethal_monster_intangible_hit_runs_ordinary_owner_death_cleanup() {
        let mut state = intangible_soul_fysh();
        state.monsters_mut()[0].hp = 1;
        let mut events = Vec::new();

        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(99),
                false,
                true,
                &mut events,
            )
            .unwrap(),
            1
        );
        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Intangible), 0);
        assert!(state.history.over);
        assert_eq!(
            events,
            vec![
                Event::MonsterDamaged {
                    uid: 0,
                    blocked: 0,
                    unblocked: 1,
                    hp: 0,
                },
                Event::MonsterDied { uid: 0 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn malformed_monster_intangible_refuses_before_damage_state_or_events() {
        for mutation in ["owner", "amount", "loop"] {
            let mut state = intangible_soul_fysh();
            match mutation {
                "owner" => state.monsters_mut()[0].kind = MonsterKind::Vantom,
                "amount" => {
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::Intangible, SlotWire::Int, 3)
                }
                "loop" => state.monsters_mut()[0].loop_pos = 3,
                _ => unreachable!(),
            }
            let before = state.clone();
            let mut events = Vec::new();
            assert!(matches!(
                damage_monster(
                    &mut state,
                    0,
                    DotNetDecimal::from_i64(9),
                    false,
                    true,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "monster Intangible owner/state"
                ))
            ));
            assert_eq!(state, before, "{mutation}");
            assert!(events.is_empty(), "{mutation}");
        }
    }

    #[test]
    fn ceremonial_beast_plow_fires_after_positive_unblocked_threshold_damage() {
        let mut state = ceremonial_beast_for_plow(161);
        let mut events = Vec::new();
        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                true,
                &mut events,
            )
            .unwrap(),
            1
        );
        let beast = &state.monsters[0];
        assert_eq!(beast.hp, 160);
        for power in [
            PowerId::PlowThreshold,
            PowerId::Strength,
            PowerId::TempStrength,
        ] {
            assert_eq!(beast.powers.value(power), 0);
        }
        assert_eq!(beast.override_state, MonsterOverride::BeastStun);
        assert_eq!(beast.forced_follow_up, MonsterFollowUp::BeastCry);
    }

    #[test]
    fn ceremonial_beast_plow_preserves_blocked_above_threshold_and_lethal_paths() {
        let mut blocked = ceremonial_beast_for_plow(161);
        blocked.monsters_mut()[0].block = 1;
        damage_monster(
            &mut blocked,
            0,
            DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(blocked.monsters[0].hp, 161);
        assert_eq!(
            blocked.monsters[0].powers.value(PowerId::PlowThreshold),
            160
        );
        assert_eq!(blocked.monsters[0].override_state, MonsterOverride::None);

        let mut above = ceremonial_beast_for_plow(162);
        damage_monster(
            &mut above,
            0,
            DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(above.monsters[0].hp, 161);
        assert_eq!(above.monsters[0].powers.value(PowerId::PlowThreshold), 160);

        let mut lethal = ceremonial_beast_for_plow(161);
        damage_monster(
            &mut lethal,
            0,
            DotNetDecimal::from_i64(200),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.monsters[0].hp <= 0);
        assert_eq!(lethal.monsters[0].override_state, MonsterOverride::None);
        assert_eq!(lethal.monsters[0].forced_follow_up, MonsterFollowUp::None);
    }

    #[test]
    fn malformed_plow_owner_refuses_before_damage_or_event_publication() {
        let mut state = ceremonial_beast_for_plow(161);
        let mut duplicate = HotMonster::new(MonsterKind::CeremonialBeast, 262);
        duplicate.max_hp = 262;
        duplicate.slot = 1;
        duplicate.uid = 1;
        state.monsters_mut().push(duplicate);
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                true,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Ceremonial Beast Plow owner/state"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #3366 (f63aba9763362bf1, A6): below A9 the installed Plow amount is 150
    /// (`get_PlowAmount` `0xb0c33`). The owner preflight accepts that tier's
    /// amount, the crossing hit stuns, and the A9+ amount now refuses there.
    #[test]
    fn below_a9_plow_owner_accepts_the_tiers_threshold_and_refuses_the_a9_one() {
        let below_a9 = |plow: i32, hp: i32| {
            let mut state = ceremonial_beast_for_plow(hp);
            assert!(state.fanouts.set_ascension(6));
            let beast = &mut state.monsters_mut()[0];
            beast.max_hp = 252;
            beast
                .powers
                .set(PowerId::PlowThreshold, SlotWire::Int, plow);
            state
        };

        let mut above = below_a9(150, 152);
        damage_monster(
            &mut above,
            0,
            DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(above.monsters[0].hp, 151);
        assert_eq!(above.monsters[0].powers.value(PowerId::PlowThreshold), 150);
        assert_eq!(above.monsters[0].override_state, MonsterOverride::None);

        let mut crossing = below_a9(150, 151);
        damage_monster(
            &mut crossing,
            0,
            DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        let beast = &crossing.monsters[0];
        assert_eq!(beast.hp, 150);
        assert_eq!(beast.powers.value(PowerId::PlowThreshold), 0);
        assert_eq!(beast.powers.value(PowerId::Strength), 0);
        assert_eq!(beast.override_state, MonsterOverride::BeastStun);
        assert_eq!(beast.forced_follow_up, MonsterFollowUp::BeastCry);
        assert!(super::super::monsters::ceremonial_beast_state_is_valid(
            &crossing
        ));

        let mut wrong_tier = below_a9(160, 200);
        let before = wrong_tier.clone();
        let mut events = Vec::new();
        assert_eq!(
            damage_monster(
                &mut wrong_tier,
                0,
                DotNetDecimal::from_i64(1),
                false,
                true,
                &mut events
            ),
            Err(EngineRefusal::MalformedArgs(
                "Ceremonial Beast Plow owner/state"
            ))
        );
        assert_eq!(wrong_tier, before);
        assert!(events.is_empty());
    }

    #[test]
    fn nested_after_damage_given_kill_runs_before_plow_and_publishes_no_follow_up() {
        let mut state = ceremonial_beast_for_plow(161);
        state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 1);
        state
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 160);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::MonarchsGaze])
        );
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.history.over);
        let beast = &state.monsters[0];
        assert!(beast.hp <= 0);
        assert_eq!(beast.powers.value(PowerId::PlowThreshold), 0);
        assert_eq!(beast.powers.value(PowerId::Strength), 9);
        assert_eq!(beast.powers.value(PowerId::TempStrength), 0);
        assert_eq!(beast.override_state, MonsterOverride::None);
        assert_eq!(beast.forced_follow_up, MonsterFollowUp::None);
    }

    #[test]
    fn ordinary_death_preserves_ritual_but_test_subject_form_reset_clears_it() {
        let mut ordinary = HotState::at_defaults();
        let mut cultist = HotMonster::new(MonsterKind::CalcifiedCultist, 0);
        cultist.powers.set(PowerId::Ritual, SlotWire::Int, 2);
        cultist.ritual_fresh = true;
        ordinary.monsters_mut().push(cultist);
        finish_monster_death(&mut ordinary, 0, &mut Vec::new()).unwrap();
        assert_eq!(ordinary.monsters[0].powers.value(PowerId::Ritual), 2);
        assert!(ordinary.monsters[0].ritual_fresh);

        let mut subject = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::TestSubject, 0);
        owner.max_hp = super::super::monsters::TEST_SUBJECT_FIRST_HP;
        owner.loop_pos = 1;
        owner.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
        owner.powers.set(PowerId::Enrage, SlotWire::Int, 3);
        owner.powers.set(PowerId::Ritual, SlotWire::Int, 2);
        subject.monsters_mut().push(owner);
        finish_monster_death(&mut subject, 0, &mut Vec::new()).unwrap();
        assert_eq!(subject.monsters[0].powers.value(PowerId::Ritual), 0);
        assert_eq!(subject.monsters[0].powers.value(PowerId::Enrage), 0);
        assert!(subject.monsters[0].test_subject_adaptable_reviving());
        assert!(!subject.monsters[0].ritual_fresh);
    }

    #[test]
    fn shrink_after_death_tracks_each_exact_applier_direction() {
        let mut beetle_death = HotState::at_defaults();
        beetle_death.hp = 50;
        beetle_death
            .powers
            .set(PowerId::PlayerShrink, SlotWire::Int, 1);
        beetle_death
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ShrinkerBeetle, 0));
        let mut events = Vec::new();
        finish_monster_death(&mut beetle_death, 0, &mut events).unwrap();
        assert_eq!(beetle_death.powers.value(PowerId::PlayerShrink), 0);
        assert!(events.contains(&Event::PowerChanged {
            subject: Subject::Player,
            power: PowerId::PlayerShrink,
            amount: 0,
        }));

        let mut player_death = HotState::at_defaults();
        player_death.hp = 1;
        player_death
            .powers
            .set(PowerId::PlayerShrink, SlotWire::Int, 1);
        player_death
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ShrinkerBeetle, 20));
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.uid = 1;
        target.powers.set(PowerId::Shrink, SlotWire::Int, 4);
        target.misery_debuff_order.push(MiseryToken::Shrink);
        player_death.monsters_mut().push(target);
        let mut events = Vec::new();
        doom_kill_player(&mut player_death, None, &mut events).unwrap();
        assert!(player_death.history.over);
        assert_eq!(player_death.powers.value(PowerId::PlayerShrink), 0);
        assert_eq!(player_death.monsters[1].powers.value(PowerId::Shrink), 0);
        assert!(
            player_death.monsters[1]
                .misery_debuff_order
                .as_slice()
                .is_empty()
        );
    }
    use crate::ids::{CardId, MonsterKind};

    fn source_spec(id: CardId) -> CardSpec {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        *catalog.spec(atom).unwrap()
    }

    fn source_catalog(id: CardId) -> (crate::catalog::Catalog, crate::catalog::CardAtom) {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(CardIdentity {
                id,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        (builder.build(), atom)
    }

    fn entomancer_attack_state(
        hive: i32,
        hp: i32,
        block: i32,
    ) -> (crate::catalog::Catalog, crate::catalog::CardAtom, HotState) {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some()
        );

        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.next_card_uid = 10;
        state.next_generated_hook_uid = 20;
        state.history.owner_generated_cards_combat = 7;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });
        for uid in [7, 8] {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        let mut owner = HotMonster::new(MonsterKind::Entomancer, hp);
        owner.max_hp = super::super::monsters::ENTOMANCER_HP;
        owner.block = block;
        owner.loop_pos = 0;
        owner.powers.set(PowerId::Hive, SlotWire::Int, hive);
        state.monsters_mut().push(owner);
        (catalog, atom, state)
    }

    #[test]
    fn personal_hive_generates_serial_random_dazed_after_normal_and_blocked_hits() {
        let (catalog, atom, mut normal) = entomancer_attack_state(2, 165, 0);
        let source = *catalog.spec(atom).unwrap();
        let rng_entering = normal.rng.get(RngStream::Rng);
        let rng_before = rng_entering.counter;
        let mut events = Vec::new();

        player_attack_from_card(&mut normal, (&catalog, &source, 9), &[0], 6, 1, &mut events)
            .unwrap();

        assert_eq!((normal.monsters[0].hp, normal.monsters[0].block), (159, 0));
        assert_eq!(normal.rng.get(RngStream::Rng).counter, rng_before + 2);
        assert_eq!(normal.next_card_uid, 12);
        assert_eq!(normal.next_generated_hook_uid, 22);
        assert_eq!(normal.history.owner_generated_cards_combat, 7);
        let dazed_atom = catalog
            .atom(&CardIdentity {
                id: CardId::Dazed,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let dazed: Vec<_> = normal
            .piles
            .get(PileId::Draw)
            .as_slice()
            .iter()
            .filter(|card| card.atom == dazed_atom)
            .map(|card| card.uid)
            .collect();
        assert_eq!(dazed.len(), 2);
        let mut expected_rng = Xoshiro256StarStar {
            words: rng_entering.words,
            counter: rng_entering.counter,
        };
        let mut expected_uids = vec![7, 8];
        for uid in [10, 11] {
            let bound = i32::try_from(expected_uids.len() + 1).unwrap();
            let index = usize::try_from(expected_rng.next_bounded(bound).unwrap()).unwrap();
            expected_uids.insert(index, uid);
        }
        assert_eq!(
            normal
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            expected_uids
        );
        assert_eq!(
            events[0],
            Event::MonsterDamaged {
                uid: 0,
                blocked: 0,
                unblocked: 6,
                hp: 159,
            }
        );
        assert_eq!(
            events[1..],
            [
                Event::CardResolved {
                    uid: 10,
                    pile: PileId::Draw,
                },
                Event::CardResolved {
                    uid: 11,
                    pile: PileId::Draw,
                },
            ]
        );

        let (catalog, atom, mut blocked) = entomancer_attack_state(1, 165, 99);
        let source = *catalog.spec(atom).unwrap();
        player_attack_from_card(
            &mut blocked,
            (&catalog, &source, 9),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            (blocked.monsters[0].hp, blocked.monsters[0].block),
            (165, 93)
        );
        assert_eq!(blocked.next_card_uid, 11);
        assert_eq!(blocked.next_generated_hook_uid, 21);
    }

    #[test]
    fn personal_hive_skips_a_lethal_owner_and_its_null_dazed_never_fire_smokestack() {
        let (catalog, atom, mut lethal) = entomancer_attack_state(3, 5, 0);
        let source = *catalog.spec(atom).unwrap();
        let before_rng = lethal.rng.get(RngStream::Rng);
        player_attack_from_card(
            &mut lethal,
            (&catalog, &source, 9),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.monsters[0].hp <= 0);
        assert_eq!(lethal.next_card_uid, 10);
        assert_eq!(lethal.next_generated_hook_uid, 20);
        assert_eq!(lethal.rng.get(RngStream::Rng), before_rng);

        // #3256 (DXLGWV6KZ1BF floor 27): Personal Hive passes a null
        // `creator` (`PersonalHivePower/<AfterDamageReceived>d__6` RVA
        // `0x340220` IL_00cd-IL_00d1), and Smokestack leaves on a null creator
        // (`SmokestackPower/<AfterCardGeneratedForCombat>d__4` RVA `0x3455e8`
        // IL_0033-IL_004e). A one-HP Entomancer therefore survives all three
        // Dazed: no Smokestack damage and no terminal stop.
        let (catalog, atom, mut quiet) = entomancer_attack_state(3, 1, 0);
        let source = *catalog.spec(atom).unwrap();
        quiet.powers.set(PowerId::Smokestack, SlotWire::Int, 5);
        assert!(
            quiet
                .fanouts
                .set_local_generated_power_order(&[PowerId::Smokestack])
        );
        let rng_before = quiet.rng.get(RngStream::Rng).counter;
        player_attack_from_card(
            &mut quiet,
            (&catalog, &source, 9),
            &[0],
            0,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!quiet.history.over);
        assert_eq!(quiet.monsters[0].hp, 1);
        assert_eq!(quiet.next_card_uid, 13);
        assert_eq!(quiet.next_generated_hook_uid, 23);
        assert_eq!(quiet.rng.get(RngStream::Rng).counter, rng_before + 3);
    }

    #[test]
    fn personal_hive_dead_owner_guard_is_independent_from_terminal_state() {
        let (catalog, _, mut state) = entomancer_attack_state(2, 1, 0);
        state.monsters_mut()[0].hp = 0;
        state.history.over = false;
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 77,
            pile: PileId::Exhaust,
        }];
        let events_before = events.clone();

        apply_personal_hive_after_powered_hit(&mut state, Some(&catalog), 0, &mut events).unwrap();

        assert!(
            !before.history.over,
            "terminal state must not mask the owner guard"
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn personal_hive_rehearses_a_late_dazed_failure_before_the_powered_hit() {
        let (catalog, atom, mut state) = entomancer_attack_state(3, 165, 0);
        let source = *catalog.spec(atom).unwrap();
        state.block = 999_999_998;
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        // #3256: Pillar of Creation leaves on Hive's null creator
        // (`PillarOfCreationPower/<AfterCardGeneratedForCombat>d__6` RVA
        // `0x340488` IL_001d-IL_0038), so Block one short of its cap neither
        // refuses nor moves.
        let mut quiet = state.clone();
        player_attack_from_card(
            &mut quiet,
            (&catalog, &source, 9),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(quiet.block, 999_999_998);
        assert_eq!(quiet.next_card_uid, 13);

        // The third Dazed's uid allocation overflows: the rehearsal refuses
        // before the powered hit or either earlier Dazed publishes.
        state.next_card_uid = u32::MAX - 2;
        let before = state.clone();
        let mut events = vec![Event::CardResolved {
            uid: 77,
            pile: PileId::Exhaust,
        }];
        let events_before = events.clone();

        assert_eq!(
            player_attack_from_card(&mut state, (&catalog, &source, 9), &[0], 6, 1, &mut events,),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn curl_up_latches_the_first_powered_physical_card_even_when_fully_blocked() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Play).make_mut().extend([
            HotCard {
                uid: 7,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 8,
                atom,
                flags: 0,
            },
        ]);
        let mut louse = HotMonster::new(MonsterKind::LouseProgenitor, 100);
        louse.block = 99;
        louse.powers.set(PowerId::CurlUp, SlotWire::Int, 18);
        state.monsters_mut().push(louse);
        let mut events = Vec::new();

        player_attack_from_card(&mut state, (&catalog, &spec, 7), &[0], 6, 1, &mut events).unwrap();
        assert_eq!((state.monsters[0].hp, state.monsters[0].block), (100, 93));
        assert_eq!(state.monsters[0].curl_up_card_uid, 7);

        player_attack_from_card(&mut state, (&catalog, &spec, 7), &[0], 1, 1, &mut events).unwrap();
        player_attack_from_card(&mut state, (&catalog, &spec, 8), &[0], 1, 1, &mut events).unwrap();
        assert_eq!(state.monsters[0].curl_up_card_uid, 7);
    }

    #[test]
    fn curl_up_rejects_missing_physical_provenance_atomically_and_clears_on_death() {
        let mut malformed = HotState::at_defaults();
        let mut louse = HotMonster::new(MonsterKind::LouseProgenitor, 10);
        louse.powers.set(PowerId::CurlUp, SlotWire::Int, 18);
        malformed.monsters_mut().push(louse);
        let before = malformed.clone();
        let refusal = damage_monster(
            &mut malformed,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(
            refusal,
            EngineRefusal::MalformedArgs("CurlUp physical card source")
        );
        assert_eq!(malformed, before);

        let mut builder = CatalogBuilder::new();
        let strike_atom = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend_atom = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let wrong_spec = *catalog.spec(defend_atom).unwrap();
        let mut mismatched = HotState::at_defaults();
        mismatched
            .piles
            .get_mut(PileId::Play)
            .make_mut()
            .push(HotCard {
                uid: 8,
                atom: strike_atom,
                flags: 0,
            });
        let mut louse = HotMonster::new(MonsterKind::LouseProgenitor, 10);
        louse.powers.set(PowerId::CurlUp, SlotWire::Int, 18);
        mismatched.monsters_mut().push(louse);
        let before = mismatched.clone();
        assert_eq!(
            player_attack_from_card(
                &mut mismatched,
                (&catalog, &wrong_spec, 8),
                &[0],
                1,
                1,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs("CurlUp physical card source"))
        );
        assert_eq!(mismatched, before);

        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();
        let mut lethal = HotState::at_defaults();
        lethal.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });
        let mut louse = HotMonster::new(MonsterKind::LouseProgenitor, 1);
        louse.set_louse_curled(true);
        louse.powers.set(PowerId::CurlUp, SlotWire::Int, 18);
        lethal.monsters_mut().push(louse);
        player_attack_from_card(
            &mut lethal,
            (&catalog, &spec, 9),
            &[0],
            1,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(lethal.monsters[0].hp <= 0);
        assert_eq!(lethal.monsters[0].powers.value(PowerId::CurlUp), 0);
        assert_eq!(lethal.monsters[0].curl_up_card_uid, -1);
        assert!(!lethal.monsters[0].louse_curled());
    }

    #[test]
    fn misery_scalar_order_reconstructs_before_append_and_preserves_stacks() {
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);

        apply_monster_debuff(&mut monster, PowerId::Weak, MiseryToken::Weak, 2);
        assert_eq!(monster.misery_debuff_order.as_slice(), [MiseryToken::Weak]);

        apply_monster_debuff(&mut monster, PowerId::Weak, MiseryToken::Weak, 3);
        assert_eq!(monster.powers.value(PowerId::Weak), 5);
        assert_eq!(monster.misery_debuff_order.as_slice(), [MiseryToken::Weak]);

        apply_monster_debuff(&mut monster, PowerId::Poison, MiseryToken::Poison, 4);
        assert_eq!(
            monster.misery_debuff_order.as_slice(),
            [MiseryToken::Weak, MiseryToken::Poison]
        );

        let mut legacy = HotMonster::new(MonsterKind::Toadpole, 20);
        legacy.powers.set(PowerId::Weak, SlotWire::Int, 2);
        apply_monster_debuff(&mut legacy, PowerId::Weak, MiseryToken::Weak, 3);
        assert_eq!(legacy.powers.value(PowerId::Weak), 5);
        assert_eq!(legacy.misery_debuff_order.as_slice(), [MiseryToken::Weak]);
    }

    #[test]
    fn misery_order_preserves_knockdown_scalar_interleavings_and_instances() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));

        apply_card_knockdown(&mut state, 0, 2, &mut Vec::new()).unwrap();
        apply_monster_debuff(
            &mut state.monsters_mut()[0],
            PowerId::Weak,
            MiseryToken::Weak,
            1,
        );
        apply_card_knockdown(&mut state, 0, 3, &mut Vec::new()).unwrap();
        assert_eq!(
            state.monsters[0].misery_debuff_order.as_slice(),
            [
                MiseryToken::Knockdown,
                MiseryToken::Weak,
                MiseryToken::Knockdown,
            ]
        );
        assert_eq!(state.monsters[0].misery_debuff_order.knockdown(), [2, 3]);
        assert_eq!(
            misery_scalar_snapshot(&state.monsters[0]).unwrap(),
            [
                MiseryScalarSnapshot {
                    power: PowerId::Knockdown,
                    token: MiseryToken::Knockdown,
                    amount: 2,
                },
                MiseryScalarSnapshot {
                    power: PowerId::Weak,
                    token: MiseryToken::Weak,
                    amount: 1,
                },
                MiseryScalarSnapshot {
                    power: PowerId::Knockdown,
                    token: MiseryToken::Knockdown,
                    amount: 3,
                },
            ]
        );

        let mut reverse = HotState::at_defaults();
        let mut legacy = HotMonster::new(MonsterKind::Toadpole, 20);
        legacy.powers.set(PowerId::Vuln, SlotWire::Int, 2);
        reverse.monsters_mut().push(legacy);
        apply_card_knockdown(&mut reverse, 0, 2, &mut Vec::new()).unwrap();
        assert_eq!(
            reverse.monsters[0].misery_debuff_order.as_slice(),
            [MiseryToken::Vuln, MiseryToken::Knockdown]
        );
        assert_eq!(reverse.monsters[0].misery_debuff_order.knockdown(), [2]);
    }

    #[test]
    fn artifact_suppresses_knockdown_before_unrelated_misery_order_reconstruction() {
        let mut state = HotState::at_defaults();
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);
        monster.powers.set(PowerId::Weak, SlotWire::Int, 2);
        monster.powers.set(PowerId::Vuln, SlotWire::Int, 3);
        monster.powers.set(PowerId::Artifact, SlotWire::Int, 1);
        state.monsters_mut().push(monster);
        let mut events = Vec::new();

        apply_card_knockdown(&mut state, 0, 2, &mut events).unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 3);
        assert!(state.monsters[0].misery_debuff_order.is_empty());
        assert!(state.monsters[0].misery_debuff_order.knockdown().is_empty());
        assert_eq!(
            events,
            [Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Artifact,
                amount: 0,
            }]
        );
    }

    #[test]
    fn misery_scalar_order_stays_unchanged_when_the_multiset_is_unreconstructible() {
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 999_999_999);
        waterfall.powers.set(PowerId::Poison, SlotWire::Int, -1);

        apply_monster_debuff(&mut waterfall, PowerId::Weak, MiseryToken::Weak, 5);

        assert_eq!(waterfall.powers.value(PowerId::Poison), -1);
        assert_eq!(waterfall.powers.value(PowerId::Weak), 5);
        assert!(waterfall.misery_debuff_order.is_empty());

        let mut malformed = HotMonster::new(MonsterKind::Toadpole, 20);
        malformed.powers.set(PowerId::Weak, SlotWire::Int, 2);
        malformed.misery_debuff_order.push(MiseryToken::Vuln);
        apply_monster_debuff(&mut malformed, PowerId::Poison, MiseryToken::Poison, 4);
        assert_eq!(malformed.powers.value(PowerId::Poison), 4);
        assert_eq!(
            malformed.misery_debuff_order.as_slice(),
            [MiseryToken::Vuln]
        );

        let mut ambiguous = HotMonster::new(MonsterKind::Toadpole, 20);
        ambiguous.powers.set(PowerId::Weak, SlotWire::Int, 2);
        ambiguous.powers.set(PowerId::Poison, SlotWire::Int, 4);
        apply_monster_debuff(&mut ambiguous, PowerId::Doom, MiseryToken::Doom, 3);
        assert_eq!(ambiguous.powers.value(PowerId::Doom), 3);
        assert!(ambiguous.misery_debuff_order.is_empty());

        let mut mismatched = HotMonster::new(MonsterKind::Toadpole, 20);
        apply_monster_debuff(&mut mismatched, PowerId::Weak, MiseryToken::Poison, 2);
        assert_eq!(mismatched.powers.value(PowerId::Weak), 2);
        assert!(mismatched.misery_debuff_order.is_empty());
    }

    #[test]
    fn misery_zero_and_poison_removal_do_not_leave_or_invent_order_entries() {
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);
        apply_monster_debuff(&mut monster, PowerId::Weak, MiseryToken::Weak, 0);
        assert_eq!(monster.powers.value(PowerId::Weak), 0);
        assert!(monster.misery_debuff_order.is_empty());

        monster.powers.set(PowerId::Poison, SlotWire::Int, 1);
        monster.poison_uid = 0;
        monster.misery_debuff_order.push(MiseryToken::Poison);
        let mut state = HotState::at_defaults();
        state.next_poison_uid = 1;
        state.monsters_mut().push(monster);
        let mut events = Vec::new();

        assert!(tick_monster_poison(&mut state, &mut events).unwrap());
        assert_eq!(state.monsters[0].hp, 19);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
        assert!(state.monsters[0].misery_debuff_order.is_empty());
    }

    #[test]
    fn accelerant_freezes_the_poison_tick_count_but_each_tick_reads_live_amount() {
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.powers.set(PowerId::Poison, SlotWire::Int, 4);
        monster.poison_uid = 0;
        monster.misery_debuff_order.push(MiseryToken::Poison);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.next_poison_uid = 1;
        state.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        state.monsters_mut().push(monster);

        assert!(tick_monster_poison(&mut state, &mut Vec::new()).unwrap());
        assert_eq!(state.monsters[0].hp, 21, "4 + 3 + 2 live poison");
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 1);

        state
            .powers
            .set(PowerId::Accelerant, SlotWire::Int, i32::MAX);
        state.monsters_mut()[0].hp = 30;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        assert!(tick_monster_poison(&mut state, &mut Vec::new()).unwrap());
        assert_eq!(state.monsters[0].hp, 27);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
    }

    #[test]
    fn poison_series_never_retargets_a_waterfall_or_axebot_replacement() {
        let mut waterfall_state = HotState::at_defaults();
        waterfall_state.hp = 50;
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 4);
        waterfall.max_hp = 250;
        waterfall.uid = 17;
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 20);
        waterfall.powers.set(PowerId::Poison, SlotWire::Int, 4);
        waterfall.poison_uid = 0;
        waterfall.misery_debuff_order.push(MiseryToken::Poison);
        waterfall_state
            .powers
            .set(PowerId::Accelerant, SlotWire::Int, 2);
        waterfall_state.next_poison_uid = 1;
        waterfall_state.monsters_mut().push(waterfall);

        assert!(tick_monster_poison(&mut waterfall_state, &mut Vec::new()).unwrap());
        let waterfall = &waterfall_state.monsters[0];
        assert_eq!(waterfall.uid, 17);
        assert_eq!(waterfall.hp, 999_999_994);
        assert_eq!(waterfall.powers.value(PowerId::Poison), 0);
        assert!(waterfall.is_about_to_blow());

        let mut axebot_state = HotState::at_defaults();
        axebot_state.hp = 50;
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 4);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        axebot.powers.set(PowerId::Poison, SlotWire::Int, 4);
        axebot.poison_uid = 0;
        axebot.misery_debuff_order.push(MiseryToken::Poison);
        axebot_state
            .powers
            .set(PowerId::Accelerant, SlotWire::Int, 2);
        axebot_state.next_poison_uid = 1;
        axebot_state.monsters_mut().push(axebot);
        let niche_before = axebot_state.rng.get(RngStream::Niche).counter;

        assert!(tick_monster_poison(&mut axebot_state, &mut Vec::new()).unwrap());
        let axebot = &axebot_state.monsters[0];
        assert_eq!(axebot.uid, 1);
        assert_eq!(axebot.powers.value(PowerId::Stock), 1);
        assert_eq!(axebot.powers.value(PowerId::Poison), 0);
        assert_eq!(
            axebot_state.rng.get(RngStream::Niche).counter,
            niche_before + 1
        );

        let mut dead_owner = HotState::at_defaults();
        dead_owner.hp = 0;
        dead_owner.multiplayer_ally_key = 1;
        dead_owner
            .fanouts
            .set_multiplayer_ally(crate::hot::MultiplayerAllyState {
                key: 1,
                alive: true,
                ..crate::hot::MultiplayerAllyState::default()
            });
        dead_owner.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        dead_owner.next_poison_uid = 1;
        let mut poisoned = HotMonster::new(MonsterKind::Toadpole, 30);
        poisoned.powers.set(PowerId::Poison, SlotWire::Int, 4);
        poisoned.poison_uid = 0;
        poisoned.misery_debuff_order.push(MiseryToken::Poison);
        dead_owner.monsters_mut().push(poisoned);

        assert!(tick_monster_poison(&mut dead_owner, &mut Vec::new()).unwrap());
        assert_eq!(dead_owner.monsters[0].hp, 26);
        assert_eq!(dead_owner.monsters[0].powers.value(PowerId::Poison), 3);

        let mut observed = HotState::at_defaults();
        observed.hp = 50;
        observed.powers.set(PowerId::Accelerant, SlotWire::Int, 2);
        observed
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, 1);
        observed.next_poison_uid = 1;
        assert!(
            observed
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let mut poisoned = HotMonster::new(MonsterKind::Toadpole, 30);
        poisoned.powers.set(PowerId::Poison, SlotWire::Int, 4);
        poisoned.poison_uid = 0;
        poisoned.misery_debuff_order.push(MiseryToken::Poison);
        observed.monsters_mut().push(poisoned);

        let mut events = Vec::new();
        assert!(tick_monster_poison(&mut observed, &mut events).unwrap());
        assert_eq!(observed.monsters[0].hp, 21, "null-applier ticks deal 4+3+2");
        assert_eq!(observed.monsters[0].powers.value(PowerId::Poison), 1);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    Event::PowerChanged {
                        subject: Subject::Monster(_),
                        power: PowerId::Poison,
                        ..
                    }
                ))
                .count(),
            3
        );
    }

    #[test]
    fn public_poison_tick_rolls_back_an_earlier_owner_before_late_death_refusal() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.next_poison_uid = 2;
        let mut first = HotMonster::new(MonsterKind::Toadpole, 30);
        first.max_hp = 30;
        first.uid = 10;
        first.powers.set(PowerId::Poison, SlotWire::Int, 3);
        first.poison_uid = 0;
        first.misery_debuff_order.push(MiseryToken::Poison);
        let mut late = HotMonster::new(MonsterKind::Axebot, 2);
        late.max_hp = 76;
        late.slot = 1;
        late.uid = 11;
        late.powers.set(PowerId::Stock, SlotWire::Int, 2);
        late.powers.set(PowerId::Poison, SlotWire::Int, 2);
        late.poison_uid = 1;
        late.misery_debuff_order.push(MiseryToken::Poison);
        state.monsters_mut().extend([first, late]);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 71 }];
        let before_events = events.clone();

        assert_eq!(
            tick_monster_poison(&mut state, &mut events),
            Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn an_empty_target_pool_consumes_no_rng_draw() {
        let mut state = HotState::at_defaults();
        let before = state.rng.get(RngStream::Targets);

        assert_eq!(roll_target(&mut state).unwrap(), None);
        assert_eq!(state.rng.get(RngStream::Targets), before);
    }

    #[test]
    fn a_single_live_target_still_consumes_one_rng_draw() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 0));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 7));
        let before = state.rng.get(RngStream::Targets);

        assert_eq!(roll_target(&mut state).unwrap(), Some(1));

        let after = state.rng.get(RngStream::Targets);
        assert_eq!(after.counter, before.counter + 1);
        assert_ne!(after, before);
    }

    #[test]
    fn inferno_fans_a_surviving_hp_loss_across_living_enemies() {
        let mut state = HotState::at_defaults();
        state.hp = 20;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 20);
        target.block = 2;
        state.monsters_mut().push(target);
        state.powers.set(PowerId::Inferno, SlotWire::Int, 6);
        let mut events = Vec::new();

        assert!(damage_player_from_card(&mut state, 1, false, &mut events).unwrap());

        assert_eq!(state.hp, 19);
        assert_eq!(state.monsters[0].block, 0);
        assert_eq!(state.monsters[0].hp, 16);
    }

    #[test]
    fn lethal_player_damage_does_not_fire_inferno() {
        let mut state = HotState::at_defaults();
        state.hp = 1;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::Inferno, SlotWire::Int, 6);
        let mut events = Vec::new();

        assert!(!damage_player_from_card(&mut state, 1, false, &mut events).unwrap());

        assert_eq!(state.hp, 0);
        assert_eq!(state.monsters[0].hp, 20);
    }

    /// #3274 witness and control. `InfernoPower/<AfterDamageReceived>d__8`
    /// (`0x33d084`) awaits ONE `CreatureCmd::Damage` over `HittableEnemies`
    /// (IL_00be-IL_00e1); `<Damage>d__12` (`0x3e96c8`) commits every HP loss
    /// (through IL_0aa4) before its one Kill (IL_0eb4). When both enemies
    /// die, Gremlin Horn's `<AfterDeath>d__6` (`0x326170`) at the first death
    /// already sees the combat ending, so it grants no energy and draws
    /// nothing; the per-enemy walk this replaces paid both at the first
    /// death. The control keeps one enemy alive, so its one death pays out.
    #[test]
    fn inferno_kills_as_one_batch_so_gremlin_horn_sees_the_ending() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        for (second_hp, pays_out) in [(2, false), (100, true)] {
            let mut state = horn_thorns_attack_state(atom, true, 0, true);
            state.monsters_mut()[1].hp = second_hp;
            state.powers.set(PowerId::Inferno, SlotWire::Int, 5);
            let energy = state.energy;
            let mut events = Vec::new();

            assert_eq!(
                damage_player_from_card_with_catalog(&mut state, &catalog, 1, false, &mut events),
                Ok(pays_out),
                "second_hp={second_hp}"
            );

            assert_eq!(state.hp, 49, "second_hp={second_hp}");
            assert!(state.monsters[0].hp <= 0, "second_hp={second_hp}");
            assert_eq!(state.history.over, !pays_out, "second_hp={second_hp}");
            if pays_out {
                assert_eq!(state.monsters[1].hp, 95);
                assert_eq!(state.energy, energy + 1);
                assert_eq!(state.piles.get(PileId::Hand).len(), 1);
                assert!(state.piles.get(PileId::Draw).is_empty());
            } else {
                assert!(state.monsters[1].hp <= 0);
                assert_eq!(state.energy, energy, "IsEnding: no Horn energy");
                assert!(state.piles.get(PileId::Hand).is_empty(), "no Horn draw");
                assert_eq!(state.piles.get(PileId::Draw).len(), 1);
                assert!(events.contains(&Event::CombatOver { player_won: true }));
            }
            // Both HP losses are committed before the first death.
            let first_death = events
                .iter()
                .position(|event| matches!(event, Event::MonsterDied { .. }))
                .unwrap();
            assert_eq!(
                events[..first_death]
                    .iter()
                    .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                    .count(),
                2,
                "second_hp={second_hp}"
            );
        }
    }

    /// #3274 witness for the catalogless batch branch
    /// (`damage_monsters_with_optional_catalog` with `None`): a card-damage
    /// entry without a catalog still batches Inferno over both enemies and
    /// ends the combat once, after both commits.
    #[test]
    fn catalogless_inferno_batches_every_living_enemy() {
        let mut state = HotState::at_defaults();
        state.hp = 20;
        for (uid, hp) in [(1, 3), (2, 3)] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
            monster.uid = uid;
            state.monsters_mut().push(monster);
        }
        state.powers.set(PowerId::Inferno, SlotWire::Int, 3);
        let mut events = Vec::new();

        assert!(!damage_player_from_card(&mut state, 1, false, &mut events).unwrap());

        assert_eq!(state.hp, 19);
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert!(state.history.over);
        let first_death = events
            .iter()
            .position(|event| matches!(event, Event::MonsterDied { .. }))
            .unwrap();
        assert_eq!(
            events[..first_death]
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::CombatOver { .. }))
                .count(),
            1
        );
    }

    /// #3274: a catalogless Inferno kill with Gremlin Horn owned still
    /// refuses by name on the batch path, as the per-enemy walk did.
    #[test]
    fn catalogless_inferno_kill_with_gremlin_horn_refuses() {
        let (_, atom) = source_catalog(CardId::StrikeIronclad);
        let mut state = horn_thorns_attack_state(atom, true, 0, true);
        state.monsters_mut()[1].hp = 2;
        state.powers.set(PowerId::Inferno, SlotWire::Int, 5);
        assert_eq!(
            damage_player_from_card(&mut state, 1, false, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("Gremlin Horn death catalog"))
        );
    }

    #[test]
    fn intangible_caps_before_block_and_buffer_consumes_only_positive_loss() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.block = 1;
        state.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        state.powers.set(PowerId::Buffer, SlotWire::Int, 2);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        let mut events = Vec::new();

        // Intangible closes the powered snapshot before block: only one
        // Block is spent, and a fully blocked result does not consume Buffer.
        monster_attack_player(&mut state, 0, 50, 1, &mut events).unwrap();
        assert_eq!(state.hp, 50);
        assert_eq!(state.block, 0);
        assert_eq!(state.powers.value(PowerId::Buffer), 2);

        // The next two integral positive results are capped to one and each
        // consume one Buffer stack; only the third reaches HP.
        monster_attack_player(&mut state, 0, 50, 3, &mut events).unwrap();
        assert_eq!(state.hp, 49);
        assert_eq!(state.powers.value(PowerId::Buffer), 0);
        assert_eq!(state.history.player_unblocked_damage_results_combat, 1);
    }

    #[test]
    fn scroll_paper_cuts_follows_each_positive_hit_with_exact_max_hp_loss() {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ScrollOfBiting, 39));
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 6, 2, &mut events).unwrap();

        assert_eq!((state.hp, state.max_hp), (88, 96));
        assert_eq!(state.history.player_unblocked_damage_results_combat, 2);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::PlayerDamaged { .. }))
                .count(),
            2,
            "HP stayed below each new cap, so Paper Cuts emitted no nested damage"
        );
    }

    #[test]
    fn scroll_paper_cuts_nested_damage_is_unblockable_and_modifier_visible() {
        let mut state = HotState::at_defaults();
        state.hp = 10;
        state.max_hp = 10;
        state.block = 4;
        state.powers.set(PowerId::Intangible, SlotWire::Int, 1);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ScrollOfBiting, 39));
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 6, 1, &mut events).unwrap();

        assert_eq!((state.hp, state.max_hp, state.block), (10, 10, 3));
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PlayerDamaged {
                        blocked, hp_lost, ..
                    } => Some((*blocked, *hp_lost)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [(1, 0)],
            "fully blocked powered hit cannot trigger Paper Cuts"
        );

        // Remove block: Intangible caps the powered hit and the following
        // unpowered/unblockable max-HP damage independently.
        state.block = 0;
        events.clear();
        monster_attack_player(&mut state, 0, 6, 1, &mut events).unwrap();
        assert_eq!((state.hp, state.max_hp), (8, 8));
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    Event::PlayerDamaged {
                        blocked, hp_lost, ..
                    } => Some((*blocked, *hp_lost)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [(0, 1), (0, 1)]
        );
    }

    #[test]
    fn scroll_paper_cuts_respects_owner_and_player_lethal_boundaries() {
        let mut owner_dead = HotState::at_defaults();
        owner_dead.hp = 10;
        owner_dead.max_hp = 10;
        owner_dead.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        owner_dead
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ScrollOfBiting, 1));
        let mut events = Vec::new();

        monster_attack_player(&mut owner_dead, 0, 3, 2, &mut events).unwrap();
        assert!(owner_dead.monsters[0].hp <= 0);
        assert_eq!((owner_dead.hp, owner_dead.max_hp), (7, 10));

        let mut player_dead = HotState::at_defaults();
        player_dead.hp = 2;
        player_dead.max_hp = 2;
        player_dead
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ScrollOfBiting, 39));
        events.clear();

        monster_attack_player(&mut player_dead, 0, 1, 2, &mut events).unwrap();
        assert!(player_dead.history.over);
        assert_eq!((player_dead.hp, player_dead.max_hp), (0, 1));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::PlayerDamaged { .. }))
                .count(),
            2,
            "the nested max-HP result ends combat before hit two"
        );

        let mut ordinary = HotState::at_defaults();
        ordinary.hp = 10;
        ordinary.max_hp = 10;
        ordinary
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        events.clear();
        monster_attack_player(&mut ordinary, 0, 1, 1, &mut events).unwrap();
        assert_eq!((ordinary.hp, ordinary.max_hp), (9, 10));
    }

    #[test]
    fn colossus_halves_only_a_vulnerable_dealers_powered_hit() {
        let mut protected = HotState::at_defaults();
        protected.hp = 100;
        protected.powers.set(PowerId::PlayerVuln, SlotWire::Int, 1);
        protected.powers.set(PowerId::Colossus, SlotWire::Int, 1);
        let mut vulnerable = HotMonster::new(MonsterKind::Toadpole, 30);
        vulnerable.powers.set(PowerId::Vuln, SlotWire::Int, 1);
        protected.monsters_mut().push(vulnerable);
        let mut events = Vec::new();

        monster_attack_player(&mut protected, 0, 8, 1, &mut events).unwrap();
        assert_eq!(protected.hp, 94); // floor(8 * 3/2 * 1/2)

        let mut plain = HotState::at_defaults();
        plain.hp = 100;
        plain.powers.set(PowerId::PlayerVuln, SlotWire::Int, 1);
        plain.powers.set(PowerId::Colossus, SlotWire::Int, 1);
        plain
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        monster_attack_player(&mut plain, 0, 8, 1, &mut events).unwrap();
        assert_eq!(plain.hp, 88);
    }

    #[test]
    fn player_thorns_freezes_the_in_flight_hit_and_cancels_later_hits() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.powers.set(PowerId::Thorns, SlotWire::Int, 3);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 5, 3, &mut events).unwrap();

        assert!(state.history.over);
        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.hp, 45); // hit one lands after lethal retaliation
    }

    #[test]
    fn reflect_flame_barrier_and_gambit_run_their_exact_result_suffixes() {
        let mut flame = HotState::at_defaults();
        flame.hp = 50;
        flame.block = 99;
        flame.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
        flame
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let mut events = Vec::new();
        monster_attack_player(&mut flame, 0, 5, 2, &mut events).unwrap();
        assert_eq!(flame.hp, 50);
        assert_eq!(flame.monsters[0].hp, 12);

        let mut reflect = HotState::at_defaults();
        reflect.hp = 50;
        reflect.block = 7;
        reflect.powers.set(PowerId::Reflect, SlotWire::Int, 1);
        reflect
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        monster_attack_player(&mut reflect, 0, 5, 2, &mut events).unwrap();
        assert_eq!(reflect.hp, 47);
        assert_eq!(reflect.monsters[0].hp, 13);

        let mut gambit = HotState::at_defaults();
        gambit.hp = 50;
        gambit.block = 4;
        gambit.powers.set(PowerId::TheGambit, SlotWire::Int, 1);
        gambit
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        monster_attack_player(&mut gambit, 0, 5, 1, &mut events).unwrap();
        assert!(gambit.history.over);
        assert_eq!(gambit.hp, 0);
        assert_eq!(gambit.powers.value(PowerId::TheGambit), 0);
    }

    #[test]
    fn hit_power_order_controls_the_terminal_retaliation_gambit_race() {
        use crate::hot::AfterDamageReceivedPower::{FlameBarrier, TheGambit};
        let build = |order: &[crate::hot::AfterDamageReceivedPower]| {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
            state.powers.set(PowerId::TheGambit, SlotWire::Int, 1);
            assert!(state.set_after_damage_received_power_order(order));
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 3));
            state
        };

        let mut retaliation_first = build(&[FlameBarrier, TheGambit]);
        monster_attack_player(&mut retaliation_first, 0, 1, 1, &mut Vec::new()).unwrap();
        assert!(retaliation_first.monsters[0].hp <= 0);
        assert_eq!(retaliation_first.hp, 0);
        assert_eq!(retaliation_first.powers.value(PowerId::TheGambit), 0);

        let mut gambit_first = build(&[TheGambit, FlameBarrier]);
        monster_attack_player(&mut gambit_first, 0, 1, 1, &mut Vec::new()).unwrap();
        assert_eq!(gambit_first.monsters[0].hp, 3);
        assert_eq!(gambit_first.hp, 0);
        assert_eq!(gambit_first.powers.value(PowerId::TheGambit), 0);
    }

    #[test]
    fn malformed_hit_order_refuses_whole_attack_without_events() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
        state.powers.set(PowerId::Reflect, SlotWire::Int, 1);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        let events_before = events.clone();

        assert!(matches!(
            monster_attack_player_positive_results(&mut state, 0, 5, 1, &mut events),
            Err(EngineRefusal::PowerOrderNotModeled(
                "after-damage-received power order"
            ))
        ));
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn gambit_kill_wins_after_player_thorns_kills_the_final_attacker() {
        let mut state = HotState::at_defaults();
        state.hp = 10;
        state.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        state.powers.set(PowerId::TheGambit, SlotWire::Int, 1);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 5, 1, &mut events).unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert!(state.history.over);
        assert_eq!(state.hp, 0);
        assert_eq!(state.powers.value(PowerId::TheGambit), 0);
    }

    #[test]
    fn rupture_is_immediate_for_owner_side_null_sources_only() {
        let mut owner_side = HotState::at_defaults();
        owner_side.hp = 50;
        owner_side.powers.set(PowerId::Rupture, SlotWire::Int, 1);
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 100);
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        owner_side.monsters_mut().push(thorny);
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut owner_side, &source, &[0], 5, 2, &mut events).unwrap();
        assert_eq!(owner_side.hp, 46);
        assert_eq!(owner_side.powers.value(PowerId::Strength), 2);
        assert_eq!(owner_side.monsters[0].hp, 89); // 5, then 6

        let mut enemy_side = HotState::at_defaults();
        enemy_side.hp = 50;
        enemy_side.powers.set(PowerId::Rupture, SlotWire::Int, 2);
        enemy_side
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        monster_attack_player(&mut enemy_side, 0, 3, 1, &mut events).unwrap();
        assert_eq!(enemy_side.hp, 47);
        assert_eq!(enemy_side.powers.value(PowerId::Strength), 0);
    }

    #[test]
    fn monster_vigor_and_tainted_share_the_snapshot_and_vigor_is_consumed() {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.powers.set(PowerId::Tainted, SlotWire::Int, 3);
        state.powers.set(PowerId::PlayerVuln, SlotWire::Int, 2);
        let mut eel = HotMonster::new(MonsterKind::TerrorEel, 150);
        eel.powers.set(PowerId::Vigor, SlotWire::Int, 6);
        state.monsters_mut().push(eel);
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 4, 3, &mut events).unwrap();

        assert_eq!(state.hp, 43); // trunc((4 + 6 + 3) * 3/2) * 3 hits
        assert_eq!(state.monsters[0].powers.value(PowerId::Vigor), 0);
    }

    #[test]
    fn a_terminal_attack_does_not_reach_the_vigor_consumption_tail() {
        let mut state = HotState::at_defaults();
        state.hp = 1;
        let mut eel = HotMonster::new(MonsterKind::TerrorEel, 150);
        eel.powers.set(PowerId::Vigor, SlotWire::Int, 6);
        state.monsters_mut().push(eel);
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 4, 1, &mut events).unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Vigor), 6);
    }

    #[test]
    fn player_vigor_applies_to_every_hit_and_is_consumed_once_after_the_command() {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut events = Vec::new();
        let source = source_spec(CardId::StrikeIronclad);

        player_attack(&mut state, &source, &[0], 2, 2, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 20);
        assert_eq!(state.powers.value(PowerId::Vigor), 0);
    }

    /// #3483: a player `AttackCommand` issued while the combat is ending or
    /// over returns at entry (`AttackCommand/<Execute>d__90::MoveNext` RVA
    /// `0x3f19c0` IL_0067-008a), before BeforeAttack (IL_00d8). Vigor and
    /// Gigantification never bind it, nothing is hit, and the state is
    /// unchanged. Before #3483 only Osty's command returned here, and a
    /// player command latched Gigantification that its ending-gated
    /// AfterAttack tail then never released. The same command in a live
    /// combat binds and consumes both.
    #[test]
    fn player_attack_command_on_an_ending_or_over_combat_does_nothing() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut live = HotState::at_defaults();
        live.hp = 50;
        live.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        live.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        assert!(live.fanouts.set_gigantification(1));
        let attack = |state: &mut HotState| {
            player_attack(state, &source, &[0], 2, 2, &mut Vec::new()).unwrap();
        };

        // Ending: the only primary enemy is dead, the fight not yet over.
        let mut ending = live.clone();
        ending.monsters_mut()[0].hp = 0;
        assert!(!ending.history.over);
        assert!(damage_combat_is_ending(&ending));
        let before = ending.clone();
        attack(&mut ending);
        assert_eq!(ending, before, "the command returns at entry");
        assert!(!ending.fanouts.gigantification_bound());

        // Over: a live enemy, but the outcome is already fixed.
        let mut over = live.clone();
        over.history.over = true;
        let before = over.clone();
        attack(&mut over);
        assert_eq!(over, before, "the command returns at entry");

        attack(&mut live);
        assert_eq!(
            live.powers.value(PowerId::Vigor),
            0,
            "Vigor bound and consumed"
        );
        assert_eq!(
            live.fanouts.gigantification(),
            0,
            "Gigantification consumed"
        );
        assert!(!live.fanouts.gigantification_bound());
        assert!(live.monsters[0].hp < 100);
    }

    /// #3483: the entry test is not repeated per hit. A two-hit command whose
    /// first hit kills the last enemy keeps running: its next hit finds no
    /// `validTargets` (IL_01a6-01b3) and leaves for the record site and
    /// AfterAttack, so the killing hit carried the Vigor BeforeAttack bound.
    /// A second command in the same card body then enters an ending combat
    /// and returns at entry.
    #[test]
    fn a_multi_hit_player_attack_that_ends_the_combat_finishes_and_the_next_command_is_inert() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);

        player_attack(&mut state, &source, &[0], 7, 2, &mut Vec::new()).unwrap();

        assert_eq!(
            state.monsters[0].hp, 0,
            "7 + Vigor 3 kills on the first hit"
        );
        assert!(state.history.over);
        assert!(damage_combat_is_ending(&state));

        let before = state.clone();
        let mut events = Vec::new();
        player_attack(&mut state, &source, &[0], 7, 2, &mut events).unwrap();
        assert_eq!(state, before, "the next command returns at entry");
        assert!(events.is_empty());
    }

    /// #3495: a command whose first hit ends the combat keeps its
    /// Gigantification bound. Native AfterAttack (`0x3f19c0` IL_0845) still
    /// runs, but `Hook.IterateCombatHookListeners` (`0x3d3bc0` IL_0028-0042)
    /// yields no listener once the combat is over or ending, so
    /// `GigantificationPower.<AfterAttack>d__8` (`0x33b668`) never clears
    /// `commandToModify` or decrements (IL_0034-003c). Vigor stays for the
    /// same reason. The same command in a live combat releases both.
    #[test]
    fn a_command_that_ends_the_combat_keeps_gigantification_bound() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        assert!(state.fanouts.set_gigantification(2));
        let mut live = state.clone();
        live.monsters_mut()[0].hp = 1000;

        player_attack(&mut state, &source, &[0], 7, 2, &mut Vec::new()).unwrap();

        assert!(state.history.over);
        assert!(state.fanouts.gigantification_bound(), "never released");
        assert_eq!(state.fanouts.gigantification(), 2, "never decremented");
        assert_eq!(state.powers.value(PowerId::Vigor), 3);

        player_attack(&mut live, &source, &[0], 7, 2, &mut Vec::new()).unwrap();
        assert!(!live.fanouts.gigantification_bound());
        assert_eq!(live.fanouts.gigantification(), 1);
        assert_eq!(live.powers.value(PowerId::Vigor), 0);
    }

    /// #3495: the hit loop leaves on native's own exits, not on
    /// `history.over`. Once a fixed multi-target hit kills every listed
    /// target, `validTargets` is empty (`0x3f19c0` IL_0168-01b3). The next hit
    /// then leaves before entering an empty Damage batch. A second hit changes
    /// nothing: the state after the command equals a one-hit command's.
    #[test]
    fn a_fixed_multi_target_command_leaves_when_no_listed_target_lives() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut state = HotState::at_defaults();
        state.hp = 50;
        for slot in 0..2 {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 5);
            monster.uid = slot;
            monster.slot = slot as i32;
            state.monsters_mut().push(monster);
        }
        let mut one_hit = state.clone();
        let mut events = Vec::new();
        let mut one_hit_events = Vec::new();

        player_attack(&mut state, &source, &[0, 1], 10, 3, &mut events).unwrap();
        player_attack(&mut one_hit, &source, &[0, 1], 10, 1, &mut one_hit_events).unwrap();

        assert!(state.history.over);
        assert_eq!(state, one_hit);
        assert_eq!(events, one_hit_events);
    }

    /// #3495: Echoing Slash's kill wave adds a hit per kill, so a wave that
    /// kills every enemy leaves pending hits behind. The next one finds no
    /// `validTargets` (`0x3f19c0` IL_0168-01b3) and the command leaves,
    /// exactly as it would with the combat still live.
    #[test]
    fn an_echoing_slash_wave_that_kills_everything_leaves_on_no_valid_targets() {
        let (catalog, atom) = source_catalog(CardId::EchoingSlash);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 3,
            atom,
            flags: 0,
        });
        for slot in 0..2 {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 5);
            monster.uid = slot;
            monster.slot = slot as i32;
            state.monsters_mut().push(monster);
        }
        let mut events = Vec::new();

        player_attack_context_from_card(
            &mut state,
            (&catalog, &spec, 3),
            &[],
            10,
            AttackContextMode::EchoingSlash,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            2,
            "one wave; the two pending hits it earned find no target"
        );
    }

    /// #2655 witness state: the thorny receiver is the last live enemy and
    /// the player's Inferno, fired by the Thorns loss, kills it.
    fn thorns_inferno_last_receiver_state() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 10;
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        state.powers.set(PowerId::Inferno, SlotWire::Int, 2);
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 2);
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 1);
        state.monsters_mut().push(thorny);
        state
    }

    #[test]
    fn thorns_inferno_killing_the_last_receiver_commits_a_zero_result() {
        let mut state = thorns_inferno_last_receiver_state();
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0], 1, 1, &mut events).unwrap();

        // Thorns 1 cost the player 1 HP; Inferno's 2 killed the 2-HP
        // receiver and won the combat before the 4-damage hit committed.
        assert_eq!(state.hp, 9);
        assert!(state.history.over);
        assert_eq!(state.monsters[0].hp, 0);
        // IsEnding skips CombatHistory.DamageReceived for the zero result.
        assert_eq!(state.monsters[0].owner_powered_damage_results_this_turn, 0);
        assert_eq!(
            events[events.len() - 2..],
            [
                Event::CombatOver { player_won: true },
                Event::MonsterDamaged {
                    uid: 0,
                    blocked: 0,
                    unblocked: 0,
                    hp: 0
                },
            ]
        );
    }

    #[test]
    fn thorns_dead_last_receiver_joins_the_command_results_as_a_zero_unkilled_row() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = thorns_inferno_last_receiver_state();
        state.monsters_mut()[0].uid = 41;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });

        let results = player_attack_results_from_card(
            &mut state,
            (&catalog, &spec, 9),
            &[0],
            6,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(
            results,
            [AttackDamageResult {
                target: 0,
                receiver_uid: 41,
                total_damage: 0,
                overkill_damage: 0,
                was_target_killed: false,
            }]
        );
    }

    #[test]
    fn thorns_dead_receiver_while_combat_continues_refuses_atomically() {
        let mut state = thorns_inferno_last_receiver_state();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        let before = state.clone();
        assert_eq!(
            player_attack(&mut state, &source, &[0], 1, 1, &mut events),
            Err(EngineRefusal::PowerOrderNotModeled(
                "Thorns retained dead receiver while combat continues"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn thorns_dead_receiver_inside_a_powered_batch_refuses_atomically() {
        let mut state = thorns_inferno_last_receiver_state();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        state.monsters_mut()[1].uid = 1;
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        let before = state.clone();
        assert_eq!(
            player_attack(&mut state, &source, &[0, 1], 1, 1, &mut events),
            Err(EngineRefusal::PowerOrderNotModeled(
                "Thorns retained dead receiver inside a damage batch"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn thorns_receiver_classification_names_every_unadmitted_shape() {
        let refusal = |name| Err(EngineRefusal::PowerOrderNotModeled(name));
        let mut dead = HotState::at_defaults();
        dead.hp = 9;
        dead.history.over = true;
        dead.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 2));
        dead.monsters_mut()[0].hp = 0;
        let classify = |state: &HotState, uid, batched, dealer| {
            thorns_receiver_after_retaliation(state, 0, uid, batched, dealer)
        };
        assert_eq!(
            classify(&dead, 0, false, AttackDealer::Player),
            Ok(ThornsReceiver::RetainedDeadAtCombatEnd)
        );

        let mut live = dead.clone();
        live.monsters_mut()[0].hp = 1;
        assert_eq!(
            classify(&live, 0, true, AttackDealer::PlayerPet),
            Ok(ThornsReceiver::Live)
        );

        assert_eq!(
            classify(&dead, 7, false, AttackDealer::Player),
            refusal("Thorns replaced receiver")
        );
        assert_eq!(
            thorns_receiver_after_retaliation(&dead, 1, 0, false, AttackDealer::Player),
            refusal("Thorns replaced receiver")
        );
        assert_eq!(
            classify(&dead, 0, true, AttackDealer::Player),
            refusal("Thorns retained dead receiver inside a damage batch")
        );
        assert_eq!(
            classify(&dead, 0, false, AttackDealer::PlayerPet),
            refusal("Thorns retained dead receiver of a pet attack")
        );

        let mut continuing = dead.clone();
        continuing.history.over = false;
        assert_eq!(
            classify(&continuing, 0, false, AttackDealer::Player),
            refusal("Thorns retained dead receiver while combat continues")
        );

        let mut player_dead = dead.clone();
        player_dead.hp = 0;
        assert_eq!(
            classify(&player_dead, 0, false, AttackDealer::Player),
            refusal("Thorns retained dead receiver after player death")
        );

        let mut blocked = dead.clone();
        blocked.monsters_mut()[0].block = 3;
        assert_eq!(
            classify(&blocked, 0, false, AttackDealer::Player),
            refusal("Thorns retained dead receiver with Block")
        );

        let mut matriarch = dead.clone();
        matriarch.monsters_mut()[0].kind = MonsterKind::LagavulinMatriarch;
        assert_eq!(
            classify(&matriarch, 0, false, AttackDealer::Player),
            refusal("Thorns retained dead Lagavulin Matriarch receiver")
        );

        let mut gaze = dead.clone();
        gaze.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 1);
        assert_eq!(
            classify(&gaze, 0, false, AttackDealer::Player),
            refusal("Thorns retained dead receiver under Monarch's Gaze")
        );
    }

    #[test]
    fn lethal_thorns_keeps_current_hit_but_skips_inactive_player_callbacks() {
        let mut state = HotState::at_defaults();
        state.hp = 1;
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        state.powers.set(PowerId::Inferno, SlotWire::Int, 20);
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 20);
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 1);
        state.monsters_mut().push(thorny);
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0], 1, 1, &mut events).unwrap();

        assert_eq!(state.hp, 0);
        assert!(state.history.over);
        assert_eq!(state.monsters[0].hp, 16);
        assert_eq!(state.powers.value(PowerId::Vigor), 3);
        assert_eq!(
            events.last(),
            Some(&Event::MonsterDamaged {
                uid: 0,
                blocked: 0,
                unblocked: 4,
                hp: 16
            })
        );
    }

    #[test]
    fn random_attack_rolls_once_per_executed_hit_and_returns_one_result_each() {
        let (catalog, atom) = source_catalog(CardId::SwordBoomerang);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: 0,
        });
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        let before = state.rng.get(RngStream::Targets).counter;

        player_attack_random_from_card(&mut state, (&catalog, &spec, 7), 3, 4, &mut Vec::new())
            .unwrap();

        assert_eq!(state.rng.get(RngStream::Targets).counter, before + 4);
        assert_eq!(state.monsters.iter().map(|m| m.hp).sum::<i32>(), 188);
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|m| m.owner_powered_damage_results_this_turn)
                .sum::<i32>(),
            4
        );
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|m| m.nonowner_same_side_powered_damage_results_this_turn)
                .sum::<i32>(),
            0,
            "ordinary player attacks never enter the same-side nonowner bucket"
        );
    }

    #[test]
    fn attack_results_separate_block_hp_loss_and_overkill_before_death() {
        let (catalog, atom) = source_catalog(CardId::Fisticuffs);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 9,
            atom,
            flags: 0,
        });
        let mut target = HotMonster::new(MonsterKind::Toadpole, 10);
        target.uid = 41;
        target.block = 4;
        state.monsters_mut().push(target);

        let results = player_attack_results_from_card(
            &mut state,
            (&catalog, &spec, 9),
            &[0],
            20,
            1,
            &mut Vec::new(),
        )
        .unwrap();

        assert_eq!(
            results,
            [AttackDamageResult {
                target: 0,
                receiver_uid: 41,
                total_damage: 14,
                overkill_damage: 6,
                was_target_killed: true,
            }]
        );
    }

    #[test]
    fn temporary_strength_and_hang_join_the_one_ordered_decimal_fold() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.temp_strength = 3;
        state.powers.set(PowerId::Strength, SlotWire::Int, 2);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.powers.set(PowerId::Hang, SlotWire::Int, 2);
        state.monsters_mut().push(target);
        let source = source_spec(CardId::Hang);

        player_attack(&mut state, &source, &[0], 5, 1, &mut Vec::new()).unwrap();

        assert_eq!(state.monsters[0].hp, 80);
    }

    #[test]
    fn batch_eight_hp_strength_and_pen_listeners_keep_their_native_receipts() {
        let mut ordered = HotState::at_defaults();
        ordered.hp = 50;
        ordered.set_deep_relic_ownership(false, false, false, true);
        ordered
            .fanouts
            .set_batch_eight_deep_relic_ownership(true, false, false, true, true);
        assert!(ordered.fanouts.set_beating_remnant_damage_received(17));
        let before = ordered.clone();
        assert!(matches!(
            commit_player_hp_loss(&mut ordered, 5, 0, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Beating Remnant and Tungsten Rod listener order"
            ))
        ));
        assert_eq!(ordered, before);

        let mut temporary = HotState::at_defaults();
        temporary
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, false, true);
        apply_owner_temporary_strength(&mut temporary, 5, &mut Vec::new()).unwrap();
        assert_eq!(temporary.temp_strength, 5);
        assert_eq!(temporary.powers.value(PowerId::Strength), 5);
        assert!(temporary.fanouts.ruined_helmet_used());

        let mut lethal = HotState::at_defaults();
        lethal.max_hp = 9;
        lethal.hp = 0;
        lethal
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, true, false);
        assert!(resolve_player_lethal(&mut lethal, &mut Vec::new()).unwrap());
        assert_eq!(lethal.hp, 4);
        assert!(lethal.fanouts.lizard_tail_used());

        let source = source_spec(CardId::StrikeIronclad);
        let mut ordinary = HotState::at_defaults();
        ordinary
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        crate::engine::play::with_test_active_play_pen_double(7, false, || {
            player_attack(&mut ordinary, &source, &[0], 6, 1, &mut Vec::new()).unwrap();
        });
        assert_eq!(ordinary.monsters[0].hp, 24);

        let mut doubled = HotState::at_defaults();
        doubled
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        crate::engine::play::with_test_active_play_pen_double(8, true, || {
            player_attack(&mut doubled, &source, &[0], 6, 1, &mut Vec::new()).unwrap();
        });
        assert_eq!(doubled.monsters[0].hp, 18);

        let mut multi_hit = HotState::at_defaults();
        multi_hit.max_hp = 6;
        multi_hit.hp = 6;
        multi_hit
            .fanouts
            .set_batch_eight_deep_relic_ownership(true, false, false, false, false);
        multi_hit
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 30));
        assert_eq!(
            monster_attack_player_positive_results(&mut multi_hit, 0, 1, 6, &mut Vec::new(),)
                .unwrap(),
            6
        );
        assert!(multi_hit.history.over);
        assert_eq!(multi_hit.fanouts.beating_remnant_damage_received(), 5);
    }

    #[test]
    fn actual_prevented_and_illusion_death_keep_reset_ledger_lifecycle_exact() {
        fn reset_owner() -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 0;
            state.max_hp = 20;
            state.powers.set(PowerId::Genesis, SlotWire::Int, 2);
            state.powers.set(PowerId::StarNextTurn, SlotWire::Int, 1);
            state.powers.set(PowerId::EnergyNextTurn, SlotWire::Int, 3);
            state.powers.set(PowerId::LightningRod, SlotWire::Int, 1);
            state.powers.set(PowerId::Spinner, SlotWire::Int, 1);
            assert!(state.fanouts.set_radiance(1));
            assert!(
                state
                    .fanouts
                    .set_star_energy_reset_order(&[PowerId::Genesis, PowerId::StarNextTurn,])
            );
            state.orbs.set_reset_order(vec![
                crate::hot::OrbResetPower::LightningRod,
                crate::hot::OrbResetPower::Spinner,
            ]);
            assert!(state.fanouts.set_after_energy_reset_order(&[
                crate::hot::AfterEnergyResetPower::Genesis,
                crate::hot::AfterEnergyResetPower::StarNextTurn,
                crate::hot::AfterEnergyResetPower::EnergyNextTurn,
                crate::hot::AfterEnergyResetPower::Radiance,
                crate::hot::AfterEnergyResetPower::LightningRod,
                crate::hot::AfterEnergyResetPower::Spinner,
            ]));
            state
        }

        let mut actual = reset_owner();
        assert!(!resolve_player_lethal(&mut actual, &mut Vec::new()).unwrap());
        assert_eq!(actual.fanouts.after_energy_reset_order(), Some(&[][..]));
        assert!(actual.fanouts.star_energy_reset_order().is_empty());
        assert!(actual.orbs.reset_order().is_empty());
        assert_eq!(actual.powers.value(PowerId::Genesis), 0);
        assert_eq!(actual.fanouts.radiance(), 0);

        let mut prevented = reset_owner();
        prevented
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, true, false);
        assert!(resolve_player_lethal(&mut prevented, &mut Vec::new()).unwrap());
        assert_eq!(
            prevented.fanouts.after_energy_reset_order().unwrap().len(),
            6,
            "prevented death returns before RemoveAllPowersAfterDeath"
        );

        let mut illusion = reset_owner();
        illusion
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Parafright, 10));
        assert!(!resolve_player_lethal(&mut illusion, &mut Vec::new()).unwrap());
        assert_eq!(
            illusion.fanouts.after_energy_reset_order().unwrap().len(),
            6
        );
        assert_eq!(illusion.powers.value(PowerId::Genesis), 2);
        assert!(illusion.fanouts.player_hooks_deactivated());

        // #2646: the veto holders are exactly the two `Apply<IllusionPower>`
        // owners, alive or retained dead. Their summoners hold no listener,
        // so a death beside them alone takes the ordinary removal.
        for (kind, hp, vetoes) in [
            (MonsterKind::Parafright, 10, true),
            (MonsterKind::Parafright, 0, true),
            (MonsterKind::EyeWithTeeth, 6, true),
            (MonsterKind::EyeWithTeeth, 0, true),
            (MonsterKind::TheObscura, 129, false),
            (MonsterKind::Fogmog, 70, false),
        ] {
            let mut owner = reset_owner();
            owner.monsters_mut().push(HotMonster::new(kind, hp));
            assert_eq!(illusion_removal_veto_in_combat(&owner), vetoes, "{kind:?}");
            assert!(!resolve_player_lethal(&mut owner, &mut Vec::new()).unwrap());
            assert!(owner.fanouts.player_hooks_deactivated(), "{kind:?}");
            let kept = if vetoes { 2 } else { 0 };
            assert_eq!(owner.powers.value(PowerId::Genesis), kept, "{kind:?}");
            assert_eq!(
                owner.fanouts.after_energy_reset_order().unwrap().len(),
                if vetoes { 6 } else { 0 },
                "{kind:?}"
            );
        }
    }

    #[test]
    fn illusion_veto_starts_when_the_summoner_spawns_its_listener() {
        // #2646 future owner: The Obscura alone does not veto, and the veto
        // starts with the Parafright its ILLUSION_MOVE summons.
        for (summoner, illusion, hp) in [
            (MonsterKind::TheObscura, MonsterKind::Parafright, 21),
            (MonsterKind::Fogmog, MonsterKind::EyeWithTeeth, 6),
        ] {
            let mut state = HotState::at_defaults();
            state.hp = 10;
            state.max_hp = 20;
            let mut owner = HotMonster::new(summoner, 100);
            owner.max_hp = 100;
            owner.uid = 1;
            state.monsters_mut().push(owner);
            assert!(!illusion_removal_veto_in_combat(&state));
            super::super::monsters::spawn_illusion(&mut state, 1, illusion, hp).unwrap();
            assert!(illusion_removal_veto_in_combat(&state), "{illusion:?}");
        }
    }

    #[test]
    fn illusion_necro_interval_needs_the_veto_a_live_osty_and_necro() {
        // #2646: each of the three conditions is necessary on its own.
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::NecroMastery, SlotWire::Int, 1);
        state.fanouts.set_osty(Some((3, 3))).unwrap();
        assert!(illusion_retained_necro_cleanup_is_reached(&state, true));
        assert!(!illusion_retained_necro_cleanup_is_reached(&state, false));
        let mut no_necro = state.clone();
        no_necro.powers.set(PowerId::NecroMastery, SlotWire::Int, 0);
        assert!(!illusion_retained_necro_cleanup_is_reached(&no_necro, true));
        let mut no_pet = state.clone();
        no_pet.fanouts.set_osty(None).unwrap();
        assert!(!illusion_retained_necro_cleanup_is_reached(&no_pet, true));
    }

    #[test]
    fn context_modes_refresh_kill_waves_and_spill_the_initial_raw_result() {
        let (echo_catalog, echo_atom) = source_catalog(CardId::EchoingSlash);
        let echo_spec = *echo_catalog.spec(echo_atom).unwrap();
        let mut echo = HotState::at_defaults();
        echo.hp = 50;
        echo.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 3,
            atom: echo_atom,
            flags: 0,
        });
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 1);
        waterfall.max_hp = 250;
        waterfall.uid = 17;
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 29);
        echo.monsters_mut().push(waterfall);

        player_attack_context_from_card(
            &mut echo,
            (&echo_catalog, &echo_spec, 3),
            &[],
            1,
            AttackContextMode::EchoingSlash,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(echo.monsters[0].hp, 999_999_998);

        let (omni_catalog, omni_atom) = source_catalog(CardId::Omnislice);
        let omni_spec = *omni_catalog.spec(omni_atom).unwrap();
        let mut omni = HotState::at_defaults();
        omni.hp = 50;
        omni.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 4,
            atom: omni_atom,
            flags: 0,
        });
        let mut original = HotMonster::new(MonsterKind::Toadpole, 100);
        original.block = 3;
        omni.monsters_mut().push(original);
        omni.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));

        player_attack_context_from_card(
            &mut omni,
            (&omni_catalog, &omni_spec, 4),
            &[0],
            8,
            AttackContextMode::Omnislice,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!((omni.monsters[0].hp, omni.monsters[1].hp), (95, 92));
    }

    #[test]
    fn accuracy_cruelty_and_tracking_share_the_one_final_attack_floor() {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
        target.powers.set(PowerId::Weak, SlotWire::Int, 1);
        target.powers.set(PowerId::Vuln, SlotWire::Int, 1);
        state.monsters_mut().push(target);
        state.powers.set(PowerId::Accuracy, SlotWire::Int, 4);
        state.powers.set(PowerId::Cruelty, SlotWire::Int, 25);
        state.powers.set(PowerId::Tracking, SlotWire::Int, 50);
        let source = source_spec(CardId::Shiv);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0], 4, 1, &mut events).unwrap();

        // floor((4 + 4 Accuracy) * (3/2 + 25/100 Cruelty) * 3/2 Tracking)
        assert_eq!(state.monsters[0].hp, 79);
    }

    #[test]
    fn phantom_blades_adds_only_to_the_first_finished_own_shiv_play() {
        let shiv = source_spec(CardId::Shiv);
        let strike = source_spec(CardId::StrikeIronclad);

        let mut first = HotState::at_defaults();
        first
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        first.powers.set(PowerId::Accuracy, SlotWire::Int, 4);
        first.powers.set(PowerId::PhantomBlades, SlotWire::Int, 9);
        player_attack(&mut first, &shiv, &[0], 4, 1, &mut Vec::new()).unwrap();
        assert_eq!(first.monsters[0].hp, 83, "4 + 4 Accuracy + 9 Phantom");

        let mut later = first.clone();
        later.monsters_mut()[0].hp = 100;
        later.history.over = false;
        later.history.shiv_plays_finished_this_turn = 1;
        player_attack(&mut later, &shiv, &[0], 4, 1, &mut Vec::new()).unwrap();
        assert_eq!(later.monsters[0].hp, 92, "later Shiv keeps Accuracy only");

        let mut non_shiv = HotState::at_defaults();
        non_shiv
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        non_shiv
            .powers
            .set(PowerId::PhantomBlades, SlotWire::Int, 9);
        player_attack(&mut non_shiv, &strike, &[0], 6, 1, &mut Vec::new()).unwrap();
        assert_eq!(non_shiv.monsters[0].hp, 94);
    }

    #[test]
    fn phantom_boosted_lethal_shiv_uses_insatiable_death_cleanup() {
        let shiv = source_spec(CardId::Shiv);
        let mut state = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::TheInsatiable, 10);
        owner.max_hp = 264;
        owner.powers.set(PowerId::Sandpit, SlotWire::Int, 4);
        state.monsters_mut().push(owner);
        state.powers.set(PowerId::PhantomBlades, SlotWire::Int, 9);

        player_attack(&mut state, &shiv, &[0], 4, 1, &mut Vec::new()).unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Sandpit), 0);
        assert!(state.history.over);
    }

    #[test]
    fn phantom_damage_overflow_refuses_before_any_hit_mutation() {
        let shiv = source_spec(CardId::Shiv);
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.powers.set(PowerId::Accuracy, SlotWire::Int, i32::MAX);
        state
            .powers
            .set(PowerId::PhantomBlades, SlotWire::Int, i32::MAX);
        let before = state.clone();
        let mut events = Vec::new();

        assert!(matches!(
            player_attack(&mut state, &shiv, &[0], i64::MAX, 1, &mut events),
            Err(EngineRefusal::CounterOverflow("attack damage"))
        ));
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn player_weak_and_shrink_share_one_final_attack_floor() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::PlayerWeak, SlotWire::Int, 2);
        state.powers.set(PowerId::PlayerShrink, SlotWire::Int, 1);
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0], 6, 1, &mut events).unwrap();

        // floor(6 * 3/4 * 7/10) = 3. Flooring after Weak would produce 2.
        assert_eq!(state.monsters[0].hp, 17);
    }

    #[test]
    fn soar_halves_only_powered_attacks_into_its_owner() {
        let source = source_spec(CardId::StrikeIronclad);

        let mut state = HotState::at_defaults();
        let mut owl = HotMonster::new(MonsterKind::OwlMagistrate, 30);
        owl.powers.set(PowerId::Soar, SlotWire::Bool, 1);
        state.monsters_mut().push(owl);
        let mut events = Vec::new();
        player_attack(&mut state, &source, &[0], 7, 1, &mut events).unwrap();
        assert_eq!(state.monsters[0].hp, 27);

        // Native null-card and other unpowered damage bypass
        // SoarPower.ModifyDamageMultiplicative entirely.
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(7),
            false,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 20);

        state.monsters_mut()[0]
            .powers
            .set(PowerId::Soar, SlotWire::Bool, 0);
        player_attack(&mut state, &source, &[0], 7, 1, &mut events).unwrap();
        assert_eq!(state.monsters[0].hp, 13);
    }

    #[test]
    fn cruelty_does_not_modify_an_incoming_vulnerable_hit() {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.powers.set(PowerId::Cruelty, SlotWire::Int, 50);
        state.powers.set(PowerId::PlayerVuln, SlotWire::Int, 1);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        let mut events = Vec::new();

        monster_attack_player(&mut state, 0, 4, 1, &mut events).unwrap();

        assert_eq!(state.hp, 94);
    }

    #[test]
    fn powered_block_folds_temp_dex_fasten_and_the_unmovable_window() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 100;
        state.powers.set(PowerId::Dexterity, SlotWire::Int, -1);
        state.powers.set(PowerId::TempDexterity, SlotWire::Int, 2);
        state.powers.set(PowerId::Fasten, SlotWire::Int, 4);
        state.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        let shrug = source_spec(CardId::ShrugItOff);
        let defend = source_spec(CardId::DefendIronclad);
        let mut events = Vec::new();

        // Shrug It Off carries no Defend tag, so Fasten does not reach it
        // (#3051): (5 - 1 + 2) doubled by the open Unmovable window.
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &shrug,
                5,
                &mut events
            )
            .unwrap(),
            12
        );
        // Defend is tagged: 5 - 1 + 2 + Fasten 4, the window now closed.
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &defend,
                5,
                &mut events
            )
            .unwrap(),
            10
        );
        assert_eq!(state.block, 22);
        assert_eq!(state.history.card_block_gains, 2);
    }

    #[test]
    fn powered_block_preview_is_the_mutating_result_without_any_mutation() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Dexterity, SlotWire::Int, -3);
        state.powers.set(PowerId::TempDexterity, SlotWire::Int, 2);
        state.powers.set(PowerId::Fasten, SlotWire::Int, 4);
        state.powers.set(PowerId::Unmovable, SlotWire::Int, 2);
        state.powers.set(PowerId::PlayerFrail, SlotWire::Int, 1);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
        let source = source_spec(CardId::ShrugItOff);
        let snapshot = state.clone();

        let preview = preview_powered_card_block(&state, &source, 5).unwrap();
        assert_eq!(preview, 24); // trunc(((5 - 3 + 2) * 2) * 3/4) * 2^2; no Defend tag, no Fasten
        assert_eq!(state, snapshot);

        let mut events = Vec::new();
        let gained = gain_powered_card_block(
            &mut state,
            &crate::catalog::CatalogBuilder::new().build(),
            &source,
            5,
            &mut events,
        )
        .unwrap();
        assert_eq!(gained, preview);
        assert_eq!(state.block, preview);
        assert_eq!(state.history.card_block_gains, 1);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn card_block_clamps_storage_at_the_native_cap_without_clamping_the_event() {
        let source = source_spec(CardId::DefendIronclad);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.block = NATIVE_PLAYER_BLOCK_CAP as i32 - 2;
        let mut events = Vec::new();

        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            5
        );
        assert_eq!(state.block, NATIVE_PLAYER_BLOCK_CAP as i32);
        assert_eq!(state.history.card_block_gains, 1);
        assert_eq!(
            events,
            [Event::PlayerBlockGained {
                amount: 5,
                block: NATIVE_PLAYER_BLOCK_CAP as i32,
            }]
        );

        events.clear();
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            5
        );
        assert_eq!(state.block, NATIVE_PLAYER_BLOCK_CAP as i32);
        assert_eq!(state.history.card_block_gains, 2);
        assert_eq!(
            events[0],
            Event::PlayerBlockGained {
                amount: 5,
                block: state.block
            }
        );
    }

    #[test]
    fn malformed_card_block_and_history_overflow_refuse_before_mutation() {
        let source = source_spec(CardId::DefendIronclad);
        for malformed_block in [-1, NATIVE_PLAYER_BLOCK_CAP as i32 + 1] {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.block = malformed_block;
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 17 }];
            let before_events = events.clone();
            assert_eq!(
                gain_powered_card_block(
                    &mut state,
                    &crate::catalog::CatalogBuilder::new().build(),
                    &source,
                    5,
                    &mut events
                ),
                Err(EngineRefusal::MalformedArgs("player block"))
            );
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }

        let mut state = HotState::at_defaults();

        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.history.card_block_gains = i16::MAX;
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            ),
            Err(EngineRefusal::CounterOverflow("card_block_gains"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn no_block_zeroes_only_powered_card_block_and_keeps_preview_pure() {
        let source = source_spec(CardId::DefendIronclad);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::NoBlock, SlotWire::Int, 2);
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 3);
        state.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
        let snapshot = state.clone();

        assert_eq!(preview_powered_card_block(&state, &source, 5).unwrap(), 0);
        assert_eq!(state, snapshot);

        let mut events = Vec::new();
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            0
        );
        assert_eq!(state.block, 0);
        assert_eq!(state.history.card_block_gains, 0);
        assert!(events.is_empty());
        assert_eq!(
            gain_card_unpowered_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            40
        );
    }

    #[test]
    fn no_block_cannot_hide_shadowmeld_native_decimal_overflow() {
        let source = source_spec(CardId::DefendIronclad);
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::NoBlock, SlotWire::Int, 1);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 95);

        let safe_snapshot = state.clone();
        assert_eq!(preview_powered_card_block(&state, &source, 1).unwrap(), 0);
        assert_eq!(state, safe_snapshot);

        let refusal_snapshot = state.clone();
        assert!(matches!(
            preview_powered_card_block(&state, &source, 2),
            Err(EngineRefusal::CounterOverflow("shadowmeld card block"))
        ));
        assert_eq!(state, refusal_snapshot);

        let mut events = Vec::new();
        assert!(matches!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            ),
            Err(EngineRefusal::CounterOverflow("shadowmeld card block"))
        ));
        assert_eq!(state, refusal_snapshot);
        assert!(events.is_empty());
    }

    #[test]
    fn powered_block_preview_pins_floor_defend_and_overflow_without_mutation() {
        let defend = source_spec(CardId::DefendIronclad);
        let shrug = source_spec(CardId::ShrugItOff);
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::Dexterity, SlotWire::Int, -9);
        state.powers.set(PowerId::Fasten, SlotWire::Int, 50);
        let snapshot = state.clone();
        // The untagged card floors at 0; Fasten lifts only the Defend.
        assert_eq!(preview_powered_card_block(&state, &shrug, 2).unwrap(), 0);
        assert_eq!(preview_powered_card_block(&state, &defend, 2).unwrap(), 43);
        assert_eq!(state, snapshot);

        state
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, i32::MAX);
        let overflow_snapshot = state.clone();
        assert!(matches!(
            preview_powered_card_block(&state, &defend, i64::MAX),
            Err(EngineRefusal::CounterOverflow("card block"))
        ));
        assert_eq!(state, overflow_snapshot);

        state.powers.set(PowerId::Dexterity, SlotWire::Int, 0);
        state.powers.set(PowerId::Fasten, SlotWire::Int, 0);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 95);
        let shadow_snapshot = state.clone();
        assert!(matches!(
            preview_powered_card_block(&state, &defend, 1),
            Err(EngineRefusal::CounterOverflow("card block"))
        ));
        assert_eq!(state, shadow_snapshot);
    }

    #[test]
    fn block_next_turn_zero_restacks_and_overflow_are_atomic() {
        let mut state = HotState::at_defaults();
        let mut events = Vec::new();
        apply_block_next_turn(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::BlockNextTurn), 0);
        assert!(state.fanouts.after_block_cleared_order().is_empty());

        apply_block_next_turn(&mut state, 4, &mut events).unwrap();
        apply_block_next_turn(&mut state, 7, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::BlockNextTurn), 11);
        assert_eq!(
            state.fanouts.after_block_cleared_order(),
            &[PowerId::BlockNextTurn]
        );

        state
            .powers
            .set(PowerId::BlockNextTurn, SlotWire::Int, i32::MAX);
        let snapshot = state.clone();
        assert!(matches!(
            apply_block_next_turn(&mut state, 1, &mut events),
            Err(EngineRefusal::CounterOverflow("block next turn"))
        ));
        assert_eq!(state, snapshot);
    }

    #[test]
    fn frail_multiplies_only_powered_card_block_after_unmovable() {
        let source = source_spec(CardId::DefendIronclad);
        let mut powered = HotState::at_defaults();
        powered.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        powered.powers.set(PowerId::PlayerFrail, SlotWire::Int, 2);
        powered.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        let mut events = Vec::new();

        assert_eq!(
            gain_powered_card_block(
                &mut powered,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            7
        );
        assert_eq!(powered.block, 7); // trunc((5 * 2) * 3/4)

        let mut unpowered = HotState::at_defaults();

        unpowered.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        unpowered.powers.set(PowerId::PlayerFrail, SlotWire::Int, 2);
        assert_eq!(
            gain_card_unpowered_block(
                &mut unpowered,
                &crate::catalog::CatalogBuilder::new().build(),
                &source,
                5,
                &mut events
            )
            .unwrap(),
            5
        );
    }

    /// #3051 witness: `FastenPower::ModifyBlockAdditive` RVA `0xa22c0`
    /// `IL_0029`-`IL_0041` adds Fasten only to Defend-tagged card Block, and
    /// `IL_001b`-`IL_0028` excludes unpowered Block even on a Defend.
    #[test]
    fn fasten_adds_only_to_powered_defend_tagged_card_block() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 100;
        state.powers.set(PowerId::Fasten, SlotWire::Int, 6);
        let defend = source_spec(CardId::DefendIronclad);
        let shrug = source_spec(CardId::ShrugItOff);
        let mut events = Vec::new();

        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &defend,
                5,
                &mut events
            )
            .unwrap(),
            11
        );
        assert_eq!(
            gain_powered_card_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &shrug,
                8,
                &mut events
            )
            .unwrap(),
            8
        );
        assert_eq!(
            gain_card_unpowered_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &defend,
                5,
                &mut events
            )
            .unwrap(),
            5
        );
        assert_eq!(state.block, 24);
    }

    #[test]
    fn unpowered_card_block_skips_additives_but_keeps_unmovable() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 100;
        state.powers.set(PowerId::Dexterity, SlotWire::Int, 9);
        state.powers.set(PowerId::TempDexterity, SlotWire::Int, 9);
        state.powers.set(PowerId::Fasten, SlotWire::Int, 9);
        state.powers.set(PowerId::Unmovable, SlotWire::Int, 1);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
        let entrench = source_spec(CardId::Entrench);
        let mut events = Vec::new();

        assert_eq!(
            gain_card_unpowered_block(
                &mut state,
                &crate::catalog::CatalogBuilder::new().build(),
                &entrench,
                10,
                &mut events
            )
            .unwrap(),
            80
        );
        assert_eq!(state.block, 80);
        assert_eq!(state.history.card_block_gains, 1);
    }

    /// #3502: `CreatureCmd/<GainBlock>d__18::MoveNext` (`0x3eaec0`) returns
    /// zero on `IsOverOrEnding` (IL_002d-0041) before any modifier, history or
    /// listener. With the only enemy dead and the over latch not yet set, the
    /// powered, unpowered, retained-Decimal and flat player Block commands all
    /// gain nothing and change nothing, and the Dampen + Juggernaut preflight
    /// in front of the card forms does not refuse a command native skips. The
    /// live control gains every Block and refuses the preflight.
    #[test]
    fn player_gain_block_returns_at_entry_while_the_combat_is_ending_before_the_over_latch() {
        let catalog = CatalogBuilder::new().build();
        let defend = source_spec(CardId::DefendIronclad);
        for ending in [false, true] {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));
            state.hp = 100;
            if ending {
                state.monsters_mut()[0].hp = 0;
            }
            assert!(!state.history.over);
            assert_eq!(damage_combat_is_ending(&state), ending);
            let before = state.clone();
            let mut events = Vec::new();

            let powered =
                gain_powered_card_block(&mut state, &catalog, &defend, 5, &mut events).unwrap();
            let unpowered =
                gain_card_unpowered_block(&mut state, &catalog, &defend, 5, &mut events).unwrap();
            let retained = gain_powered_card_block_retained_decimal(
                &mut state,
                &catalog,
                &defend,
                5,
                &mut events,
            )
            .unwrap();
            let flat = gain_flat_power_block_decimal(
                &mut state,
                Some(&catalog),
                DotNetDecimal::from_i64(5),
                &mut events,
            )
            .unwrap();
            if ending {
                assert_eq!((powered, unpowered), (0, 0));
                assert_eq!(retained, DotNetDecimal::zero());
                assert_eq!(flat, DotNetDecimal::zero());
                assert_eq!(state, before);
                assert!(events.is_empty());
            } else {
                assert_eq!((powered, unpowered), (5, 5));
                assert_eq!(retained, DotNetDecimal::from_i64(5));
                assert_eq!(flat, DotNetDecimal::from_i64(5));
                assert_eq!(state.block, 20);
                assert_eq!(state.history.card_block_gains, 3);
            }

            let mut dampened = before.clone();
            dampened
                .card_states
                .set_dampen(Some(crate::hot::DampenState {
                    caster_uid: 2,
                    cards: Vec::new(),
                }));
            dampened.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
            let untouched = dampened.clone();
            let powered = gain_powered_card_block(&mut dampened, &catalog, &defend, 5, &mut events);
            let unpowered =
                gain_card_unpowered_block(&mut dampened, &catalog, &defend, 5, &mut events);
            let retained = gain_powered_card_block_retained_decimal(
                &mut dampened,
                &catalog,
                &defend,
                5,
                &mut events,
            );
            if ending {
                assert_eq!((powered, unpowered), (Ok(0), Ok(0)));
                assert_eq!(retained, Ok(DotNetDecimal::zero()));
                assert_eq!(dampened, untouched);
            } else {
                assert_eq!(
                    powered,
                    Err(EngineRefusal::MalformedArgs(
                        "Dampen powered block Juggernaut catalog"
                    ))
                );
                assert_eq!(
                    unpowered,
                    Err(EngineRefusal::MalformedArgs(
                        "Dampen unpowered block Juggernaut catalog"
                    ))
                );
                assert_eq!(
                    retained,
                    Err(EngineRefusal::MalformedArgs(
                        "Dampen powered block Juggernaut catalog"
                    ))
                );
            }
        }
    }

    #[test]
    fn shriek_crossing_installs_the_stunned_override_once() {
        let mut state = HotState::at_defaults();
        let mut eel = HotMonster::new(MonsterKind::TerrorEel, 150);
        eel.powers.set(PowerId::Shriek, SlotWire::Bool, 1);
        state.monsters_mut().push(eel);
        let mut events = Vec::new();

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(75),
            true,
            true,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].hp, 75);
        assert_eq!(state.monsters[0].powers.value(PowerId::Shriek), 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Stunned);
    }

    /// #2539: ShriekPower's Amount is its HP threshold,
    /// `TerrorEel::get_ShriekAmount` `0xbfda2` `GetValueIfAscension(8, 75, 70)`.
    /// Below A8 a hit that leaves the eel at 72 does not stun; at A8 it does.
    #[test]
    fn the_shriek_threshold_is_the_fights_tier() {
        for (ascension, stunned) in [(7, false), (8, true)] {
            let mut state = HotState::at_defaults();
            assert!(state.fanouts.set_ascension(ascension));
            let mut eel = HotMonster::new(MonsterKind::TerrorEel, 80);
            eel.powers.set(PowerId::Shriek, SlotWire::Bool, 1);
            state.monsters_mut().push(eel);
            let mut events = Vec::new();
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(8),
                true,
                true,
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].hp, 72);
            assert_eq!(
                state.monsters[0].override_state == MonsterOverride::Stunned,
                stunned,
                "A{ascension}"
            );
            assert_eq!(
                state.monsters[0].powers.value(PowerId::Shriek),
                i32::from(!stunned)
            );
        }
    }

    #[test]
    fn burrowed_break_requires_positive_block_consumption_and_installs_dizzy() {
        let mut state = HotState::at_defaults();
        let mut tunneler = HotMonster::new(MonsterKind::Tunneler, 40);
        tunneler.block = 5;
        tunneler.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
        state.monsters_mut().push(tunneler);
        let mut events = Vec::new();

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(4),
            true,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].block, 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 1);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(3),
            true,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!((state.monsters[0].hp, state.monsters[0].block), (38, 0));
        assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
    }

    #[test]
    fn lose_all_block_dispatches_the_same_burrowed_break_hook() {
        let mut state = HotState::at_defaults();
        let mut tunneler = HotMonster::new(MonsterKind::Tunneler, 40);
        tunneler.block = 9;
        tunneler.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
        state.monsters_mut().push(tunneler);
        let mut events = Vec::new();

        assert!(lose_all_monster_block(&mut state, 0, &mut events));
        assert_eq!(state.monsters[0].block, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
        assert!(!lose_all_monster_block(&mut state, 0, &mut events));
    }

    #[test]
    fn lethal_block_break_takes_death_cleanup_instead_of_dizzy() {
        let mut state = HotState::at_defaults();
        let mut tunneler = HotMonster::new(MonsterKind::Tunneler, 1);
        tunneler.block = 1;
        tunneler.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
        state.monsters_mut().push(tunneler);
        let mut events = Vec::new();

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(2),
            true,
            true,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);

        let mut state = HotState::at_defaults();
        let mut tunneler = HotMonster::new(MonsterKind::Tunneler, 1);
        tunneler.override_state = MonsterOverride::Dizzy;
        state.monsters_mut().push(tunneler);
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    }

    fn with_monarch_sleight(state: &mut HotState, damage: i32) {
        state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 1);
        state
            .powers
            .set(PowerId::SleightOfFlesh, SlotWire::Int, damage);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::MonarchsGaze])
        );
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
    }

    #[test]
    fn nested_sleight_phrog_death_spawns_exactly_once() {
        for source in [PowerId::MonarchsGaze, PowerId::Envenom, PowerId::ReaperForm] {
            let mut state = HotState::at_defaults();
            let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, 10);
            phrog.uid = 7;
            state.monsters_mut().push(phrog);
            with_monarch_sleight(&mut state, 9);
            state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 0);
            state.powers.set(source, SlotWire::Int, 1);
            assert!(state.fanouts.set_after_damage_given_order(&[source]));
            let mut events = Vec::new();
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters.len(), 5, "{source:?}");
            assert!(state.monsters[1..].iter().all(|m| m.hp > 0));
            assert!(!state.history.over);
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, Event::MonsterDied { uid: 7 }))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn block_break_precedes_nested_given_damage_and_never_restuns_a_dead_tunneler() {
        for (hp, hand_drill) in [(20, false), (9, false), (9, true)] {
            let mut state = HotState::at_defaults();
            let mut tunneler = HotMonster::new(MonsterKind::Tunneler, hp);
            tunneler.block = 1;
            tunneler.powers.set(PowerId::Burrowed, SlotWire::Bool, 1);
            state.monsters_mut().push(tunneler);
            with_monarch_sleight(&mut state, 9);
            state.set_deep_relic_ownership(hand_drill, false, false, false);
            let mut events = Vec::new();
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut events,
            )
            .unwrap();
            assert_eq!(state.monsters[0].powers.value(PowerId::Burrowed), 0);
            if hp == 20 {
                assert_eq!(state.monsters[0].hp, 11);
                assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
                let burrowed = events
                    .iter()
                    .position(|event| {
                        matches!(
                            event,
                            Event::PowerChanged {
                                power: PowerId::Burrowed,
                                ..
                            }
                        )
                    })
                    .unwrap();
                let strength = events
                    .iter()
                    .position(|event| {
                        matches!(
                            event,
                            Event::PowerChanged {
                                power: PowerId::Strength,
                                ..
                            }
                        )
                    })
                    .unwrap();
                assert!(burrowed < strength);
            } else {
                assert!(state.history.over);
                assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
                assert_eq!(
                    events
                        .iter()
                        .filter(|event| matches!(event, Event::MonsterDied { .. }))
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn nested_sleight_retains_catalog_for_hopper_and_aeonglass_death() {
        let catalog = hopper_test_catalog();
        let mut hopper = dying_hopper_with_swipe();
        hopper.monsters_mut()[0].hp = 10;
        with_monarch_sleight(&mut hopper, 9);
        damage_monster_with_catalog(
            &mut hopper,
            &catalog,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(hopper.history.over);
        assert_eq!(hopper.card_states.hopper().unwrap().master.len(), 1);
        assert_eq!(hopper.card_states.hopper().unwrap().history.len(), 2);
        assert!(hopper.monsters[0].hopper_swipe_uid().is_none());

        let (mut aeonglass, catalog) = aeonglass_damage_fixture();
        aeonglass.monsters_mut()[0].hp = 10;
        aeonglass.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 0);
        with_monarch_sleight(&mut aeonglass, 9);
        damage_monster_with_catalog(
            &mut aeonglass,
            &catalog,
            0,
            DotNetDecimal::from_i64(1),
            true,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(aeonglass.history.over);
        assert!(super::super::cards::aeonglass_state_is_exact(
            &aeonglass, &catalog
        ));
    }

    #[test]
    fn reaper_lethal_hit_respects_live_and_dead_osty_hit_veto() {
        for pet in 0..3 {
            let mut state = HotState::at_defaults();
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 1));
            let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
            peer.uid = 1;
            peer.slot = 1;
            state.monsters_mut().push(peer);
            if pet != 0 {
                state.fanouts.set_osty(Some((1, 1))).unwrap();
            }
            if pet == 2 {
                state
                    .fanouts
                    .mutate_pet(|pet| {
                        pet.kill();
                        Ok(())
                    })
                    .unwrap();
            }
            state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_damage_given_order(&[PowerId::ReaperForm])
            );
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(!state.history.over);
            assert_eq!(
                state.history.doom_applied_by_player_this_turn,
                pet == 0,
                "{pet}"
            );
        }
    }

    #[test]
    fn monarch_artifact_gate_controls_later_reaper_and_nested_kill() {
        for artifact in 0..=2 {
            let mut state = HotState::at_defaults();
            let mut target = HotMonster::new(MonsterKind::Toadpole, 10);
            target
                .powers
                .set(PowerId::Artifact, SlotWire::Int, artifact);
            state.monsters_mut().push(target);
            let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
            peer.uid = 1;
            peer.slot = 1;
            state.monsters_mut().push(peer);
            with_monarch_sleight(&mut state, 9);
            state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_damage_given_order(&[PowerId::MonarchsGaze, PowerId::ReaperForm])
            );
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                state.history.doom_applied_by_player_this_turn,
                artifact == 1
            );
            assert_eq!(state.monsters[0].hp, if artifact == 2 { 9 } else { 0 });
            assert_eq!(state.monsters[0].powers.value(PowerId::Artifact), 0);
        }
    }

    // ------------------------------------------------------------------
    // #3338 — Monarch's Gaze + Sleight of Flesh on a retained-death owner.
    // ------------------------------------------------------------------

    /// A Waterfall Giant (SteamPressure makes its death the ABOUT sentinel)
    /// or a Parafright with a live peer, so an Illusion death revives rather
    /// than ending the combat.
    fn monarch_retained_roster(kind: MonsterKind, hp: i32, upkeep: bool) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        state.fanouts.set_misery_attachment_upkeep(upkeep);
        let mut owner = HotMonster::new(kind, hp);
        owner.max_hp = 250;
        owner.uid = 17;
        if kind == MonsterKind::WaterfallGiant {
            owner.powers.set(PowerId::SteamPressure, SlotWire::Int, 20);
        }
        state.monsters_mut().push(owner);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        with_monarch_sleight(&mut state, 9);
        state
    }

    /// Give the owner a live Monarch wrapper of `amount`, as a prior
    /// application through the shared writer would have left it.
    fn with_prior_monarch_wrapper(state: &mut HotState, amount: i32, upkeep: bool) {
        let monster = &mut state.monsters_mut()[0];
        write_monster_temp_strength_wrapper(
            monster,
            crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown,
            amount,
            crate::hot::Applier::Player,
            upkeep,
        )
        .unwrap();
    }

    fn hit_owner(state: &mut HotState) -> Result<i32, EngineRefusal> {
        damage_monster(
            state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
    }

    #[test]
    fn monarch_fresh_attach_on_a_killed_waterfall_lands_on_the_sentinel() {
        for upkeep in [false, true] {
            // 10 HP: the hit takes 1, the nested Sleight takes the other 9.
            let mut state = monarch_retained_roster(MonsterKind::WaterfallGiant, 10, upkeep);
            hit_owner(&mut state).unwrap();
            let waterfall = &state.monsters[0];
            assert!(waterfall.is_about_to_blow(), "{upkeep}");
            // Death cleanup removed the nested Strength; the wrapper attached
            // afterwards, to the sentinel, with nothing to offset it.
            assert_eq!(waterfall.powers.value(PowerId::Strength), 0);
            assert_eq!(waterfall.powers.value(PowerId::TempStrength), -1);
            assert_eq!(
                wrapper_rows(waterfall),
                if upkeep {
                    vec![crate::hot::AttachmentRecord {
                        power: crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown,
                        applier: crate::hot::Applier::Player,
                        amount: 1,
                    }]
                } else {
                    vec![]
                },
            );
            // The side-end unwind (`0x348ba8`) therefore GAINS the sentinel
            // one Strength.
            let mut sentinel = state.monsters[0].clone();
            assert_eq!(
                unwind_monster_temp_strength_wrappers(&mut sentinel, upkeep).unwrap(),
                1
            );
        }
    }

    #[test]
    fn monarch_fresh_attach_on_a_killed_illusion_never_attaches() {
        for upkeep in [false, true] {
            let mut state = monarch_retained_roster(MonsterKind::Parafright, 10, upkeep);
            hit_owner(&mut state).unwrap();
            let parafright = &state.monsters[0];
            assert!(parafright.hp <= 0);
            assert_eq!(
                parafright.revive_stage, 1,
                "the Illusion corpse is reviving"
            );
            // `CanReceivePowers` is false on a reviving owner: no wrapper,
            // and the Type-1 Strength it applied survives the death.
            assert_eq!(parafright.powers.value(PowerId::TempStrength), 0);
            assert_eq!(parafright.powers.value(PowerId::Strength), -1);
            assert!(wrapper_rows(parafright).is_empty());
        }
    }

    #[test]
    fn monarch_restack_on_a_killed_retained_owner_keeps_the_shared_order() {
        // Restack: `ModifyAmount` raises the one instance before its own
        // listener applies the Strength that the Sleight kill follows.
        let mut waterfall = monarch_retained_roster(MonsterKind::WaterfallGiant, 10, true);
        with_prior_monarch_wrapper(&mut waterfall, 2, true);
        hit_owner(&mut waterfall).unwrap();
        let sentinel = &waterfall.monsters[0];
        assert!(sentinel.is_about_to_blow());
        assert_eq!(sentinel.powers.value(PowerId::TempStrength), 0);
        assert_eq!(sentinel.powers.value(PowerId::Strength), 0);
        assert!(wrapper_rows(sentinel).is_empty());

        // Illusion keeps the raised ITemporaryPower wrapper and the Strength.
        let mut illusion = monarch_retained_roster(MonsterKind::Parafright, 10, true);
        with_prior_monarch_wrapper(&mut illusion, 2, true);
        hit_owner(&mut illusion).unwrap();
        let corpse = &illusion.monsters[0];
        assert_eq!(corpse.revive_stage, 1);
        assert_eq!(corpse.powers.value(PowerId::TempStrength), -3);
        assert_eq!(corpse.powers.value(PowerId::Strength), -3);
    }

    #[test]
    fn monarch_fresh_attach_behind_another_wrapper_is_still_fresh() {
        // A recorded ledger holding a DIFFERENT model is a fresh attach for
        // this one, so the Illusion death still leaves no Monarch wrapper.
        let mut state = monarch_retained_roster(MonsterKind::Parafright, 10, true);
        write_monster_temp_strength_wrapper(
            &mut state.monsters_mut()[0],
            crate::hot::AttachedPowerModel::DarkShackles,
            2,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        hit_owner(&mut state).unwrap();
        let corpse = &state.monsters[0];
        assert_eq!(corpse.revive_stage, 1);
        assert_eq!(corpse.powers.value(PowerId::TempStrength), -2);
        assert_eq!(corpse.powers.value(PowerId::Strength), -3);
    }

    #[test]
    fn monarch_unknown_wrapper_existence_refuses_only_on_a_nested_death() {
        // Without the ledger a nonzero TempStrength cannot say whether this
        // model is already attached.
        let mut lethal = monarch_retained_roster(MonsterKind::WaterfallGiant, 10, false);
        with_prior_monarch_wrapper(&mut lethal, 2, false);
        assert_eq!(
            hit_owner(&mut lethal),
            Err(EngineRefusal::MalformedArgs(
                "Monarch Sleight retained temporary wrapper"
            )),
        );

        // With no death, both orders reach the same state.
        let mut survives = monarch_retained_roster(MonsterKind::WaterfallGiant, 30, false);
        with_prior_monarch_wrapper(&mut survives, 2, false);
        hit_owner(&mut survives).unwrap();
        let waterfall = &survives.monsters[0];
        assert_eq!(waterfall.hp, 20);
        assert!(!waterfall.is_about_to_blow());
        assert_eq!(waterfall.powers.value(PowerId::TempStrength), -3);
        assert_eq!(waterfall.powers.value(PowerId::Strength), -3);
    }

    #[test]
    fn monarch_fresh_attach_without_a_death_keeps_the_strength_then_wrapper_ledger() {
        let mut state = monarch_retained_roster(MonsterKind::WaterfallGiant, 30, true);
        hit_owner(&mut state).unwrap();
        let waterfall = &state.monsters[0];
        assert_eq!(waterfall.hp, 20);
        assert_eq!(waterfall.powers.value(PowerId::TempStrength), -1);
        assert_eq!(waterfall.powers.value(PowerId::Strength), -1);
        let order: Vec<_> = waterfall
            .misery_debuff_order
            .attachments()
            .map(|record| record.power)
            .collect();
        assert_eq!(
            order,
            [
                crate::hot::AttachedPowerModel::Strength,
                crate::hot::AttachedPowerModel::MonarchsGazeStrengthDown,
            ],
        );
    }

    #[test]
    fn monarch_waterfall_death_without_the_sentinel_refuses() {
        // No Steam Pressure: this Rust death does not form the sentinel,
        // and a retained-but-dead attach is not represented.
        let mut state = monarch_retained_roster(MonsterKind::WaterfallGiant, 10, true);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 0);
        assert_eq!(
            hit_owner(&mut state),
            Err(EngineRefusal::MalformedArgs(
                "Monarch Sleight retained temporary wrapper"
            )),
        );
    }

    #[test]
    fn monarch_on_a_retained_owner_without_sleight_keeps_the_shared_writer() {
        let mut state = monarch_retained_roster(MonsterKind::Parafright, 10, true);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 0);
        assert!(state.fanouts.set_after_power_amount_changed_order(&[]));
        let mut events = Vec::new();
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 9);
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -1);
        // The shared writer notes TempStrength before Strength.
        let notes: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::PowerChanged { power, .. } => Some(*power),
                _ => None,
            })
            .collect();
        assert_eq!(notes, [PowerId::TempStrength, PowerId::Strength]);
    }

    // ------------------------------------------------------------------
    // #3342 — every other temporary-Strength wrapper producer takes the same
    // native fresh-attach order on a retained-death owner under Sleight.
    // ------------------------------------------------------------------

    /// The producers that reach `PowerCmd/<Apply>d__2` `0x3efbac` besides
    /// Monarch's Gaze: the card seam (Dark Shackles, and Piercing Wail through
    /// the shared gate), Shackling Potion, and a `Misery` clone.
    #[derive(Clone, Copy, Debug)]
    enum WrapperProducer {
        DarkShacklesCard,
        PiercingWailGate,
        ShacklingPotion,
        MiseryClone(crate::hot::Applier),
    }

    const PLAYER_PRODUCERS: [WrapperProducer; 4] = [
        WrapperProducer::DarkShacklesCard,
        WrapperProducer::PiercingWailGate,
        WrapperProducer::ShacklingPotion,
        WrapperProducer::MiseryClone(crate::hot::Applier::Player),
    ];

    impl WrapperProducer {
        fn model(self) -> crate::hot::AttachedPowerModel {
            use crate::hot::AttachedPowerModel as Model;
            match self {
                Self::DarkShacklesCard => Model::DarkShackles,
                Self::PiercingWailGate => Model::PiercingWail,
                Self::ShacklingPotion => Model::ShacklingPotion,
                Self::MiseryClone(_) => Model::EnfeeblingTouch,
            }
        }

        fn apply(
            self,
            state: &mut HotState,
            amount: i32,
            events: &mut Vec<Event>,
        ) -> Result<(), EngineRefusal> {
            match self {
                Self::DarkShacklesCard => crate::steps::shared::apply_temp_strength_enemy(
                    state,
                    0,
                    self.model(),
                    amount,
                    events,
                ),
                Self::PiercingWailGate => apply_monster_temp_strength_wrapper_after_type_two_gate(
                    state,
                    None,
                    0,
                    self.model(),
                    amount,
                    crate::hot::Applier::Player,
                    events,
                ),
                Self::ShacklingPotion => apply_potion_temporary_strength(state, 0, amount, events),
                Self::MiseryClone(applier) => {
                    let catalog = CatalogBuilder::new().build();
                    apply_card_monster_temp_strength_wrapper_clone(
                        state,
                        &catalog,
                        0,
                        self.model(),
                        applier,
                        amount,
                        events,
                    )
                }
            }
        }
    }

    /// [`monarch_retained_roster`] with Sleight of Flesh as the only live
    /// listener, so nothing but the producer under test applies a wrapper.
    fn sleight_retained_roster(kind: MonsterKind, hp: i32, upkeep: bool) -> HotState {
        let mut state = monarch_retained_roster(kind, 250, upkeep);
        state.monsters_mut()[0].hp = hp;
        state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 0);
        assert!(state.fanouts.set_after_damage_given_order(&[]));
        state
    }

    fn with_prior_wrapper(
        state: &mut HotState,
        model: crate::hot::AttachedPowerModel,
        amount: i32,
        upkeep: bool,
    ) {
        write_monster_temp_strength_wrapper(
            &mut state.monsters_mut()[0],
            model,
            amount,
            crate::hot::Applier::Player,
            upkeep,
        )
        .unwrap();
    }

    fn sleight_retained_refusal() -> EngineRefusal {
        EngineRefusal::MalformedArgs("Sleight retained temporary wrapper")
    }

    #[test]
    fn wrapper_producers_fresh_attach_on_a_killed_illusion_never_attaches() {
        for producer in PLAYER_PRODUCERS {
            for upkeep in [false, true] {
                // 9 HP: the nested Sleight of Flesh (9) kills it.
                let mut state = sleight_retained_roster(MonsterKind::Parafright, 9, upkeep);
                producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
                let corpse = &state.monsters[0];
                assert!(corpse.hp <= 0, "{producer:?}");
                assert_eq!(corpse.revive_stage, 1, "{producer:?}");
                // `CanReceivePowers` is false on the reviving owner
                // (`0xa3b1b`): no wrapper, and the Type-1 Strength survives.
                assert_eq!(
                    corpse.powers.value(PowerId::TempStrength),
                    0,
                    "{producer:?}"
                );
                assert_eq!(corpse.powers.value(PowerId::Strength), -3, "{producer:?}");
                assert!(wrapper_rows(corpse).is_empty(), "{producer:?}");
            }
        }
    }

    #[test]
    fn wrapper_producers_fresh_attach_on_a_killed_waterfall_lands_on_the_sentinel() {
        for producer in PLAYER_PRODUCERS {
            for upkeep in [false, true] {
                let mut state = sleight_retained_roster(MonsterKind::WaterfallGiant, 9, upkeep);
                producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
                let sentinel = &state.monsters[0];
                assert!(sentinel.is_about_to_blow(), "{producer:?}");
                // Death cleanup removed the nested Strength; the wrapper
                // attached afterwards, to the sentinel.
                assert_eq!(sentinel.powers.value(PowerId::Strength), 0, "{producer:?}");
                assert_eq!(
                    sentinel.powers.value(PowerId::TempStrength),
                    -3,
                    "{producer:?}"
                );
                let expected = if upkeep {
                    vec![crate::hot::AttachmentRecord {
                        power: producer.model(),
                        applier: crate::hot::Applier::Player,
                        amount: 3,
                    }]
                } else {
                    vec![]
                };
                assert_eq!(wrapper_rows(sentinel), expected, "{producer:?}");
                // Its side-end unwind (`0x348ba8`) GAINS the sentinel Strength.
                let mut unwound = sentinel.clone();
                assert_eq!(
                    unwind_monster_temp_strength_wrappers(&mut unwound, upkeep).unwrap(),
                    3,
                    "{producer:?}"
                );
            }
        }
    }

    #[test]
    fn wrapper_producers_restack_on_a_killed_retained_owner_keeps_the_shared_order() {
        for producer in PLAYER_PRODUCERS {
            // Restack: `ModifyAmount` raises the one instance before its own
            // listener applies the Strength the Sleight kill follows.
            let mut waterfall = sleight_retained_roster(MonsterKind::WaterfallGiant, 9, true);
            with_prior_wrapper(&mut waterfall, producer.model(), 2, true);
            producer.apply(&mut waterfall, 3, &mut Vec::new()).unwrap();
            let sentinel = &waterfall.monsters[0];
            assert!(sentinel.is_about_to_blow(), "{producer:?}");
            assert_eq!(
                sentinel.powers.value(PowerId::TempStrength),
                0,
                "{producer:?}"
            );
            assert_eq!(sentinel.powers.value(PowerId::Strength), 0, "{producer:?}");
            assert!(wrapper_rows(sentinel).is_empty(), "{producer:?}");

            // Illusion keeps the raised ITemporaryPower wrapper and Strength.
            let mut illusion = sleight_retained_roster(MonsterKind::Parafright, 9, true);
            with_prior_wrapper(&mut illusion, producer.model(), 2, true);
            producer.apply(&mut illusion, 3, &mut Vec::new()).unwrap();
            let corpse = &illusion.monsters[0];
            assert_eq!(corpse.revive_stage, 1, "{producer:?}");
            assert_eq!(
                corpse.powers.value(PowerId::TempStrength),
                -5,
                "{producer:?}"
            );
            assert_eq!(corpse.powers.value(PowerId::Strength), -5, "{producer:?}");
        }
    }

    #[test]
    fn wrapper_producers_fresh_attach_without_a_death_keeps_the_strength_then_wrapper_ledger() {
        for producer in PLAYER_PRODUCERS {
            let mut state = sleight_retained_roster(MonsterKind::WaterfallGiant, 30, true);
            producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
            let waterfall = &state.monsters[0];
            assert_eq!(waterfall.hp, 21, "{producer:?}");
            assert_eq!(
                waterfall.powers.value(PowerId::TempStrength),
                -3,
                "{producer:?}"
            );
            assert_eq!(
                waterfall.powers.value(PowerId::Strength),
                -3,
                "{producer:?}"
            );
            let order: Vec<_> = waterfall
                .misery_debuff_order
                .attachments()
                .map(|record| record.power)
                .collect();
            assert_eq!(
                order,
                [crate::hot::AttachedPowerModel::Strength, producer.model()],
                "{producer:?}"
            );
        }
    }

    #[test]
    fn wrapper_producers_unknown_wrapper_existence_refuses_only_on_a_nested_death() {
        for producer in PLAYER_PRODUCERS {
            // Without the ledger a nonzero TempStrength cannot say whether
            // this model is already attached.
            let mut lethal = sleight_retained_roster(MonsterKind::WaterfallGiant, 9, false);
            with_prior_wrapper(&mut lethal, producer.model(), 2, false);
            assert_eq!(
                producer.apply(&mut lethal, 3, &mut Vec::new()),
                Err(sleight_retained_refusal()),
                "{producer:?}"
            );

            // With no death, both orders reach the same state.
            let mut survives = sleight_retained_roster(MonsterKind::WaterfallGiant, 30, false);
            with_prior_wrapper(&mut survives, producer.model(), 2, false);
            producer.apply(&mut survives, 3, &mut Vec::new()).unwrap();
            let waterfall = &survives.monsters[0];
            assert_eq!(waterfall.hp, 21, "{producer:?}");
            assert_eq!(
                waterfall.powers.value(PowerId::TempStrength),
                -5,
                "{producer:?}"
            );
            assert_eq!(
                waterfall.powers.value(PowerId::Strength),
                -5,
                "{producer:?}"
            );
        }
    }

    #[test]
    fn wrapper_producers_dead_corpse_without_a_veto_refuses_even_under_die_for_you() {
        for producer in PLAYER_PRODUCERS {
            for osty in [false, true] {
                // No Steam Pressure: the Rust death does not form the sentinel,
                // and neither native outcome for a dead retained corpse (a
                // wrapper, or Die For You's `0xa1859` veto over a natively
                // removed Strength) is represented.
                let mut state = sleight_retained_roster(MonsterKind::WaterfallGiant, 9, true);
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::SteamPressure, SlotWire::Int, 0);
                if osty {
                    state.fanouts.set_osty(Some((10, 10))).unwrap();
                }
                assert_eq!(
                    producer.apply(&mut state, 3, &mut Vec::new()),
                    Err(sleight_retained_refusal()),
                    "{producer:?} {osty}"
                );
            }

            // The last Decimillipede segment is kept by Reattach but does not
            // revive (`0x341d30` IL_0040-IL_0056), so it has no veto either.
            let mut last = decimillipede_roster(9, 0);
            last.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(
                last.fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            assert_eq!(
                producer.apply(&mut last, 3, &mut Vec::new()),
                Err(sleight_retained_refusal()),
                "{producer:?}"
            );
        }
    }

    #[test]
    fn wrapper_fresh_attach_on_a_killed_reattaching_segment_never_attaches() {
        for producer in PLAYER_PRODUCERS {
            // A live peer: the segment reattaches, and Reattach vetoes hitting
            // it while it revives (`0xa6643`).
            let mut state = decimillipede_roster(9, 50);
            state.fanouts.set_misery_attachment_upkeep(true);
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
            let segment = &state.monsters[0];
            assert_eq!(
                segment.revive_stage,
                super::super::monsters::SEGMENT_REVIVE_DEAD,
                "{producer:?}"
            );
            assert_eq!(
                segment.powers.value(PowerId::TempStrength),
                0,
                "{producer:?}"
            );
            assert_eq!(segment.powers.value(PowerId::Strength), 0, "{producer:?}");
            assert!(wrapper_rows(segment).is_empty(), "{producer:?}");
        }
    }

    #[test]
    fn wrapper_on_the_waterfall_sentinel_attaches_even_under_die_for_you() {
        // `TriggerAboutToBlowState` (`0x3762a8` IL_0039-IL_0043) sets the
        // sentinel's HP to 999999999, so it is alive and DieForYou allows it.
        for producer in PLAYER_PRODUCERS {
            let mut state = sleight_retained_roster(MonsterKind::WaterfallGiant, 9, true);
            state.fanouts.set_osty(Some((10, 10))).unwrap();
            producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
            let sentinel = &state.monsters[0];
            assert!(
                sentinel.is_about_to_blow() && sentinel.hp > 0,
                "{producer:?}"
            );
            assert_eq!(
                sentinel.powers.value(PowerId::TempStrength),
                -3,
                "{producer:?}"
            );
        }
    }

    #[test]
    fn wrapper_fresh_attach_on_a_killed_reviving_test_subject_never_attaches() {
        for producer in PLAYER_PRODUCERS {
            for upkeep in [false, true] {
                let mut state = HotState::at_defaults();
                state.hp = 60;
                state.max_hp = 60;
                state.fanouts.set_misery_attachment_upkeep(upkeep);
                let mut owner = HotMonster::new(MonsterKind::TestSubject, 9);
                owner.max_hp = super::super::monsters::TEST_SUBJECT_FIRST_HP;
                owner.loop_pos = 1;
                owner.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
                state.monsters_mut().push(owner);
                let enrage = super::super::monsters::test_subject_enrage(&state);
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Enrage, SlotWire::Int, enrage);
                state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
                assert!(
                    state
                        .fanouts
                        .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
                );
                producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
                let corpse = &state.monsters[0];
                assert!(corpse.test_subject_adaptable_reviving(), "{producer:?}");
                // Adaptable vetoes hitting its reviving owner (`0x9f5ef`),
                // and the form reset has already wiped the nested Strength.
                assert_eq!(
                    corpse.powers.value(PowerId::TempStrength),
                    0,
                    "{producer:?}"
                );
                assert_eq!(corpse.powers.value(PowerId::Strength), 0, "{producer:?}");
                assert!(wrapper_rows(corpse).is_empty(), "{producer:?}");
            }
        }
    }

    #[test]
    fn wrapper_fresh_attach_on_a_killed_final_form_test_subject_never_attaches() {
        // The final form carries neither death veto, so the kill detaches it
        // (`0x3ebe90` IL_04d5 -> `0x1371d0` IL_009f) before the re-test.
        for producer in PLAYER_PRODUCERS {
            let mut state = HotState::at_defaults();
            state.hp = 60;
            state.max_hp = 60;
            state.fanouts.set_misery_attachment_upkeep(true);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::TestSubject, 9));
            let max_hp = super::super::monsters::test_subject_form_hp(&state, 2).unwrap();
            let owner = &mut state.monsters_mut()[0];
            owner.max_hp = max_hp;
            owner.loop_pos = 4;
            assert!(owner.set_test_subject_respawns(2));
            owner.powers.set(PowerId::Nemesis, SlotWire::Int, 1);
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
            let corpse = &state.monsters[0];
            assert!(corpse.hp <= 0, "{producer:?}");
            assert_eq!(
                corpse.powers.value(PowerId::TempStrength),
                0,
                "{producer:?}"
            );
            assert_eq!(corpse.powers.value(PowerId::Strength), 0, "{producer:?}");
            assert!(wrapper_rows(corpse).is_empty(), "{producer:?}");
        }
    }

    #[test]
    fn misery_clone_with_a_non_player_applier_cannot_kill_and_keeps_the_ledger() {
        // Sleight of Flesh requires the player as applier (`0x344dc4`
        // IL_0067-IL_0075), so a monster-applied clone never kills and the
        // fresh attach lands exactly as the shared writer would.
        let applier = crate::hot::Applier::Monster(1);
        let producer = WrapperProducer::MiseryClone(applier);
        let mut state = sleight_retained_roster(MonsterKind::Parafright, 9, true);
        producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
        let parafright = &state.monsters[0];
        assert_eq!(parafright.hp, 9);
        assert_eq!(parafright.revive_stage, 0);
        assert_eq!(parafright.powers.value(PowerId::TempStrength), -3);
        assert_eq!(parafright.powers.value(PowerId::Strength), -3);
        assert_eq!(
            wrapper_rows(parafright),
            vec![crate::hot::AttachmentRecord {
                power: producer.model(),
                applier,
                amount: 3,
            }],
        );
    }

    #[test]
    fn wrapper_producers_on_a_retained_owner_without_sleight_keep_the_shared_writer() {
        for producer in PLAYER_PRODUCERS {
            let mut state = sleight_retained_roster(MonsterKind::Parafright, 9, true);
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 0);
            assert!(state.fanouts.set_after_power_amount_changed_order(&[]));
            assert!(!temp_strength_wrapper_order_is_observable(&state, 0));
            producer.apply(&mut state, 3, &mut Vec::new()).unwrap();
            assert_eq!(state.monsters[0].hp, 9, "{producer:?}");
            assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -3);
            assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -3);
        }
    }

    #[test]
    fn pending_lethal_reaper_obeys_artifact_and_shroud_before_kill() {
        for artifact in [0, 1] {
            let mut state = HotState::at_defaults();
            let mut target = HotMonster::new(MonsterKind::Toadpole, 1);
            target
                .powers
                .set(PowerId::Artifact, SlotWire::Int, artifact);
            state.monsters_mut().push(target);
            let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
            peer.uid = 1;
            peer.slot = 1;
            state.monsters_mut().push(peer);
            state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
            state.powers.set(PowerId::Shroud, SlotWire::Int, 3);
            assert!(
                state
                    .fanouts
                    .set_after_damage_given_order(&[PowerId::ReaperForm])
            );
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::Shroud])
            );
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                true,
                true,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(state.block, if artifact == 0 { 3 } else { 0 });
            assert_eq!(
                state.history.doom_applied_by_player_this_turn,
                artifact == 0
            );
        }
    }

    #[test]
    fn given_peers_do_not_apply_to_a_fresh_axebot_in_the_old_slot() {
        let mut state = HotState::at_defaults();
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 10);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        state.monsters_mut().push(axebot);
        with_monarch_sleight(&mut state, 9);
        state.powers.set(PowerId::Envenom, SlotWire::Int, 1);
        state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
        assert!(state.fanouts.set_after_damage_given_order(&[
            PowerId::MonarchsGaze,
            PowerId::Envenom,
            PowerId::ReaperForm,
        ]));
        let mut events = Vec::new();
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[0].uid, 1);
        assert_eq!(state.monsters[0].hp, 86);
        assert_eq!(state.monsters[0].powers.value(PowerId::Stock), 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 0);
        assert!(!state.history.doom_applied_by_player_this_turn);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDied { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn retained_sic_em_instance_runs_after_nested_waterfall_death() {
        let (catalog, source, _) = pet_attack_catalog();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 10);
        waterfall.max_hp = 250;
        waterfall.uid = 17;
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 20);
        waterfall.powers.set(PowerId::SicEm, SlotWire::Int, 3);
        state.monsters_mut().push(waterfall);
        state.fanouts.set_osty(Some((2, 2))).unwrap();
        state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::ReaperForm])
        );
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        player_pet_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0],
            1,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.monsters[0].is_about_to_blow());
        assert_eq!(state.monsters[0].powers.value(PowerId::SicEm), 0);
        assert_eq!(
            state
                .fanouts
                .pet()
                .osty()
                .map(|pet| (pet.hp(), pet.max_hp())),
            Some((5, 5))
        );
    }

    #[test]
    fn reaper_after_removed_target_does_not_discount_deaths_door() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 10));
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        with_monarch_sleight(&mut state, 9);
        state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::MonarchsGaze, PowerId::ReaperForm])
        );
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(!state.history.over);
        assert!(!state.history.doom_applied_by_player_this_turn);
    }

    #[test]
    fn reaper_after_nested_terminal_kill_does_not_invent_doom_history() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 15));
        state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 1);
        state.powers.set(PowerId::Envenom, SlotWire::Int, 1);
        state.powers.set(PowerId::ReaperForm, SlotWire::Int, 1);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 13);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Hang, SlotWire::Int, 4);
        assert!(state.fanouts.set_after_damage_given_order(&[
            PowerId::MonarchsGaze,
            PowerId::Envenom,
            PowerId::ReaperForm,
        ]));
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
        );
        let mut events = Vec::new();

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(12),
            true,
            true,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert!(!state.history.doom_applied_by_player_this_turn);
        assert_eq!(state.next_poison_uid, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Hang), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 0);
    }

    fn vicious_draw_fixture() -> (Catalog, CardAtom, CardAtom, HotState) {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
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
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state.powers.set(PowerId::Vicious, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        (catalog, strike, defend, state)
    }

    #[test]
    fn vicious_draws_live_amount_only_for_positive_owner_vulnerable() {
        let (catalog, strike, defend, mut state) = vicious_draw_fixture();
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: 0,
            },
        ]);

        apply_card_monster_debuff_with_catalog(
            &mut state,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);

        state.piles.get_mut(PileId::Hand).make_mut().clear();
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: strike,
            flags: 0,
        });
        powers_after_power_amount_changed(
            &mut state,
            Some(&catalog),
            0,
            Some(PowerId::Vuln),
            -1,
            false,
            crate::hot::Applier::Player,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.piles.get(PileId::Hand).is_empty());

        apply_player_duration_affliction(
            &mut state,
            PowerId::PlayerVuln,
            PowerId::PlayerVulnFresh,
            1,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.powers.value(PowerId::PlayerVuln), 1);
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(state.piles.get(PileId::Draw).len(), 1);
    }

    /// #2727 — every live arm of the walk reads the applier, and the
    /// `ITemporaryPower` leg is the fifth Sleight condition.
    ///
    /// All three represented listeners are player-owned powers that test
    /// `applier == this.Owner` by reference: Vicious `0x34a6f8`
    /// IL_0037-IL_0045, Shroud `0x3446a4` IL_001d-IL_002b and Sleight of Flesh
    /// `0x344dc4` IL_0067-IL_0075. The walk still runs the frozen acquisition
    /// order for a non-player applier; every callback is simply inert.
    ///
    /// `temporary` is the `isinst ITemporaryPower` leg (`0x344dc4`
    /// IL_007a-IL_0087): a change to the *wrapper itself* publishes through
    /// the same hook and Sleight returns on it, which is why the wrapper
    /// writers notify for their nested `StrengthPower` alone. No production
    /// caller passes `true` today, so this is its only witness.
    ///
    /// Mutation control: on `origin/main` the function took no applier, so
    /// the three `Applier::Monster` rows below all fired.
    #[test]
    fn every_after_power_amount_changed_listener_requires_the_player_as_applier() {
        let fixture =
            || {
                let mut state = HotState::at_defaults();
                state
                    .monsters_mut()
                    .push(HotMonster::new(MonsterKind::Toadpole, 100));
                state.powers.set(PowerId::Shroud, SlotWire::Int, 6);
                state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
                assert!(state.fanouts.set_after_power_amount_changed_order(&[
                    PowerId::Shroud,
                    PowerId::SleightOfFlesh,
                ]));
                state
            };

        for (applier, fires) in [
            (crate::hot::Applier::Player, true),
            (crate::hot::Applier::Monster(0), false),
            (crate::hot::Applier::None, false),
            (crate::hot::Applier::Unknown, false),
        ] {
            let mut state = fixture();
            powers_after_power_amount_changed(
                &mut state,
                None,
                0,
                Some(PowerId::Doom),
                3,
                false,
                applier,
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(
                state.block,
                if fires { 6 } else { 0 },
                "{applier:?}: Shroud",
            );
            assert_eq!(
                state.monsters[0].hp,
                if fires { 96 } else { 100 },
                "{applier:?}: Sleight of Flesh",
            );
        }

        // The wrapper's own Type-2 application: `isinst ITemporaryPower`
        // succeeds, so nothing fires even for the player.
        let mut temporary = fixture();
        powers_after_power_amount_changed(
            &mut temporary,
            None,
            0,
            Some(PowerId::Doom),
            3,
            true,
            crate::hot::Applier::Player,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(temporary.block, 0);
        assert_eq!(temporary.monsters[0].hp, 100);
    }

    /// The Vicious arm's own applier leg, which owns an awaited Draw and
    /// therefore also decides whether the enumerator can suspend at all.
    #[test]
    fn vicious_draws_only_for_a_player_applied_vulnerable() {
        for (applier, draws) in [
            (crate::hot::Applier::Player, 2),
            (crate::hot::Applier::Monster(0), 0),
            (crate::hot::Applier::None, 0),
        ] {
            let (catalog, strike, defend, mut state) = vicious_draw_fixture();
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                HotCard {
                    uid: 1,
                    atom: strike,
                    flags: 0,
                },
                HotCard {
                    uid: 2,
                    atom: defend,
                    flags: 0,
                },
            ]);

            powers_after_power_amount_changed(
                &mut state,
                Some(&catalog),
                0,
                Some(PowerId::Vuln),
                1,
                false,
                applier,
                &mut Vec::new(),
            )
            .unwrap();

            assert_eq!(
                state.piles.get(PileId::Hand).len(),
                draws,
                "{applier:?}: Vicious",
            );
        }
    }

    #[test]
    fn vicious_ordinary_draw_keeps_no_draw_hand_cap_and_reshuffle_rng() {
        let (catalog, strike, defend, base) = vicious_draw_fixture();

        let mut no_draw = base.clone();
        no_draw.powers.set(PowerId::NoDraw, SlotWire::Int, 1);
        no_draw
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom: strike,
                flags: 0,
            });
        apply_card_monster_debuff_with_catalog(
            &mut no_draw,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(no_draw.piles.get(PileId::Hand).is_empty());

        let mut full = base.clone();
        full.piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((10..20).map(|uid| HotCard {
                uid,
                atom: strike,
                flags: 0,
            }));
        full.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 20,
            atom: defend,
            flags: 0,
        });
        apply_card_monster_debuff_with_catalog(
            &mut full,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(full.piles.get(PileId::Hand).len(), 10);
        assert_eq!(full.piles.get(PileId::Draw).len(), 1);

        let mut shuffled = base;
        shuffled.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        shuffled.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 30,
                atom: strike,
                flags: 0,
            },
            HotCard {
                uid: 31,
                atom: defend,
                flags: 0,
            },
        ]);
        apply_card_monster_debuff_with_catalog(
            &mut shuffled,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(shuffled.piles.get(PileId::Hand).len(), 2);
        assert!(shuffled.piles.get(PileId::Discard).is_empty());
        assert!(shuffled.rng.get(RngStream::Rng).counter > 0);
    }

    #[test]
    fn vicious_listener_order_is_frozen_and_terminal_breaks_the_suffix() {
        let (catalog, strike, _, mut first) = vicious_draw_fixture();
        first.monsters_mut()[0].hp = 5;
        first.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        first.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            first
                .fanouts
                .set_after_power_amount_changed_order(
                    &[PowerId::Vicious, PowerId::SleightOfFlesh,]
                )
        );
        first.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: strike,
            flags: 0,
        });
        apply_card_monster_debuff_with_catalog(
            &mut first,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(first.piles.get(PileId::Hand).len(), 1);
        assert!(first.history.over);

        let (_, strike, _, mut last) = vicious_draw_fixture();
        last.monsters_mut()[0].hp = 5;
        last.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        last.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        assert!(
            last.fanouts
                .set_after_power_amount_changed_order(
                    &[PowerId::SleightOfFlesh, PowerId::Vicious,]
                )
        );
        last.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: strike,
            flags: 0,
        });
        apply_card_monster_debuff_with_catalog(
            &mut last,
            &catalog,
            0,
            PowerId::Vuln,
            MiseryToken::Vuln,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(last.piles.get(PileId::Hand).is_empty());
        assert!(last.history.over);
    }

    #[test]
    fn vicious_late_unknown_atom_and_missing_catalog_refuse_atomically() {
        let (catalog, _, _, mut state) = vicious_draw_fixture();
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: u16::MAX,
            flags: 0,
        });
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 7 }];
        let events_before = events.clone();
        assert_eq!(
            apply_card_monster_debuff_with_catalog(
                &mut state,
                &catalog,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::UnknownAtom(u16::MAX))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        assert_eq!(
            apply_power_monster_debuff_with_catalog(
                &mut state,
                &catalog,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::UnknownAtom(u16::MAX))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        assert_eq!(
            apply_card_monster_debuff(
                &mut state,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Vicious Vulnerable producer catalog"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        assert_eq!(
            apply_power_monster_debuff(
                &mut state,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Vicious Vulnerable producer catalog"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        state.piles.get_mut(PileId::Draw).make_mut().clear();
        assert!(state.fanouts.set_after_power_amount_changed_order(&[]));
        let before = state.clone();
        let events_before = events.clone();
        assert_eq!(
            apply_card_monster_debuff_with_catalog(
                &mut state,
                &catalog,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "after-power-amount-changed listener order"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn vicious_orphan_token_at_zero_or_negative_amount_refuses_atomically() {
        for vicious in [0, -1] {
            let (catalog, _, _, mut state) = vicious_draw_fixture();
            state.powers.set(PowerId::Vicious, SlotWire::Int, vicious);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::Vicious])
            );
            state.rng.set(
                RngStream::Rng,
                RngStreamState {
                    words: [11, 22, 33, 44],
                    counter: 7,
                },
            );
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 19 }];
            let events_before = events.clone();

            assert_eq!(
                apply_card_monster_debuff_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    PowerId::Vuln,
                    MiseryToken::Vuln,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                )),
                "orphan Vicious amount {vicious}"
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);

            assert_eq!(
                apply_power_monster_debuff_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    PowerId::Vuln,
                    MiseryToken::Vuln,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                )),
                "power-sourced orphan Vicious amount {vicious}"
            );
            assert_eq!(state, before);
            assert_eq!(events, events_before);
        }

        let (catalog, _, _, mut state) = vicious_draw_fixture();
        state.powers.set(PowerId::Vicious, SlotWire::Int, -1);
        assert!(state.fanouts.set_after_power_amount_changed_order(&[]));
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [55, 66, 77, 88],
                counter: 9,
            },
        );
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 29 }];
        let events_before = events.clone();
        assert_eq!(
            apply_card_monster_debuff_with_catalog(
                &mut state,
                &catalog,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "after-power-amount-changed listener order"
            ))
        );
        assert_eq!(state, before);
        assert_eq!(events, events_before);
    }

    #[test]
    fn vicious_negative_and_wrong_wire_peers_refuse_catalog_commands_atomically() {
        for (label, vicious, shroud, sleight, order) in [
            (
                "negative Shroud without token",
                (SlotWire::Int, 2),
                (SlotWire::Int, -1),
                (SlotWire::Int, 0),
                &[PowerId::Vicious][..],
            ),
            (
                "negative Sleight without token",
                (SlotWire::Int, 2),
                (SlotWire::Int, 0),
                (SlotWire::Int, -1),
                &[PowerId::Vicious][..],
            ),
            (
                "Bool Vicious",
                (SlotWire::Bool, 1),
                (SlotWire::Int, 0),
                (SlotWire::Int, 0),
                &[PowerId::Vicious][..],
            ),
            (
                "Bool Shroud",
                (SlotWire::Int, 2),
                (SlotWire::Bool, 1),
                (SlotWire::Int, 0),
                &[PowerId::Vicious, PowerId::Shroud][..],
            ),
            (
                "Bool Sleight",
                (SlotWire::Int, 2),
                (SlotWire::Int, 0),
                (SlotWire::Bool, 1),
                &[PowerId::Vicious, PowerId::SleightOfFlesh][..],
            ),
        ] {
            let (catalog, _, _, mut state) = vicious_draw_fixture();
            state.powers.set(PowerId::Vicious, vicious.0, vicious.1);
            state.powers.set(PowerId::Shroud, shroud.0, shroud.1);
            state
                .powers
                .set(PowerId::SleightOfFlesh, sleight.0, sleight.1);
            assert!(state.fanouts.set_after_power_amount_changed_order(order));
            state.rng.set(
                RngStream::Rng,
                RngStreamState {
                    words: [91, 92, 93, 94],
                    counter: 13,
                },
            );
            let before = state.clone();
            let mut events = vec![Event::TurnBegan { turn: 31 }];
            let events_before = events.clone();

            assert_eq!(
                apply_card_monster_debuff_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    PowerId::Vuln,
                    MiseryToken::Vuln,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                )),
                "card seam: {label}"
            );
            assert_eq!(state, before, "card seam: {label}");
            assert_eq!(events, events_before, "card seam: {label}");

            assert_eq!(
                apply_power_monster_debuff_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    PowerId::Vuln,
                    MiseryToken::Vuln,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "after-power-amount-changed listener order"
                )),
                "power seam: {label}"
            );
            assert_eq!(state, before, "power seam: {label}");
            assert_eq!(events, events_before, "power seam: {label}");
        }
    }

    #[test]
    fn vicious_rehearsal_restores_the_active_play_lamp_latch() {
        let (catalog, strike, _, mut state) = vicious_draw_fixture();
        state.fanouts.set_unsettling_lamp_available(true);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: u16::MAX,
            flags: 0,
        });
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 23 }];
        let events_before = events.clone();

        crate::engine::play::with_test_active_play(700, || {
            assert_eq!(
                crate::engine::play::active_card_lamp_triggered(),
                Some(false)
            );
            assert_eq!(
                apply_card_monster_debuff_with_catalog(
                    &mut state,
                    &catalog,
                    0,
                    PowerId::Vuln,
                    MiseryToken::Vuln,
                    1,
                    &mut events,
                ),
                Err(EngineRefusal::UnknownAtom(u16::MAX))
            );
            assert_eq!(
                crate::engine::play::active_card_lamp_triggered(),
                Some(false)
            );
        });
        assert_eq!(state, before);
        assert_eq!(events, events_before);

        state.piles.get_mut(PileId::Draw).make_mut()[0].atom = strike;
        crate::engine::play::with_test_active_play(701, || {
            apply_card_monster_debuff_with_catalog(
                &mut state,
                &catalog,
                0,
                PowerId::Vuln,
                MiseryToken::Vuln,
                1,
                &mut events,
            )
            .unwrap();
            assert_eq!(
                crate::engine::play::active_card_lamp_triggered(),
                Some(true)
            );
        });
        assert_eq!(state.monsters[0].powers.value(PowerId::Vuln), 2);
        assert_eq!(state.piles.get(PileId::Hand).len(), 1);
    }

    #[test]
    fn shroud_reads_live_amount_for_player_doom_and_shadowmeld_flat_block() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::Shroud, SlotWire::Int, 4);
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud])
        );
        let mut events = Vec::new();

        apply_card_monster_debuff(
            &mut state,
            0,
            PowerId::Doom,
            MiseryToken::Doom,
            13,
            &mut events,
        )
        .unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Doom), 13);
        assert_eq!(state.block, 16);
        assert!(state.history.doom_applied_by_player_this_turn);
        assert_eq!(
            events,
            [
                Event::PowerChanged {
                    subject: Subject::Monster(state.monsters[0].uid),
                    power: PowerId::Doom,
                    amount: 13,
                },
                Event::PlayerBlockGained {
                    amount: 16,
                    block: 16,
                },
            ]
        );
    }

    #[test]
    fn shroud_ignores_every_non_doom_player_power_amount_change() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::Shroud, SlotWire::Int, 4);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud])
        );
        let mut events = Vec::new();

        apply_card_monster_debuff(
            &mut state,
            0,
            PowerId::Weak,
            MiseryToken::Weak,
            2,
            &mut events,
        )
        .unwrap();
        apply_card_monster_strength_delta(&mut state, 0, -3, &mut events).unwrap();

        assert_eq!(state.block, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -3);
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, Event::PlayerBlockGained { .. }))
        );
    }

    #[test]
    fn shroud_frozen_order_continues_after_sleight_kills_only_the_owner() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 5));
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        state.powers.set(PowerId::Shroud, SlotWire::Int, 3);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh, PowerId::Shroud,])
        );
        let mut events = Vec::new();

        apply_card_monster_debuff(
            &mut state,
            0,
            PowerId::Doom,
            MiseryToken::Doom,
            4,
            &mut events,
        )
        .unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert!(state.monsters[1].hp > 0);
        assert!(!state.history.over);
        assert_eq!(state.block, 3, "the frozen Shroud listener still follows");
    }

    #[test]
    fn lethal_shroud_juggernaut_suppresses_the_remaining_power_amount_walk() {
        let mut state = HotState::at_defaults();
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        state.powers.set(PowerId::Shroud, SlotWire::Int, 3);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 9);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Shroud, PowerId::SleightOfFlesh,])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let mut events = Vec::new();

        apply_card_monster_debuff(
            &mut state,
            0,
            PowerId::Doom,
            MiseryToken::Doom,
            4,
            &mut events,
        )
        .unwrap();

        assert!(state.history.over);
        assert_eq!(state.block, 3);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                .count(),
            1,
            "terminal Shroud -> Juggernaut suppresses later Sleight"
        );
    }

    #[test]
    fn shroud_flat_block_refuses_native_overflow_before_block_or_events() {
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 1);
        let mut events = vec![Event::PlayerBlockGained {
            amount: 1,
            block: 1,
        }];
        let before = state.clone();
        let before_events = events.clone();

        assert_eq!(
            gain_flat_power_block(&mut state, None, i32::MAX, &mut events),
            Err(EngineRefusal::CounterOverflow("flat player block"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn melancholy_discounts_all_piles_for_actual_enemy_pet_and_player_deaths() {
        let mut builder = CatalogBuilder::new();
        for level in 0..=1 {
            builder
                .intern(CardIdentity {
                    id: CardId::Melancholy,
                    upgrade: level,
                    enchantment: None,
                })
                .unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.next_card_uid = 1;
        for (n, pile) in PileId::ALL.into_iter().enumerate() {
            let card = super::super::cards::mint_card(
                &mut state,
                &catalog,
                CardIdentity {
                    id: CardId::Melancholy,
                    upgrade: (n % 2) as u8,
                    enchantment: None,
                },
            )
            .unwrap();
            state.piles.get_mut(pile).make_mut().push(card);
        }
        state.monsters_mut().extend([
            HotMonster::new(MonsterKind::Toadpole, 0),
            HotMonster::new(MonsterKind::Toadpole, 50),
        ]);
        state.monsters_mut()[1].uid = 1;
        finish_monster_death(&mut state, 0, &mut Vec::new()).unwrap();
        for uid in 1..=5 {
            assert_eq!(
                state.card_states.get(uid).local_cost_modifiers.resolve(3),
                2
            );
        }
        assert!(state.exact_piles);
        let later = super::super::cards::mint_card(
            &mut state,
            &catalog,
            CardIdentity {
                id: CardId::Melancholy,
                upgrade: 0,
                enchantment: None,
            },
        )
        .unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(later);
        assert_eq!(
            state
                .card_states
                .get(later.uid)
                .local_cost_modifiers
                .resolve(3),
            3
        );
        state.fanouts.set_osty(Some((1, 1))).unwrap();
        thorns_retaliation_pet(&mut state, None, 1, u32::MAX, &mut Vec::new()).unwrap();
        for uid in 1..=5 {
            assert_eq!(
                state.card_states.get(uid).local_cost_modifiers.resolve(3),
                1
            );
        }
        assert_eq!(
            state
                .card_states
                .get(later.uid)
                .local_cost_modifiers
                .resolve(3),
            2
        );
        let doc = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt = crate::boundary::HotBoundary::catalog_from_canonical(&doc).unwrap();
        let reloaded = crate::boundary::HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
        assert_eq!(
            crate::boundary::HotBoundary::try_to_canonical(&reloaded, &rebuilt).unwrap(),
            doc
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::FairyInABottle)],
            false,
            false,
            false,
            false,
            true
        ));
        state.hp = 0;
        assert!(resolve_player_lethal(&mut state, &mut Vec::new()).unwrap());
        assert_eq!(
            state.card_states.get(1).local_cost_modifiers.resolve(3),
            1,
            "prevented death must not discount"
        );
        state.hp = 0;
        assert!(!resolve_player_lethal(&mut state, &mut Vec::new()).unwrap());
        assert_eq!(state.card_states.get(1).local_cost_modifiers.resolve(3), 0);
        assert_eq!(
            state
                .card_states
                .get(later.uid)
                .local_cost_modifiers
                .resolve(3),
            1
        );
    }

    #[test]
    fn melancholy_counts_secondary_deaths_and_uses_discounted_cost_when_played() {
        let identity = CardIdentity {
            id: CardId::Melancholy,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = queen_damage_state(super::super::monsters::TORCH_HEAD_AMALGAM_HP, 1);
        state.next_card_uid = 1;
        let card = super::super::cards::mint_card(&mut state, &catalog, identity).unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        damage_monster(
            &mut state,
            1,
            DotNetDecimal::from_i64(1),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            state
                .card_states
                .get(card.uid)
                .local_cost_modifiers
                .resolve(3),
            1,
            "Queen and the forced secondary death each discount once"
        );
        // Isolate subsequent play with a live ordinary foe: the cost sidecar
        // must feed the public action's energy check and debit after reload.
        state.monsters = vec![HotMonster::new(MonsterKind::Toadpole, 50)].into();
        state.history.over = false;
        state.energy = 1;
        state.block = 0;
        let doc = crate::boundary::HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt = crate::boundary::HotBoundary::catalog_from_canonical(&doc).unwrap();
        let restored = crate::boundary::HotBoundary::from_canonical(&doc, &rebuilt).unwrap();
        let played = crate::engine::apply_action(
            &restored,
            &rebuilt,
            &crate::engine::Action::Play {
                uid: card.uid,
                target: None,
                selection: crate::engine::SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(played.energy, 0);
        assert_eq!(played.block, 17);
        assert_eq!(
            played.piles.get(PileId::Discard).as_slice()[0].uid,
            card.uid
        );
        assert_eq!(
            played
                .card_states
                .get(card.uid)
                .local_cost_modifiers
                .resolve(3),
            1
        );
    }

    #[test]
    fn terminal_actual_death_normalizes_all_five_piles_in_native_order() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 40;
        // Reverse insertion makes the asserted UID sequence discriminate the
        // native Hand/Draw/Discard/Exhaust/Play snapshot order.
        for pile in [
            PileId::Play,
            PileId::Exhaust,
            PileId::Discard,
            PileId::Draw,
            PileId::Hand,
        ] {
            inject_legacy_bottom(&mut state, &catalog, infection, 1, pile).unwrap();
        }
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 0);
        monster.uid = 7;
        state.monsters_mut().push(monster);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        assert_eq!(
            [
                PileId::Hand,
                PileId::Draw,
                PileId::Discard,
                PileId::Exhaust,
                PileId::Play,
            ]
            .map(|pile| state.piles.get(pile).as_slice()[0].uid),
            [40, 41, 42, 43, 44]
        );
        for pile in [
            PileId::Hand,
            PileId::Draw,
            PileId::Discard,
            PileId::Exhaust,
            PileId::Play,
        ] {
            let cards = state.piles.get(pile).as_slice();
            assert_eq!(cards.len(), 1, "{pile:?} must be non-vacuous");
            assert_eq!(cards[0].flags & CARD_FLAG_LEGACY, 0, "{pile:?}");
        }
        assert_eq!(state.next_card_uid, 45);
        assert!(!state.exact_piles);
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 7 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn reentrant_terminal_state_does_not_gate_actual_death_normalization() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 90;
        for pile in [PileId::Exhaust, PileId::Play] {
            inject_legacy_bottom(&mut state, &catalog, infection, 1, pile).unwrap();
        }
        // Model a reentrant death listener ending combat after the legacy
        // cards already exist but before this death suffix resumes.
        state.history.over = true;
        let mut dead = HotMonster::new(MonsterKind::Toadpole, 0);
        dead.uid = 11;
        let mut survivor = HotMonster::new(MonsterKind::Toadpole, 10);
        survivor.uid = 12;
        state.monsters_mut().extend([dead, survivor]);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Exhaust).as_slice()[0].uid, 90);
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].uid, 91);
        assert_eq!(state.next_card_uid, 92);
        assert!(state.history.over);
        assert_eq!(state.monsters[1].hp, 10);
        assert_eq!(events, [Event::MonsterDied { uid: 11 }]);
    }

    #[test]
    fn each_nonterminal_actual_death_allocates_only_new_legacy_cards() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 9;
        for uid in 20..23 {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 10);
            monster.uid = uid;
            state.monsters_mut().push(monster);
        }
        let mut events = Vec::new();

        inject_legacy_bottom(&mut state, &catalog, infection, 1, PileId::Hand).unwrap();
        state.monsters_mut()[0].hp = 0;
        finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 9);
        assert_eq!(state.next_card_uid, 10);
        assert!(!state.history.over);

        inject_legacy_bottom(&mut state, &catalog, infection, 1, PileId::Draw).unwrap();
        state.monsters_mut()[1].hp = 0;
        finish_monster_death(&mut state, 1, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Hand).as_slice()[0].uid, 9);
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 10);
        assert_eq!(state.next_card_uid, 11);
        assert!(!state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 20 },
                Event::MonsterDied { uid: 21 },
            ]
        );
    }

    /// The pile-card identity boundary is an ALLIED `Hook.AfterDeath`
    /// listener, so it runs before `InfestedPower` (an enemy power) adds the
    /// Wrigglers (#2656, `IterateHookListeners` `0x3f9720` lists `_allies`
    /// before `_enemies`), and before the combat-over check.
    #[test]
    fn actual_death_identity_boundary_runs_before_infested_spawn_and_combat_over() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();

        let mut spawning = HotState::at_defaults();
        spawning.next_card_uid = u32::MAX;
        let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, 0);
        phrog.uid = 7;
        spawning.monsters_mut().push(phrog);
        inject_legacy_bottom(&mut spawning, &catalog, infection, 1, PileId::Discard).unwrap();
        let mut spawn_events = Vec::new();

        assert_eq!(
            finish_monster_death(&mut spawning, 0, &mut spawn_events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(
            spawning.monsters.len(),
            1,
            "no Wriggler before the card listener"
        );
        assert!(!spawning.history.over);
        assert_eq!(spawn_events, [Event::MonsterDied { uid: 7 }]);

        let mut terminal = HotState::at_defaults();
        terminal.next_card_uid = u32::MAX;
        let mut toadpole = HotMonster::new(MonsterKind::Toadpole, 0);
        toadpole.uid = 11;
        terminal.monsters_mut().push(toadpole);
        inject_legacy_bottom(&mut terminal, &catalog, infection, 1, PileId::Discard).unwrap();
        let mut terminal_events = Vec::new();

        assert_eq!(
            finish_monster_death(&mut terminal, 0, &mut terminal_events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert!(!terminal.history.over);
        assert_eq!(terminal_events, [Event::MonsterDied { uid: 11 }]);
    }

    #[test]
    fn actual_death_leaves_ordinary_physical_cards_and_allocator_unchanged() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let mut state = HotState::at_defaults();
        state.next_card_uid = 42;
        let card = HotCard {
            uid: 41,
            atom,
            flags: 0,
        };
        state.piles.get_mut(PileId::Draw).make_mut().push(card);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 0);
        monster.uid = 5;
        state.monsters_mut().push(monster);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        assert_eq!(state.piles.get(PileId::Draw).as_slice(), [card]);
        assert_eq!(state.next_card_uid, 42);
        assert!(!state.exact_piles);
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 5 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn waterfall_first_actual_death_retains_only_steam_pressure_and_forces_about() {
        let mut state = HotState::at_defaults();
        let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 0);
        waterfall.max_hp = 250;
        waterfall.uid = 17;
        waterfall.pressure_gun_damage = 23;
        assert!(waterfall.set_pressure_buildup_idx(4));
        waterfall
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 29);
        waterfall.powers.set(PowerId::Strength, SlotWire::Int, 7);
        waterfall
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -3);
        waterfall.powers.set(PowerId::Weak, SlotWire::Int, 2);
        waterfall.powers.set(PowerId::Doom, SlotWire::Int, 9);
        waterfall.powers.set(PowerId::Strangle, SlotWire::Int, 2);
        state.monsters_mut().push(waterfall);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        let waterfall = &state.monsters[0];
        assert_eq!(waterfall.hp, 999_999_999);
        assert_eq!(waterfall.max_hp, 999_999_999);
        assert_eq!(waterfall.loop_pos, 6);
        assert!(waterfall.is_about_to_blow());
        assert_eq!(waterfall.powers.len(), 1);
        assert_eq!(waterfall.powers.value(PowerId::SteamPressure), 29);
        assert_eq!(waterfall.powers.value(PowerId::Strength), 0);
        assert_eq!(waterfall.powers.value(PowerId::TempStrength), 0);
        assert_eq!(waterfall.powers.value(PowerId::Weak), 0);
        assert_eq!(waterfall.powers.value(PowerId::Doom), 0);
        assert_eq!(waterfall.powers.value(PowerId::Strangle), 0);
        assert!(!state.history.over);
        assert_eq!(events, [Event::MonsterDied { uid: 17 }]);

        state.monsters_mut()[0]
            .powers
            .set(PowerId::SteamPressure, SlotWire::Int, 0);
        state.monsters_mut()[0].hp = 0;
        finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 17 },
                Event::MonsterDied { uid: 17 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn illusion_death_parks_the_same_body_for_revive_without_stripping_strength() {
        let mut state = HotState::at_defaults();
        let mut primary = HotMonster::new(MonsterKind::TheObscura, 129);
        primary.uid = 0;
        primary.slot = 1;
        let mut illusion = HotMonster::new(MonsterKind::Parafright, 0);
        illusion.max_hp = 21;
        illusion.uid = 1;
        illusion.powers.set(PowerId::Strength, SlotWire::Int, 7);
        illusion
            .powers
            .set(PowerId::TempStrength, SlotWire::Int, -3);
        illusion.powers.set(PowerId::Weak, SlotWire::Int, 2);
        illusion.powers.set(PowerId::Vuln, SlotWire::Int, 4);
        state.monsters_mut().extend([illusion, primary]);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        let illusion = &state.monsters[0];
        assert_eq!(illusion.hp, 0);
        assert_eq!(illusion.revive_stage, 1);
        assert_eq!(illusion.powers.value(PowerId::Strength), 7);
        assert_eq!(illusion.powers.value(PowerId::TempStrength), -3);
        assert_eq!(illusion.powers.value(PowerId::Weak), 0);
        assert_eq!(illusion.powers.value(PowerId::Vuln), 0);
        assert!(!state.history.over);
        assert_eq!(events, [Event::MonsterDied { uid: 1 }]);
    }

    fn decimillipede_roster(front_hp: i32, peer_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        let mut front = HotMonster::new(MonsterKind::DecimillipedeSegment, front_hp);
        front.max_hp = 48;
        front.uid = 0;
        front.slot = 0;
        let mut peer = HotMonster::new(MonsterKind::DecimillipedeSegment, peer_hp);
        peer.max_hp = 50;
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().extend([front, peer]);
        state
    }

    /// The #2498 writer: a segment that dies beside a live peer parks at
    /// `revive_stage = 1` and loses every power in
    /// `_finish_monster_death`'s wipe (frozen Python, deleted #2827), Block included.
    #[test]
    fn segment_death_beside_a_live_peer_parks_for_reattach_and_wipes_its_powers() {
        let mut state = decimillipede_roster(0, 50);
        {
            let segment = &mut state.monsters_mut()[0];
            segment.powers.set(PowerId::Strength, SlotWire::Int, 2);
            segment.powers.set(PowerId::TempStrength, SlotWire::Int, 1);
            segment.powers.set(PowerId::Weak, SlotWire::Int, 1);
            segment.powers.set(PowerId::Vuln, SlotWire::Int, 3);
            segment.powers.set(PowerId::Poison, SlotWire::Int, 4);
            segment.powers.set(PowerId::Hang, SlotWire::Int, 2);
            segment.block = 9;
        }
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        let segment = &state.monsters[0];
        assert_eq!(
            segment.revive_stage,
            super::super::monsters::SEGMENT_REVIVE_DEAD
        );
        for power in [
            PowerId::Strength,
            PowerId::TempStrength,
            PowerId::Weak,
            PowerId::Vuln,
            PowerId::Poison,
            PowerId::Hang,
        ] {
            assert_eq!(
                segment.powers.value(power),
                0,
                "{power:?} survived the wipe"
            );
        }
        assert_eq!(segment.block, 0);
        assert!(!state.history.over);
    }

    /// The veto half: `AreAllOtherSegmentsDead && Owner.IsDead` at
    /// `ReattachPower/<AfterDeath>d__11::MoveNext` IL_0040. The last segment
    /// stays a plain corpse, so the encounter can actually end.
    #[test]
    fn the_last_segment_to_fall_does_not_park_for_reattach() {
        let mut state = decimillipede_roster(0, 0);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        assert_eq!(state.monsters[0].revive_stage, 0);
        assert!(state.history.over);
    }

    /// `_finish_monster_death` (frozen Python, deleted #2827) clears Suck with the rest of the dying
    /// creature's ordinary power state — the slot-3 death-clear lesson.
    #[test]
    fn fossil_stalker_death_clears_its_innate_suck_power() {
        let mut state = HotState::at_defaults();
        let mut stalker = HotMonster::new(MonsterKind::FossilStalker, 0);
        stalker.max_hp = 55;
        stalker.uid = 0;
        stalker.powers.set(PowerId::Suck, SlotWire::Int, 3);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 12);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().extend([stalker, peer]);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 0, &mut events).unwrap();

        assert_eq!(state.monsters[0].powers.value(PowerId::Suck), 0);
    }

    fn fossil_stalker_attacker(hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut stalker = HotMonster::new(MonsterKind::FossilStalker, hp);
        stalker.max_hp = 55;
        stalker.uid = 0;
        stalker.powers.set(PowerId::Suck, SlotWire::Int, 3);
        state.monsters_mut().push(stalker);
        state
    }

    /// Suck's one `Apply<StrengthPower>(Amount * count)` after the whole
    /// command, not one per hit: LASH is `4x2`, so two landed hit groups grant
    /// `3 * 2 = 6` Strength in a single write.
    #[test]
    fn suck_grants_amount_times_the_landed_hit_group_count_once_per_command() {
        let mut state = fossil_stalker_attacker(55);
        let mut events = Vec::new();

        monster_attack_player_positive_results(&mut state, 0, 4, 2, &mut events).unwrap();

        assert_eq!(state.hp, 52);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 6);
    }

    /// The per-group predicate is the *result*, not the swing: a hit fully
    /// absorbed by Block leaves `UnblockedDamage == 0`
    /// (`<>c::<AfterAttack>b__6_1` RVA `0x346c1f`) and does not count, while
    /// the group that gets through does.
    #[test]
    fn suck_counts_only_hit_groups_that_actually_landed() {
        let mut state = fossil_stalker_attacker(55);
        state.block = 4;
        let mut events = Vec::new();

        monster_attack_player_positive_results(&mut state, 0, 4, 2, &mut events).unwrap();

        assert_eq!(state.block, 0);
        assert_eq!(state.hp, 56);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 3);
    }

    /// `PowerCmd::Apply` gates on `CanReceivePowers`, so a Thorns kill during
    /// the command grants nothing — and because the death clear above already
    /// zeroed Suck, the killing group is not counted either.
    #[test]
    fn a_thorns_kill_mid_command_grants_the_dead_stalker_no_suck_strength() {
        let mut state = fossil_stalker_attacker(1);
        state.powers.set(PowerId::Thorns, SlotWire::Int, 5);
        let mut events = Vec::new();

        monster_attack_player_positive_results(&mut state, 0, 4, 2, &mut events).unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Suck), 0);
    }

    /// I5: Suck is Fossil Stalker's alone at exactly 3. A live owner at any
    /// other amount, or any other kind, refuses instead of approximating
    /// (`_validated_suck_amount` (frozen Python, deleted #2827)).
    #[test]
    fn suck_on_an_unsupported_owner_refuses_at_the_after_attack_tail() {
        let mut state = fossil_stalker_attacker(55);
        state.monsters_mut()[0].kind = MonsterKind::Toadpole;
        assert_eq!(
            monster_attack_player_positive_results(&mut state, 0, 4, 2, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("SuckPower owner/amount"))
        );

        let mut state = fossil_stalker_attacker(55);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Suck, SlotWire::Int, 2);
        assert_eq!(
            monster_attack_player_positive_results(&mut state, 0, 4, 2, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs("SuckPower owner/amount"))
        );
    }

    #[test]
    fn final_primary_death_cascades_every_live_secondary_without_revive() {
        let mut state = HotState::at_defaults();
        let mut primary = HotMonster::new(MonsterKind::Ovicopter, 0);
        primary.uid = 0;
        primary.slot = 5;
        let mut egg = HotMonster::new(MonsterKind::ToughEgg, 15);
        egg.uid = 1;
        egg.slot = 4;
        egg.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        egg.powers.set(PowerId::HatchPower, SlotWire::Int, 2);
        let mut illusion = HotMonster::new(MonsterKind::Parafright, 21);
        illusion.uid = 2;
        illusion.slot = 0;
        state.monsters_mut().extend([illusion, egg, primary]);
        let mut events = Vec::new();

        finish_monster_death(&mut state, 2, &mut events).unwrap();

        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert_eq!(state.monsters[0].revive_stage, 0);
        assert_eq!(state.monsters[1].powers.value(PowerId::HatchPower), 0);
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 0 },
                Event::MonsterDied { uid: 2 },
                Event::MonsterDied { uid: 1 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn possess_owner_deaths_restore_only_their_debits_before_terminal() {
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::Strength, SlotWire::Int, -5);
        state.powers.set(PowerId::Dexterity, SlotWire::Int, -5);
        let mut lost = HotMonster::new(MonsterKind::TheLost, super::super::monsters::THE_LOST_HP);
        lost.max_hp = super::super::monsters::THE_LOST_HP;
        lost.possess_strength_debit = -4;
        let mut forgotten = HotMonster::new(
            MonsterKind::TheForgotten,
            super::super::monsters::THE_FORGOTTEN_HP,
        );
        forgotten.max_hp = super::super::monsters::THE_FORGOTTEN_HP;
        forgotten.slot = 1;
        forgotten.uid = 1;
        forgotten.possess_speed_debit = -2;
        state.monsters_mut().extend([lost, forgotten]);
        let mut events = Vec::new();

        state.monsters_mut()[0].hp = 0;
        finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::Strength), -1);
        assert_eq!(state.powers.value(PowerId::Dexterity), -5);
        assert_eq!(state.monsters[0].possess_strength_debit, 0);
        assert_eq!(state.monsters[1].possess_speed_debit, -2);
        assert!(!state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 0 },
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: -1,
                },
            ]
        );

        // #3214: the last primary owner's HP commit makes the combat
        // ending, and `PowerCmd.Apply` (RVA `0x3ef988` IL_0020-IL_0034)
        // returns at IsEnding, so its restore is skipped; the ledger still
        // clears.
        state.monsters_mut()[1].hp = 0;
        finish_monster_death(&mut state, 1, &mut events).unwrap();
        assert_eq!(state.powers.value(PowerId::Dexterity), -5);
        assert_eq!(state.monsters[1].possess_speed_debit, 0);
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 0 },
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: -1,
                },
                Event::MonsterDied { uid: 1 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn possess_restore_waits_for_physical_card_death_listeners() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.next_card_uid = u32::MAX;
        state.powers.set(PowerId::Strength, SlotWire::Int, -2);
        let mut lost = HotMonster::new(MonsterKind::TheLost, 0);
        lost.max_hp = super::super::monsters::THE_LOST_HP;
        lost.possess_strength_debit = -2;
        let mut forgotten = HotMonster::new(
            MonsterKind::TheForgotten,
            super::super::monsters::THE_FORGOTTEN_HP,
        );
        forgotten.max_hp = super::super::monsters::THE_FORGOTTEN_HP;
        forgotten.slot = 1;
        forgotten.uid = 1;
        state.monsters_mut().extend([lost, forgotten]);
        inject_legacy_bottom(&mut state, &catalog, infection, 1, PileId::Discard).unwrap();
        let mut events = Vec::new();

        assert_eq!(
            finish_monster_death(&mut state, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state.powers.value(PowerId::Strength), -2);
        assert_eq!(state.monsters[0].possess_strength_debit, -2);
        assert!(!state.history.over);
        assert_eq!(events, [Event::MonsterDied { uid: 0 }]);
    }

    fn constrict_death_state(owner_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        let mut owner = HotMonster::new(MonsterKind::SlitheringStrangler, owner_hp);
        owner.max_hp = 55;
        owner.uid = 17;
        let mut ai = crate::hot::RandomAiState::new();
        assert!(ai.set_next(Some(crate::engine::turn::STRANGLER_THWACK_INDEX)));
        assert!(ai.set_log(&[
            crate::engine::turn::STRANGLER_CONSTRICT_INDEX,
            crate::engine::turn::STRANGLER_THWACK_INDEX,
        ]));
        owner.random_ai = ai;
        state.monsters_mut().push(owner);
        assert!(state.fanouts.set_constrict_amount(6));
        assert_eq!(
            state.fanouts.register_after_side_turn_end_power(
                crate::hot::AfterSideTurnEndPowerToken::Constrict,
            ),
            Ok(0)
        );
        state
    }

    fn tender_death_state(owner_hp: i32, count: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        let mut owner = HotMonster::new(MonsterKind::HunterKiller, owner_hp);
        owner.max_hp = 126;
        owner.uid = 23;
        let mut ai = crate::hot::RandomAiState::new();
        assert!(ai.set_next(Some(crate::engine::turn::HUNTER_BITE_INDEX)));
        assert!(ai.set_log(&[
            crate::engine::turn::HUNTER_GOOP_INDEX,
            crate::engine::turn::HUNTER_BITE_INDEX,
        ]));
        owner.random_ai = ai;
        state.monsters_mut().push(owner);
        state.powers.set(PowerId::Tender, SlotWire::Int, 1);
        assert!(state.fanouts.set_tender_state(true, count));
        assert_eq!(
            state
                .fanouts
                .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::Tender),
            Ok(0)
        );
        state
    }

    #[test]
    fn constrict_exact_applier_death_clears_before_generic_events_and_peer_death_preserves() {
        let mut state = constrict_death_state(0);
        let mut events = Vec::new();
        finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.fanouts.constrict_amount(), 0);
        assert!(state.history.over);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 17 },
                Event::CombatOver { player_won: true },
            ]
        );

        let mut peer = constrict_death_state(55);
        let mut other = HotMonster::new(MonsterKind::FrogKnight, 0);
        other.max_hp = 10;
        other.uid = 18;
        peer.monsters_mut().push(other);
        let mut events = Vec::new();
        finish_monster_death(&mut peer, 1, &mut events).unwrap();
        assert_eq!(peer.fanouts.constrict_amount(), 6);
        assert!(!peer.history.over);
        assert_eq!(events, [Event::MonsterDied { uid: 18 }]);
    }

    #[test]
    fn constrict_death_action_rolls_removal_and_events_back_on_late_refusal() {
        let infection = CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(infection).unwrap();
        let catalog = builder.build();
        let mut state = constrict_death_state(0);
        state.next_card_uid = u32::MAX;
        inject_legacy_bottom(&mut state, &catalog, infection, 1, PileId::Discard).unwrap();
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 77 }];
        let before_events = events.clone();

        assert_eq!(
            finish_monster_death(&mut state, 0, &mut events),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(state.fanouts.constrict_amount(), 6);
        assert_eq!(events, before_events);
    }

    #[test]
    fn constrict_death_refuses_malformed_or_alien_carriers_before_cleanup() {
        let mut duplicate = constrict_death_state(0);
        let mut second = duplicate.monsters[0].clone();
        second.uid = 18;
        duplicate.monsters_mut().push(second);

        let mut alien = HotState::at_defaults();
        alien
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::FrogKnight, 0));
        assert!(alien.fanouts.set_constrict_amount(3));

        for mut state in [duplicate, alien] {
            let before = state.clone();
            let mut events = Vec::new();
            assert!(finish_monster_death(&mut state, 0, &mut events).is_err());
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn tender_owner_and_peer_deaths_preserve_power_counter_and_event_order() {
        let mut owner = tender_death_state(0, 4);
        let mut peer = HotMonster::new(MonsterKind::FrogKnight, 10);
        peer.max_hp = 10;
        peer.uid = 24;
        owner.monsters_mut().push(peer);
        let mut events = Vec::new();

        finish_monster_death(&mut owner, 0, &mut events).unwrap();
        assert!(!owner.history.over);
        assert!(owner.fanouts.tender_is_active());
        assert_eq!(owner.fanouts.tender_cards_played(), 4);
        assert_eq!(owner.powers.value(PowerId::Tender), 1);
        assert_eq!(events, [Event::MonsterDied { uid: 23 }]);

        let mut nonowner = tender_death_state(126, 5);
        let mut dead_peer = HotMonster::new(MonsterKind::FrogKnight, 0);
        dead_peer.max_hp = 10;
        dead_peer.uid = 24;
        nonowner.monsters_mut().push(dead_peer);
        let mut events = Vec::new();
        finish_monster_death(&mut nonowner, 1, &mut events).unwrap();
        assert!(!nonowner.history.over);
        assert!(nonowner.fanouts.tender_is_active());
        assert_eq!(nonowner.fanouts.tender_cards_played(), 5);
        assert_eq!(events, [Event::MonsterDied { uid: 24 }]);

        let mut terminal = tender_death_state(0, 6);
        let mut events = Vec::new();
        finish_monster_death(&mut terminal, 0, &mut events).unwrap();
        assert!(terminal.history.over);
        assert!(terminal.fanouts.tender_is_active());
        assert_eq!(terminal.fanouts.tender_cards_played(), 6);
        assert_eq!(
            events,
            [
                Event::MonsterDied { uid: 23 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn tender_death_refuses_malformed_raw_carrier_and_alien_owner_atomically() {
        let mut malformed = tender_death_state(0, 1);
        malformed.powers.set(PowerId::Tender, SlotWire::Int, 0);
        malformed.fanouts.forge_tender_state(false, 1);

        let mut alien = tender_death_state(0, 1);
        alien.monsters_mut()[0].kind = MonsterKind::FrogKnight;

        for mut state in [malformed, alien] {
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 91 }];
            let before_events = events.clone();
            assert!(finish_monster_death(&mut state, 0, &mut events).is_err());
            assert_eq!(state, before);
            assert_eq!(events, before_events);
        }
    }

    fn queen_damage_state(amalgam_hp: i32, queen_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, amalgam_hp);
        amalgam.max_hp = super::super::monsters::TORCH_HEAD_AMALGAM_HP;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = HotMonster::new(MonsterKind::Queen, queen_hp);
        queen.max_hp = super::super::monsters::QUEEN_HP;
        queen.slot = 1;
        queen.uid = 1;
        state.monsters_mut().extend([amalgam, queen]);
        state
    }

    #[test]
    fn damage_batch_commits_queen_and_amalgam_before_either_death_callback() {
        let mut state = queen_damage_state(5, 5);
        let catalog = CatalogBuilder::new().build();
        let mut events = Vec::new();
        damage_monsters_after_catalog_auth(
            &mut state,
            &catalog,
            &[0, 1],
            DotNetDecimal::from_i64(10),
            true,
            &mut events,
        )
        .unwrap();
        assert!(state.history.over);
        assert!(
            !state.monsters[1].queen_amalgam_dead(),
            "dead Queen cannot latch Amalgam death"
        );
        assert!(!state.fanouts.synchronous_damage_is_active());
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));
        assert_eq!(
            events,
            [
                Event::MonsterDamaged {
                    uid: 0,
                    blocked: 0,
                    unblocked: 10,
                    hp: -5
                },
                Event::MonsterDamaged {
                    uid: 1,
                    blocked: 0,
                    unblocked: 10,
                    hp: -5
                },
                Event::MonsterDied { uid: 0 },
                Event::MonsterDied { uid: 1 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn damage_batch_handles_either_queen_roster_owner_dying_first() {
        let catalog = CatalogBuilder::new().build();
        for (amalgam_hp, queen_hp) in [(100, 5), (5, 100)] {
            let mut state = queen_damage_state(amalgam_hp, queen_hp);
            damage_monsters_after_catalog_auth(
                &mut state,
                &catalog,
                &[0, 1],
                DotNetDecimal::from_i64(10),
                true,
                &mut Vec::new(),
            )
            .unwrap();
            assert!(!state.fanouts.synchronous_damage_is_active());
            assert!(super::super::monsters::queen_roster_state_is_valid(&state));
            if queen_hp == 5 {
                assert!(state.history.over);
                assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
            } else {
                assert!(!state.history.over);
                assert_eq!(state.monsters[1].hp, 90);
                assert!(state.monsters[1].queen_amalgam_dead());
            }
        }
    }

    #[test]
    fn batch_ending_skips_hand_drill_before_artifact_or_sleight() {
        let mut state = queen_damage_state(100, 5);
        state.set_deep_relic_ownership(true, false, false, false);
        state.monsters_mut()[0].block = 1;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        let catalog = CatalogBuilder::new().build();
        let mut events = Vec::new();
        damage_monsters_after_catalog_auth(
            &mut state,
            &catalog,
            &[0, 1],
            DotNetDecimal::from_i64(10),
            true,
            &mut events,
        )
        .unwrap();
        assert!(state.history.over);
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::PowerChanged {
                power: PowerId::Artifact | PowerId::Vuln,
                ..
            }
        )));
    }

    #[test]
    fn damage_batch_keeps_pending_slug_powers_until_each_cleanup() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        for uid in 0..2 {
            let mut slug = HotMonster::new(MonsterKind::CorpseSlug, 5);
            slug.slot = uid as i32;
            slug.uid = uid;
            slug.max_hp = 27 + uid as i32;
            slug.powers.set(PowerId::Ravenous, SlotWire::Int, 5);
            state.monsters_mut().push(slug);
        }
        assert!(super::super::monsters::corpse_slug_roster_is_valid(
            &state, None
        ));
        let catalog = CatalogBuilder::new().build();
        damage_monsters_after_catalog_auth(
            &mut state,
            &catalog,
            &[0, 1],
            DotNetDecimal::from_i64(10),
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.history.over);
        for slug in state.monsters.iter() {
            assert_eq!(slug.powers.value(PowerId::Strength), 0);
            assert_eq!(slug.powers.value(PowerId::Ravenous), 0);
        }
        assert!(!state.fanouts.synchronous_damage_is_active());
        assert!(super::super::monsters::corpse_slug_roster_is_valid(
            &state, None
        ));
    }

    fn spectral_hex_damage_state(spectral_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.exact_piles = true;
        state.next_card_uid = 1;
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 0,
            atom: 0,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_HEXED,
        });
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(2)));
        assert!(flail.random_ai.set_log(&[2]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, spectral_hp);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(0)));
        assert!(spectral.random_ai.set_log(&[0]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 89);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        state.monsters_mut().extend([flail, spectral, magi]);
        state
    }

    #[test]
    fn spectral_actual_death_clears_hex_but_block_and_sibling_death_do_not() {
        let mut malformed_knockdown = spectral_hex_damage_state(97);
        malformed_knockdown.monsters_mut()[1]
            .misery_debuff_order
            .push(MiseryToken::Knockdown);
        let before = malformed_knockdown.clone();
        let mut malformed_events = vec![Event::TurnBegan { turn: 142 }];
        let events_before = malformed_events.clone();
        assert_eq!(
            damage_monster(
                &mut malformed_knockdown,
                1,
                DotNetDecimal::from_i64(1),
                false,
                true,
                &mut malformed_events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state"
            ))
        );
        assert_eq!(malformed_knockdown, before);
        assert_eq!(malformed_events, events_before);

        let mut blocked = spectral_hex_damage_state(1);
        blocked.monsters_mut()[1].block = 1;
        damage_monster(
            &mut blocked,
            1,
            DotNetDecimal::from_i64(1),
            false,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(blocked.monsters[1].hp, 1);
        assert_eq!(blocked.powers.value(PowerId::HexPower), 2);

        let mut sibling = spectral_hex_damage_state(97);
        damage_monster(
            &mut sibling,
            0,
            DotNetDecimal::from_i64(108),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(sibling.powers.value(PowerId::HexPower), 2);
        assert_ne!(
            sibling.piles.get(PileId::Hand).as_slice()[0].flags & crate::hot::CARD_FLAG_HEXED,
            0
        );

        let mut lethal = spectral_hex_damage_state(1);
        let mut events = Vec::new();
        damage_monster(
            &mut lethal,
            1,
            DotNetDecimal::from_i64(1),
            false,
            false,
            &mut events,
        )
        .unwrap();
        assert_eq!(lethal.powers.value(PowerId::HexPower), 0);
        assert_eq!(
            lethal.piles.get(PileId::Hand).as_slice()[0].flags & crate::hot::CARD_FLAG_HEXED,
            0
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::HexPower,
                amount: 0,
            }
        )));
    }

    #[test]
    fn spectral_hex_damage_refuses_corrupt_late_tail_before_hp_or_events() {
        let mut state = spectral_hex_damage_state(1);
        state.piles.get_mut(PileId::Hand).make_mut()[0].flags |= crate::hot::CARD_FLAG_BOUND;
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 42 }];
        let before_events = events.clone();

        assert_eq!(
            damage_monster(
                &mut state,
                1,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Spectral Hex damage entry"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn spectral_hex_powered_attack_rehearses_the_complete_lethal_suffix() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut success = spectral_hex_damage_state(1);
        player_attack(&mut success, &source, &[1], 1, 1, &mut Vec::new()).unwrap();
        assert_eq!(success.monsters[1].hp, 0);
        assert_eq!(success.powers.value(PowerId::HexPower), 0);
        assert_eq!(
            success.piles.get(PileId::Hand).as_slice()[0].flags & crate::hot::CARD_FLAG_HEXED,
            0
        );

        let mut state = spectral_hex_damage_state(1);
        seed_temporary_strength_restoration_overflow(&mut state.monsters_mut()[1]);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 143 }];
        let before_events = events.clone();

        assert_eq!(
            player_attack(&mut state, &source, &[1], 1, 1, &mut events),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn spectral_hex_direct_death_and_doom_are_whole_command_atomic() {
        let mut direct = spectral_hex_damage_state(0);
        seed_temporary_strength_restoration_overflow(&mut direct.monsters_mut()[1]);
        let before_direct = direct.clone();
        let mut direct_events = vec![Event::TurnBegan { turn: 144 }];
        let before_direct_events = direct_events.clone();
        assert_eq!(
            finish_monster_death(&mut direct, 1, &mut direct_events),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(direct, before_direct);
        assert_eq!(direct_events, before_direct_events);

        let mut doomed = spectral_hex_damage_state(1);
        doomed.monsters_mut()[1]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 1);
        seed_temporary_strength_restoration_overflow(&mut doomed.monsters_mut()[1]);
        let before_doom = doomed.clone();
        let mut doom_events = vec![Event::TurnBegan { turn: 145 }];
        let before_doom_events = doom_events.clone();
        assert_eq!(
            super::super::turn::doom_enemy_side_end(
                &mut doomed,
                &crate::catalog::CatalogBuilder::new().build(),
                &mut doom_events,
            ),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(doomed, before_doom);
        assert_eq!(doom_events, before_doom_events);
    }

    fn seed_secondary_death_removed_powers(monster: &mut HotMonster) {
        for (power, token) in [
            (PowerId::Oblivion, MiseryToken::Oblivion),
            (PowerId::Strangle, MiseryToken::Strangle),
            (PowerId::Hang, MiseryToken::Hang),
            (PowerId::Debilitate, MiseryToken::Debilitate),
        ] {
            monster.powers.set(power, SlotWire::Int, 2);
            monster.misery_debuff_order.push(token);
        }
    }

    fn assert_secondary_death_removed_powers_are_clear(monster: &HotMonster) {
        for power in [
            PowerId::Oblivion,
            PowerId::Strangle,
            PowerId::Hang,
            PowerId::Debilitate,
        ] {
            assert_eq!(monster.powers.value(power), 0, "{power:?}");
        }
        assert!(monster.misery_debuff_order.as_slice().is_empty());
    }

    fn seed_temporary_strength_restoration_overflow(monster: &mut HotMonster) {
        monster
            .powers
            .set(PowerId::Strength, SlotWire::Int, i32::MAX);
        monster.powers.set(PowerId::TempStrength, SlotWire::Int, -1);
    }

    #[test]
    fn queen_primary_death_cascades_amalgam_and_clears_both_forced_fields() {
        let mut state = queen_damage_state(super::super::monsters::TORCH_HEAD_AMALGAM_HP, 1);
        seed_secondary_death_removed_powers(&mut state.monsters_mut()[0]);
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));
        state.monsters_mut()[1].loop_pos = 2;
        state.monsters_mut()[1].override_state = MonsterOverride::Stunned;
        state.monsters_mut()[1].forced_follow_up = MonsterFollowUp::QueenBurnBright;
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));
        let mut events = Vec::new();
        damage_monster(
            &mut state,
            1,
            DotNetDecimal::from_i64(1),
            false,
            false,
            &mut events,
        )
        .unwrap();
        assert!(state.history.over);
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert_eq!(state.monsters[1].override_state, MonsterOverride::None);
        assert_eq!(state.monsters[1].forced_follow_up, MonsterFollowUp::None);
        assert_secondary_death_removed_powers_are_clear(&state.monsters[0]);
        assert_eq!(
            events,
            [
                Event::MonsterDamaged {
                    uid: 1,
                    blocked: 0,
                    unblocked: 1,
                    hp: 0,
                },
                Event::MonsterDied { uid: 1 },
                Event::MonsterDied { uid: 0 },
                Event::CombatOver { player_won: true },
            ]
        );
    }

    #[test]
    fn ordinary_amalgam_death_clears_the_same_powers_and_retains_its_corpse() {
        let mut state = queen_damage_state(1, super::super::monsters::QUEEN_HP);
        state.monsters_mut()[1].loop_pos = 2;
        seed_secondary_death_removed_powers(&mut state.monsters_mut()[0]);
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));

        let mut events = Vec::new();
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(1),
            false,
            false,
            &mut events,
        )
        .unwrap();

        assert!(!state.history.over);
        assert_eq!(state.monsters.len(), 2);
        assert_eq!(state.monsters[0].hp, 0);
        assert_secondary_death_removed_powers_are_clear(&state.monsters[0]);
        assert!(state.monsters[1].queen_amalgam_dead());
        assert!(!state.monsters[1].queen_burn_bright_retained());
        assert_eq!(state.monsters[1].loop_pos, 5);
        assert_eq!(events.last(), Some(&Event::MonsterDied { uid: 0 }));
    }

    #[test]
    fn lethal_amalgam_death_rolls_back_temporary_strength_restoration_overflow() {
        let mut state = queen_damage_state(1, super::super::monsters::QUEEN_HP);
        state.monsters_mut()[1].loop_pos = 2;
        seed_temporary_strength_restoration_overflow(&mut state.monsters_mut()[0]);
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 94 }];
        let before_events = events.clone();

        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn queen_primary_death_rolls_back_secondary_strength_restoration_overflow() {
        let mut state = queen_damage_state(super::super::monsters::TORCH_HEAD_AMALGAM_HP, 1);
        seed_temporary_strength_restoration_overflow(&mut state.monsters_mut()[0]);
        assert!(super::super::monsters::queen_roster_state_is_valid(&state));
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 95 }];
        let before_events = events.clone();

        assert_eq!(
            damage_monster(
                &mut state,
                1,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("monster strength"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn queen_direct_damage_rolls_back_late_identity_normalization_refusal() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = queen_damage_state(1, super::super::monsters::QUEEN_HP);
        state.monsters_mut()[1].loop_pos = 2;
        inject_legacy_bottom(&mut state, &catalog, identity, 1, PileId::Discard).unwrap();
        state.next_card_uid = u32::MAX;
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 92 }];
        let before_events = events.clone();

        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    #[test]
    fn queen_direct_damage_refuses_malformed_roster_before_nonlethal_mutation() {
        let mut state = queen_damage_state(10, super::super::monsters::QUEEN_HP);
        state.monsters_mut()[1].set_queen_amalgam_dead(true);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 93 }];
        let before_events = events.clone();
        assert_eq!(
            damage_monster(
                &mut state,
                0,
                DotNetDecimal::from_i64(1),
                false,
                false,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Queen/Amalgam damage entry"))
        );
        assert_eq!(state, before);
        assert_eq!(events, before_events);
    }

    // ---- #2647 slice A: owner-side AfterDamageGiven ------------------------

    /// One monster, one dealer uid, HP high enough to survive a retaliation.
    fn dealer_state(kind: MonsterKind) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut monster = HotMonster::new(kind, 90);
        monster.uid = 0;
        state.monsters_mut().push(monster);
        state
    }

    /// The `!result.WasFullyBlocked` arm (`0x33cd60` IL_0033–IL_0040) and the
    /// `dealer != Owner` arm (IL_0020–IL_002e), from the monster-attack path.
    #[test]
    fn owner_listener_needs_a_fully_blocked_result_and_the_dealing_owner() {
        // Fully blocked by the carrier's own attack: the latch fires.
        let mut state = dealer_state(MonsterKind::BowlbugRock);
        state.block = 40;
        monster_attack_player_positive_results(&mut state, 0, 16, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);

        // Partially blocked: no result flag, so no latch.
        let mut state = dealer_state(MonsterKind::BowlbugRock);
        state.block = 4;
        monster_attack_player_positive_results(&mut state, 0, 16, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);

        // A fully blocked hit dealt by a monster that carries no listener
        // leaves every override alone — the walk's non-carrier arm.
        let mut state = dealer_state(MonsterKind::Toadpole);
        state.block = 40;
        monster_attack_player_positive_results(&mut state, 0, 16, 1, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    }

    /// A dealer killed inside its own attack gets nothing: `StunInternal`
    /// `0x11d7cc` IL_0028 returns silently on a corpse, and the collapsed Rust
    /// latch must agree. Player Thorns is the reachable killer.
    #[test]
    fn owner_listener_skips_a_dealer_killed_inside_its_own_attack() {
        let mut state = dealer_state(MonsterKind::BowlbugRock);
        state.block = 40;
        state.powers.set(PowerId::Thorns, SlotWire::Int, 200);
        state.monsters_mut()[0].hp = 1;
        monster_attack_player_positive_results(&mut state, 0, 16, 1, &mut Vec::new()).unwrap();
        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    }

    /// The dealer-identity witness for the Thorns path.
    ///
    /// `ThornsPower` `0x349954` IL_0065–IL_0086 makes the **thorny monster** the
    /// dealer of its own retaliation, so the walk must be handed that monster —
    /// not the creature the retaliation hits. Proof: a Bowlbug retaliating into
    /// the player's Block produces a fully blocked result whose owner is a
    /// carrier, and #2647 §5.4 refuses reaching the collapsed latch from any
    /// command but the carrier's own attack. If the identity were lost, the walk
    /// would see no carrier and this would silently pass.
    #[test]
    fn thorns_retaliation_reaches_the_owner_listener_as_the_dealer() {
        let mut state = HotState::at_defaults();
        state.hp = 40;
        state.max_hp = 40;
        state.block = 9;
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut thorny = HotMonster::new(MonsterKind::BowlbugRock, 90);
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        state.monsters_mut().push(thorny);
        let source = source_spec(CardId::StrikeIronclad);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            player_attack(&mut state, &source, &[0], 1, 1, &mut events),
            Err(EngineRefusal::PowerOrderNotModeled(
                "Imbalanced owner outside its own attack"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// A non-carrier's Thorns retaliation is untouched by slice A: the dealer
    /// parameter is threaded through and the guard above finds no carrier, so
    /// the existing Block/HP/lethal behaviour runs unchanged.
    #[test]
    fn a_non_carrier_thorns_retaliation_is_unchanged() {
        let mut state = HotState::at_defaults();
        state.hp = 40;
        state.max_hp = 40;
        state.block = 9;
        state.powers.set(PowerId::Vigor, SlotWire::Int, 3);
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 90);
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        state.monsters_mut().push(thorny);
        let source = source_spec(CardId::StrikeIronclad);

        player_attack(&mut state, &source, &[0], 1, 1, &mut Vec::new()).unwrap();

        assert_eq!((state.hp, state.block), (40, 7));
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    }

    /// The pet receiver's Block is not represented, so a listener that would
    /// read `originalTarget.Block` refuses instead of guessing (#2647 §5.10).
    #[test]
    fn pet_thorns_retaliation_refuses_an_imbalanced_owner() {
        let mut state = HotState::at_defaults();
        state.hp = 40;
        state.max_hp = 40;
        let mut thorny = HotMonster::new(MonsterKind::BowlbugRock, 90);
        thorny.uid = 4;
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 2);
        state.monsters_mut().push(thorny);
        state.fanouts.set_osty(Some((8, 8))).unwrap();

        let before = state.clone();
        assert_eq!(
            thorns_retaliation_pet(&mut state, None, 2, 4, &mut Vec::new()),
            Err(EngineRefusal::PowerOrderNotModeled(
                "Imbalanced owner outside its own attack"
            ))
        );
        assert_eq!(state, before);
        // A non-carrier owner on the same path is untouched by slice A.
        state.monsters_mut()[0].kind = MonsterKind::Toadpole;
        thorns_retaliation_pet(&mut state, None, 2, 4, &mut Vec::new()).unwrap();
    }

    /// The walk called directly, one case per arm it can reach.
    ///
    /// Its remaining arm — an Imbalanced carrier that is not a `BowlbugRock` —
    /// became reachable in #2693 B1 and is witnessed by
    /// `imbalanced_non_bowlbug_owner_takes_the_generic_stun_branch` below;
    /// `engine::monsters::tests::owner_carries_imbalanced_is_bowlbug_or_an_attachment`
    /// is the total enumeration pinning that the carrier set is exactly
    /// "Rock, or a ledger attachment".
    #[test]
    fn owner_listener_arms() {
        let mut state = dealer_state(MonsterKind::BowlbugRock);
        // The dealer uid is not in the roster at all.
        let before = state.clone();
        monsters_after_damage_given(&mut state, 7, true).unwrap();
        assert_eq!(state, before);
        // The flag is false.
        monsters_after_damage_given(&mut state, 0, false).unwrap();
        assert_eq!(state, before);
        // The carrier's own attack.
        monsters_after_damage_given(&mut state, 0, true).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Dizzy);
        // Combat already finished: `StunInternal` IL_001f returns silently.
        let mut finished = dealer_state(MonsterKind::BowlbugRock);
        finished.history.over = true;
        let before = finished.clone();
        monsters_after_damage_given(&mut finished, 0, true).unwrap();
        assert_eq!(finished, before);
    }

    /// #2647 §5.4 — the document-level half of the collapsed latch.
    #[test]
    fn misery_and_rend_route_the_carrier_through_one_predicate() {
        let mut bowlbug = HotMonster::new(MonsterKind::BowlbugRock, 90);
        // #2647 B2: a carrier is snapshotted rather than refused, and the
        // intrinsic instance is the first entry of the walk.
        assert!(matches!(
            misery_snapshot(&bowlbug).as_deref(),
            Ok([MiserySnapshotEntry::Attachment(_)]),
        ));
        // #2693's disjuncts are untouched and still independently sufficient.
        let mut plain = HotMonster::new(MonsterKind::Toadpole, 20);
        assert_eq!(misery_scalar_state_is_exact(&plain), Ok(()));
        for power in [PowerId::TempStrength, PowerId::Shriek] {
            let mut carrier = plain.clone();
            carrier.powers.set(power, SlotWire::Int, 1);
            assert_eq!(
                misery_scalar_state_is_exact(&carrier),
                Err(EngineRefusal::MalformedArgs(
                    "Misery concrete Type-2 power state"
                ))
            );
        }
        plain.powers.set(PowerId::Strength, SlotWire::Int, -1);
        assert_eq!(
            misery_scalar_state_is_exact(&plain),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            ))
        );
        bowlbug.powers.set(PowerId::Thorns, SlotWire::Int, 0);
        assert!(crate::engine::monsters::owner_carries_imbalanced(&bowlbug));
    }

    /// Attach a physical `ImbalancedPower` the way a `Misery` clone (PR B2)
    /// eventually will: amount 1, applied by a Bowlbug Rock.
    fn attach_imbalanced(monster: &mut HotMonster, applier_uid: u32) {
        monster
            .misery_debuff_order
            .push_attachment(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Imbalanced,
                applier: crate::hot::Applier::Monster(applier_uid),
                amount: 1,
            });
    }

    /// #2647 slice A's **deferred witness (15)**, now reachable.
    ///
    /// `ImbalancedPower/<AfterDamageGiven>d__4::MoveNext` `0x33cd60`: IL_0056
    /// `isinst BowlbugRock` yields null for any other carrier, so IL_005d
    /// falls through to IL_0068–IL_006f and awaits
    /// `CreatureCmd::Stun(Owner, null)`. Slice A had to refuse this arm
    /// because no non-Bowlbug creature could carry the power; B1's ledger
    /// attachment is what makes it live.
    ///
    /// The assertion is the *whole* installed state, not just "something
    /// happened": `override_state` is `Stunned` and `forced_follow_up` is
    /// exactly what [`crate::engine::monsters::parked_telegraph`] resolves
    /// for the carrier's own `loop_pos`, which is the identification
    /// `StunInternal` `0x11d7cc` IL_0031–IL_0055 makes out of
    /// `StateLog.Last()`.
    #[test]
    fn imbalanced_non_bowlbug_owner_takes_the_generic_stun_branch() {
        for (kind, loop_pos) in [
            (MonsterKind::BowlbugEgg, 0),
            (MonsterKind::BowlbugNectar, 0),
            (MonsterKind::BowlbugNectar, 1),
        ] {
            let mut state = dealer_state(kind);
            state.block = 40;
            state.monsters_mut()[0].loop_pos = loop_pos;
            attach_imbalanced(&mut state.monsters_mut()[0], 9);

            monster_attack_player_positive_results(&mut state, 0, 16, 1, &mut Vec::new()).unwrap();

            let expected = crate::engine::monsters::parked_telegraph(kind, loop_pos)
                .expect("an eligible carrier resolves its parked telegraph");
            assert_eq!(
                state.monsters[0].override_state,
                MonsterOverride::Stunned,
                "{kind:?}@{loop_pos}",
            );
            assert_eq!(
                state.monsters[0].forced_follow_up, expected,
                "{kind:?}@{loop_pos}",
            );
            // The Rock's bespoke latch is a *different* state, and this arm
            // must never install it.
            assert_ne!(state.monsters[0].override_state, MonsterOverride::Dizzy);
        }
    }

    /// #2647 — the pet/player (Osty) split with an attached carrier.
    ///
    /// `CreatureCmd/<Damage>d__12::MoveNext` `0x3e96c8` IL_0505–IL_0554
    /// computes `WasFullyBlocked` once, BEFORE pet interposition; the split at
    /// IL_0687–IL_06d3 copies it onto the player's result and leaves the pet's
    /// own result at the default `false`. So per hit exactly one result can
    /// carry the flag, and it is the pre-interposition value:
    ///
    /// * Block absorbs the whole hit — fully blocked, whether or not Osty is
    ///   up, so the carrier stuns;
    /// * Block absorbs part and Osty the rest — NOT fully blocked (pre-pet
    ///   unblocked damage is positive), even though the player loses no HP,
    ///   so the carrier does not stun. Unchanged player HP is not the
    ///   predicate.
    #[test]
    fn an_attached_carrier_reads_the_pre_osty_result_on_a_split() {
        let mut full = dealer_state(MonsterKind::BowlbugNectar);
        full.block = 40;
        full.fanouts.set_osty(Some((10, 10))).unwrap();
        attach_imbalanced(&mut full.monsters_mut()[0], 9);
        monster_attack_player_positive_results(&mut full, 0, 5, 1, &mut Vec::new()).unwrap();
        assert_eq!(full.monsters[0].override_state, MonsterOverride::Stunned);
        assert_eq!(full.fanouts.pet().osty().map(|osty| osty.hp()), Some(10));

        let mut spill = dealer_state(MonsterKind::BowlbugNectar);
        spill.block = 2;
        spill.fanouts.set_osty(Some((10, 10))).unwrap();
        attach_imbalanced(&mut spill.monsters_mut()[0], 9);
        let hp = spill.hp;
        monster_attack_player_positive_results(&mut spill, 0, 5, 1, &mut Vec::new()).unwrap();
        assert_eq!(spill.hp, hp, "Osty took the spill");
        assert!(
            spill
                .fanouts
                .pet()
                .osty()
                .is_some_and(|osty| osty.hp() < 10)
        );
        assert_eq!(spill.monsters[0].override_state, MonsterOverride::None);
    }

    /// The same carrier, but the result is not fully blocked or the dealer is
    /// someone else: `0x33cd60` IL_0033–IL_0040 and IL_0020–IL_002e return
    /// before `Flash` at IL_0045, so nothing is installed.
    #[test]
    fn an_attached_imbalanced_carrier_needs_the_fully_blocked_result_too() {
        let mut partial = dealer_state(MonsterKind::BowlbugEgg);
        partial.block = 4;
        attach_imbalanced(&mut partial.monsters_mut()[0], 9);
        monster_attack_player_positive_results(&mut partial, 0, 16, 1, &mut Vec::new()).unwrap();
        assert_eq!(partial.monsters[0].override_state, MonsterOverride::None);

        // A foreign dealer uid leaves the carrier alone.
        let mut foreign = dealer_state(MonsterKind::BowlbugEgg);
        attach_imbalanced(&mut foreign.monsters_mut()[0], 9);
        let before = foreign.clone();
        monsters_after_damage_given(&mut foreign, 7, true).unwrap();
        assert_eq!(foreign, before);

        // Combat already over: `StunInternal` IL_001f returns silently.
        let mut finished = dealer_state(MonsterKind::BowlbugEgg);
        attach_imbalanced(&mut finished.monsters_mut()[0], 9);
        finished.history.over = true;
        let before = finished.clone();
        monsters_after_damage_given(&mut finished, 0, true).unwrap();
        assert_eq!(finished, before);

        // A dead carrier: IL_0028 is a corpse no-op.
        let mut dead = dealer_state(MonsterKind::BowlbugEgg);
        attach_imbalanced(&mut dead.monsters_mut()[0], 9);
        dead.monsters_mut()[0].hp = 0;
        let before = dead.clone();
        monsters_after_damage_given(&mut dead, 0, true).unwrap();
        assert_eq!(dead, before);
    }

    /// A carrier already holding a foreign override refuses rather than
    /// having it overwritten — `install_generic_stun`'s own guard, reached
    /// through the listener for the first time.
    #[test]
    fn an_attached_imbalanced_carrier_over_a_foreign_override_refuses() {
        let mut state = dealer_state(MonsterKind::BowlbugEgg);
        attach_imbalanced(&mut state.monsters_mut()[0], 9);
        state.monsters_mut()[0].override_state = MonsterOverride::BeetleSnore;
        assert_eq!(
            monsters_after_damage_given(&mut state, 0, true),
            Err(EngineRefusal::MalformedArgs(
                "generic stun foreign override"
            )),
        );
    }

    /// Death cleanup resets the whole ledger, so an attached carrier's
    /// Imbalanced goes with it — witnessed rather than assumed, because the
    /// clearing is a `Default::default()` on `misery_debuff_order` that
    /// happens to cover attachments for free (#2693 B1).
    #[test]
    fn a_dead_imbalanced_carrier_is_cleaned_up() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut monster = HotMonster::new(MonsterKind::BowlbugEgg, 3);
        monster.uid = 0;
        state.monsters_mut().push(monster);
        attach_imbalanced(&mut state.monsters_mut()[0], 9);
        assert!(crate::engine::monsters::owner_carries_imbalanced(
            &state.monsters[0]
        ));

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(9),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert!(
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
        assert!(!crate::engine::monsters::owner_carries_imbalanced(
            &state.monsters[0]
        ));
    }

    /// #3311 — exactly the `CreatureCmd::Add` census: a lone monster of one of
    /// the six adding kinds, or of one of the three kinds that apply a
    /// creature-adding death power to themselves, can be joined; a lone
    /// monster of any other kind cannot; and any second roster entry, alive
    /// or not, is treated as a possible recipient.
    #[test]
    fn misery_recipient_roster_predicate_is_the_creature_add_census() {
        let lone = |kind| {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(HotMonster::new(kind, 30));
            misery_roster_can_present_a_recipient(&state)
        };
        let adders = [
            MonsterKind::Fabricator,
            MonsterKind::Fogmog,
            MonsterKind::LivingFog,
            MonsterKind::Ovicopter,
            MonsterKind::TheObscura,
            MonsterKind::TwoTailedRat,
            MonsterKind::Axebot,
            MonsterKind::GremlinMerc,
            MonsterKind::PhrogParasite,
        ];
        for kind in MonsterKind::ALL {
            assert_eq!(lone(kind), adders.contains(&kind), "{kind:?}");
        }
        assert!(!lone(MonsterKind::TerrorEel));
        assert!(!lone(MonsterKind::BygoneEffigy));

        let mut pair = HotState::at_defaults();
        pair.monsters_mut()
            .push(HotMonster::new(MonsterKind::TerrorEel, 30));
        pair.monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 0));
        assert!(misery_roster_can_present_a_recipient(&pair));
        assert!(misery_roster_can_present_a_recipient(
            &HotState::at_defaults()
        ));
    }

    /// #2647 B2 lifts the disjunct B1 deliberately left standing: an
    /// Imbalanced carrier is now snapshotted, in either representation, and
    /// the other four disjuncts stay byte-identical for #2693.
    #[test]
    fn misery_snapshots_an_imbalanced_carrier_in_either_representation() {
        let mut attached = HotMonster::new(MonsterKind::BowlbugEgg, 30);
        attached.uid = 3;
        assert_eq!(misery_snapshot(&attached), Ok(Vec::new()));
        attach_imbalanced(&mut attached, 9);
        assert_eq!(
            misery_snapshot(&attached),
            Ok(vec![MiserySnapshotEntry::Attachment(
                crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Imbalanced,
                    applier: crate::hot::Applier::Monster(9),
                    amount: 1,
                }
            )]),
        );

        // The Rock's intrinsic instance has no row and is nevertheless the
        // first entry of the walk, with its own Creature as applier.
        let mut rock = HotMonster::new(MonsterKind::BowlbugRock, 90);
        rock.uid = 4;
        assert_eq!(
            misery_snapshot(&rock),
            Ok(vec![MiserySnapshotEntry::Attachment(
                crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Imbalanced,
                    applier: crate::hot::Applier::Monster(4),
                    amount: 1,
                }
            )]),
        );

        // Every other disjunct is untouched — #2693's S-stages own them.
        for mutate in [
            (|monster: &mut HotMonster| {
                monster.powers.set(PowerId::Strength, SlotWire::Int, -1);
            }) as fn(&mut HotMonster),
            |monster: &mut HotMonster| {
                monster.powers.set(PowerId::TempStrength, SlotWire::Int, -1);
            },
            |monster: &mut HotMonster| {
                monster.powers.set(PowerId::Shriek, SlotWire::Int, 1);
            },
        ] {
            let mut monster = HotMonster::new(MonsterKind::BowlbugEgg, 30);
            mutate(&mut monster);
            assert_eq!(
                misery_snapshot(&monster),
                Err(EngineRefusal::MalformedArgs(
                    "Misery concrete Type-2 power state"
                )),
            );
        }
        assert_eq!(
            misery_snapshot(&HotMonster::new(MonsterKind::BygoneEffigy, 30)),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            )),
        );
    }

    /// The walk is one `Creature.Powers` sequence, not two passes.
    ///
    /// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_003e-IL_009c freezes one
    /// ordered walk, and a recipient's Artifact consumes on whichever entry
    /// comes first, so a scalar acquired before an attachment and one
    /// acquired after must produce different snapshots.
    #[test]
    fn the_misery_snapshot_interleaves_attachments_and_scalars_by_position() {
        let record = crate::hot::AttachmentRecord {
            power: crate::hot::AttachedPowerModel::Imbalanced,
            applier: crate::hot::Applier::Monster(9),
            amount: 1,
        };

        let mut attach_first = HotMonster::new(MonsterKind::BowlbugEgg, 30);
        attach_imbalanced(&mut attach_first, 9);
        attach_first.powers.set(PowerId::Weak, SlotWire::Int, 2);
        attach_first.misery_debuff_order.push(MiseryToken::Weak);
        assert_eq!(
            misery_snapshot(&attach_first),
            Ok(vec![
                MiserySnapshotEntry::Attachment(record),
                MiserySnapshotEntry::Scalar(MiseryScalarSnapshot {
                    power: PowerId::Weak,
                    token: MiseryToken::Weak,
                    amount: 2,
                }),
            ]),
        );

        let mut acquire_first = HotMonster::new(MonsterKind::BowlbugEgg, 30);
        acquire_first.powers.set(PowerId::Weak, SlotWire::Int, 2);
        acquire_first.misery_debuff_order.push(MiseryToken::Weak);
        attach_imbalanced(&mut acquire_first, 9);
        assert_eq!(
            misery_snapshot(&acquire_first),
            Ok(vec![
                MiserySnapshotEntry::Scalar(MiseryScalarSnapshot {
                    power: PowerId::Weak,
                    token: MiseryToken::Weak,
                    amount: 2,
                }),
                MiserySnapshotEntry::Attachment(record),
            ]),
        );
        assert_ne!(
            misery_snapshot(&attach_first),
            misery_snapshot(&acquire_first),
        );

        // A legacy document with no acquisition order at all carries no
        // position for its one reconstructible scalar, so placing the
        // attachment against it would be a guess (I5).
        let mut legacy = HotMonster::new(MonsterKind::BowlbugEgg, 30);
        legacy.powers.set(PowerId::Weak, SlotWire::Int, 2);
        attach_imbalanced(&mut legacy, 9);
        assert_eq!(
            misery_snapshot(&legacy),
            Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order"
            )),
        );
    }

    // ------------------------------------------------------------------
    // #2693 S1 — monster Strength as an ordered attachment with applier.
    // ------------------------------------------------------------------

    fn strength_row(monster: &HotMonster) -> Option<crate::hot::AttachmentRecord> {
        crate::engine::monsters::strength_attachment_record(monster)
    }

    /// The four arms of the one monster-Strength writer, against the native
    /// lifecycle each is named for.
    #[test]
    fn the_monster_strength_writer_attaches_restacks_and_removes_in_place() {
        // Fresh attach appends after everything already in the ledger
        // (`ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f).
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.uid = 5;
        monster.powers.set(PowerId::Weak, SlotWire::Int, 2);
        monster.misery_debuff_order.push(MiseryToken::Weak);
        write_monster_strength(&mut monster, 2, crate::hot::Applier::Monster(5), true);
        assert_eq!(
            strength_row(&monster),
            Some(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Monster(5),
                amount: 2,
            }),
        );
        assert_eq!(
            monster.misery_debuff_order.placed_attachments()[0].0,
            1,
            "the append lands after the token that was already there",
        );

        // `+2 -> -1` crosses the sign without touching position or applier:
        // `ShouldRemoveDueToAmount` `0x83b0d` removes an `AllowNegative`
        // power at exactly zero and at no other amount, and `ModifyAmount`
        // never reaches `set_Applier` (`0x3efbac` IL_013d).
        write_monster_strength(&mut monster, -1, crate::hot::Applier::Player, true);
        assert_eq!(
            strength_row(&monster),
            Some(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Monster(5),
                amount: -1,
            }),
            "a sign crossing keeps the instance, its position and its applier",
        );
        assert_eq!(monster.misery_debuff_order.placed_attachments()[0].0, 1);

        // ...and back the other way, `-1 -> +1`, still one instance.
        write_monster_strength(&mut monster, 1, crate::hot::Applier::None, true);
        assert_eq!(strength_row(&monster).map(|record| record.amount), Some(1));
        assert_eq!(
            strength_row(&monster).map(|record| record.applier),
            Some(crate::hot::Applier::Monster(5)),
        );

        // Exactly zero removes in place; a later application appends at the
        // END rather than reclaiming the old slot
        // (`Creature::RemovePowerInternal` `0x11db0b` IL_0015-IL_001c, then
        // `ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f).
        monster.powers.set(PowerId::Poison, SlotWire::Int, 3);
        monster.misery_debuff_order.push(MiseryToken::Poison);
        write_monster_strength(&mut monster, 0, crate::hot::Applier::Player, true);
        assert_eq!(strength_row(&monster), None);
        assert!(monster.misery_debuff_order.attachments().is_empty());
        write_monster_strength(&mut monster, -3, crate::hot::Applier::Player, true);
        assert_eq!(
            monster.misery_debuff_order.placed_attachments()[0].0,
            2,
            "a re-application appends behind BOTH tokens",
        );
        assert_eq!(
            strength_row(&monster).map(|record| record.applier),
            Some(crate::hot::Applier::Player),
            "the re-attach writes the applier the new command was given",
        );
    }

    /// Upkeep off writes the scalar and nothing else, and an unrecorded
    /// legacy instance is given up on rather than invented.
    #[test]
    fn the_monster_strength_writer_is_gated_and_gives_up_rather_than_guessing() {
        let mut ungated = HotMonster::new(MonsterKind::Toadpole, 30);
        write_monster_strength(&mut ungated, -3, crate::hot::Applier::Player, false);
        assert_eq!(ungated.powers.value(PowerId::Strength), -3);
        assert!(
            ungated.misery_debuff_order.is_empty(),
            "a fight that cannot reach Misery records no provenance at all",
        );

        // A legacy checkpoint: a live instance with no row. Its position is
        // not knowable from the scalar, so the writer leaves the ledger
        // alone, exactly as `record_misery_debuff_application` gives up on an
        // unreconstructible scalar multiset.
        let mut legacy = HotMonster::new(MonsterKind::Toadpole, 30);
        legacy.powers.set(PowerId::Strength, SlotWire::Int, 2);
        legacy.powers.set(PowerId::Weak, SlotWire::Int, 1);
        legacy.misery_debuff_order.push(MiseryToken::Weak);
        write_monster_strength(&mut legacy, -1, crate::hot::Applier::Player, true);
        assert_eq!(legacy.powers.value(PowerId::Strength), -1);
        assert_eq!(strength_row(&legacy), None);
        // ...and because the provenance is still unrecorded, the reader
        // refuses exactly where it refused before this stage.
        assert_eq!(
            misery_snapshot(&legacy),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            )),
        );
    }

    /// `Misery`'s filter is `TypeForCurrentAmount == 2` (`0x3ad30a`), so a
    /// Strength row is in the dictionary iff its amount is negative while
    /// Imbalanced is always — and an unselected row still holds its position.
    #[test]
    fn misery_selects_a_strength_attachment_only_while_it_is_negative() {
        for (amount, selected) in [(-5, true), (-1, true), (1, false), (5, false)] {
            let record = crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Player,
                amount,
            };
            assert_eq!(attachment_is_misery_selected(&record), selected, "{amount}");
        }
        for amount in [1, 2, 999_999_999] {
            assert!(attachment_is_misery_selected(
                &crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Imbalanced,
                    applier: crate::hot::Applier::Monster(1),
                    amount,
                }
            ));
        }

        // A positive Strength row holds a ledger position and contributes no
        // snapshot entry; the scalar beside it still does.
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.uid = 5;
        write_monster_strength(&mut monster, 4, crate::hot::Applier::Monster(5), true);
        monster.powers.set(PowerId::Weak, SlotWire::Int, 2);
        monster.misery_debuff_order.push(MiseryToken::Weak);
        assert_eq!(
            misery_snapshot(&monster),
            Ok(vec![MiserySnapshotEntry::Scalar(MiseryScalarSnapshot {
                power: PowerId::Weak,
                token: MiseryToken::Weak,
                amount: 2,
            })]),
        );
        // The same monster with the sign flipped replays the row FIRST,
        // because that is where the instance actually sits.
        write_monster_strength(&mut monster, -4, crate::hot::Applier::Monster(5), true);
        assert_eq!(
            misery_snapshot(&monster),
            Ok(vec![
                MiserySnapshotEntry::Attachment(crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Strength,
                    applier: crate::hot::Applier::Monster(5),
                    amount: -4,
                }),
                MiserySnapshotEntry::Scalar(MiseryScalarSnapshot {
                    power: PowerId::Weak,
                    token: MiseryToken::Weak,
                    amount: 2,
                }),
            ]),
        );
    }

    /// Strength acquired before vs after another debuff is a different state
    /// and a different replay order.
    #[test]
    fn a_strength_attachment_before_and_after_a_debuff_are_distinct_ledgers() {
        let mut before = HotMonster::new(MonsterKind::Toadpole, 30);
        before.uid = 5;
        write_monster_strength(&mut before, -3, crate::hot::Applier::Player, true);
        before.powers.set(PowerId::Weak, SlotWire::Int, 2);
        before.misery_debuff_order.push(MiseryToken::Weak);

        let mut after = HotMonster::new(MonsterKind::Toadpole, 30);
        after.uid = 5;
        after.powers.set(PowerId::Weak, SlotWire::Int, 2);
        after.misery_debuff_order.push(MiseryToken::Weak);
        write_monster_strength(&mut after, -3, crate::hot::Applier::Player, true);

        assert_ne!(before.misery_debuff_order, after.misery_debuff_order);
        let (before_snapshot, after_snapshot) = (
            misery_snapshot(&before).unwrap(),
            misery_snapshot(&after).unwrap(),
        );
        assert_ne!(before_snapshot, after_snapshot);
        assert!(matches!(
            before_snapshot.as_slice(),
            [
                MiserySnapshotEntry::Attachment(_),
                MiserySnapshotEntry::Scalar(_)
            ]
        ));
        assert!(matches!(
            after_snapshot.as_slice(),
            [
                MiserySnapshotEntry::Scalar(_),
                MiserySnapshotEntry::Attachment(_)
            ]
        ));
    }

    /// Enemy-applied, player-applied, relic-applied (null) and
    /// replaced-in-slot appliers are four distinguishable states.
    #[test]
    fn strength_appliers_are_distinguishable_including_a_replacement_uid() {
        let mut ledgers = Vec::new();
        for applier in [
            crate::hot::Applier::None,
            crate::hot::Applier::Player,
            crate::hot::Applier::Monster(4),
            crate::hot::Applier::Monster(9),
        ] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.uid = 5;
            write_monster_strength(&mut monster, -2, applier, true);
            assert_eq!(
                crate::engine::monsters::strength_applier(&monster),
                Some(applier),
            );
            ledgers.push(monster.misery_debuff_order.clone());
        }
        for (index, left) in ledgers.iter().enumerate() {
            for right in &ledgers[index + 1..] {
                assert_ne!(left, right, "two appliers collapsed onto one ledger");
            }
        }
    }

    /// The exactness disjunct S1 replaces, in all three directions.
    #[test]
    fn strength_provenance_decides_the_misery_refusal_not_the_sign_alone() {
        // Unrecorded provenance + negative: refuses, exactly as the
        // categorical `Strength < 0` disjunct did.
        let mut legacy = HotMonster::new(MonsterKind::Toadpole, 30);
        legacy.powers.set(PowerId::Strength, SlotWire::Int, -1);
        assert_eq!(
            misery_scalar_state_is_exact(&legacy),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            )),
        );

        // Unrecorded provenance + NONNEGATIVE: admits, byte-identically to
        // before this stage — a type-1 instance is not in the dictionary, so
        // its position is unobservable.
        let mut positive = HotMonster::new(MonsterKind::Toadpole, 30);
        positive.powers.set(PowerId::Strength, SlotWire::Int, 3);
        assert_eq!(misery_scalar_state_is_exact(&positive), Ok(()));
        assert_eq!(misery_snapshot(&positive), Ok(Vec::new()));

        // Recorded provenance: admits and is snapshotted.
        let mut recorded = HotMonster::new(MonsterKind::Toadpole, 30);
        recorded.uid = 5;
        write_monster_strength(&mut recorded, -1, crate::hot::Applier::Player, true);
        assert_eq!(misery_scalar_state_is_exact(&recorded), Ok(()));
        assert_eq!(misery_snapshot(&recorded).unwrap().len(), 1);

        // A row that disagrees with the scalar is malformed: the row adds
        // position and applier, never a second amount.
        let mut disagreeing = recorded.clone();
        disagreeing.powers.set(PowerId::Strength, SlotWire::Int, -2);
        assert_eq!(
            misery_scalar_state_is_exact(&disagreeing),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            )),
        );
    }

    /// Artifact tests the INCOMING DELTA's type (`0x9f844` IL_001f-IL_0032),
    /// so it consumes against a negative Strength delta and passes a positive
    /// one, whatever the live amount is.
    #[test]
    fn artifact_consumes_on_a_negative_strength_delta_and_not_a_positive_one() {
        // -1 against a live +5: `GetTypeForAmount(-1)` `0x83a94`
        // IL_0013-IL_003e is 2, so the Artifact blocks and is consumed and
        // `ApplyInternal` `0x84012` never reaches the owner.
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.uid = 5;
        monster.powers.set(PowerId::Strength, SlotWire::Int, 5);
        monster.powers.set(PowerId::Artifact, SlotWire::Int, 1);
        state.monsters_mut().push(monster);
        state.fanouts.set_misery_attachment_upkeep(true);
        let mut events = Vec::new();
        assert_eq!(
            card_monster_type_two_amount(&mut state, 0, -1, &mut events).unwrap(),
            None,
            "the delta is zeroed and the Artifact is consumed",
        );
        assert_eq!(state.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 5);

        // `GetTypeForAmount(+1)` falls through IL_0045's `brtrue` to
        // `get_Type` `0xa8943` = 1, so a restoring delta is NOT a debuff and
        // an Artifact beside it is untouched. No card applies a positive
        // Strength delta to a monster today, so this is witnessed at the
        // predicate rather than through a command.
        for (amount, expected) in [(-1, true), (1, false), (-999, true), (999, false)] {
            assert_eq!(
                attachment_is_misery_selected(&crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Strength,
                    applier: crate::hot::Applier::Player,
                    amount,
                }),
                expected,
                "GetTypeForAmount({amount}) == 2",
            );
        }
    }

    /// The clone command: fresh attach on a bare recipient, `ModifyAmount` on
    /// one that already carries an instance, Artifact, and the Lamp refusal.
    #[test]
    fn the_strength_clone_stacks_without_a_new_position_and_refuses_a_lamp() {
        fn roster() -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 60;
            state.max_hp = 60;
            state.fanouts.set_misery_attachment_upkeep(true);
            for uid in 0..2u32 {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
                monster.uid = uid;
                monster.slot = uid as i32;
                state.monsters_mut().push(monster);
            }
            state
        }
        let catalog = CatalogBuilder::new().build();
        let mut events = Vec::new();

        // Fresh attach on a bare recipient, carrying the SOURCE's applier.
        let mut state = roster();
        apply_card_monster_strength_clone(
            &mut state,
            &catalog,
            1,
            crate::hot::Applier::Monster(0),
            -3,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), -3);
        assert_eq!(
            strength_row(&state.monsters[1]),
            Some(crate::hot::AttachmentRecord {
                power: crate::hot::AttachedPowerModel::Strength,
                applier: crate::hot::Applier::Monster(0),
                amount: -3,
            }),
        );

        // A recipient that already carries an instance takes `ModifyAmount`
        // (`0x3ad358` IL_029b-IL_02bd): one position, the ORIGINAL applier.
        apply_card_monster_strength_clone(
            &mut state,
            &catalog,
            1,
            crate::hot::Applier::Player,
            -2,
            &mut events,
        )
        .unwrap();
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), -5);
        assert_eq!(state.monsters[1].misery_debuff_order.attachments().len(), 1);
        assert_eq!(
            strength_row(&state.monsters[1]).map(|record| record.applier),
            Some(crate::hot::Applier::Monster(0)),
            "a stacking application never reaches set_Applier",
        );

        // Artifact blocks the copy and is consumed; nothing attaches.
        let mut blocked = roster();
        blocked.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        apply_card_monster_strength_clone(
            &mut blocked,
            &catalog,
            1,
            crate::hot::Applier::Monster(0),
            -3,
            &mut events,
        )
        .unwrap();
        assert_eq!(blocked.monsters[1].powers.value(PowerId::Strength), 0);
        assert_eq!(blocked.monsters[1].powers.value(PowerId::Artifact), 0);
        assert_eq!(strength_row(&blocked.monsters[1]), None);

        // A reachable Lamp beside a non-player applier refuses; the same
        // Lamp with a PLAYER applier is the case the model represents.
        let mut lamp = roster();
        lamp.fanouts.set_unsettling_lamp_available(true);
        assert_eq!(
            apply_card_monster_strength_clone(
                &mut lamp,
                &catalog,
                1,
                crate::hot::Applier::Monster(0),
                -3,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Misery Strength clone Unsettling Lamp applier"
            )),
        );

        // #2693 S3 made a POSITIVE frozen value reachable: the
        // temporary-Strength fold raises the copied value by each selected
        // wrapper's amount. It is a Type-1 application, so the recipient's
        // Artifact passes it through untouched and is NOT consumed
        // (`0x9f844` IL_001f-IL_0032 against `GetTypeForAmount(+2)` = 1).
        let mut positive = roster();
        positive.monsters_mut()[1]
            .powers
            .set(PowerId::Artifact, crate::powers::SlotWire::Int, 1);
        apply_card_monster_strength_clone(
            &mut positive,
            &catalog,
            1,
            crate::hot::Applier::Player,
            2,
            &mut events,
        )
        .unwrap();
        assert_eq!(positive.monsters[1].powers.value(PowerId::Strength), 2);
        assert_eq!(positive.monsters[1].powers.value(PowerId::Artifact), 1);
        assert_eq!(
            strength_row(&positive.monsters[1]).map(|record| (record.amount, record.applier)),
            Some((2, crate::hot::Applier::Player)),
        );

        // Exactly zero is the one value that cannot arrive: `0x3ad358`
        // IL_0268-IL_026f skips the entry before the lookup.
        let mut zero = roster();
        assert_eq!(
            apply_card_monster_strength_clone(
                &mut zero,
                &catalog,
                1,
                crate::hot::Applier::Player,
                0,
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("Misery Strength clone amount")),
        );
    }

    // ------------------------------------------------------------------
    // #2693 S2 — concrete temporary-Strength wrapper records on monsters.
    // ------------------------------------------------------------------

    fn wrapper_rows(monster: &HotMonster) -> Vec<crate::hot::AttachmentRecord> {
        crate::engine::monsters::temp_strength_wrapper_rows(monster)
            .map(|(_, record)| record)
            .collect()
    }

    fn wrapper_carrier(uid: u32) -> HotMonster {
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
        monster.uid = uid;
        monster
    }

    /// The **derived** writer set, one witness each.
    ///
    /// The eight are re-derived here rather than transcribed from #2693: every
    /// `TemporaryStrengthPower` subclass whose `get_IsPositive` override
    /// returns false and which the assembly actually applies. Each row must
    /// name its own native class, so a card that silently reused a sibling's
    /// model would fail — and that matters because `Misery`'s stacking lookup
    /// is by model id (`0x1338d8` IL_0058), so the wrong class merges two
    /// instances native keeps apart.
    #[test]
    fn each_derived_temporary_strength_writer_records_its_own_native_model() {
        use crate::hot::AttachedPowerModel as Model;
        // The six card-sourced members, through the one derivation their
        // steps use.
        for (card, model) in [
            (CardId::CrushUnder, Model::CrushUnder),
            (CardId::DarkShackles, Model::DarkShackles),
            (CardId::DyingStar, Model::DyingStar),
            (CardId::EnfeeblingTouch, Model::EnfeeblingTouch),
            (CardId::Mangle, Model::Mangle),
            (CardId::PiercingWail, Model::PiercingWail),
        ] {
            assert_eq!(
                crate::steps::shared::temp_strength_enemy_model(card),
                Ok(model),
                "{card:?} attaches its own native wrapper class",
            );
            let mut monster = wrapper_carrier(7);
            let (temp, strength) = write_monster_temp_strength_wrapper(
                &mut monster,
                model,
                4,
                crate::hot::Applier::Player,
                true,
            )
            .unwrap();
            assert_eq!((temp, strength), (-4, -4));
            assert_eq!(
                wrapper_rows(&monster),
                vec![crate::hot::AttachmentRecord {
                    power: model,
                    applier: crate::hot::Applier::Player,
                    amount: 4,
                }],
                "the row carries the POSITIVE native Amount, not the signed effect",
            );
        }
        // A card outside the derived set has no model, rather than a guess.
        assert_eq!(
            crate::steps::shared::temp_strength_enemy_model(CardId::Malaise),
            Err(EngineRefusal::MalformedArgs(
                "temporary monster Strength wrapper model"
            )),
        );

        // Monarch's Gaze: the one member applied by a player POWER's
        // `AfterDamageGiven` (`0x33e544` IL_0042-IL_0061), through that arm.
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        state.fanouts.set_misery_attachment_upkeep(true);
        state.powers.set(PowerId::MonarchsGaze, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_damage_given_order(&[PowerId::MonarchsGaze])
        );
        state.monsters_mut().push(wrapper_carrier(3));
        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(5),
            true,
            true,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            wrapper_rows(&state.monsters[0]),
            vec![crate::hot::AttachmentRecord {
                power: Model::MonarchsGazeStrengthDown,
                applier: crate::hot::Applier::Player,
                amount: 1,
            }],
        );
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -1);

        // Shackling Potion: the one member applied by a potion
        // (`0x34ffb8` IL_007b-IL_00ae).
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        state.fanouts.set_misery_attachment_upkeep(true);
        state.monsters_mut().push(wrapper_carrier(9));
        let mut events = Vec::new();
        apply_potion_temporary_strength(&mut state, 0, 7, &mut events).unwrap();
        assert_eq!(
            wrapper_rows(&state.monsters[0]),
            vec![crate::hot::AttachmentRecord {
                power: Model::ShacklingPotion,
                applier: crate::hot::Applier::Player,
                amount: 7,
            }],
        );
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), -7);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), -7);
    }

    /// Two DIFFERENT models are two instances at two ordered positions; the
    /// SAME model twice is one instance whose value moves and whose position
    /// does not.
    ///
    /// This is the fact an aggregate signed scalar cannot hold, and the
    /// position is observable: `Misery` replays the frozen dictionary in
    /// `Creature.Powers` order, so a recipient's Artifact consumes on
    /// whichever copy comes first.
    #[test]
    fn distinct_wrapper_models_take_ordered_positions_and_one_model_stacks_in_place() {
        use crate::hot::AttachedPowerModel as Model;
        let mut monster = wrapper_carrier(4);
        monster.powers.set(PowerId::Weak, SlotWire::Int, 2);
        monster.misery_debuff_order.push(MiseryToken::Weak);

        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::Mangle,
            10,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::DarkShackles,
            9,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(
            wrapper_rows(&monster)
                .iter()
                .map(|record| (record.power, record.amount))
                .collect::<Vec<_>>(),
            vec![(Model::Mangle, 10), (Model::DarkShackles, 9)],
            "two classes, two instances, in application order",
        );
        // The Strength row leads both of them: `BeforeApplied` `0x348d20`
        // runs at `Apply` IL_02d9, before `ApplyInternal` IL_0360 attaches
        // the wrapper that triggered it.
        assert_eq!(
            monster
                .misery_debuff_order
                .attachments()
                .map(|record| record.power)
                .collect::<Vec<_>>(),
            vec![Model::Strength, Model::Mangle, Model::DarkShackles],
        );
        assert_eq!(monster.powers.value(PowerId::TempStrength), -19);
        assert_eq!(monster.powers.value(PowerId::Strength), -19);
        assert!(crate::engine::monsters::temp_strength_provenance_is_recorded(&monster));

        // A second Mangle is `ModifyAmount` `0x3f032c` on the instance that
        // exists: `SetAmount` `0x83f8c` IL_0033 writes `_amount` and never
        // touches `_powers`, so no position is created and the applier
        // recorded at the fresh attach stands.
        let before = monster.misery_debuff_order.placed_attachments();
        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::Mangle,
            15,
            crate::hot::Applier::None,
            true,
        )
        .unwrap();
        assert_eq!(
            wrapper_rows(&monster)
                .iter()
                .map(|record| (record.power, record.amount, record.applier))
                .collect::<Vec<_>>(),
            vec![
                (Model::Mangle, 25, crate::hot::Applier::Player),
                (Model::DarkShackles, 9, crate::hot::Applier::Player),
            ],
            "a stacking application moves a value, never a position or an applier",
        );
        assert_eq!(
            monster
                .misery_debuff_order
                .placed_attachments()
                .iter()
                .map(|(position, record)| (*position, record.power))
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|(position, record)| (*position, record.power))
                .collect::<Vec<_>>(),
        );
        assert_eq!(monster.powers.value(PowerId::TempStrength), -34);
    }

    /// A wrapper applied while Strength is already live leaves that row where
    /// it is; applied while Strength is absent it creates the row **first**.
    #[test]
    fn a_wrapper_orders_behind_the_strength_it_applies_however_strength_stood() {
        use crate::hot::AttachedPowerModel as Model;
        // Strength absent: the nested `Apply<StrengthPower>` from
        // `BeforeApplied` `0x348d20` appends it, then the wrapper appends
        // behind it at `ApplyInternal` `0x84012` -> `0x11da0c` IL_0063.
        let mut fresh = wrapper_carrier(1);
        write_monster_temp_strength_wrapper(
            &mut fresh,
            Model::PiercingWail,
            6,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(
            fresh
                .misery_debuff_order
                .attachments()
                .map(|record| record.power)
                .collect::<Vec<_>>(),
            vec![Model::Strength, Model::PiercingWail],
        );

        // Strength already live and recorded ahead of a token: it restacks in
        // place, so the wrapper lands behind it for a different reason.
        let mut live = wrapper_carrier(2);
        write_monster_strength(&mut live, 4, crate::hot::Applier::Monster(2), true);
        live.powers.set(PowerId::Vuln, SlotWire::Int, 1);
        live.misery_debuff_order.push(MiseryToken::Vuln);
        write_monster_temp_strength_wrapper(
            &mut live,
            Model::PiercingWail,
            6,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(
            live.misery_debuff_order.placed_attachments(),
            vec![
                (
                    0,
                    crate::hot::AttachmentRecord {
                        power: Model::Strength,
                        applier: crate::hot::Applier::Monster(2),
                        amount: -2,
                    }
                ),
                (
                    1,
                    crate::hot::AttachmentRecord {
                        power: Model::PiercingWail,
                        applier: crate::hot::Applier::Player,
                        amount: 6,
                    }
                ),
            ],
            "the live Strength keeps its position and applier; the wrapper appends",
        );
    }

    /// Side end: each wrapper is removed BEFORE its own restoration, in
    /// ledger order, and the Strength it hands back is applied with the
    /// **Owner** as applier — including the case where one restoration lands
    /// Strength on exactly zero and the next has to re-attach it at the end.
    #[test]
    fn the_side_end_unwind_removes_each_wrapper_first_and_restores_with_the_owner() {
        use crate::hot::AttachedPowerModel as Model;
        // Two wrappers on a monster with no intrinsic Strength: -4 then -6.
        let mut monster = wrapper_carrier(11);
        for (model, amount) in [(Model::DarkShackles, 4), (Model::Mangle, 6)] {
            write_monster_temp_strength_wrapper(
                &mut monster,
                model,
                amount,
                crate::hot::Applier::Player,
                true,
            )
            .unwrap();
        }
        assert_eq!(monster.powers.value(PowerId::Strength), -10);
        assert_eq!(
            monster
                .misery_debuff_order
                .attachments()
                .map(|record| record.power)
                .collect::<Vec<_>>(),
            vec![Model::Strength, Model::DarkShackles, Model::Mangle],
        );

        let restored = unwind_monster_temp_strength_wrappers(&mut monster, true).unwrap();
        assert_eq!(restored, 0);
        assert_eq!(monster.powers.value(PowerId::TempStrength), 0);
        assert!(
            monster.misery_debuff_order.attachments().is_empty(),
            "both wrappers and the Strength they applied are gone",
        );

        // Now the observable-order case. Strength +6 applied by a DIFFERENT
        // creature (an enemy move passes its own `Creature`, `0x3703a4`
        // IL_00ab-IL_00c5), then two wrappers of 4 and 6, leaving -4. Dark
        // Shackles restores first (ledger order), which lands Strength on
        // exactly 0 and REMOVES the row (`ShouldRemoveDueToAmount` `0x83b0d`
        // IL_0012-IL_0023); Mangle's restoration then re-attaches it at the
        // END, and the applier it writes is the OWNER (`0x348ba8`
        // IL_009d-IL_00c4) — neither the `Monster(99)` that first applied the
        // Strength nor the `Player` that applied the wrappers.
        let mut monster = wrapper_carrier(11);
        write_monster_strength(&mut monster, 6, crate::hot::Applier::Monster(99), true);
        monster.powers.set(PowerId::Weak, SlotWire::Int, 1);
        monster.misery_debuff_order.push(MiseryToken::Weak);
        for (model, amount) in [(Model::DarkShackles, 4), (Model::Mangle, 6)] {
            write_monster_temp_strength_wrapper(
                &mut monster,
                model,
                amount,
                crate::hot::Applier::Player,
                true,
            )
            .unwrap();
        }
        assert_eq!(monster.powers.value(PowerId::Strength), -4);
        assert_eq!(
            monster.misery_debuff_order.placed_attachments()[0],
            (
                0,
                crate::hot::AttachmentRecord {
                    power: Model::Strength,
                    applier: crate::hot::Applier::Monster(99),
                    amount: -4,
                }
            ),
            "the pre-existing instance leads, still carrying its own applier",
        );

        let restored = unwind_monster_temp_strength_wrappers(&mut monster, true).unwrap();
        assert_eq!(restored, 6);
        assert_eq!(
            monster.misery_debuff_order.placed_attachments(),
            vec![(
                1,
                crate::hot::AttachmentRecord {
                    power: Model::Strength,
                    applier: crate::hot::Applier::Monster(11),
                    amount: 6,
                }
            )],
            "the row was removed at zero and re-appended behind the Weak token, \
             with the OWNER as its applier",
        );
    }

    /// Upkeep off records nothing; a scalar this writer never recorded is
    /// given up on rather than attributed to a position; and a stack that
    /// would clamp gives up rather than disagreeing.
    #[test]
    fn the_temporary_strength_writer_is_gated_and_gives_up_rather_than_guessing() {
        use crate::hot::AttachedPowerModel as Model;
        // Upkeep off: the scalars move exactly as the pre-#2693 engine moved
        // them, and no row exists to project.
        let mut ungated = wrapper_carrier(1);
        let (temp, strength) = write_monster_temp_strength_wrapper(
            &mut ungated,
            Model::Mangle,
            10,
            crate::hot::Applier::Player,
            false,
        )
        .unwrap();
        assert_eq!((temp, strength), (-10, -10));
        assert!(ungated.misery_debuff_order.attachments().is_empty());
        assert_eq!(
            unwind_monster_temp_strength_wrappers(&mut ungated, false).unwrap(),
            0,
        );
        assert_eq!(ungated.powers.value(PowerId::TempStrength), 0);

        // A monster that ENTERED with a nonzero scalar has provenance this
        // writer cannot know. Applying a wrapper to it does not invent one,
        // and does not half-record either: the ledger stays empty so
        // "unrecorded" keeps meaning unrecorded.
        let mut legacy = wrapper_carrier(2);
        legacy.powers.set(PowerId::TempStrength, SlotWire::Int, -5);
        legacy.powers.set(PowerId::Strength, SlotWire::Int, 5);
        write_monster_temp_strength_wrapper(
            &mut legacy,
            Model::Mangle,
            10,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(legacy.powers.value(PowerId::TempStrength), -15);
        assert!(
            wrapper_rows(&legacy).is_empty(),
            "an unrecorded total is never attributed to a position",
        );
        // ...and its side end still restores the whole aggregate.
        assert_eq!(
            unwind_monster_temp_strength_wrappers(&mut legacy, true).unwrap(),
            10,
        );

        // `SetAmount` `0x83f8c` IL_0013-IL_0022 clamps a native `Amount` to
        // +/-999999999, but the aggregate scalar is not clamped the same way,
        // so a stack that crossed it could not mirror the scalar. The scalar
        // arithmetic is untouched; the rows are dropped.
        let mut clamped = wrapper_carrier(3);
        write_monster_temp_strength_wrapper(
            &mut clamped,
            Model::Mangle,
            999_999_999,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(wrapper_rows(&clamped).len(), 1);
        write_monster_temp_strength_wrapper(
            &mut clamped,
            Model::Mangle,
            1,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(clamped.powers.value(PowerId::TempStrength), -1_000_000_000);
        assert!(
            wrapper_rows(&clamped).is_empty(),
            "a row that cannot mirror the scalar is dropped, not written",
        );

        // A non-positive application is not a native state at all.
        let mut monster = wrapper_carrier(4);
        for amount in [0, -1] {
            assert_eq!(
                write_monster_temp_strength_wrapper(
                    &mut monster,
                    Model::Mangle,
                    amount,
                    crate::hot::Applier::Player,
                    true,
                ),
                Err(EngineRefusal::MalformedArgs(
                    "temporary monster Strength wrapper amount"
                )),
            );
        }
    }

    /// Plow's threshold crossing and the segment revive remove the wrappers
    /// without restoring them, and a death walk unwinds then drops the whole
    /// ledger.
    #[test]
    fn death_prevention_and_revival_clear_every_temporary_strength_row() {
        use crate::hot::AttachedPowerModel as Model;
        let mut monster = wrapper_carrier(6);
        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::DyingStar,
            9,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        clear_monster_temp_strength_wrappers(&mut monster, true);
        assert_eq!(monster.powers.value(PowerId::TempStrength), 0);
        assert!(wrapper_rows(&monster).is_empty());
        assert_eq!(
            monster.powers.value(PowerId::Strength),
            -9,
            "the wipe restores nothing — Plow zeroes Strength through its own writer",
        );

        // A dead monster's whole ledger goes, wrappers included, after the
        // unwind has run.
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        state.fanouts.set_misery_attachment_upkeep(true);
        state.monsters_mut().push(wrapper_carrier(8));
        write_monster_temp_strength_wrapper(
            &mut state.monsters_mut()[0],
            Model::CrushUnder,
            3,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        state.monsters_mut()[0].hp = 0;
        let mut events = Vec::new();
        finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::TempStrength), 0);
        assert!(
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
    }

    /// S2 made the state representable and refused it; #2693 S3 reads it.
    /// A RECORDED wrapper is exact, an unrecorded scalar is not, and the
    /// narrowing is in that direction only.
    #[test]
    fn misery_reads_a_recorded_temporary_strength_wrapper_and_refuses_an_unrecorded_one() {
        use crate::hot::AttachedPowerModel as Model;
        let mut monster = wrapper_carrier(5);
        assert_eq!(misery_scalar_state_is_exact(&monster), Ok(()));
        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::EnfeeblingTouch,
            8,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(
            misery_scalar_state_is_exact(&monster),
            Ok(()),
            "#2693 S3 lifts the categorical `TempStrength != 0` disjunct",
        );
        // The same scalar with its rows abandoned is the *unrecorded* state
        // the disjunct is replaced by, and it still refuses.
        let mut unrecorded = monster.clone();
        abandon_monster_temp_strength_provenance(&mut unrecorded);
        assert_eq!(unrecorded.powers.value(PowerId::TempStrength), -8);
        assert_eq!(
            misery_scalar_state_is_exact(&unrecorded),
            Err(EngineRefusal::MalformedArgs(
                "Misery concrete Type-2 power state"
            )),
        );
        // ...and the filter tells the truth about it in the meantime: a loss
        // wrapper is Type 2 whatever its amount, unlike Strength.
        for amount in [1, 8, 999_999_999] {
            assert!(attachment_is_misery_selected(
                &crate::hot::AttachmentRecord {
                    power: Model::EnfeeblingTouch,
                    applier: crate::hot::Applier::Player,
                    amount,
                }
            ));
        }
        assert!(Model::EnfeeblingTouch.is_temporary_strength_wrapper());
        assert!(!Model::Strength.is_temporary_strength_wrapper());
        assert!(!Model::Imbalanced.is_temporary_strength_wrapper());
    }

    // ------------------------------------------------------------------
    // #2693 S3 — the temporary-Strength fold and the S4 provenance debt.
    // ------------------------------------------------------------------

    fn snapshot_amounts(monster: &HotMonster) -> Vec<(crate::hot::AttachedPowerModel, i32)> {
        misery_snapshot(monster)
            .unwrap()
            .into_iter()
            .filter_map(|entry| match entry {
                MiserySnapshotEntry::Attachment(record) => Some((record.power, record.amount)),
                MiserySnapshotEntry::Scalar(_) => None,
            })
            .collect()
    }

    /// The fold at `0x3ad358` IL_00a1-IL_0133, read off the snapshot rather
    /// than through a play: the frozen `StrengthPower` value gains each
    /// selected wrapper's own frozen amount, the wrapper entries keep their
    /// positions and values, and nothing is synthesised when the Strength
    /// entry is not in the dictionary (IL_00ff).
    #[test]
    fn the_temporary_strength_fold_adjusts_the_frozen_strength_and_synthesises_nothing() {
        use crate::hot::AttachedPowerModel as Model;

        // Intrinsic +2 with a loss wrapper of 5: the source stands at -3 and
        // the copy is +2.
        let mut monster = wrapper_carrier(5);
        write_monster_strength(&mut monster, 2, crate::hot::Applier::Player, true);
        write_monster_temp_strength_wrapper(
            &mut monster,
            Model::DarkShackles,
            5,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(monster.powers.value(PowerId::Strength), -3);
        assert_eq!(
            snapshot_amounts(&monster),
            [(Model::Strength, 2), (Model::DarkShackles, 5)],
        );

        // Two wrappers accumulate onto the one entry, each contributing its
        // OWN frozen value exactly once.
        let mut pair = wrapper_carrier(5);
        write_monster_strength(&mut pair, 2, crate::hot::Applier::Player, true);
        for (model, amount) in [(Model::DarkShackles, 4), (Model::EnfeeblingTouch, 3)] {
            write_monster_temp_strength_wrapper(
                &mut pair,
                model,
                amount,
                crate::hot::Applier::Player,
                true,
            )
            .unwrap();
        }
        assert_eq!(pair.powers.value(PowerId::Strength), -5);
        assert_eq!(
            snapshot_amounts(&pair),
            [
                (Model::Strength, 2),
                (Model::DarkShackles, 4),
                (Model::EnfeeblingTouch, 3),
            ],
        );

        // A wrapper whose internally-applied Strength is NOT in the
        // dictionary copies alone. Both reachable shapes: a net-positive
        // Strength (present but unselected) and no Strength at all.
        let mut positive = wrapper_carrier(5);
        write_monster_strength(&mut positive, 8, crate::hot::Applier::Player, true);
        write_monster_temp_strength_wrapper(
            &mut positive,
            Model::DarkShackles,
            5,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(positive.powers.value(PowerId::Strength), 3);
        assert_eq!(snapshot_amounts(&positive), [(Model::DarkShackles, 5)]);

        // Exactly cancelling: the entry stays in the dictionary at zero, and
        // the executor is what skips it (IL_026f).
        let mut cancels = wrapper_carrier(5);
        write_monster_temp_strength_wrapper(
            &mut cancels,
            Model::DarkShackles,
            5,
            crate::hot::Applier::Player,
            true,
        )
        .unwrap();
        assert_eq!(cancels.powers.value(PowerId::Strength), -5);
        assert_eq!(
            snapshot_amounts(&cancels),
            [(Model::Strength, 0), (Model::DarkShackles, 5)],
        );
        assert_eq!(
            misery_snapshot(&cancels)
                .unwrap()
                .iter()
                .map(misery_snapshot_entry_amount)
                .collect::<Vec<_>>(),
            [0, 5],
        );
    }

    /// #2693 S4 — an entering Strength gets its row exactly where the ledger
    /// leaves its position undetermined by nothing, and nowhere else.
    #[test]
    fn entering_strength_provenance_materialises_only_where_its_position_is_determined() {
        use crate::hot::AttachedPowerModel as Model;
        fn root(strength: i32, upkeep: bool) -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 60;
            state.max_hp = 60;
            state.fanouts.set_misery_attachment_upkeep(upkeep);
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
            monster.uid = 4;
            monster
                .powers
                .set(PowerId::Strength, SlotWire::Int, strength);
            state.monsters_mut().push(monster);
            state
        }

        // The determined case: nothing else is recorded, so the instance
        // precedes everything that can be added later.
        let mut state = root(3, true);
        materialize_entering_strength_provenance(&mut state);
        assert_eq!(
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .copied()
                .collect::<Vec<_>>(),
            [crate::hot::AttachmentRecord {
                power: Model::Strength,
                applier: crate::hot::Applier::Unknown,
                amount: 3,
            }],
        );
        // ...and the row then stays in step through a sign crossing, which is
        // the whole point: the play that drives it negative no longer has to
        // give up.
        write_monster_strength(
            &mut state.monsters_mut()[0],
            -6,
            crate::hot::Applier::Player,
            true,
        );
        assert_eq!(
            crate::engine::monsters::strength_attachment_record(&state.monsters[0]),
            Some(crate::hot::AttachmentRecord {
                power: Model::Strength,
                applier: crate::hot::Applier::Unknown,
                amount: -6,
            }),
            "`ModifyAmount` moves a value, never a position or an applier",
        );
        assert_eq!(misery_scalar_state_is_exact(&state.monsters[0]), Ok(()));

        // A zero scalar has no instance to place.
        let mut zero = root(0, true);
        materialize_entering_strength_provenance(&mut zero);
        assert!(
            zero.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );

        // The upkeep gate: a fight that cannot reach `Misery` records
        // nothing, so its documents keep the pre-#2693 bytes.
        let mut ungated = root(3, false);
        materialize_entering_strength_provenance(&mut ungated);
        assert!(
            ungated.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );

        // Not determined: something is already recorded to be ordered
        // against.
        let mut tokened = root(3, true);
        tokened.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        materialize_entering_strength_provenance(&mut tokened);
        assert!(
            tokened.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
        assert!(!crate::engine::monsters::strength_provenance_is_recorded(
            &tokened.monsters[0]
        ));

        // ...and a by-kind Bowlbug Rock, whose intrinsic `ImbalancedPower`
        // `misery_snapshot` replays at position 0.
        let mut rock = root(3, true);
        state_kind(&mut rock, MonsterKind::BowlbugRock);
        materialize_entering_strength_provenance(&mut rock);
        assert!(
            rock.monsters[0]
                .misery_debuff_order
                .attachments()
                .is_empty()
        );
    }

    /// #3364 — the generated `MONSTER_MODELS` spawn powers (`Apply<XPower>`
    /// at `AfterAddedToRoom`) and the IL-cited self-applier set agree: every
    /// kind whose row spawns `StrengthPower` is attributed, and no other.
    #[test]
    fn spawn_strength_self_appliers_are_exactly_the_modelled_strength_spawners() {
        let spawners = MonsterKind::NAMES
            .iter()
            .map(|name| MonsterKind::from_str(name).unwrap())
            .filter(|kind| crate::encounters::initial_power(*kind, "StrengthPower", 0).is_ok())
            .collect::<Vec<_>>();
        assert_eq!(spawners, [MonsterKind::MysteriousKnight]);
        for name in MonsterKind::NAMES {
            let kind = MonsterKind::from_str(name).unwrap();
            assert_eq!(
                spawn_strength_is_self_applied(kind),
                spawners.contains(&kind),
                "{kind:?}"
            );
        }
    }

    /// #3364 — an opening names its owner as the applier of a spawn Strength
    /// exactly where the IL proves `AfterAddedToRoom` self-applied it, and
    /// leaves every other shape `Unknown`.
    #[test]
    fn spawn_strength_is_attributed_to_its_owner_only_where_it_is_the_spawn() {
        use crate::hot::AttachedPowerModel as Model;
        fn root(kind: MonsterKind, strength: i32, upkeep: bool) -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 60;
            state.max_hp = 60;
            state.fanouts.set_misery_attachment_upkeep(upkeep);
            let mut monster = HotMonster::new(kind, 60);
            monster.uid = 7;
            monster
                .powers
                .set(PowerId::Strength, SlotWire::Int, strength);
            state.monsters_mut().push(monster);
            materialize_entering_strength_provenance(&mut state);
            state
        }
        fn rows(state: &HotState) -> Vec<crate::hot::AttachmentRecord> {
            state.monsters[0]
                .misery_debuff_order
                .attachments()
                .copied()
                .collect()
        }
        let row = |applier, amount| crate::hot::AttachmentRecord {
            power: Model::Strength,
            applier,
            amount,
        };

        // The Mysterious Knight's spawn: `0x3642a0` IL_0087-IL_00a0.
        let mut knight = root(MonsterKind::MysteriousKnight, 6, true);
        assert_eq!(rows(&knight), [row(crate::hot::Applier::Unknown, 6)]);
        attribute_spawn_strength_to_its_owner(&mut knight);
        assert_eq!(rows(&knight), [row(crate::hot::Applier::Monster(7), 6)]);
        assert!(crate::engine::monsters::strength_provenance_is_recorded(
            &knight.monsters[0]
        ));
        // Idempotent: a recorded owner is not rewritten again.
        let before = knight.clone();
        attribute_spawn_strength_to_its_owner(&mut knight);
        assert_eq!(knight, before);
        // ...and the owner survives a later sign crossing: `ModifyAmount`
        // never touches the applier.
        write_monster_strength(
            &mut knight.monsters_mut()[0],
            -3,
            crate::hot::Applier::Player,
            true,
        );
        assert_eq!(rows(&knight), [row(crate::hot::Applier::Monster(7), -3)]);

        // A kind whose `AfterAddedToRoom` applies no Strength stays Unknown.
        let mut toadpole = root(MonsterKind::Toadpole, 6, true);
        attribute_spawn_strength_to_its_owner(&mut toadpole);
        assert_eq!(rows(&toadpole), [row(crate::hot::Applier::Unknown, 6)]);

        // A Knight whose Strength is not the spawn amount is not the spawn
        // instance this can vouch for.
        let mut moved = root(MonsterKind::MysteriousKnight, 4, true);
        attribute_spawn_strength_to_its_owner(&mut moved);
        assert_eq!(rows(&moved), [row(crate::hot::Applier::Unknown, 4)]);

        // A row whose applier is already recorded is not overwritten.
        let mut player = root(MonsterKind::MysteriousKnight, 0, true);
        write_monster_strength(
            &mut player.monsters_mut()[0],
            6,
            crate::hot::Applier::Player,
            true,
        );
        attribute_spawn_strength_to_its_owner(&mut player);
        assert_eq!(rows(&player), [row(crate::hot::Applier::Player, 6)]);

        // Anything else recorded beside the row: not the bare spawn ledger.
        let mut tokened = root(MonsterKind::MysteriousKnight, 6, true);
        tokened.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        attribute_spawn_strength_to_its_owner(&mut tokened);
        assert_eq!(rows(&tokened), [row(crate::hot::Applier::Unknown, 6)]);

        // The upkeep gate: no row was materialised, so none is invented.
        let mut ungated = root(MonsterKind::MysteriousKnight, 6, false);
        attribute_spawn_strength_to_its_owner(&mut ungated);
        assert!(rows(&ungated).is_empty());
    }

    fn state_kind(state: &mut HotState, kind: MonsterKind) {
        let uid = state.monsters[0].uid;
        let strength = state.monsters[0].powers.value(PowerId::Strength);
        let mut monster = HotMonster::new(kind, 60);
        monster.uid = uid;
        monster
            .powers
            .set(PowerId::Strength, SlotWire::Int, strength);
        state.monsters_mut()[0] = monster;
    }

    /// Brimstone's `+1` to every enemy (`0x32098c` IL_012d-IL_0130) is a
    /// TYPE 1 application, so `SleightOfFleshPower` does not fire for it —
    /// `0x344dc4` IL_003d-IL_0049 returns unless
    /// `GetTypeForAmount(delta) == 2`, and `0x83a94` IL_0013-IL_003e reports
    /// 2 for `StrengthPower` only while the delta is negative. The negative
    /// direction is the control.
    #[test]
    fn a_positive_monster_strength_delta_does_not_wake_sleight_of_flesh() {
        fn roster() -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 60;
            state.max_hp = 60;
            state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 4);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::SleightOfFlesh])
            );
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
            monster.uid = 9;
            state.monsters_mut().push(monster);
            state
        }

        let mut positive = roster();
        apply_monster_strength_delta_after_type_two_gate(
            &mut positive,
            None,
            0,
            1,
            crate::hot::Applier::None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(positive.monsters[0].powers.value(PowerId::Strength), 1);
        assert_eq!(positive.monsters[0].hp, 60, "no Sleight damage");

        let mut negative = roster();
        apply_monster_strength_delta_after_type_two_gate(
            &mut negative,
            None,
            0,
            -1,
            crate::hot::Applier::Player,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            negative.monsters[0].hp, 56,
            "the Type-2 control still bites"
        );
    }

    /// Death cleanup already clears both halves of the stun state, so #2647 D1
    /// adds nothing to clear — which is exactly what makes a later D2' field an
    /// incomplete change unless it is added here too.
    #[test]
    fn dead_monster_cleanup_clears_the_generic_stun_state() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut monster = HotMonster::new(MonsterKind::BowlbugEgg, 3);
        monster.uid = 0;
        state.monsters_mut().push(monster);
        crate::engine::monsters::install_generic_stun(&mut state, 0).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Stunned);

        damage_monster(
            &mut state,
            0,
            DotNetDecimal::from_i64(9),
            false,
            false,
            &mut Vec::new(),
        )
        .unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
        assert_eq!(state.monsters[0].forced_follow_up, MonsterFollowUp::None);
    }

    // ---- #2655: native powered AttackCommand batches -------------------

    /// `0x3e96c8` IL_0078-IL_00af gates the dealer's liveness at Damage ENTRY
    /// only; nothing re-tests it per receiver, and the commit at IL_02be-IL_04c1
    /// follows `BeforeDamageReceived`'s resume (IL_02b9) unconditionally. The
    /// next HIT is what a dead dealer cancels (`0x3f19c0` IL_0156-IL_0161).
    #[test]
    fn lethal_thorns_mid_batch_commits_every_snapshotted_receiver_then_stops_later_hits() {
        let mut state = HotState::at_defaults();
        state.hp = 1;
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 20);
        thorny.uid = 0;
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 5);
        state.monsters_mut().push(thorny);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 20);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0, 1], 4, 2, &mut events).unwrap();

        assert_eq!(state.hp, 0);
        assert!(state.fanouts.player_hooks_deactivated());
        assert_eq!(state.monsters[0].hp, 16);
        assert_eq!(
            state.monsters[1].hp, 16,
            "the receiver snapshotted at IL_00c6 still commits after the dealer dies"
        );
    }

    /// `CombatState.IterateHookListeners` (`0x3f9720`) rebuilds the listener
    /// list at every `Hook.ModifyDamage` enumeration (`0x106aa0` IL_002b,
    /// IL_0098). A creature's Powers join unconditionally (IL_008f-IL_0097),
    /// but a player's relics are gated on `Player.IsActiveForHooks`
    /// (IL_00bb-IL_00c2), which `DeactivateHooks` clears inside Kill. So the
    /// receiver reached after the dealer's own death folds no owner relic.
    #[test]
    fn a_receiver_after_the_dealer_dies_rebuilds_modifiers_without_owner_relics() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .set_relics(&[RelicId::RelicMiniatureCannon])
            .unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 1;
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 20);
        thorny.uid = 0;
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 5);
        state.monsters_mut().push(thorny);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 20);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        // Miniature Cannon reads the live physical card's IsUpgraded (#3245).
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut events = Vec::new();

        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0, 1],
            4,
            1,
            &mut events,
        )
        .unwrap();

        assert!(state.fanouts.player_hooks_deactivated());
        assert_eq!(
            state.monsters[0].hp, 13,
            "the first receiver folds Miniature Cannon's +3 while the owner is live"
        );
        assert_eq!(
            state.monsters[1].hp, 16,
            "the later receiver rebuilds its modifiers without the owner's relics"
        );
    }

    /// Phase 1 commits every receiver (`0x3e96c8` IL_02be-IL_04c1, advancing at
    /// IL_0a99-IL_0aa4) before phase 3 drains one `Kill` list at IL_0ead-IL_0eb4.
    /// #2654's Kaiser Crab needs exactly this: both outer receivers have taken
    /// their damage before the first CrabRage death hook can grant the sibling
    /// anything.
    #[test]
    fn every_receiver_commits_before_the_first_death_drains() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        for (uid, slot) in [(0_u32, 0_i32), (1, 1)] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 3);
            monster.uid = uid;
            monster.slot = slot;
            state.monsters_mut().push(monster);
        }
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0, 1], 4, 1, &mut events).unwrap();

        let first_death = events
            .iter()
            .position(|event| matches!(event, Event::MonsterDied { .. }))
            .expect("both receivers die");
        let damaged: Vec<usize> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                matches!(event, Event::MonsterDamaged { .. }).then_some(index)
            })
            .collect();
        assert_eq!(damaged.len(), 2);
        assert!(
            damaged.iter().all(|index| *index < first_death),
            "every commit precedes the first drained death: {events:?}"
        );
        assert!(state.monsters.iter().all(|monster| monster.hp <= 0));
        assert!(state.history.over);
    }

    /// #2655 C2: the batch runs `PersonalHivePower.AfterDamageReceived`
    /// (`0x340220`) inside phase 2 — at `0x3e96c8` IL_0d86, before any death
    /// drains — instead of after the receiver's own death suffix. That move is
    /// RNG-observable, because each live stack injects a generated Dazed at a
    /// RANDOM Draw position and therefore consumes Shuffle RNG against whatever
    /// the Draw pile holds at that moment.
    ///
    /// It is nevertheless UNREACHABLE from admitted content, and this pins the
    /// reason rather than asserting it. `PersonalHivePower` has exactly one
    /// admitted owner, Entomancer (`src/encounters/elite.rs::build_entomancer`
    /// builds a SOLO roster), and `entomancer_state_is_valid` destructures
    /// `[owner]` — a one-monster roster. Admission refuses any state where a
    /// monster carries `Hive != 0` and that quotient fails
    /// (`admission.rs` "Entomancer Hive lifecycle"), and `player_attack_inner`
    /// re-checks it per command. A live Hive owner therefore implies exactly
    /// one monster, which implies a one-receiver hit, which takes the
    /// sequential path: the batch can never see a Hive receiver.
    ///
    /// If a future roster admits Hive beside a peer, THIS test fails first and
    /// forces the ordering to be re-derived against native.
    #[test]
    fn a_multi_monster_roster_carrying_hive_cannot_reach_the_batch() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        let mut owner = HotMonster::new(MonsterKind::Entomancer, 30);
        owner.uid = 0;
        owner.slot = 0;
        owner.powers.set(PowerId::Hive, SlotWire::Int, 1);
        state.monsters_mut().push(owner);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 30);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        assert!(
            !super::super::monsters::entomancer_state_is_valid(&state),
            "a Hive owner beside a peer is not an admitted roster"
        );
        let source = source_spec(CardId::StrikeIronclad);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            player_attack(&mut state, &source, &[0, 1], 4, 1, &mut events),
            Err(EngineRefusal::MalformedArgs("Entomancer Hive owner/state"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// #2655 C4: the whole owner-relic additive fold, not one term of it.
    /// Strike Dummy (+3), Fake Strike Dummy (+1) and Miniature Cannon (+3)
    /// share one expression, and all three vanish together for the receiver
    /// reached after the dealer's hooks deactivate (`0x3f9720`
    /// IL_00bb-IL_00c2).
    ///
    /// Red Skull's +3 used to be a fourth relic term here. It is a real
    /// `StrengthPower` (#3044, `red_skull_after_player_hp_changed`), and a
    /// creature's powers fold unconditionally (IL_008f-IL_0097), so it now
    /// SURVIVES for the later receiver as the rest of the Strength does.
    #[test]
    fn every_owner_relic_in_the_additive_fold_drops_for_a_later_receiver() {
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 1,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .set_relics(&[
                RelicId::RelicStrikeDummy,
                RelicId::RelicFakeStrikeDummy,
                RelicId::RelicMiniatureCannon,
                RelicId::RelicRedSkull,
            ])
            .unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        assert!(source.strike_tag, "Strike Dummy needs the Strike tag");
        let mut state = HotState::at_defaults();
        // Red Skull holds its Strength while hp * 2 <= max_hp, and this HP is
        // also what the Thorns receiver kills.
        state.hp = 5;
        state.max_hp = 60;
        state.fanouts.set_red_skull_owned(true);
        state.powers.set(PowerId::Strength, SlotWire::Int, 3);
        let mut thorny = HotMonster::new(MonsterKind::Toadpole, 60);
        thorny.uid = 0;
        thorny.powers.set(PowerId::Thorns, SlotWire::Int, 5);
        state.monsters_mut().push(thorny);
        let mut peer = HotMonster::new(MonsterKind::Toadpole, 60);
        peer.uid = 1;
        peer.slot = 1;
        state.monsters_mut().push(peer);
        // Miniature Cannon reads the live physical card's IsUpgraded (#3245).
        state.piles.get_mut(PileId::Play).make_mut().push(HotCard {
            uid: 7,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut events = Vec::new();

        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0, 1],
            4,
            1,
            &mut events,
        )
        .unwrap();

        assert!(state.fanouts.player_hooks_deactivated());
        // 4 + StrikeDummy 3 + FakeStrikeDummy 1 + MiniatureCannon 3 + Strength 3
        assert_eq!(
            state.monsters[0].hp,
            60 - 14,
            "the live owner contributes every relic in the fold"
        );
        assert_eq!(
            state.monsters[1].hp,
            60 - 7,
            "the later receiver folds the base and Red Skull's real Strength"
        );
    }

    /// Knights with Dampen applied, Miniature Cannon owned, and a Dampened
    /// (L0) Strike sitting in the Play pile as UID 4.
    fn miniature_cannon_dampen_fixture(magi_hp: i32) -> (HotState, Catalog, CardAtom, CardAtom) {
        let l0 = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        let (l0_atom, l1_atom) = builder.intern_dampen_card_pair(l0).unwrap();
        builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        builder
            .set_relics(&[RelicId::RelicMiniatureCannon])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        state.exact_piles = true;
        state.next_card_uid = 5;
        for (uid, pile) in crate::engine::cards::DAMPEN_PILES.into_iter().enumerate() {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: uid as u32,
                atom: l1_atom,
                flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_HEXED,
            });
        }
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(0)));
        assert!(flail.random_ai.set_log(&[2, 0]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(1)));
        assert!(spectral.random_ai.set_log(&[0, 1]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, magi_hp);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        magi.loop_pos = 1;
        state.monsters_mut().extend([flail, spectral, magi]);
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        crate::engine::cards::apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].atom, l0_atom);
        (state, catalog, l0_atom, l1_atom)
    }

    /// #3245: `MiniatureCannon::ModifyDamageAdditive` (RVA `0x96ec8`
    /// IL_0024-IL_002b) reads the live `cardSource.IsUpgraded` on every
    /// `Hook.ModifyDamage` call (`CreatureCmd/<Damage>d__12` `0x3e96c8`
    /// IL_01b2, once per receiver of each hit). The first hit kills Magi
    /// Knight with Flail alive, so `DampenPower::AfterRemoved` restores the
    /// source to L1 mid-command and the second hit takes the +3.
    #[test]
    fn miniature_cannon_rereads_the_source_upgrade_after_a_mid_command_dampen_restore() {
        let (mut state, catalog, l0, l1) = miniature_cannon_dampen_fixture(4);
        let source = *catalog.spec(l0).unwrap();
        let mut events = Vec::new();
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 4),
            &[0, 2],
            4,
            2,
            &mut events,
        )
        .unwrap();
        assert!(state.monsters[2].hp <= 0);
        assert!(state.card_states.dampen().is_none());
        assert_eq!(state.piles.get(PileId::Play).as_slice()[0].atom, l1);
        // Hit 1: 4 at L0 (no Cannon). Hit 2: 4 + 3 on the restored L1 card.
        assert_eq!(state.monsters[0].hp, 108 - 4 - 7);
    }

    /// The control: with no mid-command death the Dampened source stays L0
    /// for every hit, and Miniature Cannon never contributes.
    #[test]
    fn miniature_cannon_skips_a_dampened_source_that_stays_downgraded() {
        let (mut state, catalog, l0, _l1) = miniature_cannon_dampen_fixture(89);
        let source = *catalog.spec(l0).unwrap();
        let mut events = Vec::new();
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 4),
            &[0, 2],
            4,
            2,
            &mut events,
        )
        .unwrap();
        assert!(state.card_states.dampen().is_some());
        assert_eq!(state.monsters[0].hp, 108 - 8);
        assert_eq!(state.monsters[2].hp, 89 - 8);
    }

    /// The live read refuses by name rather than falling back to the entry
    /// snapshot: a missing or duplicated physical source, a live row that is
    /// no longer the same card, and a plan without a physical source.
    #[test]
    fn miniature_cannon_live_read_refuses_an_unauthenticated_source() {
        let (state, catalog, l0, l1) = miniature_cannon_dampen_fixture(89);
        let source = *catalog.spec(l0).unwrap();
        fn plan(uid: Option<u32>, catalog: Option<&Catalog>) -> AttackPlan<'_, '_> {
            AttackPlan {
                targeting: AttackTargeting::Fixed(&[0]),
                base: 4,
                decimal_base: None,
                hits: 1,
                source_uid: uid,
                frozen_target_identity: None,
                catalog,
                context_mode: AttackContextMode::Ordinary,
                result_sink: None,
                dealer: AttackDealer::Player,
            }
        }
        assert_eq!(
            miniature_cannon_source_is_upgraded(&state, &source, &plan(Some(4), Some(&catalog))),
            Ok(false)
        );
        let mut restored = state.clone();
        restored.piles.get_mut(PileId::Play).make_mut()[0].atom = l1;
        assert_eq!(
            miniature_cannon_source_is_upgraded(&restored, &source, &plan(Some(4), Some(&catalog))),
            Ok(true)
        );
        assert_eq!(
            miniature_cannon_source_is_upgraded(&state, &source, &plan(Some(9), Some(&catalog))),
            Err(EngineRefusal::ActiveCardNotUnique { uid: 9, matches: 0 })
        );
        let mut duplicated = state.clone();
        let duplicate = duplicated.piles.get(PileId::Play).as_slice()[0];
        duplicated
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(duplicate);
        assert_eq!(
            miniature_cannon_source_is_upgraded(
                &duplicated,
                &source,
                &plan(Some(4), Some(&catalog))
            ),
            Err(EngineRefusal::ActiveCardNotUnique { uid: 4, matches: 2 })
        );
        let other = catalog
            .atom(&CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .and_then(|atom| catalog.spec(atom))
            .copied()
            .unwrap();
        assert_eq!(
            miniature_cannon_source_is_upgraded(&state, &other, &plan(Some(4), Some(&catalog))),
            Err(EngineRefusal::MalformedArgs(
                "MiniatureCannon physical card source"
            ))
        );
        for bad_plan in [plan(None, Some(&catalog)), plan(Some(4), None)] {
            assert_eq!(
                miniature_cannon_source_is_upgraded(&state, &source, &bad_plan),
                Err(EngineRefusal::MalformedArgs(
                    "MiniatureCannon physical card source"
                ))
            );
        }
    }

    /// #2655 C4, the multiplicative side: Vitruvian Minion (x2) is its own
    /// `if` in the fold, so it is not covered by the additive-fold witness
    /// above. It drops for the receiver reached after the dealer's hooks
    /// deactivate. (Undying Sigil used to be pinned here as an outgoing x1/2;
    /// #3035 moved it to the incoming monster-attack snapshot, where the IL
    /// puts it — see `undying_sigil_halves_only_a_doomed_dealers_attack_on_the_player`.)
    #[test]
    fn owner_relic_multipliers_drop_for_a_later_receiver() {
        fn two_receivers() -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 5;
            state.max_hp = 200;
            for (uid, slot) in [(0_u32, 0_i32), (1, 1)] {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
                monster.uid = uid;
                monster.slot = slot;
                if uid == 0 {
                    monster.powers.set(PowerId::Thorns, SlotWire::Int, 5);
                }
                state.monsters_mut().push(monster);
            }
            state
        }

        // Vitruvian Minion doubles a Minion-tagged source.
        let identity = CardIdentity {
            id: CardId::MinionDiveBomb,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder
            .set_relics(&[RelicId::RelicVitruvianMinion])
            .unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        assert!(source.row.tags.contains(&"Minion"));
        let mut state = two_receivers();
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0, 1],
            4,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.fanouts.player_hooks_deactivated());
        assert_eq!(state.monsters[0].hp, 60 - 8, "x2 while the owner is live");
        assert_eq!(state.monsters[1].hp, 60 - 4, "no x2 after it dies");
    }

    /// #3035: `UndyingSigil::ModifyDamageMultiplicative` RVA `0x9d400`
    /// halves a powered attack whose TARGET is the owner (IL_0024-IL_0030)
    /// and whose DEALER is not (IL_0038-IL_0045) when the dealer's own
    /// `CurrentHp <= GetPowerAmount<DoomPower>` (IL_004d-IL_005b).
    #[test]
    fn undying_sigil_halves_only_a_doomed_dealers_attack_on_the_player() {
        fn incoming(sigil: bool, hp: i32, doom: i32, weak: bool, base: i64) -> i32 {
            let mut state = HotState::at_defaults();
            state.hp = 100;
            state.max_hp = 100;
            state.fanouts.set_undying_sigil_owned(sigil);
            let mut dealer = HotMonster::new(MonsterKind::Toadpole, hp);
            if doom > 0 {
                dealer.powers.set(PowerId::Doom, SlotWire::Int, doom);
            }
            if weak {
                dealer.powers.set(PowerId::Weak, SlotWire::Int, 1);
            }
            state.monsters_mut().push(dealer);
            let snapshot = monster_attack_hit_damage(&state, &state.monsters[0], base).unwrap();
            monster_attack_player(&mut state, 0, base, 1, &mut Vec::new()).unwrap();
            assert_eq!(
                i64::from(100 - state.hp),
                snapshot,
                "intent == executed hit"
            );
            100 - state.hp
        }

        // Doomed dealer (HP == Doom, and HP < Doom): half, truncated.
        assert_eq!(incoming(true, 10, 10, false, 9), 4, "9 * 1/2 = 4.5 -> 4");
        assert_eq!(incoming(true, 10, 30, false, 10), 5);
        // Composes with the dealer's Weak in the one fold: 10 * 3/4 * 1/2.
        assert_eq!(incoming(true, 10, 10, true, 10), 3, "3.75 -> 3");
        // Not doomed (HP one above Doom), no Doom, or no relic: full.
        assert_eq!(incoming(true, 10, 9, false, 9), 9);
        assert_eq!(incoming(true, 10, 0, false, 9), 9);
        assert_eq!(incoming(false, 10, 10, false, 9), 9);

        // The player hitting a doomed monster is NOT halved (the owner is the
        // dealer, IL_0024-IL_0030 wants it as the target).
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.set_relics(&[RelicId::RelicUndyingSigil]).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.fanouts.set_undying_sigil_owned(true);
        let mut target = HotMonster::new(MonsterKind::Toadpole, 60);
        target.powers.set(PowerId::Doom, SlotWire::Int, 100);
        state.monsters_mut().push(target);
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0],
            8,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(state.monsters[0].hp, 60 - 8);
    }

    /// #2655 C4, the last two reachable owner relics. Mystic Lighter (+9) is
    /// gated on the source carrying an enchantment; Paper Phrog rewrites the
    /// Vulnerable multiplier itself (7/4 instead of 3/2) rather than adding a
    /// factor, so it is invisible unless the target is Vulnerable. Both drop
    /// for the receiver reached after the dealer's hooks deactivate.
    #[test]
    fn enchantment_and_vulnerable_owner_relics_drop_for_a_later_receiver() {
        fn two_receivers(vulnerable: bool) -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 5;
            state.max_hp = 200;
            for (uid, slot) in [(0_u32, 0_i32), (1, 1)] {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
                monster.uid = uid;
                monster.slot = slot;
                if uid == 0 {
                    monster.powers.set(PowerId::Thorns, SlotWire::Int, 5);
                }
                if vulnerable {
                    monster.powers.set(PowerId::Vuln, SlotWire::Int, 1);
                }
                state.monsters_mut().push(monster);
            }
            state
        }

        // Mystic Lighter: +9 for any enchanted source. Sharp at amount 0 keeps
        // the enchantment's own additive term out of the arithmetic.
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Sharp,
                amount: 0,
            }),
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.set_relics(&[RelicId::RelicMysticLighter]).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let mut state = two_receivers(false);
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0, 1],
            4,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.fanouts.player_hooks_deactivated());
        assert_eq!(state.monsters[0].hp, 60 - 13, "+9 while the owner is live");
        assert_eq!(state.monsters[1].hp, 60 - 4, "no +9 after it dies");

        // Paper Phrog: the Vulnerable factor itself becomes 7/4.
        let identity = CardIdentity {
            id: CardId::StrikeIronclad,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        builder.set_relics(&[RelicId::RelicPaperPhrog]).unwrap();
        let catalog = builder.build();
        let source = *catalog.spec(atom).unwrap();
        let mut state = two_receivers(true);
        player_attack_from_card(
            &mut state,
            (&catalog, &source, 7),
            &[0, 1],
            8,
            1,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(state.fanouts.player_hooks_deactivated());
        assert_eq!(
            state.monsters[0].hp,
            60 - 14,
            "8 * 7/4 while the owner is live"
        );
        assert_eq!(state.monsters[1].hp, 60 - 12, "8 * 3/2 after it dies");
    }

    /// Corpse Slug is the admitted death hook that reads the live roster, so
    /// it is the closest standing analogue of #2654's CrabRage. The batch makes
    /// its two arms visible at once: a survivor still takes one application per
    /// drained death, while a sibling killed by the SAME command takes none,
    /// because `RavenousPower/<AfterDeath>d__6::MoveNext` (`0x341844`
    /// IL_0069-IL_0076) returns for a dead owner and that sibling's HP was
    /// already committed at `0x3e96c8` IL_04bc.
    ///
    /// Frozen Python's per-target loop resolved the first death while the
    /// second slug was still at full HP and therefore gave it an application
    /// native cannot: the one advisory release-census row this slice moves.
    #[test]
    fn a_same_batch_corpse_takes_no_ravenous_while_a_survivor_takes_both() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.max_hp = 60;
        for (uid, slot, max_hp, hp, loop_pos) in [
            (0_u32, 0_i32, 27_i32, 3_i32, 0_i32),
            (1, 1, 28, 3, 1),
            (2, 2, 29, 29, 2),
        ] {
            let mut monster = HotMonster::new(MonsterKind::CorpseSlug, hp);
            monster.uid = uid;
            monster.slot = slot;
            monster.max_hp = max_hp;
            monster.loop_pos = loop_pos;
            monster.powers.set(
                PowerId::Ravenous,
                SlotWire::Int,
                crate::engine::admission::CORPSE_SLUG_RAVENOUS,
            );
            state.monsters_mut().push(monster);
        }
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0, 1, 2], 4, 1, &mut events).unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert!(state.monsters[1].hp <= 0);
        assert_eq!(state.monsters[2].hp, 25);
        assert_eq!(
            state.monsters[1].powers.value(PowerId::Strength),
            0,
            "a sibling committed by the same batch is already dead when the \
             first AfterDeath runs"
        );
        assert_eq!(
            state.monsters[2].powers.value(PowerId::Strength),
            2 * crate::engine::admission::CORPSE_SLUG_RAVENOUS,
            "the live survivor still takes one application per drained death"
        );
    }

    /// The single-receiver hit keeps the sequential walk, so a one-target
    /// command is byte-identical to the pre-#2655 path.
    #[test]
    fn a_single_receiver_hit_does_not_open_a_batch() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 3);
        monster.uid = 0;
        state.monsters_mut().push(monster);
        let source = source_spec(CardId::StrikeIronclad);
        let mut events = Vec::new();

        player_attack(&mut state, &source, &[0], 4, 1, &mut events).unwrap();

        assert!(state.monsters[0].hp <= 0);
        assert!(!state.fanouts.synchronous_damage_is_active());
        assert!(state.history.over);
    }

    /// RETAINED wall (#2651): a Strike-tagged child issuing an AoE while a
    /// synchronous Damage batch is open still needs its own port. This slice
    /// deliberately does not widen it — and now reaches it from the powered
    /// batch's own phases as well, because those phases keep the receipt open.
    #[test]
    fn a_powered_multi_target_hit_inside_a_synchronous_damage_batch_refuses() {
        let mut state = HotState::at_defaults();
        state.hp = 60;
        for (uid, slot) in [(0_u32, 0_i32), (1, 1)] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.uid = uid;
            monster.slot = slot;
            state.monsters_mut().push(monster);
        }
        state.fanouts.enter_damage_batch();
        let source = source_spec(CardId::StrikeIronclad);
        let before = state.clone();
        let mut events = Vec::new();

        assert_eq!(
            player_attack(&mut state, &source, &[0, 1], 4, 1, &mut events),
            Err(EngineRefusal::PowerOrderNotModeled(
                "powered multi-target damage inside Damage callback"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// Batch entry authenticates exactly what the unpowered primitive does:
    /// the Queen roster quotient must be exact before a pending-death receipt
    /// exists. (The Phrog/Horn wall is gone with #2656; see
    /// `horn_batch_honors_pending_infested_veto_then_spawns_last`.)
    #[test]
    fn batch_entry_retains_the_queen_roster_wall() {
        let source = source_spec(CardId::StrikeIronclad);
        let mut queen = HotState::at_defaults();
        queen.hp = 60;
        for (uid, slot, kind) in [
            (0_u32, 0_i32, MonsterKind::Queen),
            (1, 1, MonsterKind::Toadpole),
        ] {
            let mut monster = HotMonster::new(kind, 30);
            monster.uid = uid;
            monster.slot = slot;
            queen.monsters_mut().push(monster);
        }
        let before = queen.clone();
        let mut events = Vec::new();
        assert_eq!(
            player_attack(&mut queen, &source, &[0, 1], 4, 1, &mut events),
            Err(EngineRefusal::MalformedArgs("Queen/Amalgam attack batch"))
        );
        assert_eq!(queen, before);
    }

    /// The phase-2 retained-object guard, driven directly: no admitted listener
    /// can replace a committed receiver before its own result dispatches (the
    /// Thorns guard and the synchronous wall refuse earlier), so the branch is
    /// exercised here rather than left unmeasured. Both arms are covered: a
    /// roster slot holding a different object, and a roster that no longer has
    /// the slot at all.
    ///
    /// The two phase-3 guards ("retained death replacement", "restored queued
    /// receiver") are the same fail-closed mirror of the #2651 unpowered
    /// guards and are DELIBERATELY left without a witness: reaching either
    /// needs a listener that revives or swaps an already-queued corpse between
    /// the result walk and `Kill`, which nothing admitted can do, and a forged
    /// result cannot stand in because a live roster entry never queues a kill.
    #[test]
    fn the_batch_retained_object_guards_refuse_a_replaced_receiver() {
        fn forged(uid: u32, hp_after: i32) -> FrozenMonsterDamage {
            FrozenMonsterDamage {
                target: 0,
                uid,
                kind: MonsterKind::Toadpole,
                hp_after,
                block_broken: false,
                blocked_int: 0,
                hp_lost_int: 1,
                powered: true,
                source_uid: None,
                dealer: AttackDealer::Player,
            }
        }
        let mut state = HotState::at_defaults();
        state.hp = 60;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.uid = 7;
        state.monsters_mut().push(monster);

        // Phase 2: the roster slot no longer holds the committed receiver.
        let mut replaced = state.clone();
        replaced.fanouts.enter_damage_batch();
        let depth = replaced.frames.len();
        assert_eq!(
            finish_powered_damage_batch(
                &mut replaced,
                None,
                depth,
                &[forged(9, 29)],
                &mut Vec::new(),
            ),
            Err(EngineRefusal::PowerOrderNotModeled(
                "attack Damage retained receiver replacement"
            ))
        );

        // Same guard, second arm: the roster no longer has the slot at all.
        let mut emptied = HotState::at_defaults();
        emptied.hp = 60;
        emptied.fanouts.enter_damage_batch();
        let depth = emptied.frames.len();
        assert_eq!(
            finish_powered_damage_batch(
                &mut emptied,
                None,
                depth,
                &[forged(7, 29)],
                &mut Vec::new(),
            ),
            Err(EngineRefusal::PowerOrderNotModeled(
                "attack Damage retained receiver replacement"
            ))
        );
    }

    /// Phrog Parasite's `InfestedPower.AfterDeath` Wrigglers roll
    /// `GetValueIfAscension(8, 18..22, 17..21)` at the fight's tier (#2539).
    #[test]
    fn spawned_wrigglers_roll_the_fights_band() {
        for (ascension, band) in [(7_u8, 17..=21), (8, 18..=22)] {
            let mut seen = std::collections::BTreeSet::new();
            for seed in 0..16 {
                let mut state = HotState::at_defaults();
                assert!(state.fanouts.set_ascension(ascension));
                state.rng.set(
                    RngStream::Niche,
                    crate::hot::RngStreamState {
                        words: crate::rng::Xoshiro256StarStar::from_seed(seed).words,
                        counter: 0,
                    },
                );
                let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, 0);
                phrog.max_hp = 66;
                state.monsters_mut().push(phrog);
                spawn_wrigglers(&mut state).unwrap();
                seen.extend(state.monsters[1..].iter().map(|monster| monster.max_hp));
            }
            assert!(
                seen.iter().all(|hp| band.contains(hp)),
                "A{ascension}: {seen:?}"
            );
            assert!(
                seen.contains(band.start()) && seen.contains(band.end()),
                "{seen:?}"
            );
        }
    }

    /// #3028, `PowerCmd.<Apply>d__2::MoveNext` RVA `0x3efbac`: an existing
    /// instance stacks through `ModifyAmount` (`IL_00a3`-`IL_0121`), so only a
    /// new Player-side Debuff instance sets `SkipNextDurationTick`
    /// (`IL_0429`-`IL_044c`). Stacking leaves the live latch untouched.
    #[test]
    fn player_duration_latch_is_set_only_for_a_new_instance() {
        let mut state = HotState::at_defaults();
        let mut events = Vec::new();
        for (power, fresh) in [
            (PowerId::PlayerVuln, PowerId::PlayerVulnFresh),
            (PowerId::PlayerWeak, PowerId::PlayerWeakFresh),
            (PowerId::PlayerFrail, PowerId::PlayerFrailFresh),
            (PowerId::Smoggy, PowerId::SmoggyFresh),
        ] {
            // A fresh application still skips its first tick.
            apply_player_duration_affliction(&mut state, power, fresh, 1, true, &mut events)
                .unwrap();
            assert_eq!(state.powers.value(power), 1);
            assert_eq!(state.powers.value(fresh), 1);
            // Stacking before the tick keeps the one live latch.
            apply_player_duration_affliction(&mut state, power, fresh, 1, true, &mut events)
                .unwrap();
            assert_eq!(state.powers.value(power), 2);
            assert_eq!(state.powers.value(fresh), 1);
            // After the side-end tick consumed the latch, stacking onto the
            // live instance does not re-arm it.
            state.powers.set(fresh, SlotWire::Bool, 0);
            apply_player_duration_affliction(&mut state, power, fresh, 1, true, &mut events)
                .unwrap();
            assert_eq!(state.powers.value(power), 3);
            assert_eq!(state.powers.value(fresh), 0);
            // `mark_fresh = false` stays latch-free on a new instance.
            let mut unlatched = HotState::at_defaults();
            apply_player_duration_affliction(&mut unlatched, power, fresh, 1, false, &mut events)
                .unwrap();
            assert_eq!(unlatched.powers.value(power), 1);
            assert_eq!(unlatched.powers.value(fresh), 0);
        }
    }

    /// #3028 end to end: Bowlbug Silk's Toxic Spit (Weak 1) repeated across
    /// enemy turns leaves the player at Weak 1 each turn, as the game does.
    /// The first application skips its tick through the new-instance latch
    /// (`PowerCmd.<TickDownDuration>d__5::MoveNext` RVA `0x3f0b18`
    /// `IL_001c`-`IL_002a`); the stacked ones tick down normally.
    #[test]
    fn repeated_enemy_weak_ticks_down_normally_after_the_first_turn() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::BowlbugSilk).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::BowlbugSilk, 40));
        let mut events = Vec::new();
        for _ in 0..3 {
            state.monsters_mut()[0].loop_pos = 0;
            crate::engine::turn::end_player_turn(&mut state, &catalog, &mut events).unwrap();
            assert_eq!(state.powers.value(PowerId::PlayerWeak), 1);
            assert_eq!(state.powers.value(PowerId::PlayerWeakFresh), 0);
        }
    }

    fn aoe3023_seeded_niche(state: &mut HotState, seed: u64) {
        state.rng.set(
            RngStream::Niche,
            crate::hot::RngStreamState {
                words: crate::rng::Xoshiro256StarStar::from_seed(seed).words,
                counter: 0,
            },
        );
    }

    fn aoe3023_phrog_state(phrog_hp: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        aoe3023_seeded_niche(&mut state, 11);
        let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, phrog_hp);
        phrog.max_hp = 66;
        state.monsters_mut().push(phrog);
        state
    }

    fn aoe3023_damage_targets(events: &[Event]) -> Vec<u32> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::MonsterDamaged { uid, .. } => Some(*uid),
                _ => None,
            })
            .collect()
    }

    /// #3023, `AttackCommand.<Execute>d__90::MoveNext` RVA `0x3f19c0`
    /// IL_0168-IL_0196: an all-opponents command re-reads `GetOpponentsOf`
    /// per hit. Whirlwind-shaped (6 x 3) against a 10-HP Phrog: hit 1 leaves 4,
    /// hit 2 kills it and Infested adds four Wrigglers, hit 3 lands on all four
    /// (the fc90b52173919819 shape). A `Targeting(creature)` command with the
    /// same numbers stays on the dead Phrog and never touches them.
    #[test]
    fn all_opponents_hits_re_resolve_to_wrigglers_spawned_mid_command() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();

        let mut state = aoe3023_phrog_state(10);
        let mut events = Vec::new();
        player_attack_all_from_card(&mut state, (&catalog, &spec, 99), 6, 3, &mut events).unwrap();
        assert!(state.monsters[0].hp <= 0);
        assert_eq!(state.monsters.len(), 5);
        assert!(!state.history.over);
        for wriggler in &state.monsters[1..] {
            assert_eq!(wriggler.kind, MonsterKind::Wriggler);
            assert_eq!(wriggler.hp, wriggler.max_hp - 6, "{wriggler:?}");
        }
        // Hit 3 is one batch over the four Wrigglers, in roster order.
        assert_eq!(aoe3023_damage_targets(&events), vec![0, 0, 1, 2, 3, 4]);

        let mut fixed = aoe3023_phrog_state(10);
        let mut events = Vec::new();
        player_attack_from_card(&mut fixed, (&catalog, &spec, 99), &[0], 6, 3, &mut events)
            .unwrap();
        assert_eq!(fixed.monsters.len(), 5);
        assert!(
            fixed.monsters[1..]
                .iter()
                .all(|wriggler| wriggler.hp == wriggler.max_hp)
        );
        assert_eq!(aoe3023_damage_targets(&events), vec![0, 0]);
    }

    /// #3023: once every opponent is dead and nothing spawned, the remaining
    /// hits do nothing. With the combat over, the loop's top-of-hit gate stops
    /// it; with the ending vetoed (a dead Test-Subject-shaped Adaptable owner),
    /// an empty `validTargets` in a live combat leaves the loop at IL_01a6-IL_01b3.
    #[test]
    fn all_opponents_hits_stop_once_no_opponent_lives() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        for uid in 0..2_u32 {
            let mut toadpole = HotMonster::new(MonsterKind::Toadpole, 3);
            toadpole.uid = uid;
            toadpole.slot = i32::try_from(uid).unwrap();
            state.monsters_mut().push(toadpole);
        }
        let mut events = Vec::new();
        player_attack_all_from_card(&mut state, (&catalog, &spec, 99), 5, 3, &mut events).unwrap();
        assert!(state.history.over);
        assert_eq!(aoe3023_damage_targets(&events), vec![0, 1]);

        let mut vetoed = HotState::at_defaults();
        vetoed.hp = 50;
        let mut corpse = HotMonster::new(MonsterKind::Toadpole, 0);
        corpse.powers.set(PowerId::Adaptable, SlotWire::Int, 1);
        vetoed.monsters_mut().push(corpse);
        let before_monsters = vetoed.monsters.clone();
        let mut events = Vec::new();
        player_attack_all_from_card(&mut vetoed, (&catalog, &spec, 99), 5, 3, &mut events).unwrap();
        assert!(!vetoed.history.over);
        assert_eq!(vetoed.monsters, before_monsters);
        assert!(aoe3023_damage_targets(&events).is_empty());
    }

    /// #3023, the other spawners: Gremlin Merc's Surprise pair and Axebot's
    /// Stock respawn join between hits and take the later hits.
    #[test]
    fn all_opponents_hits_reach_gremlin_merc_and_axebot_spawns() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();

        let mut merc_state = HotState::at_defaults();
        merc_state.hp = 70;
        merc_state.max_hp = 70;
        let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 4);
        merc.max_hp = 51;
        merc_state.monsters_mut().push(merc);
        aoe3023_seeded_niche(&mut merc_state, 5);
        let mut events = Vec::new();
        player_attack_all_from_card(&mut merc_state, (&catalog, &spec, 99), 5, 2, &mut events)
            .unwrap();
        assert_eq!(merc_state.monsters.len(), 3);
        assert!(merc_state.monsters[0].hp <= 0);
        for child in &merc_state.monsters[1..] {
            assert!(matches!(
                child.kind,
                MonsterKind::SneakyGremlin | MonsterKind::FatGremlin
            ));
            assert_eq!(child.hp, child.max_hp - 5, "{child:?}");
        }

        let mut axebot_state = HotState::at_defaults();
        axebot_state.hp = 50;
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 4);
        axebot.max_hp = 76;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        axebot_state.monsters_mut().push(axebot);
        let mut events = Vec::new();
        player_attack_all_from_card(&mut axebot_state, (&catalog, &spec, 99), 5, 2, &mut events)
            .unwrap();
        let replacement = &axebot_state.monsters[0];
        assert_eq!(replacement.uid, 1);
        assert_eq!(replacement.powers.value(PowerId::Stock), 1);
        assert_eq!(replacement.hp, replacement.max_hp - 5);
    }

    /// #2656: Gremlin Horn is a player relic and `InfestedPower` an enemy
    /// power, and `CombatState.<IterateHookListeners>d__69` (`0x3f9720`)
    /// lists `_allies` before `_enemies`, so the Horn's GainEnergy + Draw run
    /// before Infested adds the Wrigglers. A Hellraiser Strike the Horn draws
    /// is auto-played while no Wriggler exists, and the combat is not ending
    /// because the dead Phrog still owns Infested.
    #[test]
    fn horn_hellraiser_strike_runs_before_the_wrigglers_exist() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = aoe3023_phrog_state(5);
        state.fanouts.set_gremlin_horn_owned(true);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.next_card_uid = 2;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: 0,
        });
        let energy = state.energy;
        let mut events = Vec::new();
        player_attack_from_card(&mut state, (&catalog, &spec, 99), &[0], 10, 1, &mut events)
            .unwrap();
        assert_eq!(state.energy, energy + 1);
        assert!(state.piles.get(PileId::Draw).is_empty());
        // `CardCmd.AutoPlay` IL_0295-IL_02a8: no hittable enemy, so the drawn
        // Strike goes to its result pile without being played.
        assert!(state.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            state.piles.get(PileId::Discard).as_slice(),
            &[HotCard {
                uid: 1,
                atom,
                flags: 0
            }]
        );
        assert_eq!(state.monsters.len(), 5);
        assert!(
            state.monsters[1..]
                .iter()
                .all(|wriggler| wriggler.kind == MonsterKind::Wriggler
                    && wriggler.hp == wriggler.max_hp),
            "the auto-played Strike found no Wriggler: {:?}",
            state.monsters
        );
        assert_eq!(aoe3023_damage_targets(&events), vec![0]);
        assert!(!state.history.over);
    }

    /// #2656 lifts the Phrog/Horn batch wall. A synthetic peer shares the
    /// batch: at the Toadpole's death the Phrog's HP is already committed to 0
    /// but it still owns Infested, so the combat is not ending and the Horn
    /// pays out (the former `monsters.len() == 1` proxy dropped this veto).
    /// The Phrog's own death pays out again, and only then do the Wrigglers
    /// join; after they exist the retained dead Phrog row no longer vetoes.
    #[test]
    fn horn_batch_honors_pending_infested_veto_then_spawns_last() {
        let (catalog, atom) = source_catalog(CardId::StrikeIronclad);
        let spec = *catalog.spec(atom).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 60;
        state.fanouts.set_gremlin_horn_owned(true);
        aoe3023_seeded_niche(&mut state, 3);
        for (uid, slot, kind) in [
            (0_u32, 0_i32, MonsterKind::Toadpole),
            (1, 1, MonsterKind::PhrogParasite),
        ] {
            let mut monster = HotMonster::new(kind, 4);
            monster.max_hp = 30;
            monster.uid = uid;
            monster.slot = slot;
            state.monsters_mut().push(monster);
        }
        state.next_card_uid = 3;
        for uid in 1..=2_u32 {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid,
                atom,
                flags: 0,
            });
        }
        let energy = state.energy;
        let mut events = Vec::new();
        player_attack_all_from_card(&mut state, (&catalog, &spec, 99), 4, 1, &mut events).unwrap();
        assert_eq!(state.energy, energy + 2);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        assert_eq!(state.monsters.len(), 6);
        assert!(!state.history.over);
        assert!(!infested_is_pending(&state));

        let mut done = state.clone();
        for wriggler in done.monsters_mut()[2..].iter_mut() {
            wriggler.hp = 0;
        }
        assert!(damage_combat_is_ending(&done));
    }
}

/// #3044: Red Skull's +3 is a real `StrengthPower`, written at the player's
/// HP transitions (`red_skull_after_player_hp_changed`).
#[cfg(test)]
mod red_skull_tests {
    use super::*;

    fn skull(hp: i32, max_hp: i32, strength: i32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = hp;
        state.max_hp = max_hp;
        state.fanouts.set_red_skull_owned(true);
        state.powers.set(PowerId::Strength, SlotWire::Int, strength);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 100));
        state
    }

    fn strength(state: &HotState) -> i32 {
        state.powers.value(PowerId::Strength)
    }

    /// `<ModifyStrengthIfNecessary>d__14::MoveNext` `0x32f790` IL_0033-IL_006f:
    /// "above" is the decimal `CurrentHp > MaxHp * 0.5`, so exactly half holds.
    #[test]
    fn the_threshold_holds_at_exactly_half_and_rounds_nothing() {
        assert!(red_skull_threshold_met(40, 80));
        assert!(!red_skull_threshold_met(41, 80));
        assert!(red_skull_threshold_met(40, 81));
        assert!(!red_skull_threshold_met(41, 81));
        assert!(red_skull_threshold_met(0, 1));
        assert!(!red_skull_threshold_met(1, 1));
        assert!(red_skull_threshold_met(-5, 10));
    }

    #[test]
    fn crossing_down_applies_three_and_crossing_up_removes_it() {
        let mut state = skull(41, 80, 4);
        let mut events = Vec::new();
        state.hp = 40;
        red_skull_after_player_hp_changed(&mut state, 41, 80, &mut events).unwrap();
        assert_eq!(strength(&state), 7);
        // Staying at or below: no second application.
        state.hp = 10;
        red_skull_after_player_hp_changed(&mut state, 40, 80, &mut events).unwrap();
        assert_eq!(strength(&state), 7);
        state.hp = 41;
        red_skull_after_player_hp_changed(&mut state, 10, 80, &mut events).unwrap();
        assert_eq!(strength(&state), 4);
        // Staying above: nothing.
        state.hp = 80;
        red_skull_after_player_hp_changed(&mut state, 41, 80, &mut events).unwrap();
        assert_eq!(strength(&state), 4);
        assert_eq!(
            events,
            vec![
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: 7,
                },
                Event::PowerChanged {
                    subject: Subject::Player,
                    power: PowerId::Strength,
                    amount: 4,
                },
            ]
        );
    }

    #[test]
    fn an_unowned_skull_a_dead_owner_and_an_ended_combat_write_nothing() {
        let mut unowned = skull(40, 80, 0);
        unowned.fanouts.set_red_skull_owned(false);
        red_skull_after_player_hp_changed(&mut unowned, 60, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&unowned), 0);

        let mut dead = skull(0, 80, 0);
        dead.fanouts.set_player_hooks_deactivated(true);
        red_skull_after_player_hp_changed(&mut dead, 60, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&dead), 0);

        // `PowerCmd.Apply`'s ending gate (`apply_owner_strength`).
        let mut over = skull(40, 80, 0);
        over.history.over = true;
        red_skull_after_player_hp_changed(&mut over, 60, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&over), 0);
    }

    /// The application is an ordinary owner `Apply<StrengthPower>`, so Ruined
    /// Helmet doubles the first positive one; the removal is -3.
    #[test]
    fn ruined_helmet_doubles_the_first_application_and_not_the_removal() {
        let mut state = skull(60, 80, 0);
        state
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, false, true);
        state.hp = 30;
        red_skull_after_player_hp_changed(&mut state, 60, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&state), 6);
        assert!(state.fanouts.ruined_helmet_used());
        state.hp = 60;
        red_skull_after_player_hp_changed(&mut state, 30, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&state), 3);
        state.hp = 30;
        red_skull_after_player_hp_changed(&mut state, 60, 80, &mut Vec::new()).unwrap();
        assert_eq!(strength(&state), 6, "the Helmet latched on the first");
    }

    /// The opening's stale latch (Red Skull before Planisphere's crossing
    /// heal) flips the derived answer once, then clears.
    #[test]
    fn a_stale_latch_is_consumed_by_the_next_player_hp_write() {
        let mut state = skull(47, 87, 3);
        state.fanouts.set_red_skull_latch_stale(true);
        state.hp = 49;
        red_skull_after_player_hp_changed(&mut state, 47, 87, &mut Vec::new()).unwrap();
        assert_eq!(strength(&state), 0);
        assert!(!state.fanouts.red_skull_latch_stale());
        state.hp = 50;
        red_skull_after_player_hp_changed(&mut state, 49, 87, &mut Vec::new()).unwrap();
        assert_eq!(strength(&state), 0);
    }

    /// The `commit_player_hp_loss` site, which every Damage against the
    /// player shares: the census witness's shape (Strength 4, 28/80).
    #[test]
    fn a_damage_result_that_crosses_half_applies_the_strength() {
        let mut state = skull(50, 80, 4);
        damage_player_from_card(&mut state, 22, false, &mut Vec::new()).unwrap();
        assert_eq!(state.hp, 28);
        assert_eq!(strength(&state), 7);
        // A Buffered result loses no HP and raises no AfterCurrentHpChanged.
        let mut buffered = skull(50, 80, 4);
        buffered.powers.set(PowerId::Buffer, SlotWire::Int, 1);
        damage_player_from_card(&mut buffered, 22, false, &mut Vec::new()).unwrap();
        assert_eq!((buffered.hp, strength(&buffered)), (50, 4));
    }

    /// `Kill` `0x3ebe90` raises `AfterCurrentHpChanged` (IL_0200) before
    /// `ShouldDie`, so both kill paths apply the Strength first.
    /// A monster's hit is one Damage command whose result raises
    /// `AfterCurrentHpChanged` (`0x3e96c8` IL_0be4); The Gambit's Kill after
    /// a surviving hit raises its own (`0x3ebe90` IL_0200).
    #[test]
    fn a_monster_hit_and_the_gambits_kill_both_run_red_skull() {
        let mut hit = skull(45, 80, 0);
        monster_attack_player(&mut hit, 0, 10, 1, &mut Vec::new()).unwrap();
        assert_eq!((hit.hp, strength(&hit)), (35, 3));

        let mut gambit = skull(80, 80, 0);
        gambit.powers.set(PowerId::TheGambit, SlotWire::Int, 1);
        monster_attack_player(&mut gambit, 0, 5, 1, &mut Vec::new()).unwrap();
        assert!(gambit.history.over);
        assert_eq!((gambit.hp, strength(&gambit)), (0, 3));
    }

    #[test]
    fn doom_and_force_kills_apply_the_strength_before_the_death() {
        let mut doomed = skull(60, 80, 0);
        doom_kill_player(&mut doomed, None, &mut Vec::new()).unwrap();
        assert_eq!(strength(&doomed), 3);
        assert!(doomed.history.over);

        let mut forced = skull(60, 80, 0);
        force_kill_player(&mut forced, &mut Vec::new()).unwrap();
        assert_eq!(strength(&forced), 3);
        assert!(forced.history.over);
    }

    #[test]
    fn demon_tongues_heal_removes_the_strength_it_lifts_past() {
        let mut state = skull(38, 80, 3);
        state.set_batch_six_deep_relic_ownership(true, false, false, false);
        state.player_side_active = true;
        after_owner_damage_received_relics(&mut state, None, 5, &mut Vec::new()).unwrap();
        assert_eq!(state.hp, 43);
        assert_eq!(strength(&state), 0);
    }

    /// Lizard Tail heals to half max HP, which leaves the threshold only at
    /// max HP 1.
    #[test]
    fn lizard_tails_revive_is_a_heal_red_skull_hears() {
        let mut state = skull(0, 1, 3);
        state
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, true, false);
        assert!(resolve_player_lethal(&mut state, &mut Vec::new()).unwrap());
        assert_eq!(state.hp, 1);
        assert_eq!(strength(&state), 0);
        let mut ordinary = skull(0, 80, 3);
        ordinary
            .fanouts
            .set_batch_eight_deep_relic_ownership(false, false, false, true, false);
        assert!(resolve_player_lethal(&mut ordinary, &mut Vec::new()).unwrap());
        assert_eq!((ordinary.hp, strength(&ordinary)), (40, 3));
    }

    /// `SetMaxHp`'s clamp raises no `AfterCurrentHpChanged`, so a max-HP loss
    /// that lifts the owner above half would leave native's latch stale:
    /// refused by name. One that does not cross runs.
    #[test]
    fn a_max_hp_loss_that_crosses_refuses_and_one_that_does_not_runs() {
        let mut crossing = skull(40, 80, 3);
        assert_eq!(
            lose_player_max_hp_from_card(&mut crossing, None, 5, &mut Vec::new()),
            Err(EngineRefusal::MalformedArgs(
                "Red Skull Strength held across a max-HP loss"
            ))
        );
        let mut clear = skull(30, 80, 3);
        lose_player_max_hp_from_card(&mut clear, None, 5, &mut Vec::new()).unwrap();
        assert_eq!((clear.max_hp, strength(&clear)), (75, 3));
        let mut unowned = skull(40, 80, 0);
        unowned.fanouts.set_red_skull_owned(false);
        lose_player_max_hp_from_card(&mut unowned, None, 5, &mut Vec::new()).unwrap();
        assert_eq!(unowned.max_hp, 75);
    }
}

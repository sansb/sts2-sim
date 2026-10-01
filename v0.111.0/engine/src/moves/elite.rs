//! Monster-move bodies for the `content/encounters/elite.py` pool — GENERATED ONCE, THEN OWNED BY HAND.
//!
//! PORT_PLAN.md D4. `sim/v0.111.0/engine/tools/generate_content.py`
//! created this file with a refusing stub per kind and will APPEND a stub for
//! any kind that later joins this family, but it never rewrites, reorders, or
//! removes what is already here: the bodies are hand-written ports and this
//! file is the wave PR's private edit surface.
//!
//! Filling a stub is a three-line contract:
//!
//! 1. replace the `Err(...)` body with the port of the cited Python branch;
//! 2. add the kind to [`IMPLEMENTED`] — the capability manifest and the
//!    admission gate are both derived from it (D6), so an unlisted body is
//!    unreachable. The source-derived family-triage gate also rejects a listed
//!    body that directly names its own `*KindNotModeled` refusal; focused crate
//!    tests remain the runtime evidence;
//! 3. leave the signature alone; [`super`]'s generated `match` calls it.
//!
//! # R47 status (#2035): all 7 move kinds ported
//!
//! R2 adds Mecha's exact attack-then-Burn-to-hand command shape and the full
//! Entomancer/Hive writer plus surviving powered-hit Dazed suffix. Native
//! `PersonalHivePower/<AfterDamageReceived>d__6::MoveNext` (current v0.111.0
//! RVA 0x340220) gates on a powered result at IL_004d, then dispatches one
//! CreateCard<Dazed> IL_00af / AddGeneratedCardToCombat IL_00d1 transaction
//! per stack. R42 adds Hex's exact all-card affliction and death cleanup;
//! R47 adds Dampen's one-caster downgrade/restore snapshot behind the same
//! owner-disjoint cold card-state handle.

use super::MoveCtx;
use crate::catalog::{CardIdentity, CompiledArg};
use crate::content_tables::move_constants as mc;
use crate::engine::cards::inject_legacy_bottom;
use crate::engine::damage::{monster_attack_player_with_catalog, note_power};
use crate::engine::{EngineRefusal, Subject};
use crate::hot::PileId;
use crate::ids::{CardId, MoveKind, PowerId};
use crate::powers::SlotWire;

/// The MoveKinds this family implements.
///
/// Add a kind here in the same diff that fills its body — this
/// slice is the manifest's and the admission gate's only source of
/// truth for what this family can do (D6).
///
/// The Phrog move plus issue #1374's Eel/Prism power moves.
pub const IMPLEMENTED: &[MoveKind] = &[
    MoveKind::AddStatus,
    MoveKind::AttackBlockVital,
    MoveKind::AttackPileInject,
    MoveKind::AttackVigor,
    MoveKind::DampenPlayer,
    MoveKind::EntoSpit,
    MoveKind::HexPlayer,
];

/// Every kind this file owns a body for, ascending — the list the escalation
/// tests walk. `super::FAMILY_OF` is the generated authority; this mirrors the
/// `elite` rows of it, and a test below pins the two against each other so a
/// kind appended by the generator cannot slip past the interlock.
#[cfg(test)]
const OWNED: [MoveKind; 7] = [
    MoveKind::AddStatus,
    MoveKind::AttackBlockVital,
    MoveKind::AttackPileInject,
    MoveKind::AttackVigor,
    MoveKind::DampenPlayer,
    MoveKind::EntoSpit,
    MoveKind::HexPlayer,
];

/// `("add_status", count)` — Phrog Parasite INFECT.
///
/// Python: `monster_act` (frozen, deleted #2827).
/// `_pile_inject(s, ("INFECTION", 0), args[0], "discard", "bottom", log)` —
/// `AddToCombatAndPreview<Infection>(…, CardPilePosition.Bottom)` appends
/// `count` Infections to the discard end, with no Shuffle draw.
///
/// Native `_pile_inject` first appends compact legacy tuples. The next
/// identity-sensitive turn-start pass allocates their physical uids in
/// all-piles order, so this body deliberately does not advance
/// `next_card_uid` itself.
pub(crate) fn add_status(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(count)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("add_status"));
    };
    let count =
        usize::try_from(*count).map_err(|_| EngineRefusal::CounterOverflow("add_status count"))?;
    inject_legacy_bottom(
        ctx.state,
        ctx.catalog,
        CardIdentity {
            id: CardId::Infection,
            upgrade: 0,
            enchantment: None,
        },
        count,
        PileId::Discard,
    )
}

/// `("attack_block_vital", damage, hits, block, vital)` — Infested Prism
/// PULSATE `(10, 1, 22, 3)`.
///
/// Python: `monster_act` (frozen, deleted #2827), and the resumed body
/// `_finish_monster_move_after_attack`. Attack, then — only while
/// `not s.over and m.hp > 0` — `m.block += args[2]` and `m.vital += args[3]`.
/// Vital's completed-Skill listener and the Tainted attack term live in the
/// play and damage pipelines; this suffix only writes the actor's new Block
/// and Vital amounts after the attack completes while combat remains live.
pub(crate) fn attack_block_vital(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(block),
        CompiledArg::I(vital),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_block_vital"));
    };
    if *damage < 0 || *hits < 1 || *block < 0 || *vital < 0 {
        return Err(EngineRefusal::MalformedArgs("attack_block_vital"));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let block: i32 = (*block)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("attack_block_vital block"))?;
        let vital: i32 = (*vital)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("attack_block_vital vital"))?;
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let updated_block = monster
            .block
            .checked_add(block)
            .ok_or(EngineRefusal::CounterOverflow("monster block"))?;
        let updated_vital = monster
            .powers
            .value(PowerId::Vital)
            .checked_add(vital)
            .ok_or(EngineRefusal::CounterOverflow("vital"))?;
        let uid = monster.uid;
        monster.block = updated_block;
        monster
            .powers
            .set(PowerId::Vital, SlotWire::Int, updated_vital);
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Vital,
            updated_vital,
        );
    }
    Ok(())
}

/// `("attack_pile_inject", damage, hits, card, count, pile, position)` — Mecha
/// Knight FLAMETHROWER `(damage, 1, ("BURN", 0), 4, "hand", "bottom")`, the
/// damage `MechaKnight::get_FlamethrowerDamage` RVA `0xb98ec`,
/// `GetValueIfAscension(9, 12, 8)` at the fight's tier (#2828).
///
/// Python: `monster_act` (frozen, deleted #2827), resumed at
/// `_finish_monster_move_after_attack` with the same admitted-shape
/// re-check that `_advance_monster_move_frame` performs first. Attack,
/// then `_pile_inject` four Burns to the hand end.
///
/// Authenticate the six compiled operands, await the attack, then append four
/// compact Burns through the exact per-card Hand/Bottom overflow rule while
/// combat and owner remain live. Burn keywords, Artifact, and the remaining
/// Mecha Knight loop controls remain independent entry refusals.
pub(crate) fn attack_pile_inject(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::List(identity),
        CompiledArg::I(count),
        CompiledArg::Pile(PileId::Hand),
        CompiledArg::Word(crate::ids::StepWord::Bottom),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_pile_inject"));
    };
    if *damage < 0 || *hits < 1 {
        return Err(EngineRefusal::MalformedArgs("attack_pile_inject"));
    }
    let count = usize::try_from(*count)
        .map_err(|_| EngineRefusal::CounterOverflow("attack_pile_inject count"))?;
    let [CompiledArg::Card(id), CompiledArg::I(upgrade)] = ctx.catalog.args(*identity) else {
        return Err(EngineRefusal::MalformedArgs("attack_pile_inject identity"));
    };
    let upgrade = u8::try_from(*upgrade)
        .map_err(|_| EngineRefusal::MalformedArgs("attack_pile_inject upgrade"))?;
    if ctx.state.monsters[ctx.actor].kind != crate::ids::MonsterKind::MechaKnight
        || (*damage, *hits, *id, upgrade, count)
            != (
                ctx.tier(mc::MECHA_KNIGHT_FLAMETHROWER_DAMAGE),
                1,
                CardId::Burn,
                0,
                4,
            )
    {
        return Err(EngineRefusal::MalformedArgs("attack_pile_inject owner/row"));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        inject_legacy_bottom(
            ctx.state,
            ctx.catalog,
            CardIdentity {
                id: *id,
                upgrade,
                enchantment: None,
            },
            count,
            PileId::Hand,
        )?;
    }
    Ok(())
}

/// `("attack_vigor", damage, hits, vigor)` — Terror Eel THRASH `(4, 3, 6)`.
///
/// Python: `monster_act` (frozen, deleted #2827), resumed at
/// `_finish_monster_move_after_attack`. Attack, then `m.vigor +=
/// args[2]` while the actor is live.
/// `monster_attack_player_with_catalog` consumes any pre-existing Vigor only when its
/// command tail is reached. This suffix then installs the fresh amount for the
/// Eel's next attack, provided the actor and combat are still live.
pub(crate) fn attack_vigor(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [
        CompiledArg::I(damage),
        CompiledArg::I(hits),
        CompiledArg::I(vigor),
    ] = ctx.args
    else {
        return Err(EngineRefusal::MalformedArgs("attack_vigor"));
    };
    if *damage < 0 || *hits < 1 || *vigor < 0 {
        return Err(EngineRefusal::MalformedArgs("attack_vigor"));
    }
    monster_attack_player_with_catalog(
        ctx.state,
        ctx.catalog,
        ctx.actor,
        *damage,
        *hits,
        ctx.events,
    )?;
    if !ctx.state.history.over && ctx.state.monsters[ctx.actor].hp > 0 {
        let vigor: i32 = (*vigor)
            .try_into()
            .map_err(|_| EngineRefusal::CounterOverflow("attack_vigor vigor"))?;
        let monster = &mut ctx.state.monsters_mut()[ctx.actor];
        let updated = monster
            .powers
            .value(PowerId::Vigor)
            .checked_add(vigor)
            .ok_or(EngineRefusal::CounterOverflow("vigor"))?;
        let uid = monster.uid;
        monster.powers.set(PowerId::Vigor, SlotWire::Int, updated);
        note_power(ctx.events, Subject::Monster(uid), PowerId::Vigor, updated);
    }
    Ok(())
}

/// `("dampen_player",)` — Magi Knight DAMPEN.
///
/// Current-build authority: v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
/// Magi `DampenMove`/body are `0xb93a4`/`0x362ef0`; DampenPower
/// Type/Stack/Init/AfterApplied/AfterDeath wrapper+body/AfterRemoved/AddCaster
/// are `0xa118b/0xa118e/0xa1191/0xa1198/0xa123c/0x33871c/0xa1290/0xa1310`.
/// `AllCards`
/// (`0x117e3c`) traverses Hand, Draw, Discard, Exhaust, Play once after the
/// caster is added; card Downgrade/Upgrade commands are `0x12f588/0x12f660`
/// and `DowngradeInternal` is `0x7e12c`.
///
/// Python: `monster_act` (frozen, deleted #2827) → `_apply_dampen_power`. The body is an
/// I5-guarded call: the caster must be the live, exactly-once Magi Knight.
/// The power publishes
/// the caster uid set and uid/upgrade rows, after rebuilding
/// identities through `_normalize_card_identities`,
/// plus the power's caster-death lifecycle (`_dampen_after_actual_death`). The first exact caster snapshots each L1 physical uid in native
/// pile order and swaps only its atom to L0; it
/// *downgrades* them: `_card_downgrade_to_zero` over every upgraded
/// card in every pile. Reapplication is idempotent, and death restores those
/// exact identities before relic/card listeners, clearing the cold state only
/// after the complete restore succeeds.
///
/// Pre-R47 escalation citation map (historical statements superseded by the
/// implemented current-build behavior above):
///
/// * **DampenPower's shared per-card state** — `_apply_dampen_power` writes
///   the caster uid set and uid/upgrade rows (frozen Python, deleted #2827), after rebuilding
///   identities through `_normalize_card_identities`,
///   plus the power's caster-death lifecycle (`_dampen_after_actual_death`). Nothing in `hot.rs` carries a caster set or cross-pile upgrade
///   snapshot was the superseded pre-R47 conclusion; the cold state now does.
/// * **card identity rewriting.** The apply does not mark cards, it
///   *downgrades* them: `_card_downgrade_to_zero` over every upgraded
///   card in every pile; R47 now carries the exact derived counterpart atoms.
pub(crate) fn dampen_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let exact_entry = crate::engine::cards::dampen_application_is_exact(ctx.state, ctx.catalog)
        || crate::engine::cards::dampen_state_is_exact(ctx.state, ctx.catalog);
    if !ctx.args.is_empty() || ctx.actor != 2 || !exact_entry {
        return Err(EngineRefusal::MalformedArgs("Knights Dampen state"));
    }
    let mut probe = ctx.state.clone();
    crate::engine::cards::apply_dampen_power(&mut probe, ctx.catalog, 2)?;
    crate::engine::cards::apply_dampen_power(ctx.state, ctx.catalog, 2)
}

/// `("ento_spit",)` — Entomancer PHEROMONE_SPIT.
///
/// Python: `monster_act` (frozen, deleted #2827). `if m.hive < HIVE_CAP (3) { m.hive += 1;
/// m.strength += 1 } else { m.strength += 2 }`. No attack, no draw.
///
/// [`ento_spit_exact`] pins the complete command-local writer:
/// below Hive 3 it increments Hive and Strength by one, while at the cap it
/// leaves Hive unchanged and grants Strength two.
///
/// * **Hive's consumer, which is itself card creation.** The counter is not a
///   scoreboard: `PersonalHivePower.AfterDamageReceived` runs `_insert_dazed`
///   for `m.hive` in `player_attack` (frozen Python, deleted #2827), and again from
///   `_advance_player_attack_frame`,
///   on every powered damage instance the Entomancer survives, fully blocked
///   ones included. `_insert_dazed` inserts that many Dazed into
///   the **draw** pile at random positions, taking one Shuffle draw per card.
///   The complete suffix is clone-rehearsed with the attack before publication,
///   so a later generated listener failure cannot expose a partial hit or card
///   batch.
///   Owner-created history does not advance for the null creator.
///
/// (The batch-1 note that Hive "spawns Wrigglers" was wrong. The Entomancer
/// builder creates only the Entomancer; InfestedPower spawns four Wrigglers
/// when the *Phrog Parasite* dies. Hive's actual consumer is random Dazed
/// insertion after surviving player-powered damage.)
pub(crate) fn ento_spit(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    ento_spit_exact(ctx)
}

/// The exact PHEROMONE_SPIT writer.
///
/// Python: `monster_act` (frozen, deleted #2827). The fixed Entomancer loop reaches this
/// body at position two with one integer Hive slot. Native performs both
/// arithmetic operations before yielding another command, so Rust preflights
/// the complete Hive/Strength pair before publishing either update.
fn ento_spit_exact(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    if !ctx.args.is_empty() {
        return Err(EngineRefusal::MalformedArgs("ento_spit"));
    }
    let Some(owner) = ctx.state.monsters.get(ctx.actor) else {
        return Err(EngineRefusal::MalformedArgs("ento_spit actor"));
    };
    let Some(hive) = owner.powers.get(PowerId::Hive) else {
        return Err(EngineRefusal::MalformedArgs("ento_spit Hive"));
    };
    let strength = owner.powers.get(PowerId::Strength);
    if ctx.state.history.over
        || ctx.state.monsters.len() != 1
        || ctx.actor != 0
        || owner.kind != crate::ids::MonsterKind::Entomancer
        || owner.slot != 0
        || owner.uid != 0
        || owner.hp <= 0
        || Some(owner.max_hp)
            != crate::engine::monsters::native_fixed_hp(
                ctx.state,
                crate::ids::MonsterKind::Entomancer,
            )
        || owner.loop_pos != 2
        || hive.wire != SlotWire::Int
        || !(1..=3).contains(&hive.value)
        || strength.is_some_and(|slot| slot.wire != SlotWire::Int)
    {
        return Err(EngineRefusal::MalformedArgs("ento_spit owner/state"));
    }

    let hive_gain = i32::from(hive.value < 3);
    let strength_gain = if hive_gain == 1 { 1 } else { 2 };
    let updated_hive = hive
        .value
        .checked_add(hive_gain)
        .ok_or(EngineRefusal::CounterOverflow("ento_spit Hive"))?;
    let updated_strength = crate::engine::damage::checked_monster_strength_successor(
        owner,
        strength_gain,
        "ento_spit Strength",
    )?;
    let uid = owner.uid;

    let upkeep = ctx.state.fanouts.misery_attachment_upkeep();
    let owner = &mut ctx.state.monsters_mut()[ctx.actor];
    if hive_gain == 1 {
        owner.powers.set(PowerId::Hive, SlotWire::Int, updated_hive);
        note_power(
            ctx.events,
            Subject::Monster(uid),
            PowerId::Hive,
            updated_hive,
        );
    }
    crate::engine::damage::write_monster_self_strength(owner, updated_strength, upkeep);
    note_power(
        ctx.events,
        Subject::Monster(uid),
        PowerId::Strength,
        updated_strength,
    );
    Ok(())
}

/// `("hex_player", amount)` — Spectral Knight HEX `(2,)`.
///
/// Current-build authority: v0.111.0 `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`;
/// `HexMove`/`MoveNext` are `0xbf6d8`/`0x36c53c`; Hex apply, entry, death,
/// and nested death cleanup are `0xa3755/0xa37c8/0xa3814`/`0x33c6b0`.
///
/// Python: `monster_act` (frozen, deleted #2827) → `_apply_hex_power`. Stacks
/// `s.hex_power`, then afflicts **every eligible physical card in combat**
/// with `CARD_AFFLICTION_HEXED` at the new amount, refusing loudly if another
/// card-affliction power (Chains of Binding, Ringing) overlaps.
///
/// The shared exact closure is now implemented:
///
/// * **the card-affliction layer** — `_map_combat_cards` (frozen Python, deleted #2827) run with the
///   exact-commit flag, `card_afflict` / `card_clear_affliction`, and
/// * **the player-side power transaction.** `_apply_hex_power` writes
///   `_apply_hex_power` publishes `s.hex_power`, mirrored by a compact Rust slot.
///   Rust has a compact `PowerId`; apply/remove remain coupled to the all-piles
///   rewrite plus `_validate_card_affliction_power_overlap`.
///   is also read back by `physical_card_after_entered_combat` and
///   `card_effective_keywords`, so the slot alone would not be enough.
///
/// Production execution authenticates the live Knights roster and opener;
/// the separate smoke API still requires its exact fresh fixture.
pub(crate) fn hex_player(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    let [CompiledArg::I(2)] = ctx.args else {
        return Err(EngineRefusal::MalformedArgs("hex_player"));
    };
    if ctx.actor != 1 || !crate::engine::cards::spectral_hex_application_is_exact(ctx.state) {
        return Err(EngineRefusal::MalformedArgs("Knights Hex state"));
    }
    let mut probe = ctx.state.clone();
    crate::engine::cards::apply_hex_power(&mut probe, 2)?;
    crate::engine::cards::apply_hex_power(ctx.state, 2)?;
    note_power(ctx.events, Subject::Player, PowerId::HexPower, 2);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CatalogBuilder, CompiledMove};
    use crate::engine::admission::{AdmissionRefusal, MissingCapability, admit};
    use crate::ids::MonsterKind;
    use serde_json::Value;

    /// The R0.5 entry document every test here starts from.
    const FIXTURE: &str = include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    /// Admit the fixture with its single monster swapped for `kind`.
    ///
    /// The measurement the doc comments above quote. It is the entry-level
    /// question — what an operator holding that encounter would actually read
    /// — rather than a peek at one internal predicate, which is why the
    /// escalation record can be checked with it instead of narrated.
    fn admit_with_monster(kind: MonsterKind) -> Result<(), AdmissionRefusal> {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.monsters[0].insert("kind".to_owned(), Value::from(kind.as_str()));
        if kind == MonsterKind::SpectralKnight {
            document.monsters[0].remove("loop_pos");
            document.monsters[0].insert("next_move".to_owned(), Value::from("HEX_MOVE"));
            document.monsters[0].insert("move_log".to_owned(), serde_json::json!(["HEX_MOVE"]));
        }
        if kind == MonsterKind::Entomancer {
            document.monsters.truncate(1);
            document.monsters[0].insert("hp".to_owned(), Value::from(165));
            document.monsters[0].insert("max_hp".to_owned(), Value::from(165));
            document.monsters[0].insert(PowerId::Hive.as_str().to_owned(), Value::from(1));
        }
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog)
    }

    /// Every compiled loop row in the build that uses one of this family's
    /// kinds, with the monster that carries it.
    fn owned_rows() -> Vec<(MonsterKind, CompiledMove)> {
        let mut builder = CatalogBuilder::new();
        for kind in MonsterKind::ALL {
            builder.intern_monster(kind).unwrap();
        }
        let catalog = builder.build();
        let mut rows = Vec::new();
        for monster in MonsterKind::ALL {
            for entry in catalog.moves(monster) {
                if OWNED.contains(&entry.kind) {
                    rows.push((monster, *entry));
                }
            }
        }
        // Every kind this file owns is reachable from some monster's move
        // table, or the generator's family split and the content tables
        // disagree. `hex_player` is reachable only from Spectral Knight's
        // random-AI table, which the catalog does not compile and the
        // admission gate never reaches (it refuses on `MonsterAi` first) —
        // which is itself part of that kind's escalation.
        for kind in OWNED {
            let in_loop = rows.iter().any(|(_, entry)| entry.kind == kind);
            let in_random_ai = MonsterKind::ALL.iter().any(|monster| {
                crate::content_tables::random_moves(*monster)
                    .is_some_and(|table| table.iter().any(|entry| entry.kind == kind))
            });
            assert!(
                in_loop || in_random_ai,
                "no monster move table uses {:?}",
                kind.as_str()
            );
        }
        rows
    }

    /// [`OWNED`] is the walk order for the escalation tests below, so it
    /// has to be ascending and duplicate-free for them to be total over it —
    /// and nothing may be claimed in [`IMPLEMENTED`] that this file does not
    /// own a body for.
    ///
    /// (The family split itself is the generator's: `super::FAMILY_OF` assigns
    /// each kind to a module, and CI re-runs `generate_content.py --check`, so
    /// a kind appended to this file later arrives as a visible diff here.
    /// Cross-checking the two by name would mean comparing text in a dispatch
    /// module, which D2 bans and `tests/hot_path_contract.rs` enforces.)
    #[test]
    fn the_owned_list_is_ascending_and_covers_every_claim() {
        assert!(OWNED.windows(2).all(|pair| pair[0] < pair[1]));
        for kind in IMPLEMENTED {
            assert!(
                OWNED.contains(kind),
                "elite claims {:?}, which is not one of its own stubs",
                kind.as_str()
            );
        }
    }

    /// The batch-1 interlock, rewritten for the post-#1366 gate.
    ///
    /// Batch 1 could not claim a kind because the gate held a central shape
    /// opinion on every implemented move kind, so a claim traded a `MoveKind`
    /// refusal for an `ArgumentShape` one. #1366 made wave kinds' shapes
    /// body-owned, and this is the property that replaced it: a kind this
    /// family claims must never make its own monster's entry refuse for the
    /// *shape* of the row that kind carries — if it does, the claim admits
    /// nothing and only changes which refusal an operator reads.
    ///
    /// Checking the old way — asking `move_args` directly — would now assert
    /// the opposite of the truth: that function still answers a refusal for
    /// every kind outside the R0.5 three, by design, because it is no longer
    /// the gate's opinion for anything else. The entry-level question is the
    /// honest one.
    ///
    /// The guard trips whenever a filled body and its row disagree.
    #[test]
    fn a_claimed_kind_is_never_refused_for_its_argument_shape() {
        for (monster, entry) in owned_rows() {
            if !IMPLEMENTED.contains(&entry.kind) {
                continue;
            }
            // Dampen's zero-argument row is executable only in the exact
            // three-Knight turn-two foundation. The generic singleton helper
            // intentionally refuses that encounter topology, so pin the row
            // shape here while the public smoke exercises its full entry.
            if entry.kind == MoveKind::DampenPlayer {
                assert!(entry.args.is_empty());
                continue;
            }
            let Err(refusal) = admit_with_monster(monster) else {
                continue;
            };
            assert!(
                !refusal
                    .missing()
                    .any(|item| matches!(item, MissingCapability::ArgumentShape(_))),
                "elite claims {:?}, but {:?}'s entry refuses on an argument \
                 shape ({refusal}), so every entry carrying it refuses instead \
                 of playing",
                entry.kind.as_str(),
                monster.as_str()
            );
        }
    }

    /// The escalation record, measured rather than narrated (#1347 batch 1b).
    ///
    /// Each stub above says the gate names its kind on the monster that
    /// carries it. Prose
    /// rots — batch 1's did, in two places, between the wave and slice 2 — so
    /// the claim is asserted here instead: for every kind this family has NOT
    /// claimed, the entry-level refusal for its own monster must name it.
    ///
    /// It cannot go stale in the awkward direction either. A kind that becomes
    /// claimed is skipped by the same registry the manifest is derived from,
    /// so an engine slice landing a primitive does not redden this file — the
    /// wave PR that claims the kind does the work of removing it from the walk.
    #[test]
    fn the_gate_names_every_escalated_kind_on_its_own_monster() {
        let rows = owned_rows();
        let mut measured: Vec<MoveKind> = Vec::new();
        for (monster, entry) in rows.iter().copied() {
            if IMPLEMENTED.contains(&entry.kind) {
                continue;
            }
            measured.push(entry.kind);
            let refusal = admit_with_monster(monster).expect_err(
                "an unclaimed kind cannot be admitted: the gate walks every \
                 loop entry of every monster in the entry",
            );
            assert!(
                refusal.contains(MissingCapability::MoveKind(entry.kind)),
                "{:?} carries {:?}, which this family has not claimed, so the \
                 gate must name it; it said {refusal}",
                monster.as_str(),
                entry.kind.as_str()
            );
        }
        // Total over the family, not merely non-vacuous: every unclaimed kind
        // is either measured above or has no compiled loop row at all, and the
        // only kind in the second class is the random-AI one the next test
        // takes.
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            assert!(
                measured.contains(&kind) || !rows.iter().any(|(_, entry)| entry.kind == kind),
                "{:?} sits in a compiled loop but was not measured",
                kind.as_str()
            );
        }
    }

    /// Every owned kind is now implemented; the walk remains as a closed
    /// guard against a generated family row without a manifest claim.
    #[test]
    fn a_random_ai_only_kind_is_refused_before_its_table_is_read() {
        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let carriers: Vec<MonsterKind> = MonsterKind::ALL
                .into_iter()
                .filter(|monster| {
                    crate::content_tables::random_moves(*monster)
                        .is_some_and(|table| table.iter().any(|entry| entry.kind == kind))
                })
                .collect();
            // A kind with neither a loop row nor a random-AI carrier would be
            // dead in the tables, which `owned_rows` already refuses; this
            // keeps the walk honest from the other side.
            assert!(
                !carriers.is_empty() || owned_rows().iter().any(|(_, entry)| entry.kind == kind),
                "{:?} is in no move table at all",
                kind.as_str()
            );
            for monster in carriers {
                let refusal =
                    admit_with_monster(monster).expect_err("a random-AI monster is never admitted");
                assert!(
                    refusal.contains(MissingCapability::MonsterAi(monster)),
                    "{:?} reaches {:?} only through its random-AI table, so the \
                     gate must refuse on the AI; it said {refusal}",
                    monster.as_str(),
                    kind.as_str()
                );
            }
        }
    }

    /// An unclaimed kind's body must actually refuse, by its own name. The
    /// manifest is derived from [`IMPLEMENTED`] (D6), and a body that had been
    /// filled without being listed would be unreachable *and* invisible;
    /// one that refuses under a different kind would misdirect the operator
    /// reading the refusal.
    #[test]
    fn every_unclaimed_kind_refuses_by_its_own_name() {
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let mut events = Vec::new();

        for kind in OWNED {
            if IMPLEMENTED.contains(&kind) {
                continue;
            }
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: &[],
                events: &mut events,
            };
            assert_eq!(
                super::super::apply_move(kind, &mut ctx),
                Err(EngineRefusal::MoveKindNotModeled(kind)),
                "{:?} is unclaimed, so its body must refuse under its own kind",
                kind.as_str()
            );
        }
        // A refusing dispatch touches nothing.
        assert!(events.is_empty());
    }

    fn entomancer_for_spit(hive: i32, strength: i32) -> crate::hot::HotState {
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        let mut owner = crate::hot::HotMonster::new(MonsterKind::Entomancer, 165);
        owner.max_hp = 165;
        owner.loop_pos = 2;
        owner.powers.set(PowerId::Hive, SlotWire::Int, hive);
        owner.powers.set(PowerId::Strength, SlotWire::Int, strength);
        state.monsters_mut().push(owner);
        state
    }

    fn ento_spit_entry(catalog: &crate::catalog::Catalog) -> CompiledMove {
        catalog
            .moves(MonsterKind::Entomancer)
            .iter()
            .find(|entry| entry.kind == MoveKind::EntoSpit)
            .copied()
            .unwrap()
    }

    #[test]
    fn ento_spit_public_body_pins_the_exact_carrier_operands_and_event_order() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let catalog = builder.build();
        let rows = catalog.moves(MonsterKind::Entomancer);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, MoveKind::Attack);
        assert_eq!(
            catalog.args(rows[0].args),
            &[CompiledArg::I(3), CompiledArg::I(8)]
        );
        assert_eq!(rows[1].kind, MoveKind::Attack);
        assert_eq!(
            catalog.args(rows[1].args),
            &[CompiledArg::I(20), CompiledArg::I(1)]
        );
        assert_eq!(rows[2].kind, MoveKind::EntoSpit);
        assert!(catalog.args(rows[2].args).is_empty());
        assert!(IMPLEMENTED.contains(&MoveKind::EntoSpit));

        let mut state = entomancer_for_spit(1, 0);
        let mut events = Vec::new();
        ento_spit(&mut MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(rows[2].args),
            events: &mut events,
        })
        .unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Hive), 2);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
        assert_eq!(
            events,
            [
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Hive,
                    amount: 2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 1,
                },
            ]
        );
    }

    #[test]
    fn ento_spit_private_writer_stacks_then_uses_the_exact_cap_branch() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let catalog = builder.build();
        let entry = ento_spit_entry(&catalog);
        let mut state = entomancer_for_spit(1, 0);
        let mut events = Vec::new();

        for _ in 0..3 {
            ento_spit_exact(&mut MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 0,
                args: catalog.args(entry.args),
                events: &mut events,
            })
            .unwrap();
        }

        assert_eq!(state.monsters[0].powers.value(PowerId::Hive), 3);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 4);
        assert_eq!(
            events,
            vec![
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Hive,
                    amount: 2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 1,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Hive,
                    amount: 3,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 2,
                },
                crate::engine::Event::PowerChanged {
                    subject: Subject::Monster(0),
                    power: PowerId::Strength,
                    amount: 4,
                },
            ]
        );
    }

    #[test]
    fn ento_spit_private_writer_refuses_overflow_and_malformed_shapes_atomically() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let catalog = builder.build();
        let entry = ento_spit_entry(&catalog);
        let exact_args = catalog.args(entry.args);

        let mut overflow = entomancer_for_spit(3, i32::MAX);
        let before = overflow.clone();
        let mut events = vec![crate::engine::Event::PowerChanged {
            subject: Subject::Player,
            power: PowerId::Strength,
            amount: 17,
        }];
        let events_before = events.clone();
        assert_eq!(
            ento_spit_exact(&mut MoveCtx {
                state: &mut overflow,
                catalog: &catalog,
                actor: 0,
                args: exact_args,
                events: &mut events,
            }),
            Err(EngineRefusal::CounterOverflow("ento_spit Strength"))
        );
        assert_eq!(overflow, before);
        assert_eq!(events, events_before);

        #[derive(Clone, Copy, Debug)]
        enum Malformed {
            Args,
            Owner,
            Loop,
            Dead,
            Hive,
            Wire,
            Roster,
        }

        let bad_args = [CompiledArg::I(1)];
        for malformed in [
            Malformed::Args,
            Malformed::Owner,
            Malformed::Loop,
            Malformed::Dead,
            Malformed::Hive,
            Malformed::Wire,
            Malformed::Roster,
        ] {
            let mut state = entomancer_for_spit(1, 0);
            let args = if matches!(malformed, Malformed::Args) {
                bad_args.as_slice()
            } else {
                exact_args
            };
            match malformed {
                Malformed::Args => {}
                Malformed::Owner => state.monsters_mut()[0].kind = MonsterKind::PhrogParasite,
                Malformed::Loop => state.monsters_mut()[0].loop_pos = 1,
                Malformed::Dead => state.monsters_mut()[0].hp = 0,
                Malformed::Hive => {
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::Hive, SlotWire::Int, 4)
                }
                Malformed::Wire => {
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::Hive, SlotWire::Bool, 1)
                }
                Malformed::Roster => state
                    .monsters_mut()
                    .push(crate::hot::HotMonster::new(MonsterKind::Wriggler, 10)),
            }
            let before = state.clone();
            let mut events = events_before.clone();
            assert!(matches!(
                ento_spit_exact(&mut MoveCtx {
                    state: &mut state,
                    catalog: &catalog,
                    actor: 0,
                    args,
                    events: &mut events,
                }),
                Err(EngineRefusal::MalformedArgs(_))
            ));
            assert_eq!(state, before, "{malformed:?}");
            assert_eq!(events, events_before, "{malformed:?}");
        }
    }

    #[test]
    fn mecha_flamethrower_attacks_then_injects_four_burns_with_hand_overflow() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::MechaKnight).unwrap();
        let catalog = builder.build();
        let entry = catalog
            .moves(MonsterKind::MechaKnight)
            .iter()
            .find(|entry| entry.kind == MoveKind::AttackPileInject)
            .copied()
            .unwrap();
        let mut state = crate::hot::HotState::at_defaults();
        state.hp = 100;
        state
            .monsters_mut()
            .push(crate::hot::HotMonster::new(MonsterKind::MechaKnight, 100));
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 0,
            args: catalog.args(entry.args),
            events: &mut events,
        };
        attack_pile_inject(&mut ctx).unwrap();
        assert_eq!(state.hp, 88);
        assert_eq!(state.piles.get(PileId::Hand).len(), 4);
        assert_eq!(
            state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect::<Vec<_>>(),
            vec![CardId::Burn; 4]
        );
    }

    /// A live turn-one Knights roster whose piles the opening left non-exact
    /// (the #2959 card-offer roots: no tied deck group), holding one plain
    /// physical Wound in each of Hand/Draw/Discard.
    fn knights_hex_turn_one() -> (crate::hot::HotState, crate::catalog::Catalog) {
        use crate::hot::{HotCard, HotMonster, HotState};
        let identity = CardIdentity {
            id: CardId::Wound,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(identity).unwrap();
        let catalog = builder.build();
        let atom = catalog.atom(&identity).unwrap();
        let mut state = HotState::at_defaults();
        assert!(!state.exact_piles);
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 100);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(2)));
        assert!(flail.random_ai.set_log(&[2]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 89);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(0)));
        assert!(spectral.random_ai.set_log(&[0]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 81);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        state.monsters_mut().extend([flail, spectral, magi]);
        for (uid, pile) in [PileId::Hand, PileId::Draw, PileId::Discard]
            .into_iter()
            .enumerate()
        {
            state.piles.get_mut(pile).make_mut().push(HotCard {
                uid: uid as u32,
                atom,
                flags: 0,
            });
        }
        state.next_card_uid = 3;
        (state, catalog)
    }

    /// #2959: Spectral's live HEX on non-exact piles is the promotion, as in
    /// the oracle's `force_exact=True` commit, instead of refusing
    /// `Knights Hex state` at the first monster turn of every such root.
    #[test]
    fn live_hex_on_non_exact_piles_promotes_and_afflicts_every_card() {
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_HEXED};
        let (mut state, catalog) = knights_hex_turn_one();
        let mut events = Vec::new();
        let mut ctx = MoveCtx {
            state: &mut state,
            catalog: &catalog,
            actor: 1,
            args: &[CompiledArg::I(2)],
            events: &mut events,
        };
        hex_player(&mut ctx).unwrap();
        assert!(state.exact_piles);
        assert_eq!(state.powers.value(PowerId::HexPower), 2);
        for pile in [PileId::Hand, PileId::Draw, PileId::Discard] {
            let [card] = state.piles.get(pile).as_slice() else {
                panic!("{pile:?} lost its card");
            };
            assert_eq!(
                card.flags & (CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED),
                CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED,
                "{pile:?}"
            );
        }
        assert!(!events.is_empty());
    }

    /// The widened gate keeps every other conjunct: an overlapping affliction
    /// power or a pre-afflicted card still refuses, with nothing written.
    #[test]
    fn live_hex_on_non_exact_piles_still_refuses_affliction_overlap() {
        for overlap in 0..3 {
            let (mut state, catalog) = knights_hex_turn_one();
            match overlap {
                0 => state.set_ringing(true),
                1 => state.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 3),
                _ => {
                    state.piles.get_mut(PileId::Hand).make_mut()[0].flags |=
                        crate::hot::CARD_FLAG_BOUND;
                }
            }
            let before = state.clone();
            let mut events = Vec::new();
            let mut ctx = MoveCtx {
                state: &mut state,
                catalog: &catalog,
                actor: 1,
                args: &[CompiledArg::I(2)],
                events: &mut events,
            };
            assert_eq!(
                hex_player(&mut ctx),
                Err(EngineRefusal::MalformedArgs("Knights Hex state")),
                "overlap {overlap}"
            );
            assert_eq!(state, before, "overlap {overlap}");
            assert!(events.is_empty());
        }
    }
}

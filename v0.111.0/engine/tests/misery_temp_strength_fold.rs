//! #2693 S3/S4 — `Misery` over the temporary-Strength wrapper fold, and the
//! entering-Strength provenance debt, at root level.
//!
//! The unit and command witnesses live beside the code they pin. This file
//! carries the chains only a public root can show: a wrapper applied by a real
//! card play, projected, **cold reloaded**, and then read by a real `Misery`
//! play from the cold copy; the copied wrapper's own side-end restoration on
//! its new owner; and a monster that walks into the root already carrying
//! Strength, whose provenance is materialised at hydration so nothing refuses
//! late.
//!
//! The native fold is `Misery/<OnPlay>d__3::MoveNext` `0x3ad358`
//! IL_00a1-IL_0133 (predicate `Misery/<>c__DisplayClass3_0::<OnPlay>b__2`
//! `0x3ad335`): each selected `ITemporaryPower` adds its own frozen `Amount`
//! back onto the frozen `StrengthPower` value, which is then copied as a
//! signed delta while the wrapper copies separately through its own Artifact
//! gate.
use serde_json::{Value, json};
use sts_sim::hot::{Applier, AttachedPowerModel};
use sts_sim::ids::PowerId;
use sts_sim::{boundary::HotBoundary, canonical::CanonicalStateV2, engine};

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

/// Two Toadpoles — a source and one recipient — with Dark Shackles and Misery
/// in hand.
///
/// `misery` is what turns the per-fight ledger upkeep gate on, so passing
/// `false` gives the identical root as a control.
fn fold_root(misery: bool, monster_strength: Option<i64>) -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let mut source = json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100, "uid": 41});
    if let Some(strength) = monster_strength {
        source["strength"] = json!(strength);
    }
    document.monsters = vec![
        serde_json::from_value(source).unwrap(),
        serde_json::from_value(
            json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 1, "uid": 42}),
        )
        .unwrap(),
    ];
    let mut hand = vec![json!({"id": "DARK_SHACKLES", "uid": 0, "upgrade": 0})];
    if misery {
        hand.push(json!({"id": "MISERY", "uid": 2, "upgrade": 0}));
    }
    let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
    piles["hand"] = Value::Array(hand);
    document.piles = serde_json::from_value(piles).unwrap();
    document
}

fn hydrate(document: &CanonicalStateV2) -> (sts_sim::catalog::Catalog, sts_sim::hot::HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    (catalog, state)
}

fn play(
    state: &sts_sim::hot::HotState,
    catalog: &sts_sim::catalog::Catalog,
    uid: u32,
    target: u8,
) -> sts_sim::hot::HotState {
    engine::apply_action_into(
        state,
        catalog,
        &engine::Action::Play {
            uid,
            target: Some(target),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap()
}

fn rows(monster: &sts_sim::hot::HotMonster) -> Vec<(AttachedPowerModel, Applier, i32)> {
    monster
        .misery_debuff_order
        .attachments()
        .map(|record| (record.power, record.applier, record.amount))
        .collect()
}

/// Dark Shackles -> project -> **cold reload** -> Misery, all through public
/// actions, with the warm and cold plays compared against each other and
/// against the state the ENGINE built.
///
/// Before this stage the reloaded root admitted (#2693 S2) and the `Misery`
/// play refused `Misery concrete Type-2 power state`. It now replays, and the
/// recipient ends in a state no aggregate signed scalar could describe: the
/// source stands at `Strength -9`, and the recipient — whose Artifact blocks
/// the wrapper — ends at **0**, because the folded Strength value is `0` and
/// `0x3ad358` IL_0268-IL_026f skips it without spending anything.
#[test]
fn a_dark_shackles_root_cold_reloads_and_then_folds_through_misery() {
    let document = fold_root(true, None);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(state.fanouts.misery_attachment_upkeep());

    // `DarkShackles/<OnPlay>d__8::MoveNext` `0x396954` IL_00b3-IL_00e6.
    let played = play(&state, &catalog, 0, 0);
    assert_eq!(played.monsters[0].powers.value(PowerId::Strength), -9);
    assert_eq!(played.monsters[0].powers.value(PowerId::TempStrength), -9);
    assert_eq!(
        rows(&played.monsters[0]),
        [
            (AttachedPowerModel::Strength, Applier::Player, -9),
            (AttachedPowerModel::DarkShackles, Applier::Player, 9),
        ],
    );

    let projected = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    let (cold_catalog, cold) = hydrate(&projected);
    engine::admit(&projected, &cold, &cold_catalog).expect("#2693 S3 admits the recorded fold");
    assert_eq!(
        cold.monsters, played.monsters,
        "the cold-reloaded roster is not what the engine built",
    );

    // Play Misery from the COLD copy and from the warm state; the results
    // agree, and the recipient carries the copied wrapper as a real instance.
    let warm_after = play(&played, &catalog, 2, 0);
    let cold_after = play(&cold, &cold_catalog, 2, 0);
    assert_eq!(cold_after.monsters, warm_after.monsters);

    let recipient = &warm_after.monsters[1];
    assert_eq!(recipient.powers.value(PowerId::Strength), -9);
    assert_eq!(recipient.powers.value(PowerId::TempStrength), -9);
    assert_eq!(
        rows(recipient),
        [
            (AttachedPowerModel::Strength, Applier::Player, -9),
            (AttachedPowerModel::DarkShackles, Applier::Player, 9),
        ],
        "the copy is a wrapper instance of its own, behind the Strength its \
         `BeforeApplied` `0x348d20` applied",
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&warm_after, &catalog)
            .unwrap()
            .monsters[1]["power_attachments"],
        json!([
            ["strength", "player", 0, -9, 0],
            ["dark_shackles", "player", 0, 9, 0]
        ]),
    );
}

/// The recipient's Artifact is spent on the copied WRAPPER, and the folded
/// Strength value of zero costs it nothing.
#[test]
fn the_recipients_artifact_is_spent_on_the_copied_wrapper() {
    let mut document = fold_root(true, None);
    document.monsters[1].insert("artifact".to_owned(), json!(2));
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    let after = play(&play(&state, &catalog, 0, 0), &catalog, 2, 0);
    let recipient = &after.monsters[1];
    assert_eq!(
        recipient.powers.value(PowerId::Artifact),
        1,
        "one Artifact, spent on the wrapper — the zero Strength entry is \
         skipped before the lookup",
    );
    assert_eq!(recipient.powers.value(PowerId::Strength), 0);
    assert_eq!(recipient.powers.value(PowerId::TempStrength), 0);
    assert!(rows(recipient).is_empty());
}

/// The copied wrapper takes part in its NEW owner's side end: removed first,
/// then its amount handed back to Strength with the recipient itself as the
/// applier (`<AfterSideTurnEnd>d__22::MoveNext` `0x348ba8` IL_0043,
/// IL_009d-IL_00c4).
#[test]
fn a_copied_wrapper_unwinds_on_its_new_owner_at_the_side_end() {
    let document = fold_root(true, None);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    let after = play(&play(&state, &catalog, 0, 0), &catalog, 2, 0);
    assert_eq!(rows(&after.monsters[1]).len(), 2);

    let ended =
        engine::apply_action_into(&after, &catalog, &engine::Action::EndTurn, &mut Vec::new())
            .unwrap();
    for index in [0, 1] {
        let monster = &ended.monsters[index];
        assert_eq!(
            monster.powers.value(PowerId::TempStrength),
            0,
            "monster {index}",
        );
        assert_eq!(
            monster.powers.value(PowerId::Strength),
            0,
            "monster {index}"
        );
        assert!(
            monster.misery_debuff_order.attachments().is_empty(),
            "monster {index}: the wrapper went, and the Strength it left at \
             exactly zero went with `ShouldRemoveDueToAmount` `0x83b0d`",
        );
    }

    let projected = HotBoundary::try_to_canonical(&ended, &catalog).unwrap();
    let (cold_catalog, cold) = hydrate(&projected);
    engine::admit(&projected, &cold, &cold_catalog).unwrap();
    assert_eq!(cold.monsters, ended.monsters);
}

/// #2693 S4 — a monster ENTERING the root with Strength.
///
/// Its provenance is materialised at hydration, so the Dark Shackles that
/// drives it negative keeps the row in step and `Misery` reads it. Before
/// this stage the write gave up, the scalar stayed unrecorded, and the play
/// refused **late inside an admitted root** — which is what #2693 forbids.
#[test]
fn an_entering_strength_is_recorded_at_hydration_and_never_refuses_late() {
    let document = fold_root(true, Some(4));
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert_eq!(
        rows(&state.monsters[0]),
        [(AttachedPowerModel::Strength, Applier::Unknown, 4)],
        "the ledger was empty, so the entering instance precedes everything",
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .monsters[0]["power_attachments"],
        json!([["strength", "unknown", 0, 4, 0]]),
    );

    let played = play(&state, &catalog, 0, 0);
    assert_eq!(played.monsters[0].powers.value(PowerId::Strength), -5);
    assert_eq!(
        rows(&played.monsters[0]),
        [
            (AttachedPowerModel::Strength, Applier::Unknown, -5),
            (AttachedPowerModel::DarkShackles, Applier::Player, 9),
        ],
        "`ModifyAmount` moved the value and neither the position nor the \
         applier",
    );

    // The play that used to refuse now replays: the fold copies `-5 + 9 = +4`
    // as a Type-1 Strength application and the wrapper separately.
    let after = play(&played, &catalog, 2, 0);
    let recipient = &after.monsters[1];
    assert_eq!(recipient.powers.value(PowerId::Strength), -5);
    assert_eq!(
        rows(recipient),
        [
            (AttachedPowerModel::Strength, Applier::Unknown, -5),
            (AttachedPowerModel::DarkShackles, Applier::Player, 9),
        ],
        "each copy carries `entry.Key.Applier` (`0x3ad358` IL_02b6/IL_0354) — \
         the source Strength row's unobserved one, and the wrapper's player",
    );

    // The same root without a `MISERY` records nothing at all, so a fight
    // that cannot reach the reader keeps the pre-#2693 bytes.
    let control = fold_root(false, Some(4));
    let (control_catalog, control_state) = hydrate(&control);
    engine::admit(&control, &control_state, &control_catalog).unwrap();
    assert!(
        control_state.monsters[0]
            .misery_debuff_order
            .attachments()
            .is_empty()
    );
    assert!(
        !HotBoundary::try_to_canonical(&control_state, &control_catalog)
            .unwrap()
            .monsters[0]
            .contains_key("power_attachments"),
    );
}

/// A root whose monster walks in with `strength: 1` beside a `weak`
/// acquisition token — the shape of the three #2693 residual roots, in which
/// the Python-projected document records the Weak's position without ordering
/// the Strength against it, so the entering instance has **no determined
/// position**.
///
/// `producer` chooses whether the fight can still drive that Strength
/// negative: `true` puts a real reducer in hand (Dark Shackles, the
/// `TempStrengthEnemy` loss wrapper), `false` puts an ordinary Strike there
/// instead. That is the only difference between the two roots, and it is the
/// whole of the #2693 S4b rule.
fn unplaceable_root(misery: bool, producer: bool) -> CanonicalStateV2 {
    let mut document = fold_root(misery, Some(1));
    document.monsters[0].insert("weak".to_owned(), json!(1));
    document.monsters[0].insert("misery_debuff_order".to_owned(), json!(["weak"]));
    if !producer {
        let mut hand: Vec<Value> = vec![json!({"id": "STRIKE_IRONCLAD", "uid": 0, "upgrade": 0})];
        if misery {
            hand.push(json!({"id": "MISERY", "uid": 2, "upgrade": 0}));
        }
        let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
        piles["hand"] = Value::Array(hand);
        document.piles = serde_json::from_value(piles).unwrap();
    }
    document
}

/// #2693 S4b — an entering Strength the ledger cannot place refuses at
/// ADMISSION when something reachable can drive it negative.
///
/// This is the refusal that closes the last ungated late refusal on the
/// `Misery` surface. Before it, this exact root admitted and then refused
/// `Misery concrete Type-2 power state` mid-fight, once Dark Shackles had
/// taken the unrecorded `strength: 1` below zero.
#[test]
fn an_unplaceable_entering_strength_refuses_beside_a_reachable_reducer() {
    let document = unplaceable_root(true, true);
    let (catalog, state) = hydrate(&document);
    assert!(
        state.monsters[0]
            .misery_debuff_order
            .attachments()
            .is_empty(),
        "the position is not determined, so nothing is invented",
    );

    let refusal = engine::admit(&document, &state, &catalog).unwrap_err();
    assert!(
        refusal.contains(engine::MissingCapability::ArgumentShape(
            "unplaceable entering Strength beside a reachable reducer"
        )),
        "expected the #2693 S4b admission refusal, got {refusal:?}",
    );
}

/// The other side of the same rule, and the reason it is narrow: with no
/// reducer reachable the root still ADMITS, and **no** play can refuse late.
///
/// Argued and witnessed. The argument is that `Misery` selects a
/// `StrengthPower` only below zero (`PowerModel::GetTypeForAmount` `0x83a94`
/// IL_0028-IL_003e), so an instance nothing can drive negative is never
/// selected and its missing position is never read. The witness is exhaustive
/// over this root: every card in hand is played at every target, the turn is
/// ended, and nothing returns a refusal.
#[test]
fn an_unplaceable_entering_strength_admits_and_never_refuses_late_without_a_reducer() {
    let document = unplaceable_root(true, false);
    let (catalog, state) = hydrate(&document);
    assert!(
        state.fanouts.misery_attachment_upkeep(),
        "the ledger has a reader, so the rule's other precondition holds",
    );
    assert!(
        state.monsters[0]
            .misery_debuff_order
            .attachments()
            .is_empty(),
    );
    assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);
    engine::admit(&document, &state, &catalog)
        .expect("nothing reachable can drive that Strength negative (#2693 S4b)");

    // Every reachable action from the root, including the `Misery` that would
    // read the ledger. None refuses, and the unrecorded Strength never leaves
    // the non-negative range where `Misery` cannot select it.
    let hand: Vec<u32> = state
        .piles
        .get(sts_sim::hot::PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect();
    assert_eq!(hand.len(), 2, "one Strike and one Misery");
    for uid in hand {
        for target in 0..state.monsters.len() as u8 {
            let mut events = Vec::new();
            let after = engine::apply_action_into(
                &state,
                &catalog,
                &engine::Action::Play {
                    uid,
                    target: Some(target),
                    selection: engine::SelectionRef::NONE,
                },
                &mut events,
            )
            .unwrap_or_else(|refusal| {
                panic!("uid {uid} at {target} refused inside an admitted root: {refusal:?}")
            });
            for monster in after.monsters.iter() {
                assert!(
                    monster.powers.value(PowerId::Strength) >= 0,
                    "uid {uid} at {target} drove a monster's Strength negative, \
                     so a reducer IS reachable and the gate under-refused",
                );
            }
        }
    }
    engine::apply_action_into(&state, &catalog, &engine::Action::EndTurn, &mut Vec::new())
        .expect("the enemy side raises Strength and never lowers it");
}

/// The control that keeps the rule paired with its reader: without a `Misery`
/// nothing reads the ledger, so the same unplaceable Strength admits even
/// beside a real reducer.
#[test]
fn an_unplaceable_entering_strength_admits_without_a_misery() {
    let document = unplaceable_root(false, true);
    let (catalog, state) = hydrate(&document);
    assert!(!state.fanouts.misery_attachment_upkeep());
    engine::admit(&document, &state, &catalog)
        .expect("no `Misery`, no reader, nothing to refuse (#2693 S4b)");
}

/// #2693 S3 — a reachable Unsettling Lamp beside a wrapper refuses at
/// admission, so the command-level refusal can never fire inside an admitted
/// root.
#[test]
fn a_wrapper_beside_an_unsettling_lamp_refuses_at_admission() {
    let document = fold_root(true, None);
    let (catalog, state) = hydrate(&document);
    let played = play(&state, &catalog, 0, 0);
    let mut projected = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    projected.player.insert("lamp".to_owned(), json!(true));
    projected.player.insert(
        "relics_entering".to_owned(),
        json!(["RELIC.BURNING_BLOOD", "RELIC.UNSETTLING_LAMP"]),
    );

    let (lamp_catalog, lamp_state) = hydrate(&projected);
    let refusal = engine::admit(&projected, &lamp_state, &lamp_catalog).unwrap_err();
    assert!(
        refusal.contains(engine::MissingCapability::ArgumentShape(
            "Misery temporary Strength clone Unsettling Lamp order"
        )),
        "expected the Lamp-order refusal, got {refusal:?}",
    );
}

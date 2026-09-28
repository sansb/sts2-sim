//! #2693 S2 — concrete temporary-Strength wrapper records, at root level.
//!
//! The unit witnesses live beside the code they pin. This file carries the
//! chains only a public root can show: two DIFFERENT wrapper classes applied
//! to one monster take two ordered ledger positions behind the Strength row
//! the first of them created, that state projects and **cold reloads**
//! exactly, and the native side end then unwinds each wrapper in ledger order
//! — removing it before handing its amount back to Strength with the OWNER as
//! applier.
//!
//! Before this stage the whole of that provenance was one signed scalar. The
//! native fact it could not hold is that each wrapper is a separate
//! `PowerModel` instance: `TemporaryStrengthPower::get_Sign` `0xa9add` is -1
//! for a loss wrapper and its `Amount` is positive, every subclass takes the
//! base `PowerModel::get_InstanceType` `0x83751` = 0 so
//! `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058 resolves it
//! by model id, and `Misery`'s filter (`0x3ad30a`) selects every one of them.
//!
//! S2 did **not** teach `Misery` to copy them; #2693 S3 does, so the two
//! assertions that pinned the wall now pin the other side of it — the reloaded
//! root admits, and the `Misery` play replays. The fold itself is witnessed in
//! `misery_temp_strength_fold.rs`; what stays here is the provenance the
//! writers record and the side end unwinds.
use serde_json::{Value, json};
use sts_sim::{boundary::HotBoundary, canonical::CanonicalStateV2, engine};

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

/// One Toadpole, with the two single-target wrapper cards in hand.
///
/// `misery` puts a `MISERY` beside them, which is what turns the per-fight
/// upkeep gate on: `Misery` is the ledger's only reader, so a hand without one
/// must record nothing at all. Passing `false` gives the identical root as a
/// control.
fn wrapper_root(misery: bool) -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    document.monsters = vec![
        serde_json::from_value(json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100})).unwrap(),
    ];
    let mut hand = vec![
        json!({"id": "DARK_SHACKLES", "uid": 0, "upgrade": 0}),
        json!({"id": "ENFEEBLING_TOUCH", "uid": 1, "upgrade": 0}),
    ];
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

/// Project, hydrate the projection, and assert the reload is the roster the
/// ENGINE built rather than a re-hydration of its own output.
///
/// `admits` says whether the reloaded root is also *admissible*. It is not the
/// same question — S2 made the state representable, projectable and reloadable
/// while `Misery` still refused to read it, and #2693 S3 lifted that wall, so
/// a recorded wrapper now admits and an unrecorded scalar still does not.
fn cold_reload(
    state: &sts_sim::hot::HotState,
    catalog: &sts_sim::catalog::Catalog,
    admits: bool,
) -> CanonicalStateV2 {
    let projected = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let (reload_catalog, restored) = hydrate(&projected);
    assert_eq!(
        engine::admit(&projected, &restored, &reload_catalog).is_ok(),
        admits,
    );
    assert_eq!(
        restored.monsters, state.monsters,
        "the cold-reloaded roster is not what the engine built",
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&restored, &reload_catalog).unwrap(),
        projected,
    );
    projected
}

fn play(
    state: &sts_sim::hot::HotState,
    catalog: &sts_sim::catalog::Catalog,
    uid: u32,
) -> sts_sim::hot::HotState {
    engine::apply_action_into(
        state,
        catalog,
        &engine::Action::Play {
            uid,
            target: Some(0),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap()
}

/// Two distinct wrapper classes, two ordered records, one cold reload.
#[test]
fn two_distinct_wrappers_record_two_ordered_positions_and_cold_reload() {
    let document = wrapper_root(true);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(state.fanouts.misery_attachment_upkeep());

    // Dark Shackles 9 (`0x396954` IL_00e6), then Enfeebling Touch 8
    // (`0x39bb64` IL_00e6). Each attaches its OWN instance, because the
    // stacking lookup is by model id.
    let played = play(&play(&state, &catalog, 0), &catalog, 1);
    let monster = &played.monsters[0];
    assert_eq!(
        monster.powers.value(sts_sim::ids::PowerId::TempStrength),
        -17
    );
    assert_eq!(monster.powers.value(sts_sim::ids::PowerId::Strength), -17);
    assert_eq!(
        monster.misery_debuff_order.placed_attachments(),
        vec![
            (
                0,
                sts_sim::hot::AttachmentRecord {
                    power: sts_sim::hot::AttachedPowerModel::Strength,
                    applier: sts_sim::hot::Applier::Player,
                    amount: -17,
                }
            ),
            (
                0,
                sts_sim::hot::AttachmentRecord {
                    power: sts_sim::hot::AttachedPowerModel::DarkShackles,
                    applier: sts_sim::hot::Applier::Player,
                    amount: 9,
                }
            ),
            (
                0,
                sts_sim::hot::AttachmentRecord {
                    power: sts_sim::hot::AttachedPowerModel::EnfeeblingTouch,
                    applier: sts_sim::hot::Applier::Player,
                    amount: 8,
                }
            ),
        ],
        "the nested Strength leads (`BeforeApplied` `0x348d20` runs at `Apply` \
         IL_02d9, before `ApplyInternal` IL_0360), then each wrapper in \
         application order",
    );

    // The projection and the reload are exact, and since #2693 S3 read the
    // fold the reloaded root ADMITS as well.
    let projected = cold_reload(&played, &catalog, true);
    assert_eq!(
        projected.monsters[0]["power_attachments"],
        json!([
            ["strength", "player", 0, -17, 0],
            ["dark_shackles", "player", 0, 9, 0],
            ["enfeebling_touch", "player", 0, 8, 0]
        ]),
    );

    // S2 recorded the provenance and refused to read it; #2693 S3 reads it.
    // This roster has one monster, so `HittableEnemies` minus the original
    // target is empty and no copy runs — but the source snapshot, and with it
    // the fold, is taken all the same, so the refusal is gone.
    let after = engine::apply_action_into(
        &played,
        &catalog,
        &engine::Action::Play {
            uid: 2,
            target: Some(0),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .expect("#2693 S3 lifts the temporary-Strength wall");
    assert_eq!(
        after.monsters[0].misery_debuff_order.placed_attachments(),
        played.monsters[0].misery_debuff_order.placed_attachments(),
        "the source's own ledger is untouched by its own snapshot",
    );
}

/// The enemy side end removes each wrapper before restoring its amount, in
/// ledger order, with the OWNER as the applier of the Strength it re-applies.
#[test]
fn the_enemy_side_end_unwinds_every_wrapper_and_restores_with_the_owner() {
    let document = wrapper_root(true);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    let played = play(&play(&state, &catalog, 0), &catalog, 1);
    assert_eq!(
        played.monsters[0].misery_debuff_order.attachments().len(),
        3
    );

    let ended =
        engine::apply_action_into(&played, &catalog, &engine::Action::EndTurn, &mut Vec::new())
            .unwrap();
    let monster = &ended.monsters[0];
    assert_eq!(monster.powers.value(sts_sim::ids::PowerId::TempStrength), 0);
    assert_eq!(
        monster.powers.value(sts_sim::ids::PowerId::Strength),
        0,
        "both wrappers handed their whole amount back",
    );
    assert!(
        monster.misery_debuff_order.attachments().is_empty(),
        "`PowerCmd::Remove` at `0x348ba8` IL_0043 takes each wrapper, and the \
         Strength they leave at exactly zero goes with `ShouldRemoveDueToAmount` \
         `0x83b0d`",
    );
    cold_reload(&ended, &catalog, true);

    // With the wrappers gone the monster is representable again, so the
    // refusal really was the temporary state and not something permanent.
    assert_eq!(
        sts_sim::engine::admit(
            &HotBoundary::try_to_canonical(&ended, &catalog).unwrap(),
            &ended,
            &catalog
        ),
        Ok(())
    );
}

/// The upkeep gate: the identical root without a `MISERY` records nothing,
/// and its projection carries no `power_attachments` slot at all.
///
/// This is what keeps a Misery-free fight — which is nearly every fight —
/// projecting the bytes the pre-#2693 engine projected.
#[test]
fn a_fight_without_misery_records_no_temporary_strength_provenance() {
    let document = wrapper_root(false);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(!state.fanouts.misery_attachment_upkeep());

    let played = play(&play(&state, &catalog, 0), &catalog, 1);
    let monster = &played.monsters[0];
    assert_eq!(
        monster.powers.value(sts_sim::ids::PowerId::TempStrength),
        -17
    );
    assert_eq!(monster.powers.value(sts_sim::ids::PowerId::Strength), -17);
    assert!(monster.misery_debuff_order.attachments().is_empty());

    let projected = cold_reload(&played, &catalog, true);
    assert!(
        !projected.monsters[0].contains_key("power_attachments"),
        "an empty ledger is omitted, so the document is byte-identical to the \
         pre-#2693 engine's",
    );

    // ...and the side end still restores the whole aggregate through the
    // fallback arm, because the rows were never the value of record.
    let ended =
        engine::apply_action_into(&played, &catalog, &engine::Action::EndTurn, &mut Vec::new())
            .unwrap();
    assert_eq!(
        ended.monsters[0]
            .powers
            .value(sts_sim::ids::PowerId::Strength),
        0,
    );
    assert_eq!(
        ended.monsters[0]
            .powers
            .value(sts_sim::ids::PowerId::TempStrength),
        0,
    );
}

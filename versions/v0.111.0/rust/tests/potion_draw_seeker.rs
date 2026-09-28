//! #2688: direct potion-drawn Hellraiser Seeker children cold-reload.
//!
//! A Swift/Clarity/Snecko/Gambler-Sly/UnceasingTop/Pagestorm Draw can draw a
//! SeekerStrike straight into Hellraiser AutoPlay, parking an automatic
//! `SeekerStrike` pending selection whose preceding Draw caller is the potion
//! (or power) Draw itself. Cold import must admit those states: the decoder
//! gate in `boundary.rs` authenticates exactly the engine-admitted Draw
//! callers, while every parent/body-stage/caller-local check and the root
//! ActionReplay receipt still apply unchanged.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, SelectionAnswer},
    hot::HotState,
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("hp".into(), json!(50));
    doc.player.insert("max_hp".into(), json!(50));
    // Fresh-combat draw counters: the fixture's mid-combat count is omitted so
    // the default zero applies to the replaced piles.
    doc.player.remove("cards_drawn_combat");
    doc.player.insert("player_phase".into(), json!(3));
    doc.player.insert("exact_piles".into(), json!(true));
    doc.player.insert("hellraiser".into(), json!(1));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["hellraiser", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player
        .insert("fully_unlocked_potion_pool".into(), json!(true));
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc
}

fn with_potion(mut doc: CanonicalStateV2, potion: &str) -> CanonicalStateV2 {
    doc.player.insert("potions".into(), json!([potion]));
    doc.player.insert("potion_slots".into(), json!([potion]));
    doc
}

fn with_piles(
    mut doc: CanonicalStateV2,
    hand: serde_json::Value,
    draw: serde_json::Value,
    next_uid: u64,
) -> CanonicalStateV2 {
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": draw,
        "discard": [],
    }))
    .unwrap();
    doc.player.insert("next_card_uid".into(), json!(next_uid));
    doc
}

fn card(id: &str, uid: u64) -> serde_json::Value {
    json!({"id": id, "uid": uid, "upgrade": 0})
}

fn load(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn use_potion(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let (state, catalog) = load(doc);
    let parked = engine::apply_action(
        &state,
        &catalog,
        &Action::UsePotion {
            slot: 0,
            target: None,
        },
    )
    .unwrap()
    .state;
    assert!(parked.pending.is_some(), "{parked:#?}");
    (parked, catalog)
}

/// Cold-reload `parked` through a rebuilt catalog, admit it, and prove the
/// reloaded state is bit-for-bit the live one (RNG, physical UIDs, history).
fn cold_roundtrip(parked: &HotState, catalog: &Catalog) -> (CanonicalStateV2, HotState, Catalog) {
    let wire = HotBoundary::try_to_canonical(parked, catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
    engine::admit(&wire, &cold, &rebuilt).unwrap();
    assert_eq!(cold, *parked, "cold reload preserves exact state");
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &rebuilt).unwrap(),
        wire,
        "reloaded state re-encodes to the same wire document"
    );
    (wire, cold, rebuilt)
}

/// The pending selection must hang directly off `caller`; this pins the exact
/// decoder seam each test exercises.
fn assert_direct_draw_caller(wire: &CanonicalStateV2, caller: &str) {
    let frames = &wire.continuations;
    assert!(frames.len() >= 2, "{wire:#?}");
    let pending = frames.last().unwrap();
    assert_eq!(pending.frame_type, "CardPlayFrame", "{wire:#?}");
    let draw = &frames[frames.len() - 2];
    assert_eq!(draw.frame_type, "DrawFrame", "{wire:#?}");
    assert_eq!(
        draw.fields.get("caller").and_then(|v| v.as_str()),
        Some(caller),
        "{wire:#?}"
    );
}

/// Every offered completion tree of the cold-reloaded pending selection must
/// run the potion suffix exactly once and leave no parked frames behind. Each
/// intermediate park cold-roundtrips bit-for-bit before its own offers run, so
/// multi-park chains (Gambler Sly tails) prove every level, not just the seam.
fn complete_all_offers(wire: &CanonicalStateV2) -> Vec<HotState> {
    fn dfs(wire: &CanonicalStateV2, depth: usize, leaves: &mut Vec<HotState>) {
        assert!(depth < 6, "completion chain too deep: {wire:#?}");
        let (state, catalog) = load(wire);
        let offers: Vec<SelectionAnswer> = engine::legal_actions(&state, &catalog)
            .into_iter()
            .filter_map(|action| match action {
                Action::Select { answer } => Some(answer),
                _ => None,
            })
            .collect();
        assert!(!offers.is_empty(), "{wire:#?}");
        for answer in offers {
            let (fresh, catalog) = load(wire);
            let done = engine::apply_action(&fresh, &catalog, &Action::Select { answer })
                .unwrap()
                .state;
            if done.pending.is_none() {
                assert!(done.frames.is_empty(), "{done:#?}");
                // The potion suffix ran exactly once: the belt slot is empty.
                let redoc = HotBoundary::try_to_canonical(&done, &catalog).unwrap();
                let slots = redoc
                    .player
                    .get("potion_slots")
                    .and_then(|slots| slots.as_array())
                    .expect("completed wire keeps potion_slots");
                assert!(!slots.is_empty() && slots.iter().all(|slot| slot.is_null()));
                leaves.push(done);
            } else {
                let (next_wire, _cold, _rebuilt) = cold_roundtrip(&done, &catalog);
                dfs(&next_wire, depth + 1, leaves);
            }
        }
    }
    let mut leaves = Vec::new();
    dfs(wire, 0, &mut leaves);
    assert!(!leaves.is_empty(), "{wire:#?}");
    leaves
}

#[test]
fn swift_direct_first_seeker_cold_admits_and_completes() {
    let doc = with_piles(
        with_potion(entry(), "SWIFT_POTION"),
        json!([card("DEFEND_IRONCLAD", 6)]),
        json!([
            card("SEEKER_STRIKE", 1),
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
            card("DEFEND_IRONCLAD", 5),
        ]),
        7,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "potion_epilogue");
    let completed = complete_all_offers(&wire);
    assert_eq!(completed[0].cards_drawn_combat, 3);
}

#[test]
fn swift_direct_later_seeker_cold_admits_and_completes() {
    let doc = with_piles(
        with_potion(entry(), "SWIFT_POTION"),
        json!([card("DEFEND_IRONCLAD", 6)]),
        json!([
            card("STRIKE_IRONCLAD", 1),
            card("SEEKER_STRIKE", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
            card("DEFEND_IRONCLAD", 5),
        ]),
        7,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "potion_epilogue");
    let completed = complete_all_offers(&wire);
    assert_eq!(completed[0].cards_drawn_combat, 3);
}

#[test]
fn clarity_direct_seeker_cold_admits_and_completes() {
    let doc = with_piles(
        with_potion(entry(), "CLARITY"),
        json!([card("DEFEND_IRONCLAD", 6)]),
        json!([
            card("SEEKER_STRIKE", 1),
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
        ]),
        7,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "clarity_potion");
    let completed = complete_all_offers(&wire);
    assert_eq!(completed[0].cards_drawn_combat, 1);
}

#[test]
fn snecko_direct_first_seeker_cold_admits_and_rolls_once() {
    let doc = with_piles(
        with_potion(entry(), "SNECKO_OIL"),
        json!([card("DEFEND_IRONCLAD", 9)]),
        json!([
            card("SEEKER_STRIKE", 1),
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
            card("DEFEND_IRONCLAD", 5),
            card("DEFEND_IRONCLAD", 6),
            card("DEFEND_IRONCLAD", 7),
            card("DEFEND_IRONCLAD", 8),
        ]),
        10,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "snecko_oil");
    let completed = complete_all_offers(&wire);
    assert_eq!(completed[0].cards_drawn_combat, 7);
}

#[test]
fn gambler_sly_tail_seeker_cold_admits_and_completes() {
    let mut doc = with_potion(entry(), "GAMBLERS_BREW");
    doc.player.insert("master_planner".into(), json!(1));
    // The two power ledgers share one uid allocator: MasterPlanner takes a
    // fresh uid rather than aliasing Hellraiser's side-turn row.
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(2));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["master_planner", 1]]),
    );
    let doc = with_piles(
        doc,
        json!([
            {"id":"PURITY","uid":1,"upgrade":0,"local_keywords":["Sly"]},
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 4),
        ]),
        json!([
            card("DEFEND_IRONCLAD", 3),
            card("SEEKER_STRIKE", 5),
            card("DEFEND_IRONCLAD", 6),
            card("DEFEND_IRONCLAD", 7),
        ]),
        8,
    );
    let (selecting, catalog) = load(&doc);
    let parked = engine::apply_action(
        &selecting,
        &catalog,
        &Action::UsePotion {
            slot: 0,
            target: None,
        },
    )
    .unwrap()
    .state;
    assert!(parked.pending.is_some(), "{parked:#?}");
    // Discard the Sly-marked Purity through every offered discard answer and
    // keep the resolution whose Hellraiser child hangs off the Sly tail Draw.
    let mut sly_park = None;
    let mut discard_offers = 0;
    for action in engine::legal_actions(&parked, &catalog) {
        let Action::Select { .. } = action else {
            continue;
        };
        discard_offers += 1;
        let Ok(candidate) = engine::apply_action(&parked, &catalog, &action) else {
            continue;
        };
        let Ok(wire) = HotBoundary::try_to_canonical(&candidate.state, &catalog) else {
            continue;
        };
        let frames = &wire.continuations;
        if frames.len() == 4
            && frames[0].frame_type == "ActionReplayFrame"
            && frames[1].frame_type == "PotionFinishFrame"
            && frames[2].frame_type == "DrawFrame"
            && frames[2]
                .fields
                .get("caller")
                .and_then(|caller| caller.as_str())
                == Some("sly_after_draw")
            && frames[3].frame_type == "CardPlayFrame"
        {
            sly_park = Some(candidate.state);
            break;
        }
    }
    assert!(discard_offers > 0, "{parked:#?}");
    let sly_park = sly_park.expect("a discard answer parks the Sly tail Seeker");
    let (wire, _cold, _rebuilt) = cold_roundtrip(&sly_park, &catalog);
    assert_direct_draw_caller(&wire, "sly_after_draw");
    complete_all_offers(&wire);
}

#[test]
fn unceasing_top_potion_seeker_cold_admits_and_completes() {
    let mut doc = with_potion(entry(), "BLOCK_POTION");
    doc.player.insert("unceasing_top".into(), json!(true));
    doc.player.insert(
        "relics_entering".into(),
        json!(["RELIC.BURNING_BLOOD", "RELIC.UNCEASING_TOP"]),
    );
    // Block Potion draws nothing, so with an empty-hand root the hand is
    // still empty when the finish-phase Unceasing Top Draw(1) fires; that
    // Draw takes the Seeker, parking the Hellraiser child off
    // `unceasing_top_potion` under the AfterTop potion wrapper.
    let doc = with_piles(
        doc,
        json!([]),
        json!([
            card("SEEKER_STRIKE", 1),
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
        ]),
        5,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "unceasing_top_potion");
    let completed = complete_all_offers(&wire);
    assert_eq!(completed[0].cards_drawn_combat, 1);
}

#[test]
fn swift_iteration_drawn_seeker_cold_admits_and_completes() {
    let mut doc = with_potion(entry(), "SWIFT_POTION");
    doc.player.insert("iteration".into(), json!(1));
    doc.player
        .insert("after_card_drawn_power_order".into(), json!(["iteration"]));
    // Swift draws the Dazed status, which fires the Iteration AfterCardDrawn
    // walk; that walk draws the Seeker, so the pending child hangs off the
    // power Draw at potion depth.
    let doc = with_piles(
        doc,
        json!([card("DEFEND_IRONCLAD", 8)]),
        json!([
            card("DAZED", 1),
            card("SEEKER_STRIKE", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
            card("DEFEND_IRONCLAD", 5),
            card("DEFEND_IRONCLAD", 6),
            card("DEFEND_IRONCLAD", 7),
        ]),
        9,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "after_card_drawn_power_order");
    complete_all_offers(&wire);
}

/// A forged or orphaned pending selection must never decode into an admitted
/// live state: refusal is atomic at the boundary or at admission. The catalog
/// build uses `expect` deliberately: every forgery below targets frames, so a
/// catalog failure would be a vacuous pass rather than a refusal.
fn refuses(doc: &CanonicalStateV2) {
    let catalog = HotBoundary::catalog_from_canonical(doc).expect("forgery keeps its cards");
    match HotBoundary::from_canonical(doc, &catalog) {
        Err(_) => {}
        Ok(state) => {
            assert!(
                engine::admit(doc, &state, &catalog).is_err(),
                "forged document admitted: {doc:#?}"
            );
        }
    }
}

fn frame_index(wire: &CanonicalStateV2, frame_type: &str) -> usize {
    wire.continuations
        .iter()
        .position(|frame| frame.frame_type == frame_type)
        .unwrap_or_else(|| panic!("no {frame_type} frame in {wire:#?}"))
}

#[test]
fn forged_draw_caller_and_orphan_pending_are_refused() {
    let doc = with_piles(
        with_potion(entry(), "SWIFT_POTION"),
        json!([card("DEFEND_IRONCLAD", 6)]),
        json!([
            card("SEEKER_STRIKE", 1),
            card("DEFEND_IRONCLAD", 2),
            card("DEFEND_IRONCLAD", 3),
            card("DEFEND_IRONCLAD", 4),
            card("DEFEND_IRONCLAD", 5),
        ]),
        7,
    );
    let (parked, catalog) = use_potion(&doc);
    let (wire, _cold, _rebuilt) = cold_roundtrip(&parked, &catalog);
    assert_direct_draw_caller(&wire, "potion_epilogue");

    // Unknown caller string.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "DrawFrame")]
        .fields
        .insert("caller".into(), json!("bogus_caller"));
    refuses(&forged);

    // Valid caller string but the wrong owner for this potion Draw.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "DrawFrame")]
        .fields
        .insert("caller".into(), json!("none"));
    refuses(&forged);

    // Newly admitted caller string under the wrong potion parent: the Draw
    // decode must refuse the (clarity_potion, SwiftPotion) pairing even
    // though the gate now admits the string itself.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "DrawFrame")]
        .fields
        .insert("caller".into(), json!("clarity_potion"));
    refuses(&forged);

    // Missing ActionReplay root.
    let mut forged = wire.clone();
    forged.continuations.remove(0);
    refuses(&forged);

    // Wrong potion identity under the same Draw.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "PotionFinishFrame")]
        .fields
        .insert("name".into(), json!("SNECKO_OIL"));
    refuses(&forged);

    // Manual source claims a Hellraiser-owned automatic selection.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "CardPlayFrame")]
        .fields
        .insert("source".into(), json!("manual"));
    refuses(&forged);

    // Late stage with a non-empty remaining body: the parked Seeker child is
    // mid-body, so forcing after_body must refuse rather than skip it.
    let mut forged = wire.clone();
    let idx = frame_index(&wire, "CardPlayFrame");
    let stage = forged.continuations[idx]
        .fields
        .get("stage")
        .and_then(|stage| stage.as_str())
        .expect("parked CardPlay carries a stage");
    assert_eq!(stage, "body", "{wire:#?}");
    forged.continuations[idx]
        .fields
        .insert("stage".into(), json!("after_body"));
    refuses(&forged);

    // Drawn card UID that was never drawn.
    let mut forged = wire.clone();
    forged.continuations[frame_index(&wire, "DrawFrame")]
        .fields
        .insert("card_uid".into(), json!(9999));
    refuses(&forged);
}

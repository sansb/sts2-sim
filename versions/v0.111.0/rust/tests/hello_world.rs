//! Native Creature.BeforeTurnStart freezes Hello World's independent amount.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::RewardPool,
    engine::{self, Action, SelectionRef},
    hot::{RngStream, RngStreamState},
};
fn entry() -> CanonicalStateV2 {
    owner_entry(RewardPool::Defect)
}
fn owner_entry(owner: RewardPool) -> CanonicalStateV2 {
    let doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    let cat = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let mut state = HotBoundary::from_canonical(&doc, &cat).unwrap();
    state.hp = 100;
    state.max_hp = 100;
    state.energy = 10;
    state.reward_card_pool = Some(owner);
    state.entropy_card_pool = Some(owner);
    state.fully_unlocked_card_pool_epochs = true;
    state.next_card_uid = 12;
    state.rng.set(
        RngStream::Generation,
        RngStreamState {
            words: [1, 2, 3, 4],
            counter: 0,
        },
    );
    let mut doc = HotBoundary::try_to_canonical(&state, &cat).unwrap();
    doc.piles = serde_json::from_value(json!({"hand":[{"id":"HELLO_WORLD","upgrade":0,"uid":0},{"id":"HELLO_WORLD","upgrade":1,"uid":1}],"draw":(2..12).map(|uid|json!({"id":"DEFEND_DEFECT","upgrade":0,"uid":uid})).collect::<Vec<_>>(),"discard":[]})).unwrap();
    doc.monsters =
        vec![serde_json::from_value(json!({"kind":"SEWER_CLAM","hp":1000,"max_hp":1000})).unwrap()];
    doc
}
fn step(doc: &CanonicalStateV2, action: Action) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    engine::admit(&wire, &cold, &cold_catalog).unwrap();
    wire
}
#[test]
fn two_public_plays_preserve_zero_snapshot_then_turn_start_generates_two() {
    let entry = entry();
    let first = step(
        &entry,
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    let second = step(
        &first,
        Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    assert_eq!(second.player["hello_world"], 2);
    assert!(
        !second
            .player
            .contains_key("hello_world_amount_on_turn_start")
    );
    assert_ne!(first.differential_digest(), second.differential_digest());
    let ended = step(&second, Action::EndTurn);
    assert_eq!(ended.player["hello_world_amount_on_turn_start"], 2);
    assert_eq!(ended.player["next_card_uid"], 14);
    let generated = ended
        .piles
        .values()
        .flatten()
        .filter(|c| c.uid.is_some_and(|uid| uid >= 12))
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);
    assert_ne!(generated[0].id, generated[1].id);
    assert!(generated.iter().all(|c| c.upgrade == 0));
    // The independent first-play branch still owns its own one-stack snapshot.
    let sibling = step(&first, Action::EndTurn);
    assert_eq!(sibling.player["hello_world_amount_on_turn_start"], 1);
    assert_eq!(sibling.player["next_card_uid"], 13);
}

#[test]
fn replayed_hello_world_keeps_original_snapshot_through_every_body() {
    for extra_plays in [1, 4] {
        let doc = entry();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let mut state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let mut physical = state.card_states.get(0);
        physical.set_base_replay_count(Some(extra_plays)).unwrap();
        state.card_states.set(0, physical);
        state.piles.get_mut(sts_sim::hot::PileId::Hand).make_mut()[0].flags |=
            sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let played = step(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        assert_eq!(played.player["hello_world"], extra_plays + 1);
        assert!(
            !played
                .player
                .contains_key("hello_world_amount_on_turn_start")
        );
        let ended = step(&played, Action::EndTurn);
        assert_eq!(
            ended.player["hello_world_amount_on_turn_start"],
            extra_plays + 1
        );
        assert_eq!(ended.player["next_card_uid"], 12 + extra_plays + 1);
    }
}

#[test]
fn live_hello_world_rejects_wrong_wire_and_impossible_snapshot() {
    let played = step(
        &entry(),
        Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    );
    let mut boolean = played.clone();
    boolean.player.insert("hello_world".into(), json!(true));
    let catalog = HotBoundary::catalog_from_canonical(&boolean).unwrap();
    let state = HotBoundary::from_canonical(&boolean, &catalog).unwrap();
    assert!(engine::admit(&boolean, &state, &catalog).is_err());
    for snapshot in [-1, 2] {
        let mut forged = played.clone();
        forged
            .player
            .insert("hello_world_amount_on_turn_start".into(), json!(snapshot));
        let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
        assert!(HotBoundary::from_canonical(&forged, &catalog).is_err());
    }
}

#[test]
fn foreign_hello_world_uses_owner_common_pool_after_cold_reload() {
    for owner in [
        RewardPool::Ironclad,
        RewardPool::Silent,
        RewardPool::Regent,
        RewardPool::Necrobinder,
        RewardPool::Defect,
    ] {
        let doc = owner_entry(owner);
        let first = step(
            &doc,
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::NONE,
            },
        );
        let ended = step(&first, Action::EndTurn);
        assert_eq!(ended.player["next_card_uid"], 13);
        assert_eq!(ended.rng["generation"].counter, 19);
        let generated = ended
            .piles
            .values()
            .flatten()
            .find(|c| c.uid == Some(12))
            .unwrap();
        let oracle: serde_json::Value =
            serde_json::from_str(include_str!("../../solver/card_pool_census.json")).unwrap();
        let name = format!("{owner:?}").to_uppercase();
        assert!(
            oracle["pools"][&name]["cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["id"] == format!("CARD.{}", generated.id)
                    && row["rarity"] == "Common"),
            "{owner:?}/{generated:?}"
        );
    }
}

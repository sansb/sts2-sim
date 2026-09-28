//! Current-build native SlumberPower and Beetle move-machine seam tests.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, SelectionRef},
    hot::{HotState, MonsterOverride, RngStream},
    ids::PowerId,
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({"hp":100,"max_hp":100,
        "energy":10,"exact_piles":true,
        "player_phase":3,"next_card_uid":4}))
    .unwrap();
    doc.monsters = vec![
        serde_json::from_value(json!({"kind":"SLUMBERING_BEETLE",
        "hp":89,"max_hp":89,"override":"SNORE","slumber":3,"mplating":18}))
        .unwrap(),
    ];
    doc.piles = serde_json::from_value(json!({"draw":[],"discard":[],"hand":[
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":0},{"id":"STRIKE_IRONCLAD","upgrade":0,"uid":1},
        {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":2},{"id":"STRIKE_IRONCLAD","upgrade":0,"uid":3}]})).unwrap();
    doc
}
fn load(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let cat = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &cat).unwrap();
    engine::admit(doc, &state, &cat).unwrap();
    (state, cat)
}
fn step(state: &mut HotState, cat: &mut Catalog, action: Action) {
    assert!(engine::legal_actions(state, cat).contains(&action));
    let ai = state.rng.get(RngStream::Ai);
    *state = engine::apply_action_into(state, cat, &action, &mut Vec::new()).unwrap();
    assert_eq!(state.rng.get(RngStream::Ai), ai);
    let doc = HotBoundary::try_to_canonical(state, cat).unwrap();
    let next_cat = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let cold = HotBoundary::from_canonical(&doc, &next_cat).unwrap();
    if !cold.history.over {
        engine::admit(&doc, &cold, &next_cat).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &next_cat).unwrap(),
        doc
    );
    *state = cold;
    *cat = next_cat;
}
fn strike(uid: u32) -> Action {
    Action::Play {
        uid,
        target: Some(0),
        selection: SelectionRef::new(None),
    }
}

#[test]
fn beetle_natural_expiry_removes_plating_and_attacks_next_turn() {
    let (mut state, mut cat) = load(&entry());
    // #2809: Plating does not decrement at round one's enemy side start and
    // grants its live amount at every enemy side end (`BeforeSideTurnEndEarly`)
    // — before `SlumberPower.AfterSideTurnEnd` (`0x34519c`) wakes the beetle
    // and removes Plating on the third end, which keeps that Block.
    for (slumber, plating, block) in [(2, 18, 18), (1, 17, 17), (0, 0, 16)] {
        step(&mut state, &mut cat, Action::EndTurn);
        assert_eq!(state.hp, 100);
        let m = &state.monsters[0];
        assert_eq!(
            (
                m.powers.value(PowerId::Slumber),
                m.powers.value(PowerId::Mplating),
                m.block
            ),
            (slumber, plating, block)
        );
    }
    assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    step(&mut state, &mut cat, Action::EndTurn);
    assert_eq!(state.hp, 82);
    assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 2);
    step(&mut state, &mut cat, Action::EndTurn);
    assert_eq!(state.hp, 62);
    assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 4);
}

#[test]
fn beetle_damage_counts_hits_and_queues_one_wake_action() {
    let mut doc = entry();
    doc.monsters[0].insert("block".into(), json!(6));
    let (mut state, mut cat) = load(&doc);
    step(&mut state, &mut cat, strike(0));
    assert_eq!(state.monsters[0].powers.value(PowerId::Slumber), 3);
    for (uid, remaining) in [(1, 2), (2, 1), (3, 0)] {
        step(&mut state, &mut cat, strike(uid));
        assert_eq!(state.monsters[0].powers.value(PowerId::Slumber), remaining);
        assert_eq!(state.monsters[0].powers.value(PowerId::Mplating), 18);
    }
    assert_eq!(
        state.monsters[0].override_state,
        MonsterOverride::BeetleWake
    );
    step(&mut state, &mut cat, Action::EndTurn);
    assert_eq!(state.hp, 100);
    assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    assert_eq!(state.monsters[0].powers.value(PowerId::Mplating), 0);
    step(&mut state, &mut cat, Action::EndTurn);
    assert_eq!(state.hp, 82);
}

#[test]
fn beetle_state_owner_and_counter_are_authenticated() {
    for change in [
        json!({"slumber":4}),
        json!({"slumber":0}),
        json!({"kind":"TOADPOLE"}),
        json!({"override":"WAKE"}),
        json!({"override":"","slumber":0}),
        json!({"loop_pos":1}),
    ] {
        let mut doc = entry();
        doc.monsters[0].extend(
            change
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        doc.monsters[0].retain(|_, v| *v != json!(0) && *v != json!(""));
        let cat = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let state = HotBoundary::from_canonical(&doc, &cat).unwrap();
        assert!(engine::admit(&doc, &state, &cat).is_err(), "{change}");
    }
}

#[test]
fn ea6_floor24_complete_capture_admits() {
    let doc = serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor24.canonical.json"
    ))
    .unwrap();
    load(&doc);
}

#[test]
fn beetle_unpowered_hit_and_lethal_hit_follow_native_receiver_gate() {
    let mut doc = entry();
    doc.player.insert("potions".into(), json!(["FIRE_POTION"]));
    doc.player
        .insert("fully_unlocked_potion_pool".into(), json!(true));
    doc.player
        .insert("potion_slots".into(), json!(["FIRE_POTION", null, null]));
    let (mut state, mut cat) = load(&doc);
    step(
        &mut state,
        &mut cat,
        Action::UsePotion {
            slot: 0,
            target: Some(0),
        },
    );
    assert_eq!(state.monsters[0].hp, 69);
    assert_eq!(state.monsters[0].powers.value(PowerId::Slumber), 2);
    // An ordinary lethal result skips AfterDamageReceived; it must not
    // schedule a wake or spend another stack on its corpse.
    let mut doc = entry();
    doc.monsters[0].insert("hp".into(), json!(5));
    let (mut state, mut cat) = load(&doc);
    step(&mut state, &mut cat, strike(0));
    assert!(state.history.over);
    assert_eq!(state.monsters[0].powers.value(PowerId::Slumber), 0);
    assert_ne!(
        state.monsters[0].override_state,
        MonsterOverride::BeetleWake
    );
}

#[test]
fn beetle_go_for_the_eyes_reads_post_damage_intent() {
    for (slumber, after, weak) in [
        (3, MonsterOverride::BeetleSnore, 0),
        (1, MonsterOverride::BeetleWake, 0),
        (0, MonsterOverride::None, 1),
    ] {
        let mut doc = entry();
        doc.piles.get_mut("hand").unwrap()[0].id = "GO_FOR_THE_EYES".into();
        if slumber == 0 {
            doc.monsters[0].remove("slumber");
            doc.monsters[0].remove("override");
            doc.monsters[0].remove("mplating");
        } else {
            doc.monsters[0].insert("slumber".into(), json!(slumber));
        }
        let (mut state, mut cat) = load(&doc);
        step(&mut state, &mut cat, strike(0));
        assert_eq!(state.monsters[0].override_state, after);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), weak);
    }
}

#[test]
fn beetle_death_clears_sleep_without_blocking_a_surviving_peer() {
    let mut doc = entry();
    let mut peer = doc.monsters[0].clone();
    peer.insert("slot".into(), json!(1));
    peer.insert("uid".into(), json!(1));
    doc.monsters.push(peer);
    doc.monsters[0].insert("hp".into(), json!(5));
    let (mut state, mut cat) = load(&doc);
    step(&mut state, &mut cat, strike(0));
    assert!(!state.history.over);
    assert_eq!(state.monsters[0].powers.value(PowerId::Slumber), 0);
    assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
    assert_eq!(state.monsters[1].powers.value(PowerId::Slumber), 3);
    step(&mut state, &mut cat, Action::EndTurn);
    assert_eq!(state.monsters[1].powers.value(PowerId::Slumber), 2);
}

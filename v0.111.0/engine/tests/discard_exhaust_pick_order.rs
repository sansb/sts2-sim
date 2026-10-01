//! Discard and Exhaust selections keep the player's pick order (#2524).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//! `CardSelectCmd/<FromHand>d__28::MoveNext` (RVA `0x3e7568`) returns the
//! synchronized answer through `PlayerChoiceResult::AsCombatCards` (IL_0400)
//! in the player's own pick order, and every sink below consumes it serially:
//!
//! * `Prepared/<OnPlay>d__3::MoveNext` (RVA `0x3b3678`) hands
//!   `FromHandForDiscard` (IL_00d3) to plural `CardCmd::Discard` (IL_013c),
//!   i.e. `DiscardAndDraw` (RVA `0x3e0274`), which adds each card at
//!   Discard/Bottom in list order (IL_011d).
//! * `Ashwater/<OnUse>d__9::MoveNext` (RVA `0x34bb90`) awaits
//!   `CardCmd::Exhaust` once per element of the `FromHand` answer (IL_0101).
//! * `GamblersBrew/<OnUse>d__6::MoveNext` (RVA `0x34e3a4`) hands the
//!   `FromHandForDiscard` answer to `DiscardAndDraw(list, list.Count)`
//!   (IL_00e8).
//!
//! Prepared L1's two-card choice enumerates both orders in `legal_actions`.
//! The two potions accept every ordered answer, but offer only the canonical
//! representatives: `engine::ordered_selection_answer` names the rest.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, SelectionAnswer, SelectionRef},
    hot::HotState,
};

fn entry(hand: serde_json::Value, draw: usize) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("hp".into(), json!(50));
    doc.player.insert("max_hp".into(), json!(50));
    doc.player.remove("cards_drawn_combat");
    doc.player.insert("player_phase".into(), json!(3));
    doc.player.insert("exact_piles".into(), json!(true));
    doc.player.insert("next_card_uid".into(), json!(20));
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    let draw = (10..10 + draw as u64)
        .map(|uid| card("STRIKE_IRONCLAD", uid))
        .collect::<Vec<_>>();
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": draw,
        "discard": [],
    }))
    .unwrap();
    doc
}

fn card(id: &str, uid: u64) -> serde_json::Value {
    json!({"id": id, "uid": uid, "upgrade": 0})
}

fn plain_hand() -> Vec<serde_json::Value> {
    vec![
        card("STRIKE_IRONCLAD", 1),
        card("DEFEND_IRONCLAD", 2),
        card("BASH", 3),
    ]
}

fn load(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

/// Cold-reload a parked state through a rebuilt catalog.
fn cold(parked: &HotState, catalog: &Catalog) -> (HotState, Catalog) {
    let wire = HotBoundary::try_to_canonical(parked, catalog).unwrap();
    let (state, rebuilt) = load(&wire);
    assert_eq!(state, *parked, "cold reload preserves the parked choice");
    (state, rebuilt)
}

fn uids(state: &HotState, pile: sts_sim::hot::PileId) -> Vec<u32> {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect()
}

fn select(ordinal: u32) -> Action {
    Action::Select {
        answer: SelectionAnswer::OptionIndex(ordinal),
    }
}

/// Every ordered pick list over `pool` (lengths 0..=len).
fn every_order(pool: &[u32]) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new()];
    let mut frontier = vec![Vec::new()];
    for _ in 0..pool.len() {
        let mut next = Vec::new();
        for prefix in &frontier {
            for uid in pool {
                if !prefix.contains(uid) {
                    let mut longer: Vec<u32> = prefix.clone();
                    longer.push(*uid);
                    next.push(longer);
                }
            }
        }
        out.extend(next.iter().cloned());
        frontier = next;
    }
    out
}

fn use_potion(potion: &str) -> (HotState, Catalog) {
    // A deep Draw pile, so Gambler's Brew's refill never reshuffles the
    // Discard pile it just ordered.
    let mut doc = entry(json!(plain_hand()), 6);
    doc.player.insert("potions".into(), json!([potion]));
    doc.player.insert("potion_slots".into(), json!([potion]));
    doc.player
        .insert("fully_unlocked_potion_pool".into(), json!(true));
    let (state, catalog) = load(&doc);
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
    assert!(parked.pending.is_some(), "{potion} suspends on its choice");
    (parked, catalog)
}

/// Every ordered answer of a potion choice is accepted, is described by
/// exactly its own uids, and lands in `destination` in pick order; the legal
/// list is the unchanged representative set.
fn assert_every_pick_order_is_native(potion: &str, destination: sts_sim::hot::PileId) {
    let (parked, catalog) = use_potion(potion);
    let representatives = engine::legal_actions(&parked, &catalog);
    assert_eq!(
        representatives.len(),
        8,
        "{potion}: the searched surface is still one answer per subset"
    );
    let orders = every_order(&[1, 2, 3]);
    assert_eq!(orders.len(), 16, "1 + 3 + 6 + 6 ordered answers");
    let mut ordinals = std::collections::BTreeSet::new();
    let mut extension = 0;
    for order in &orders {
        let answer = engine::ordered_selection_answer(&parked, &catalog, order)
            .unwrap()
            .unwrap_or_else(|| panic!("{potion}: {order:?} is a legal game answer"));
        let SelectionAnswer::OptionIndex(ordinal) = answer else {
            panic!("ordinal answer")
        };
        assert!(ordinals.insert(ordinal), "{potion}: one ordinal per order");
        let offered = representatives.contains(&Action::Select { answer });
        if !offered {
            extension += 1;
            assert!(ordinal >= 8, "{potion}: extension follows representatives");
        }
        assert_eq!(
            engine::selected_card_uids(&parked, &catalog, answer)
                .unwrap()
                .unwrap(),
            *order,
            "{potion}: the ordinal describes exactly its pick order"
        );
        // Hot and cold agree, and the destination pile records pick order.
        let hot = engine::apply_action(&parked, &catalog, &Action::Select { answer })
            .unwrap_or_else(|refusal| panic!("{potion}: {order:?} ({ordinal}): {refusal:?}"))
            .state;
        let (cold_state, cold_catalog) = cold(&parked, &catalog);
        let cold_done =
            engine::apply_action(&cold_state, &cold_catalog, &Action::Select { answer })
                .unwrap()
                .state;
        assert_eq!(
            HotBoundary::try_to_canonical(&hot, &catalog).unwrap(),
            HotBoundary::try_to_canonical(&cold_done, &cold_catalog).unwrap()
        );
        assert!(hot.pending.is_none(), "{potion}: the answer completes");
        assert_eq!(
            uids(&hot, destination),
            *order,
            "{potion}: {destination:?} follows the pick order"
        );
    }
    // Only single-card answers and the representative multi-card orders are
    // offered; every other order of a 2- or 3-card pick is extension-only.
    assert_eq!(extension, 16 - 8, "{potion}: 8 order-only answers");
    // One past the accepted range is refused, atomically.
    let past = 8 + 16;
    assert!(
        engine::selected_card_uids(&parked, &catalog, SelectionAnswer::OptionIndex(past)).is_err()
    );
    assert!(engine::apply_action(&parked, &catalog, &select(past)).is_err());
    // A pick naming a card outside the candidates resolves to nothing.
    assert_eq!(
        engine::ordered_selection_answer(&parked, &catalog, &[1, 9]).unwrap(),
        None
    );
}

#[test]
fn ashwater_exhausts_every_pick_order_in_native_order() {
    assert_every_pick_order_is_native("ASHWATER", sts_sim::hot::PileId::Exhaust);
}

#[test]
fn gamblers_brew_discards_every_pick_order_in_native_order() {
    assert_every_pick_order_is_native("GAMBLERS_BREW", sts_sim::hot::PileId::Discard);
}

fn play_prepared(upgrade: u8) -> (HotState, Catalog) {
    let mut hand = vec![json!({"id": "PREPARED", "uid": 0, "upgrade": upgrade})];
    hand.extend(plain_hand());
    // An empty Draw pile keeps Prepared's own Draw from growing the Hand.
    let (state, catalog) = load(&entry(json!(hand), 0));
    let parked = engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
    .state;
    assert!(parked.pending.is_some(), "Prepared+{upgrade} suspends");
    (parked, catalog)
}

#[test]
fn prepared_plus_offers_both_pick_orders_and_discards_in_pick_order() {
    let (parked, catalog) = play_prepared(1);
    let actions = engine::legal_actions(&parked, &catalog);
    assert_eq!(actions.len(), 6, "3 choose 2, times 2 orders");
    // The legal list already names every order: no ordered extension.
    assert_eq!(
        engine::ordered_selection_answer(&parked, &catalog, &[3, 1]).unwrap(),
        None
    );
    let (cold_state, cold_catalog) = cold(&parked, &catalog);
    assert_eq!(engine::legal_actions(&cold_state, &cold_catalog), actions);
    let mut picked = Vec::new();
    for action in &actions {
        let Action::Select { answer } = *action else {
            panic!("selection answer")
        };
        let order = engine::selected_card_uids(&parked, &catalog, answer)
            .unwrap()
            .unwrap();
        let done = engine::apply_action(&parked, &catalog, action)
            .unwrap()
            .state;
        assert!(done.pending.is_none());
        let discard = uids(&done, sts_sim::hot::PileId::Discard);
        assert_eq!(discard[..2], order[..], "Discard follows the pick order");
        picked.push(order);
    }
    picked.sort_unstable();
    assert_eq!(
        picked,
        [[1, 2], [1, 3], [2, 1], [2, 3], [3, 1], [3, 2]].map(Vec::from)
    );
}

#[test]
fn prepared_single_card_choice_is_unchanged() {
    let (parked, catalog) = play_prepared(0);
    let actions = engine::legal_actions(&parked, &catalog);
    assert_eq!(actions.len(), 3, "one answer per candidate");
    for action in &actions {
        let done = engine::apply_action(&parked, &catalog, action)
            .unwrap()
            .state;
        assert_eq!(done.history.discarded_cards_this_turn, 1);
    }
}

/// Gambler's Brew with Tingsha against a Toadpole at `monster_hp`, parked
/// on its pick.
fn use_brew_with_tingsha(monster_hp: u64) -> (HotState, Catalog) {
    let mut doc = entry(json!(plain_hand()), 6);
    doc.player
        .insert("potions".into(), json!(["GAMBLERS_BREW"]));
    doc.player
        .insert("potion_slots".into(), json!(["GAMBLERS_BREW"]));
    doc.player
        .insert("fully_unlocked_potion_pool".into(), json!(true));
    doc.player
        .insert("relics_entering".into(), json!(["RELIC.TINGSHA"]));
    doc.player.insert("tingsha".into(), json!(true));
    doc.monsters[0].insert("hp".into(), json!(monster_hp));
    let (state, catalog) = load(&doc);
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
    assert!(parked.pending.is_some());
    (parked, catalog)
}

/// Follow-up to #3075: Gambler's Brew hands its ToList'ed answer
/// (`GamblersBrew/<OnUse>d__6` RVA 0x34e3a4 IL_00d5) to DiscardAndDraw
/// (IL_00e8). A Tingsha kill on the first pick leaves the later picks in
/// Hand (CardPileCmd.Add RVA 0x3e1ba4 IL_004e-008a) while every pick is
/// still counted (DiscardAndDraw RVA 0x3e0274 IL_018e/IL_01a5), and the
/// paired Draw never runs. The ordered answer [3, 1, 2] discards Bash first.
#[test]
fn gamblers_brew_tingsha_kill_keeps_later_picks_in_hand() {
    let (parked, catalog) = use_brew_with_tingsha(3);
    let answer = engine::ordered_selection_answer(&parked, &catalog, &[3, 1, 2])
        .unwrap()
        .unwrap();
    let draw_before = uids(&parked, sts_sim::hot::PileId::Draw);
    let done = engine::apply_action(&parked, &catalog, &Action::Select { answer })
        .unwrap()
        .state;
    assert!(done.history.over);
    assert_eq!(done.monsters[0].hp, 0);
    assert_eq!(uids(&done, sts_sim::hot::PileId::Discard), [3]);
    assert_eq!(uids(&done, sts_sim::hot::PileId::Hand), [1, 2]);
    assert_eq!(uids(&done, sts_sim::hot::PileId::Draw), draw_before);
    assert_eq!(done.history.discarded_cards_this_turn, 3);
}

/// The same ordered answer against a surviving Toadpole moves every pick in
/// pick order, then draws three: the ordinary path is unchanged.
#[test]
fn gamblers_brew_nonlethal_tingsha_discards_every_pick_in_order_and_draws() {
    let (parked, catalog) = use_brew_with_tingsha(100);
    let answer = engine::ordered_selection_answer(&parked, &catalog, &[3, 1, 2])
        .unwrap()
        .unwrap();
    let done = engine::apply_action(&parked, &catalog, &Action::Select { answer })
        .unwrap()
        .state;
    assert!(!done.history.over);
    assert_eq!(done.monsters[0].hp, 91);
    assert_eq!(uids(&done, sts_sim::hot::PileId::Discard), [3, 1, 2]);
    assert_eq!(uids(&done, sts_sim::hot::PileId::Hand).len(), 3);
    assert_eq!(done.history.discarded_cards_this_turn, 3);
}

/// Once combat is over, DiscardAndDraw's entry gate (RVA 0x3e0274
/// IL_0029-0035) makes the Brew's discard a complete no-op: every pick
/// stays in Hand, nothing is counted, and nothing is drawn.
#[test]
fn gamblers_brew_after_combat_is_over_moves_and_counts_nothing() {
    let (mut parked, catalog) = use_brew_with_tingsha(100);
    parked.history.over = true;
    let answer = engine::ordered_selection_answer(&parked, &catalog, &[2, 1])
        .unwrap()
        .unwrap();
    let draw_before = uids(&parked, sts_sim::hot::PileId::Draw);
    let done = engine::apply_action(&parked, &catalog, &Action::Select { answer })
        .unwrap()
        .state;
    assert_eq!(uids(&done, sts_sim::hot::PileId::Hand), [1, 2, 3]);
    assert!(uids(&done, sts_sim::hot::PileId::Discard).is_empty());
    assert_eq!(uids(&done, sts_sim::hot::PileId::Draw), draw_before);
    assert_eq!(done.history.discarded_cards_this_turn, 0);
    assert_eq!(done.monsters[0].hp, 100);
    assert!(done.pending.is_none(), "the potion still finishes");
}

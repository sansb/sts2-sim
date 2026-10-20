//! Hot equality must agree with canonical equality (#3499 probe).
//!
//! `HotState` derives `PartialEq` and `Hash`, and the exact-DFS memo keys
//! nodes by that `Hash` (#3522). If two states that render the same canonical
//! document compare unequal hot, the memo misses a transposition and any
//! byte-level comparison (replay receipts) disagrees with the hot one. This
//! walks every eval fixture — its certified human line, then seeded random
//! walks — and requires that every reached, unfinished state decodes from its
//! own canonical document to a state `==` itself, once the one filed gap
//! (#3982 decimal scale) is erased from both sides. Fetch's played-this-turn
//! set (#3981) was the other, until the boundary projected it; dropping its
//! restore fails this test on about 110 states.
//!
//! The first run found dead listener slots: a removal shifted the live prefix
//! left and left the old last entry behind, invisible to the projection but
//! not to `==` (8% of reached states).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::boundary::HotBoundary;
use crate::canonical::CanonicalStateV2;
use crate::engine::{self, Action, LegalActionBuffer};
use crate::hot::HotState;
use crate::hot::test_diagnostics::normalize_known_gaps;

fn fights() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights")
}

fn differing(a: &HotState, b: &HotState) -> Vec<String> {
    let mut out = Vec::new();
    if a.rng != b.rng {
        out.push("rng".to_owned());
    }
    if a.piles != b.piles {
        out.push("piles".to_owned());
    }
    if a.monsters != b.monsters {
        out.push("monsters".to_owned());
    }
    if a.powers != b.powers {
        out.push("powers".to_owned());
    }
    if a.card_states != b.card_states {
        out.push("card_states".to_owned());
    }
    if a.frames != b.frames {
        out.push("frames".to_owned());
    }
    if a.pending != b.pending {
        out.push("pending".to_owned());
    }
    if a.orbs != b.orbs {
        out.push("orbs".to_owned());
    }
    if a.hp != b.hp {
        out.push("hp".to_owned());
    }
    if a.max_hp != b.max_hp {
        out.push("max_hp".to_owned());
    }
    if a.block != b.block {
        out.push("block".to_owned());
    }
    if a.gold != b.gold {
        out.push("gold".to_owned());
    }
    if a.cards_drawn_combat != b.cards_drawn_combat {
        out.push("cards_drawn_combat".to_owned());
    }
    if a.ps_strikes != b.ps_strikes {
        out.push("ps_strikes".to_owned());
    }
    if a.next_card_uid != b.next_card_uid {
        out.push("next_card_uid".to_owned());
    }
    if a.next_generated_hook_uid != b.next_generated_hook_uid {
        out.push("next_generated_hook_uid".to_owned());
    }
    if a.next_poison_uid != b.next_poison_uid {
        out.push("next_poison_uid".to_owned());
    }
    if a.energy != b.energy {
        out.push("energy".to_owned());
    }
    if a.stars != b.stars {
        out.push("stars".to_owned());
    }
    if a.innate_min_draw != b.innate_min_draw {
        out.push("innate_min_draw".to_owned());
    }
    if a.turn != b.turn {
        out.push("turn".to_owned());
    }
    if a.inky_attack_damage != b.inky_attack_damage {
        out.push("inky_attack_damage".to_owned());
    }
    if a.regalite_block_amount != b.regalite_block_amount {
        out.push("regalite_block_amount".to_owned());
    }
    if a.temp_strength != b.temp_strength {
        out.push("temp_strength".to_owned());
    }
    if a.player_phase != b.player_phase {
        out.push("player_phase".to_owned());
    }
    if a.exact_piles != b.exact_piles {
        out.push("exact_piles".to_owned());
    }
    if a.player_side_active != b.player_side_active {
        out.push("player_side_active".to_owned());
    }
    if a.reward_card_pool != b.reward_card_pool {
        out.push("reward_card_pool".to_owned());
    }
    if a.entropy_card_pool != b.entropy_card_pool {
        out.push("entropy_card_pool".to_owned());
    }
    if a.fully_unlocked_card_pool_epochs != b.fully_unlocked_card_pool_epochs {
        out.push("fully_unlocked_card_pool_epochs".to_owned());
    }
    if a.reward_card_rarity_odds != b.reward_card_rarity_odds {
        out.push("reward_card_rarity_odds".to_owned());
    }
    if a.multiplayer_ally_key != b.multiplayer_ally_key {
        out.push("multiplayer_ally_key".to_owned());
    }
    if a.history != b.history {
        out.push("history".to_owned());
    }
    for field in crate::hot::test_diagnostics::differing_fields(&a.fanouts, &b.fanouts) {
        out.push(format!("fanouts.{field}"));
    }
    out
}

fn next_rng(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut word = *seed;
    word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    word ^ (word >> 31)
}

#[test]
fn every_reached_state_equals_its_canonical_round_trip() {
    let mut ids: Vec<String> = std::fs::read_dir(fights())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    ids.sort();
    let mut legal = LegalActionBuffer::new();
    let mut events = Vec::new();
    let (mut checked, mut undecodable) = (0usize, 0usize);
    let mut mismatches: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for id in ids {
        let dir = fights().join(&id);
        let Ok(bytes) = std::fs::read(dir.join("entry.canonical.json")) else {
            continue;
        };
        let document: CanonicalStateV2 = serde_json::from_slice(&bytes).unwrap();
        let Ok(catalog) = HotBoundary::catalog_from_canonical(&document) else {
            continue;
        };
        let Ok(root) = HotBoundary::from_canonical(&document, &catalog) else {
            continue;
        };
        if engine::admit(&document, &root, &catalog).is_err() {
            continue;
        }
        let mut check = |state: &HotState, at: String| {
            // Nothing is applied to a finished fight, so its private latches
            // (e.g. Scroll of Biting's mid-phase branch bit, left set when
            // the player dies in the enemy phase) never reach a transition.
            if state.history.over {
                return;
            }
            let Ok(document) = HotBoundary::try_to_canonical(state, &catalog) else {
                undecodable += 1;
                return;
            };
            match HotBoundary::from_canonical(&document, &catalog) {
                Ok(mut decoded) => {
                    checked += 1;
                    let mut live = state.clone();
                    // The filed gap (#3982) is erased from both sides;
                    // everything else must be equal, private fields included.
                    normalize_known_gaps(&mut live);
                    normalize_known_gaps(&mut decoded);
                    if decoded != live {
                        let mut fields = differing(&decoded, &live);
                        if fields.is_empty() {
                            fields.push("an unlisted (private) field".to_owned());
                        }
                        let entry = mismatches.entry(format!("{fields:?}")).or_insert((0, at));
                        entry.0 += 1;
                    }
                }
                Err(_) => undecodable += 1,
            }
        };
        let line: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
        let mut state = root.clone();
        for (step, wire) in line["actions"].as_array().unwrap().iter().enumerate() {
            let wire: crate::exact_solve_v1::ExactSolveActionV1 =
                serde_json::from_value(wire.clone()).unwrap();
            let action = Action::try_from(wire).unwrap();
            state = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
            check(&state, format!("{id} human step {step}"));
        }
        for salt in [1u64, 2] {
            let mut rng = salt ^ u64::from_str_radix(&id[1..], 16).unwrap();
            let mut state = root.clone();
            for step in 0..80 {
                if state.history.over {
                    break;
                }
                let actions = engine::legal_actions_into(&state, &catalog, &mut legal);
                if actions.is_empty() {
                    break;
                }
                let action = actions[(next_rng(&mut rng) % actions.len() as u64) as usize];
                let Ok(next) = engine::apply_action_into(&state, &catalog, &action, &mut events)
                else {
                    break;
                };
                state = next;
                check(&state, format!("{id} walk {salt} step {step}"));
            }
        }
    }
    eprintln!("canonical round trip: {checked} states checked, {undecodable} undecodable");
    for (fields, (count, first)) in &mismatches {
        eprintln!("  {count:6} {fields} (first: {first})");
    }
    assert!(checked > 30_000, "only {checked} states checked");
    assert!(
        mismatches.is_empty(),
        "hot states differ from their canonical round trip: {mismatches:?}"
    );
}

const FETCH_UID: u32 = 14;

/// The fixture state right after `f062a3de752da212` human step 11, the play of
/// the physical Fetch uid 14 (#3981's first reached witness).
fn after_fetch_play() -> (HotState, crate::catalog::Catalog) {
    let dir = fights().join("f062a3de752da212");
    let document: CanonicalStateV2 =
        serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).unwrap()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    let line: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
    let mut events = Vec::new();
    for wire in &line["actions"].as_array().unwrap()[..=11] {
        let wire: crate::exact_solve_v1::ExactSolveActionV1 =
            serde_json::from_value(wire.clone()).unwrap();
        let action = Action::try_from(wire).unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    }
    assert!(state.fanouts.fetch_finished(FETCH_UID));
    (state, catalog)
}

/// #3981: the played-this-turn set survives the canonical round trip, and an
/// empty set is elided so documents without one are unchanged.
#[test]
fn fetch_played_this_turn_set_round_trips_mid_turn() {
    let (live, catalog) = after_fetch_play();
    let document = HotBoundary::try_to_canonical(&live, &catalog).unwrap();
    assert_eq!(
        document.player.get("fetch_finished_uids_this_turn"),
        Some(&serde_json::json!([FETCH_UID]))
    );
    let decoded = HotBoundary::from_canonical(&document, &catalog).unwrap();
    assert_eq!(decoded.fanouts.fetch_finished_uids(), &[FETCH_UID]);
    assert!(decoded == live);

    let mut forgot = live.clone();
    forgot.fanouts.reset_fetch_finished();
    let document = HotBoundary::try_to_canonical(&forgot, &catalog).unwrap();
    assert!(
        !document
            .player
            .contains_key("fetch_finished_uids_this_turn")
    );
}

/// #3981: replaying the same physical Fetch later that turn does not draw
/// from a reloaded state, exactly as in the live one; a state that forgot
/// the set draws (the regression this witness discriminates).
#[test]
fn reloaded_fetch_second_play_this_turn_does_not_draw() {
    use crate::hot::PileId;
    let (live, catalog) = after_fetch_play();
    let document = HotBoundary::try_to_canonical(&live, &catalog).unwrap();
    let decoded = HotBoundary::from_canonical(&document, &catalog).unwrap();
    let mut forgot = live.clone();
    forgot.fanouts.reset_fetch_finished();

    let replay = |mut state: HotState| -> HotState {
        // Return the Fetch from Discard to Hand and pay for it, so the same
        // physical card is played a second time this turn.
        let discard = state.piles.get_mut(PileId::Discard).make_mut();
        let index = discard
            .iter()
            .position(|card| card.uid == FETCH_UID)
            .expect("Fetch is in Discard after its play");
        let card = discard.remove(index);
        state.piles.get_mut(PileId::Hand).make_mut().push(card);
        state.energy = 9;
        let action = Action::Play {
            uid: FETCH_UID,
            target: Some(0),
            selection: engine::SelectionRef::NONE,
        };
        engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap()
    };
    let hand = |state: &HotState| state.piles.get(PileId::Hand).len();
    let before = hand(&live);
    let live_after = replay(live);
    let decoded_after = replay(decoded);
    let forgot_after = replay(forgot);
    // The Fetch leaves Hand again and draws nothing.
    assert_eq!(hand(&live_after), before);
    assert!(decoded_after == live_after);
    assert_eq!(
        hand(&forgot_after),
        before + 1,
        "the forgotten set draws one"
    );
}

/// #3981: a forged document naming a uid that is not a live Fetch refuses by
/// name rather than being dropped or accepted.
#[test]
fn forged_fetch_played_uid_refuses_by_name() {
    use crate::boundary::BoundaryRefusal;
    use crate::hot::PileId;
    let (live, catalog) = after_fetch_play();
    let document = HotBoundary::try_to_canonical(&live, &catalog).unwrap();
    let card_id = |uid: u32| {
        PileId::ALL.iter().find_map(|pile| {
            live.piles
                .get(*pile)
                .as_slice()
                .iter()
                .find(|card| card.uid == uid)
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
        })
    };
    let other = (0..live.next_card_uid)
        .find(|uid| card_id(*uid).is_some_and(|id| id != crate::ids::CardId::Fetch))
        .unwrap();
    for (forged, reason) in [
        (
            serde_json::json!([other]),
            "uid names a live card that is not Fetch",
        ),
        (
            serde_json::json!([live.next_card_uid + 5]),
            "uid was never allocated",
        ),
        (
            serde_json::json!([FETCH_UID, FETCH_UID]),
            "repeated physical uid",
        ),
        (
            serde_json::json!([-1]),
            "expected a nonnegative 32-bit physical uid",
        ),
    ] {
        let mut document = document.clone();
        document
            .player
            .insert("fetch_finished_uids_this_turn".to_owned(), forged.clone());
        match HotBoundary::from_canonical(&document, &catalog) {
            Err(BoundaryRefusal::UnrepresentableValue { field, detail, .. }) => {
                assert_eq!(field, "fetch_finished_uids_this_turn", "{forged}");
                assert_eq!(detail, reason, "{forged}");
            }
            other => panic!("{forged} was not refused by name: {:?}", other.err()),
        }
    }
}

/// #3981 review: a played Fetch that later leaves combat (a Duplication
/// clone's `RemoveFromCombat`, say) leaves its uid in the hot set, where it is
/// inert. The document keeps it and the reload equals the live state, rather
/// than projecting a document the engine refuses to load.
#[test]
fn a_played_fetch_that_left_combat_still_round_trips() {
    use crate::hot::PileId;
    let (mut live, catalog) = after_fetch_play();
    let discard = live.piles.get_mut(PileId::Discard).make_mut();
    let index = discard
        .iter()
        .position(|card| card.uid == FETCH_UID)
        .unwrap();
    discard.remove(index);
    assert_eq!(
        crate::engine::play::unique_live_card_location(&live, FETCH_UID),
        Ok(None)
    );
    let document = HotBoundary::try_to_canonical(&live, &catalog).unwrap();
    assert_eq!(
        document.player.get("fetch_finished_uids_this_turn"),
        Some(&serde_json::json!([FETCH_UID]))
    );
    let decoded = HotBoundary::from_canonical(&document, &catalog).unwrap();
    assert!(decoded == live);
}

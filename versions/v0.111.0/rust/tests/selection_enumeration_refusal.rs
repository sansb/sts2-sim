//! #2985: a pick over a Hexed enchanted/plain twin, and what happens when a
//! selection's enumeration refuses.
//!
//! `issue2985_headbutt_hexed_discard.json` is run KD13JGCDPB3U fight 19
//! (Knights elite), parked on Headbutt's Discard pick
//! (`["select", "discard", 1, 1, null, ["move", "draw", "top"]]`) with two
//! Hexed Stokes in Discard: uid 3 enchanted with Steady, uid 42 bare. Python's
//! payload tuple cannot order the pair, and `legal_actions` used to swallow
//! that refusal into an empty list, which the search reported as
//! "no legal rollout actions". Native `CardSelectCmd.FromCombatPile` never
//! compares the candidates (see `candidate_order_is_label_only` in
//! `src/engine/selection.rs` for the IL), so a one-card pick now enumerates
//! both cards.
//!
//! `issue2985_prepared_enchanted_sly_twins.json` holds the same kind of pair
//! under Prepared+'s two-card Discard. That pick used to refuse too, because
//! the enumeration applied its canonical order to game state; since #2524
//! every pick order is its own answer, so the candidate order only numbers
//! them and the pick enumerates.
//!
//! `issue2524_gamblers_brew_enchanted_sly_twins.json` is the same Hand under
//! Gambler's Brew, whose searched surface is still one canonical
//! representative per subset (#2524): that enumeration still refuses, and the
//! refusal reaches the caller: `legal_actions_checked` returns it, and the
//! search prunes it instead of aborting.
use serde_json::Value;
use std::process::Command;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, EngineRefusal, LegalActionBuffer, SelectionAnswer, SelectionRef},
    hot::{HotState, PileId},
};

const HEADBUTT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/issue2985_headbutt_hexed_discard.json"
);
const PREPARED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/issue2985_prepared_enchanted_sly_twins.json"
);
const GAMBLERS_BREW: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/issue2524_gamblers_brew_enchanted_sly_twins.json"
);

fn load(path: &str) -> (HotState, Catalog) {
    let doc: CanonicalStateV2 =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn uids(state: &HotState, pile: PileId) -> Vec<u32> {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect()
}

fn search(entry: &str, method: &str, seed: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_sts-sim"))
        .args([
            "search", entry, method, seed, "600", "1.414", "0.05", "200", "20",
        ])
        .output()
        .expect("run sts-sim search");
    assert!(
        output.status.success(),
        "{method}/{seed} search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("search JSON")
}

#[test]
fn headbutt_offers_both_hexed_stoke_twins_and_moves_the_chosen_one() {
    let (state, catalog) = load(HEADBUTT);
    assert_eq!(uids(&state, PileId::Discard), [3, 42]);
    assert_eq!(uids(&state, PileId::Play), [44]);
    let mut buffer = LegalActionBuffer::new();
    let actions = engine::legal_actions_checked(&state, &catalog, &mut buffer)
        .unwrap()
        .to_vec();
    assert_eq!(
        actions,
        [0, 1].map(|index| Action::Select {
            answer: SelectionAnswer::OptionIndex(index)
        })
    );
    assert_eq!(engine::legal_actions(&state, &catalog), actions);
    // The bare twin sorts first and the subset expansion publishes the
    // singletons in reverse; either way the order only numbers the answers.
    for (action, chosen, left) in [(actions[0], 3, 42), (actions[1], 42, 3)] {
        let Action::Select { answer } = action else {
            unreachable!()
        };
        assert_eq!(
            engine::selected_card_uids(&state, &catalog, answer).unwrap(),
            Some(vec![chosen])
        );
        let next = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap();
        // Headbutt (uid 44) follows the unchosen twin into Discard.
        assert_eq!(uids(&next, PileId::Discard), [left, 44], "chose {chosen}");
        assert!(
            uids(&next, PileId::Draw).contains(&chosen),
            "chose {chosen}"
        );
        assert!(!uids(&next, PileId::Draw).contains(&left), "chose {chosen}");
    }
}

#[test]
fn headbutt_fight_19_search_no_longer_aborts() {
    for (method, seed) in [("uct", "1"), ("random", "1"), ("uct", "4"), ("random", "4")] {
        let result = search(HEADBUTT, method, seed);
        assert_eq!(result["playouts"], 200, "{method}/{seed}");
        assert_eq!(result["refusals"], 0, "{method}/{seed}");
        assert_eq!(result["best"]["won"], true, "{method}/{seed}");
    }
}

#[test]
fn prepared_plus_enumerates_every_order_of_the_twins_python_cannot_order() {
    let (root, catalog) = load(PREPARED);
    let play = Action::Play {
        uid: 0,
        target: None,
        selection: SelectionRef::NONE,
    };
    let parked = engine::apply_action_into(&root, &catalog, &play, &mut Vec::new()).unwrap();
    assert!(parked.pending.is_some(), "Prepared+ parks its Discard pick");
    let mut buffer = LegalActionBuffer::new();
    let actions = engine::legal_actions_checked(&parked, &catalog, &mut buffer)
        .unwrap()
        .to_vec();
    // Prepared+ draws two first, so five Hand candidates (the twins 1 and 2,
    // Strike 3, and the drawn 4 and 5) choose two, in both pick orders.
    assert_eq!(actions.len(), 20);
    let mut picked = actions
        .iter()
        .map(|action| {
            let Action::Select { answer } = *action else {
                unreachable!()
            };
            engine::selected_card_uids(&parked, &catalog, answer)
                .unwrap()
                .unwrap()
        })
        .collect::<Vec<_>>();
    picked.sort_unstable();
    picked.dedup();
    assert_eq!(picked.len(), 20, "every answer is a distinct pick order");
    // The twins Python could not order are offered both ways round.
    assert!(picked.contains(&vec![1, 2]) && picked.contains(&vec![2, 1]));
}

fn use_gamblers_brew() -> (HotState, HotState, Catalog) {
    let (root, catalog) = load(GAMBLERS_BREW);
    let potion = Action::UsePotion {
        slot: 0,
        target: None,
    };
    let parked = engine::apply_action_into(&root, &catalog, &potion, &mut Vec::new()).unwrap();
    assert!(parked.pending.is_some(), "Gambler's Brew parks its pick");
    assert!(!parked.history.over);
    (root, parked, catalog)
}

#[test]
fn an_enumeration_refusal_is_returned_never_an_empty_list() {
    let (root, parked, catalog) = use_gamblers_brew();
    // The unchecked enumerator still reads the refusal as no actions ...
    assert!(engine::legal_actions(&parked, &catalog).is_empty());
    // ... and the checked one names it.
    let mut buffer = LegalActionBuffer::new();
    assert_eq!(
        engine::legal_actions_checked(&parked, &catalog, &mut buffer),
        Err(EngineRefusal::MalformedArgs(
            "selection enchantment payload order"
        ))
    );
    // A reused buffer does not carry the refusal into a clean state.
    assert!(
        !engine::legal_actions_checked(&root, &catalog, &mut buffer)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_live_state_with_nothing_to_enumerate_is_a_refusal_and_a_terminal_one_is_not() {
    let (mut state, catalog) = load(HEADBUTT);
    // A continuation stack with no pending selection enumerates nothing and
    // names no more specific refusal.
    state.pending = None;
    assert!(!state.frames.is_empty());
    assert!(engine::legal_actions(&state, &catalog).is_empty());
    let mut buffer = LegalActionBuffer::new();
    assert_eq!(
        engine::legal_actions_checked(&state, &catalog, &mut buffer),
        Err(EngineRefusal::NoLegalActions)
    );
    state.history.over = true;
    assert_eq!(
        engine::legal_actions_checked(&state, &catalog, &mut buffer),
        Ok(&[][..])
    );
}

#[test]
fn search_prunes_an_enumeration_refusal_instead_of_aborting() {
    for (method, seed) in [("uct", "1"), ("random", "1")] {
        let result = search(GAMBLERS_BREW, method, seed);
        assert_eq!(result["playouts"], 200, "{method}/{seed}");
        let refusals = result["refusals"].as_u64().unwrap();
        assert!(refusals > 0, "{method}/{seed}");
        let first = &result["first_refusal"];
        assert_eq!(first["status"], "refused");
        assert_eq!(
            first["detail"],
            "MalformedArgs(\"selection enchantment payload order\")"
        );
        // The witness line ends on the Gambler's Brew use whose pick refused.
        let actions = first["actions"].as_array().unwrap();
        assert_eq!(actions.last().unwrap()["kind"], "potion", "{method}/{seed}");
        if method == "uct" {
            // A node whose enumeration refused is marked and becomes a dead
            // end: revisits back up the refused reward without re-noting it.
            assert!(
                result["refused_playouts"].as_u64().unwrap() > refusals,
                "{result}"
            );
        }
    }
}

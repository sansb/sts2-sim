//! #3877 / #3914 — two states the engine reaches and could not take back.
//!
//! The composition sweep reloads sampled states of each line. Two classes
//! were states the engine itself had produced and then refused as a
//! document:
//!
//! * `entry needs unsupported orb queue state` (544 roots). The opening
//!   seeds Voltaic's `lightning_channeled` count when the fight's closure
//!   holds a Voltaic, which a Colorless Potion's pool does. Once the potion
//!   is used, a catalog built from the document no longer holds one, and
//!   admission read "a tracked count without a Voltaic in the closure" as
//!   malformed.
//! * `invalid state for power "call_of_the_void"` (11 roots). A live Call of
//!   the Void had no arm in admission's player-power validity match.
//!
//! Neither is a claim about the game: both are the engine's own bookkeeping
//! round-tripping. Each test walks to the state through public actions and
//! reloads it.

#[allow(dead_code)]
#[path = "../examples/composition_sweep/sweep.rs"]
mod sweep;

use serde_json::Value;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::{Catalog, GameBuild},
    engine::{self, Action},
    entry::{self, EntryInput, EntryRequest, RootOutcome},
    hot::HotState,
};

fn open(
    character: &str,
    cards: &[&str],
    potions: &[&str],
) -> (CanonicalStateV2, HotState, Catalog) {
    let character = sweep::Character::named(character).unwrap();
    let cards: Vec<String> = cards.iter().map(|card| card.to_string()).collect();
    let potions: Vec<String> = potions.iter().map(|potion| potion.to_string()).collect();
    let root = sweep::custom_root(&cards, &[], &potions, Some("TOADPOLES_WEAK")).unwrap();
    let save = sweep::save_json(character, &root);
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: Some("ENCOUNTER.TOADPOLES_WEAK"),
        node_type: Some("monster"),
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let RootOutcome::Opened(opening) = entry::build_root(&request, None) else {
        panic!("the synthetic root opens");
    };
    let doc: CanonicalStateV2 = opening.document.clone();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (doc, state, catalog)
}

/// Project, rebuild the catalog from the document alone, load, admit, and
/// project again.
fn reload(state: &HotState, catalog: &Catalog) -> CanonicalStateV2 {
    let doc = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let reloaded = HotBoundary::from_canonical(&doc, &reloaded_catalog).unwrap();
    engine::admit(&doc, &reloaded, &reloaded_catalog)
        .unwrap_or_else(|refusal| panic!("the engine's own state is not admitted: {refusal}"));
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog).unwrap(),
        doc
    );
    doc
}

/// The first legal action whose debug form names `kind`.
fn first(state: &HotState, catalog: &Catalog, kind: &str) -> Action {
    engine::legal_actions(state, catalog)
        .iter()
        .find(|action| format!("{action:?}").starts_with(kind))
        .cloned()
        .unwrap_or_else(|| panic!("a legal {kind}"))
}

fn names_voltaic(catalog: &Catalog) -> bool {
    catalog
        .specs()
        .any(|spec| spec.identity.id == sts_sim::ids::CardId::Voltaic)
}

/// An Ironclad holding a Colorless Potion: the root tracks the Lightning
/// count because the potion's pool holds a Voltaic. After the potion is
/// used and its pick taken, the document's own closure has none, the count
/// is still there, and the state reloads.
#[test]
fn a_tracked_lightning_count_outlives_the_potion_that_put_voltaic_in_the_closure() {
    let (doc, mut state, catalog) = open("IRONCLAD", &[], &["COLORLESS_POTION"]);
    assert!(names_voltaic(&catalog), "the potion's pool holds a Voltaic");
    assert_eq!(doc.player.get("lightning_channeled"), Some(&Value::from(0)));

    for kind in ["UsePotion", "Select"] {
        let action = first(&state, &catalog, kind);
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap()
            .state;
    }
    let after = reload(&state, &catalog);
    assert_eq!(
        after.player.get("lightning_channeled"),
        Some(&Value::from(0))
    );
    let rebuilt = HotBoundary::catalog_from_canonical(&after).unwrap();
    // The picked card is the only way a Voltaic could still be named.
    // The pick here is not a Voltaic, so nothing names one any more.
    assert!(after.piles["hand"].iter().all(|card| card.id != "VOLTAIC"));
    assert!(!names_voltaic(&rebuilt));
}

/// A played Call of the Void leaves a live power, at one and at two plays,
/// and the state reloads.
#[test]
fn a_live_call_of_the_void_reloads() {
    let (_, mut state, catalog) = open(
        "NECROBINDER",
        &["CALL_OF_THE_VOID+0", "CALL_OF_THE_VOID+1"],
        &[],
    );
    let mut doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    // Both copies in the Hand, and the Energy to play them.
    for slot in 0..2 {
        let is_it = |card: &sts_sim::canonical::CanonicalCardV2| card.id == "CALL_OF_THE_VOID";
        if doc.piles["hand"][slot..].iter().any(is_it) {
            let at = slot + doc.piles["hand"][slot..].iter().position(is_it).unwrap();
            doc.piles.get_mut("hand").unwrap().swap(slot, at);
            continue;
        }
        let at = doc.piles["draw"].iter().position(is_it).unwrap();
        let hand = doc.piles["hand"][slot].clone();
        let drawn = std::mem::replace(&mut doc.piles.get_mut("draw").unwrap()[at], hand);
        doc.piles.get_mut("hand").unwrap()[slot] = drawn;
    }
    doc.player.insert("energy".to_string(), 9.into());
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();

    for amount in 1..=2 {
        let uid = HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .piles["hand"]
            .iter()
            .find(|card| card.id == "CALL_OF_THE_VOID")
            .and_then(|card| card.uid)
            .unwrap() as u32;
        let action = Action::Play {
            uid,
            target: None,
            selection: engine::SelectionRef::NONE,
        };
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap()
            .state;
        let after = reload(&state, &catalog);
        assert_eq!(
            after.player.get("call_of_the_void"),
            Some(&Value::from(amount))
        );
    }
}

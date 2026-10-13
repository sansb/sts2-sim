//! #3877 — a Glowwater Potion whose Draw parks on a Stratagem choice is a
//! state the fight continues from.
//!
//! The composition sweep found it as a live state with no legal action, for
//! every character: Glowwater beside anything that puts a Stratagem in play
//! (Orange Dough, Cosmic Concoction, the card itself). Glowwater exhausts the
//! Hand and then draws ten (`GlowwaterPotion/<OnUse>d__10::MoveNext` RVA
//! `0x34e764`: `CardCmd::Exhaust` per snapshot card at `IL_009e`, then
//! `CardPileCmd::Draw` at `IL_014a`; `get_CanonicalVars` `0xacb35` is
//! `CardsVar(10)`). Ten cards empty the Draw pile, the reshuffle fires
//! `StratagemPower::AfterShuffle` (`0xa88e8`), and the potion parks on that
//! choice with its Draw open. `potions::resume_after_draw` resumes that pair;
//! `play::rooted_potion_draw_suffix_is_exact` did not list it, so the engine
//! refused to enumerate the state it had just produced.
//!
//! The fixture is the document before the potion, as the sweep's root
//! `IRONCLAD / STRATAGEM & GLOWWATER_POTION` reached it: Stratagem live, two
//! Defends in Hand, four Strikes in Draw, two Defends in Discard. IL-read
//! only; no recorded fight holds a Glowwater reshuffling under Stratagem.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, LegalActionBuffer, SelectionAnswer},
    hot::{HotState, PileId},
};

const FIXTURE: &str =
    include_str!("../fixtures/issue3877_glowwater_before_stratagem_reshuffle.json");

fn load() -> (HotState, Catalog) {
    let doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn uids(state: &HotState, pile: PileId) -> Vec<u32> {
    let mut uids: Vec<u32> = state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect();
    uids.sort_unstable();
    uids
}

fn drink(state: &HotState, catalog: &Catalog) -> HotState {
    let action = Action::UsePotion {
        slot: 0,
        target: None,
    };
    engine::apply_action(state, catalog, &action).unwrap().state
}

/// The parked potion offers Stratagem's one-of-two choice and both answers
/// apply: the Hand is exhausted, the four Draw cards are drawn, the chosen
/// reshuffled card goes to Hand, and the Draw then takes the other one.
#[test]
fn a_glowwater_parked_on_the_stratagem_choice_offers_and_applies_every_answer() {
    let (entry, catalog) = load();
    let hand_before = uids(&entry, PileId::Hand);
    let draw_before = uids(&entry, PileId::Draw);
    let discard_before = uids(&entry, PileId::Discard);
    assert_eq!(
        (hand_before.len(), draw_before.len(), discard_before.len()),
        (2, 4, 2)
    );

    let parked = drink(&entry, &catalog);
    assert!(!parked.history.over);
    assert!(parked.pending.is_some(), "the potion parks on the choice");
    assert_eq!(uids(&parked, PileId::Exhaust), hand_before);
    assert_eq!(uids(&parked, PileId::Hand), draw_before);
    assert_eq!(uids(&parked, PileId::Draw), discard_before);

    let mut buffer = LegalActionBuffer::new();
    let offered = engine::legal_actions_checked(&parked, &catalog, &mut buffer)
        .expect("a parked Glowwater enumerates its Stratagem choice")
        .to_vec();
    assert_eq!(offered.len(), 2, "one answer per reshuffled card");
    let mut chosen = Vec::new();
    for (index, action) in offered.iter().enumerate() {
        let answer = SelectionAnswer::OptionIndex(index as u32);
        assert_eq!(*action, Action::Select { answer });
        let picked = engine::selected_card_uids(&parked, &catalog, answer)
            .unwrap()
            .expect("a Stratagem answer names a physical card");
        assert_eq!(picked.len(), 1);
        chosen.push(picked[0]);
        let done = engine::apply_action(&parked, &catalog, action)
            .unwrap()
            .state;
        assert!(done.pending.is_none());
        // Ten were asked for and six exist: every card ends in Hand.
        let mut all = draw_before.clone();
        all.extend(&discard_before);
        all.sort_unstable();
        assert_eq!(uids(&done, PileId::Hand), all);
        assert!(uids(&done, PileId::Draw).is_empty());
        assert!(uids(&done, PileId::Discard).is_empty());
        assert_eq!(uids(&done, PileId::Exhaust), hand_before);
        // The potion is spent and the fight goes on.
        let mut after = LegalActionBuffer::new();
        let next = engine::legal_actions_checked(&done, &catalog, &mut after).unwrap();
        assert!(!next.is_empty());
        assert!(
            !next
                .iter()
                .any(|action| matches!(action, Action::UsePotion { .. }))
        );
    }
    chosen.sort_unstable();
    assert_eq!(chosen, discard_before, "each reshuffled card is one answer");
}

/// The parked state is a document: it projects, reloads and offers the same
/// choice. This is the reader that did not list Glowwater.
#[test]
fn the_parked_glowwater_reloads_as_the_same_choice() {
    let (entry, catalog) = load();
    let parked = drink(&entry, &catalog);
    let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let reloaded = HotBoundary::from_canonical(&document, &reloaded_catalog).unwrap();
    engine::admit(&document, &reloaded, &reloaded_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog)
            .unwrap()
            .differential_digest(),
        document.differential_digest()
    );
    assert_eq!(engine::legal_actions(&reloaded, &reloaded_catalog).len(), 2);
}

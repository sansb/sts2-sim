//! #3877 — a Fetch whose Draw parks on a Stratagem choice is a state the
//! fight continues from.
//!
//! The composition sweep found it as a live state with no legal action:
//! Fetch beside Stratagem (or beside Orange Dough, Toolbox or a Cosmic
//! Concoction that generated one), for a Necrobinder. Fetch's Draw meets an
//! empty Draw pile, the reshuffle fires `StratagemPower::AfterShuffle`, and
//! the play parks on that choice with its Draw still open. The parked Draw
//! requests one card (`Fetch::get_CanonicalVars` RVA `0xdfa45` `IL_0017`-
//! `IL_0019`, `CardsVar(1)`; `Fetch/<OnPlay>d__9::MoveNext` `0x39de84`
//! `IL_0103`-`IL_0120`), and `play::cardplay_draw_expected_requested` still
//! expected the frozen registry's three, so the engine refused to enumerate
//! the state it had just produced.
//!
//! The fixture is the document before that play, as the sweep's root
//! `NECROBINDER / FETCH & STRATAGEM` reached it: Stratagem live, Draw empty,
//! five cards in Discard, Fetch in Hand. IL-read only; no recorded fight
//! holds a Fetch reshuffling under Stratagem.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, LegalActionBuffer, SelectionAnswer, SelectionRef},
    hot::{HotState, PileId},
};

const FIXTURE: &str = include_str!("../fixtures/issue3877_fetch_before_stratagem_reshuffle.json");
const FETCH_UID: u32 = 9;

fn load(upgrade: i64) -> (HotState, Catalog) {
    let mut doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let fetch = doc
        .piles
        .get_mut("hand")
        .unwrap()
        .iter_mut()
        .find(|card| card.uid == Some(u64::from(FETCH_UID)))
        .unwrap();
    fetch.upgrade = upgrade;
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

fn play_fetch(state: &HotState, catalog: &Catalog) -> HotState {
    let action = Action::Play {
        uid: FETCH_UID,
        target: Some(1),
        selection: SelectionRef::NONE,
    };
    engine::apply_action(state, catalog, &action).unwrap().state
}

/// The parked state offers Stratagem's one-of-five choice, and every answer
/// applies: the chosen card goes to Hand, then Fetch's own Draw takes exactly
/// one more card off the reshuffled pile, at both upgrade levels.
#[test]
fn a_fetch_parked_on_the_stratagem_choice_offers_and_applies_every_answer() {
    for upgrade in [0, 1] {
        let (entry, catalog) = load(upgrade);
        assert!(uids(&entry, PileId::Draw).is_empty());
        let hand_before = uids(&entry, PileId::Hand).len();
        let parked = play_fetch(&entry, &catalog);
        assert!(!parked.history.over);
        assert!(parked.pending.is_some(), "the play parks on the choice");
        let reshuffled = uids(&parked, PileId::Draw);
        assert_eq!(reshuffled.len(), 5);

        let mut buffer = LegalActionBuffer::new();
        let offered = engine::legal_actions_checked(&parked, &catalog, &mut buffer)
            .expect("a parked Fetch enumerates its Stratagem choice")
            .to_vec();
        assert_eq!(offered.len(), 5, "one answer per reshuffled card");
        let mut chosen = Vec::new();
        for (index, action) in offered.iter().enumerate() {
            let answer = SelectionAnswer::OptionIndex(index as u32);
            assert_eq!(*action, Action::Select { answer });
            let picked = engine::selected_card_uids(&parked, &catalog, answer)
                .unwrap()
                .expect("a Stratagem answer names a physical card");
            assert_eq!(picked.len(), 1);
            let done = engine::apply_action(&parked, &catalog, action)
                .unwrap()
                .state;
            assert!(done.pending.is_none());
            let hand = uids(&done, PileId::Hand);
            // Fetch left Hand; Stratagem's pick and Fetch's one card arrived.
            assert_eq!(hand.len(), hand_before - 1 + 2, "upgrade {upgrade}");
            assert!(hand.contains(&picked[0]));
            assert_eq!(uids(&done, PileId::Draw).len(), 3);
            assert!(uids(&done, PileId::Discard).contains(&FETCH_UID));
            // The fight goes on from there.
            let mut after = LegalActionBuffer::new();
            assert!(
                !engine::legal_actions_checked(&done, &catalog, &mut after)
                    .unwrap()
                    .is_empty()
            );
            chosen.push(picked[0]);
        }
        chosen.sort_unstable();
        let mut all = reshuffled;
        all.sort_unstable();
        assert_eq!(chosen, all, "each reshuffled card is one answer");
    }
}

/// The parked state is a document: it projects, reloads and offers the same
/// choice. This is the reader the stale count refused.
#[test]
fn the_parked_fetch_reloads_as_the_same_choice() {
    let (entry, catalog) = load(0);
    let parked = play_fetch(&entry, &catalog);
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
    assert_eq!(engine::legal_actions(&reloaded, &reloaded_catalog).len(), 5);
}

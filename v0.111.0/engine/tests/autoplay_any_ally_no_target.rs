//! #3877 — an AutoPlay of an AnyAlly card with no ally to target routes the
//! card without playing it.
//!
//! The composition sweep met `a targeted card was played without a target`
//! on legal lines: Mayhem (or Havoc, or Cascade) turning up Believe in You,
//! Constellation, Coordinate or Imitation Learning in a solo fight.
//! `CardCmd/<AutoPlay>d__0::MoveNext` (RVA `0x3df9d4`) handles
//! `TargetType == 6` at IL_0315-IL_0390: the pool is the living Players other
//! than the owner (`<AutoPlay>b__0` `0x3df4bc`), `CombatTargets.NextItem` is
//! rolled over it, and a null target goes to `MoveToResultPileWithoutPlaying`.
//! The engine had that exit for AnyEnemy only.
//!
//! The fixture is the document before an `EndTurn`, as the sweep's root
//! `IRONCLAD / BELIEVE_IN_YOU & MAYHEM` reached it: Mayhem live, Believe in
//! You in Hand, and a reshuffle that puts it on top of Draw for the next
//! turn's Mayhem. The `native_*` tests compare the engine with the game itself.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, Event, LegalActionBuffer},
    hot::{HotState, PileId, RngStream},
};

const FIXTURE: &str = include_str!("../fixtures/issue3877_mayhem_turns_up_believe_in_you.json");
const BELIEVE_IN_YOU_UID: u32 = 8;

fn load() -> (HotState, Catalog) {
    let doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn holds(state: &HotState, pile: PileId, uid: u32) -> bool {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .any(|card| card.uid == uid)
}

/// Mayhem turns the card up at the next turn start. It is discarded with no
/// body: no CardPlayed event names it, nothing is drawn from CombatTargets
/// for it, and the turn goes on.
#[test]
fn mayhem_discards_an_any_ally_card_it_cannot_target() {
    let (entry, catalog) = load();
    assert!(holds(&entry, PileId::Hand, BELIEVE_IN_YOU_UID));
    let targets_before = entry.rng.get(RngStream::Targets);
    let done = engine::apply_action(&entry, &catalog, &Action::EndTurn).unwrap();
    let state = done.state;
    assert!(!state.history.over);
    assert!(
        holds(&state, PileId::Discard, BELIEVE_IN_YOU_UID),
        "routed to its result pile"
    );
    assert!(
        !done.events.iter().any(
            |event| matches!(event, Event::CardPlayed { uid, .. } if *uid == BELIEVE_IN_YOU_UID)
        ),
        "without playing"
    );
    assert_eq!(
        state.rng.get(RngStream::Targets),
        targets_before,
        "an empty pool draws nothing"
    );
    // Believe in You gives its target Energy; nobody got any.
    assert_eq!(state.energy, 3);
    let mut buffer = LegalActionBuffer::new();
    assert!(
        engine::legal_actions_checked(&state, &catalog, &mut buffer)
            .unwrap()
            .contains(&Action::EndTurn)
    );
}

// ---------------------------------------------------------------------------
// Native witnesses: the engine against the game.
// ---------------------------------------------------------------------------

/// The game's own results, recorded by `tests/native/
/// issue3877_autoplay_any_ally_no_target.py` on the archived v0.111.0
/// `sts2.dll` through the headless harness. Each case is keyed
/// `<mode>:<CARD>[:osty]`: `manualself` reads `CardModel.CanPlay`, `auto`
/// calls `CardCmd.AutoPlay` on a Hand card with no target, `mayhem` lets a
/// live `MayhemPower` turn the card up at the next turn start. `:osty` is the
/// same on a Necrobinder with a living Osty.
const NATIVE: &str = include_str!("native/issue3877_autoplay_any_ally_no_target.json");

/// Every AnyAlly card the native run covered: Believe in You, and the four
/// cards whose solo decks admission refused until #3877.
/// Where the card sits in the Draw pile for the Mayhem half.
const MAYHEM_SLOT: usize = 5;

const CARDS: [&str; 5] = ["BELIEVE_IN_YOU", "BLAZE", "DEMONIC_SHIELD", "FADE", "TUTOR"];

fn native(case: &str) -> serde_json::Value {
    let all: serde_json::Value = serde_json::from_str(NATIVE).unwrap();
    let run = all[case].clone();
    assert_eq!(run["build"]["version"], "v0.111.0", "{case}");
    assert_eq!(run["target_type"], "AnyAlly", "{case}");
    run
}

/// The fixture with its Believe in You replaced by `card`, the player's
/// Energy refilled so nothing but the card's own rule decides playability,
/// and optionally a living Osty.
fn load_card(card: &str, osty: bool) -> (HotState, Catalog) {
    load_card_at(card, osty, false)
}

/// [`load_card`], with the card in Hand or, for `under_mayhem`, sixth from
/// the top of an eight-card Draw pile: the next turn draws five with no
/// reshuffle and Mayhem turns the card up whatever its id sorts as.
fn load_card_at(card: &str, osty: bool, under_mayhem: bool) -> (HotState, Catalog) {
    let mut doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    if under_mayhem {
        let mut discard = std::mem::take(doc.piles.get_mut("discard").unwrap());
        let hand = doc.piles.get_mut("hand").unwrap();
        let at = hand
            .iter()
            .position(|held| held.uid == Some(u64::from(BELIEVE_IN_YOU_UID)))
            .unwrap();
        let mut moved = hand.remove(at);
        moved.id = card.to_string();
        let draw = doc.piles.get_mut("draw").unwrap();
        draw.append(&mut discard);
        draw.insert(MAYHEM_SLOT, moved);
    }
    if let Some(held) = doc
        .piles
        .get_mut("hand")
        .unwrap()
        .iter_mut()
        .find(|held| held.uid == Some(u64::from(BELIEVE_IN_YOU_UID)))
    {
        held.id = card.to_string();
    }
    // Absent is the default: a full 3.
    doc.player.remove("energy");
    if osty {
        doc.player
            .insert("ally".to_string(), serde_json::json!(["OSTY", 1, 1]));
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap_or_else(|refusal| panic!("{card}: {refusal:?}"));
    (state, catalog)
}

fn pile_of(state: &HotState, uid: u32) -> &'static str {
    [
        (PileId::Hand, "Hand"),
        (PileId::Draw, "Draw"),
        (PileId::Discard, "Discard"),
        (PileId::Exhaust, "Exhaust"),
    ]
    .into_iter()
    .find(|(pile, _)| holds(state, *pile, uid))
    .map_or("none", |(_, name)| name)
}

/// One card, with or without Osty, against its native cases.
fn check(card: &str, osty: bool) {
    let suffix = if osty { ":osty" } else { "" };
    let (entry, catalog) = load_card(card, osty);

    // Manual play. Native `CanPlay` is false with `NoLivingAllies`; the
    // engine offers no Play of the card.
    let manual = native(&format!("manualself:{card}{suffix}"));
    assert_eq!(manual["can_play"], false, "{card}{suffix}");
    assert_eq!(manual["unplayable_reason"], "NoLivingAllies");
    let mut buffer = LegalActionBuffer::new();
    let legal = engine::legal_actions_checked(&entry, &catalog, &mut buffer).unwrap();
    assert!(
        !legal
            .iter()
            .any(|action| matches!(action, Action::Play { uid, .. } if *uid == BELIEVE_IN_YOU_UID)),
        "{card}{suffix}: native CanPlay is false"
    );
    assert!(legal.contains(&Action::EndTurn));

    // AutoPlay. Native moves the card to its result pile with no CardPlay
    // and no RNG draw; Mayhem does the same at the next turn start.
    let auto_case = format!("auto:{card}{suffix}");
    let all: serde_json::Value = serde_json::from_str(NATIVE).unwrap();
    if all.get(&auto_case).is_none() {
        // Not every card was run with Osty; the cards that were agree with
        // their Osty-less runs.
        assert!(osty, "{auto_case} is missing");
        return;
    }
    let auto = native(&auto_case);
    assert_eq!(auto["rng_delta"], serde_json::json!({}), "{auto_case}");
    let history = auto["history_types"].as_object().unwrap();
    assert!(!history.contains_key("CardPlayStartedEntry"), "{auto_case}");
    let native_pile = auto["card_pile_after"].as_str().unwrap();
    assert_eq!(
        history.contains_key("CardExhaustedEntry"),
        native_pile == "Exhaust"
    );

    let (entry, catalog) = load_card_at(card, osty, true);
    let targets_before = entry.rng.get(RngStream::Targets);
    let done = engine::apply_action(&entry, &catalog, &Action::EndTurn).unwrap();
    let state = done.state;
    assert!(!state.history.over);
    assert_eq!(
        pile_of(&state, BELIEVE_IN_YOU_UID),
        native_pile,
        "{card}{suffix}: the result pile"
    );
    assert!(
        !done.events.iter().any(
            |event| matches!(event, Event::CardPlayed { uid, .. } if *uid == BELIEVE_IN_YOU_UID)
        ),
        "{card}{suffix}: without playing"
    );
    assert_eq!(state.rng.get(RngStream::Targets), targets_before);
    assert_eq!(
        state.history.owner_cards_exhausted_combat - entry.history.owner_cards_exhausted_combat,
        i32::from(native_pile == "Exhaust"),
        "{card}{suffix}: an Exhaust result is a real exhaust"
    );
}

/// The native Mayhem runs agree with the direct `CardCmd.AutoPlay` ones: no
/// CardPlay starts in the turn Mayhem fires, and Demonic Shield is exhausted.
#[test]
fn native_mayhem_runs_have_the_autoplay_outcome() {
    for card in CARDS {
        let mayhem = native(&format!("mayhem:{card}"));
        let entries: Vec<&str> = mayhem["history_round2_player"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry[0].as_str().unwrap())
            .collect();
        assert!(!entries.contains(&"CardPlayStartedEntry"), "{card}");
        assert_eq!(
            entries.contains(&"CardExhaustedEntry"),
            card == "DEMONIC_SHIELD",
            "{card}"
        );
        assert!(mayhem["rng_delta"].get("CombatTargets").is_none(), "{card}");
    }
}

#[test]
fn native_any_ally_cards_solo_match() {
    for card in CARDS {
        check(card, false);
    }
}

#[test]
fn native_any_ally_cards_solo_with_osty_match() {
    for card in CARDS {
        check(card, true);
    }
}

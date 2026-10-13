//! #3022: Sculpting Strike applies Ethereal (not Retain) to the chosen card.
//!
//! v0.111.0 `SculptingStrike/<OnPlay>d__7::MoveNext` RVA 0x3b8cec builds a
//! one-element `CardKeyword[]` holding 2 (Ethereal) at IL_0187 and passes it to
//! `CardCmd::ApplyKeyword` at IL_0189. Its `FromHand` filter
//! `<>c::<OnPlay>b__7_0` (RVA 0x3b8cda) admits only cards whose
//! `GetKeywordsWithSources(2)` lacks Ethereal. The frozen registry carried
//! Snap's Retain step; these witnesses pin the native behaviour end to end:
//! the chosen card exhausts at turn end, and an already-Ethereal card (canonical
//! or local keyword) is never a candidate.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::{HotState, PileId},
};

fn entry(hand: serde_json::Value) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":10
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc.piles = serde_json::from_value(json!({
        "hand": hand,
        "draw": [
            {"id":"STRIKE_IRONCLAD","uid":7,"upgrade":0},
            {"id":"STRIKE_IRONCLAD","uid":8,"upgrade":0},
            {"id":"STRIKE_IRONCLAD","uid":9,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    doc
}

/// Cold roundtrip plus admission: the Ethereal marker must survive the wire
/// and pass the local-Ethereal provenance gate.
fn wire(doc: &CanonicalStateV2, state: &HotState) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    assert_eq!(cold, *state, "cold publication preserves exact state");
    if !state.history.over {
        let rebuilt_state = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
        engine::admit(&wire, &rebuilt_state, &rebuilt).unwrap();
    }
    wire
}

fn load(doc: &CanonicalStateV2) -> (HotState, sts_sim::catalog::Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn play_sculpting(doc: &CanonicalStateV2) -> (HotState, sts_sim::catalog::Catalog) {
    let (state, catalog) = load(doc);
    let next = engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: Some(0),
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
    .state;
    (next, catalog)
}

/// The card's published local keywords (the wire is the public surface).
fn local_keywords(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> Vec<String> {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    wire.piles
        .values()
        .flatten()
        .find(|card| card.uid == Some(u64::from(uid)))
        .map(|card| card.local_keywords.clone())
        .unwrap_or_default()
}

fn local_ethereal(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> bool {
    local_keywords(doc, state, uid)
        .iter()
        .any(|k| k == "Ethereal")
}

fn local_retain(doc: &CanonicalStateV2, state: &HotState, uid: u32) -> bool {
    local_keywords(doc, state, uid)
        .iter()
        .any(|k| k == "Retain")
}

fn pile_uids(state: &HotState, pile: PileId) -> Vec<u32> {
    state
        .piles
        .get(pile)
        .as_slice()
        .iter()
        .map(|c| c.uid)
        .collect()
}

#[test]
fn chosen_card_gains_ethereal_not_retain_and_exhausts_at_turn_end() {
    for level in 0..=1 {
        let doc = entry(json!([
            {"id":"SCULPTING_STRIKE","uid":0,"upgrade":level},
            {"id":"DEFEND_IRONCLAD","uid":1,"upgrade":0},
            {"id":"BASH","uid":2,"upgrade":0}
        ]));
        let (parked, catalog) = play_sculpting(&doc);
        assert!(parked.pending.is_some(), "two candidates park a choice");
        let choices = engine::legal_actions(&parked, &catalog);
        assert_eq!(
            choices.len(),
            2,
            "level {level}: both hand cards are candidates"
        );
        let chosen = choices
            .iter()
            .map(|choice| {
                engine::apply_action(&parked, &catalog, choice)
                    .unwrap()
                    .state
            })
            .find(|state| local_ethereal(&doc, state, 1))
            .expect("one answer marks Defend Ethereal");
        assert!(
            !local_ethereal(&doc, &chosen, 2),
            "only the chosen card changes"
        );
        assert!(
            !local_retain(&doc, &chosen, 1),
            "Sculpting Strike never writes Retain"
        );
        wire(&doc, &chosen);

        let ended = engine::apply_action(&chosen, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        let exhaust = pile_uids(&ended, PileId::Exhaust);
        assert!(
            exhaust.contains(&1),
            "level {level}: the Ethereal Defend exhausts"
        );
        assert!(
            !exhaust.contains(&2),
            "level {level}: the unchosen Bash does not"
        );
        // The unchosen Bash was discarded, then reshuffled by next turn's
        // draw; it is live outside Exhaust.
        assert!(
            [PileId::Hand, PileId::Draw, PileId::Discard]
                .into_iter()
                .any(|pile| pile_uids(&ended, pile).contains(&2))
        );
        assert!(!ended.history.over);
    }
}

#[test]
fn already_ethereal_cards_are_not_candidates() {
    // Dazed is canonically Ethereal; the Defend carries a local Ethereal
    // keyword (a prior Sculpting Strike). Only the Bash is selectable, so the
    // one-card choice resolves on it.
    let doc = entry(json!([
        {"id":"SCULPTING_STRIKE","uid":0,"upgrade":0},
        {"id":"DAZED","uid":1,"upgrade":0},
        {"id":"DEFEND_IRONCLAD","uid":2,"upgrade":0,"local_keywords":["Ethereal"]},
        {"id":"BASH","uid":3,"upgrade":0}
    ]));
    let (state, catalog) = play_sculpting(&doc);
    let resolved = if state.pending.is_some() {
        let choices = engine::legal_actions(&state, &catalog);
        assert_eq!(choices.len(), 1, "only the Bash is a candidate");
        engine::apply_action(&state, &catalog, &choices[0])
            .unwrap()
            .state
    } else {
        state
    };
    assert!(
        local_ethereal(&doc, &resolved, 3),
        "the Bash is the one choice"
    );
    assert!(
        !local_ethereal(&doc, &resolved, 1),
        "Dazed keeps only its canonical keyword"
    );
    assert!(local_ethereal(&doc, &resolved, 2));
    wire(&doc, &resolved);
}

#[test]
fn no_candidate_when_every_hand_card_is_ethereal() {
    let doc = entry(json!([
        {"id":"SCULPTING_STRIKE","uid":0,"upgrade":0},
        {"id":"DAZED","uid":1,"upgrade":0}
    ]));
    let (state, _catalog) = play_sculpting(&doc);
    assert!(
        state.pending.is_none(),
        "an empty candidate set asks nothing"
    );
    assert!(!local_ethereal(&doc, &state, 1));
    wire(&doc, &state);
}

/// #3877: the marked card outlives its writer. Seance transforms the
/// Sculpting Strike itself into a Soul (`Seance/<OnPlay>d__7::MoveNext` RVA
/// 0x3b8ed8, `TransformTo<Soul>` at IL_016a; `CardCmd/<Transform>d__13`
/// 0x3e0ae0 removes the original at IL_018c), and the keyword stays on the
/// card it was applied to (`CardCmd::ApplyKeyword` 0x12fd98 IL_0018). The
/// composition sweep met this as a live state with no legal action: the
/// provenance reader wanted a Sculpting Strike in a pile.
#[test]
fn a_document_that_never_recorded_the_mark_refuses_by_name_once_its_strike_is_gone() {
    // A document serialized before #3877: the Defend is marked and a live
    // Sculpting Strike authenticates it, but nothing records that the Strike
    // wrote the mark. Every state this engine plays into carries that record
    // (`native_*` below); this one cannot, so once Seance removes the Strike
    // the mark has no evidence left. That is a named refusal, not a live
    // state with nothing to do.
    let mut doc = entry(json!([
        {"id":"DEFEND_IRONCLAD","uid":2,"upgrade":0,"local_keywords":["Ethereal"]},
        {"id":"SEANCE","uid":3,"upgrade":0}
    ]));
    doc.piles.get_mut("draw").unwrap().insert(
        0,
        serde_json::from_value(json!({"id":"SCULPTING_STRIKE","uid":1,"upgrade":0})).unwrap(),
    );
    let (state, catalog) = load(&doc);
    assert!(local_ethereal(&doc, &state, 2));

    let seance = Action::Play {
        uid: 3,
        target: None,
        selection: SelectionRef::NONE,
    };
    let choosing = engine::apply_action(&state, &catalog, &seance)
        .unwrap()
        .state;
    let answers = engine::legal_actions(&choosing, &catalog);
    let transform_the_strike = answers
        .iter()
        .copied()
        .find(|action| {
            let Action::Select { answer } = action else {
                return false;
            };
            engine::selected_card_uids(&choosing, &catalog, *answer).unwrap() == Some(vec![1])
        })
        .expect("Seance offers the Sculpting Strike in Draw");
    let after = engine::apply_action(&choosing, &catalog, &transform_the_strike)
        .unwrap()
        .state;
    let mut buffer = engine::LegalActionBuffer::new();
    assert_eq!(
        engine::legal_actions_checked(&after, &catalog, &mut buffer).map(|_| ()),
        Err(engine::EngineRefusal::MalformedArgs(
            "Call of the Void local Ethereal provenance"
        ))
    );
}

/// The game's own run of this, recorded by `tests/native/probe.py` from
/// `tests/native/issue3877_sculpting_writer_transformed.spec.json` on the
/// archived v0.111.0 `sts2.dll` (headless harness): an Ironclad plays
/// Sculpting Strike (the only Hand card without Ethereal is a Defend, which
/// it marks), the Strike is put back in the Draw pile, and Seance turns it
/// into a Soul. `sculpting_seance_call` does the same under a live Call of the
/// Void. The run with answer `1` is the one where Seance picks the Strike.
const NATIVE_SEANCE: &str = include_str!("native/sculpting_seance.native.json");
const NATIVE_SEANCE_CALL: &str = include_str!("native/sculpting_seance_call.native.json");

/// What the native run says about the marked Defend: `(Ethereal after the
/// transform, the Strike still in a pile, a Soul in Draw, the pile the Defend
/// is in after the turn ends)`.
fn native_transform(recorded: &str, play_index: usize) -> (bool, bool, bool, String) {
    let all: serde_json::Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(all["build"], "v0.111.0");
    let run = all["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["answers"] == json!(["1"]))
        .expect("the run where Seance picks the Sculpting Strike");
    let snap = |suffix: &str| {
        run["trace"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["label"].as_str().unwrap().ends_with(suffix))
            .unwrap_or_else(|| panic!("no {suffix} entry"))["snap"]
            .clone()
    };
    let after = snap(&format!("{play_index}:SEANCE[Ethereal]:done"));
    let everywhere = |snap: &serde_json::Value| -> Vec<(String, String)> {
        snap["piles"]
            .as_object()
            .unwrap()
            .iter()
            .flat_map(|(pile, cards)| {
                cards
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|card| (pile.clone(), card.as_str().unwrap().to_string()))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let cards = everywhere(&after);
    let ended = everywhere(&snap(":end"));
    let defend_pile = ended
        .iter()
        .find(|(_, card)| card.starts_with("DEFEND_IRONCLAD"))
        .map(|(pile, _)| pile.clone())
        .unwrap();
    (
        cards
            .iter()
            .any(|(_, card)| card == "DEFEND_IRONCLAD[Ethereal]"),
        cards.iter().any(|(_, card)| card == "SCULPTING_STRIKE"),
        cards
            .iter()
            .any(|(pile, card)| pile == "DrawPile" && card == "SOUL"),
        defend_pile,
    )
}

/// The engine's run of the native scenario. `call` plays Call of the Void
/// first. The Sculpting Strike marks the Defend by being played, so the
/// fight's own record of the mark is what authenticates it afterwards.
fn engine_transform(call: bool, defend_in_hand: bool) -> (bool, bool, bool, String) {
    let mut hand = vec![
        json!({"id":"SCULPTING_STRIKE","uid":0,"upgrade":0}),
        json!({"id":"DEFEND_IRONCLAD","uid":2,"upgrade":0}),
        json!({"id":"SEANCE","uid":3,"upgrade":0}),
    ];
    if call {
        hand.push(json!({"id":"CALL_OF_THE_VOID","uid":4,"upgrade":0}));
    }
    let mut doc = entry(json!(hand));
    // The native scenario sets Energy to 9 so every play is affordable.
    doc.player.insert("energy".to_string(), json!(9));
    if call {
        // What a generator needs to be admitted: whose pool it draws from
        // and the profile's unlocks (everything, as in the native run).
        doc.player
            .insert("reward_card_pool".to_string(), json!("Ironclad"));
        doc.player.insert(
            "reward_card_rarity_odds".to_string(),
            json!("RegularEncounter"),
        );
        doc.player.insert(
            "splash_unlock_epochs".to_string(),
            json!(sts_sim::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101.as_slice()),
        );
    }
    let (mut state, catalog) = load(&doc);
    let play = |uid, target| Action::Play {
        uid,
        target,
        selection: SelectionRef::NONE,
    };
    if call {
        state = engine::apply_action(&state, &catalog, &play(4, None))
            .unwrap()
            .state;
    }
    // One candidate (Seance is Ethereal already): the game picks it without
    // asking, and so does the engine.
    state = engine::apply_action(&state, &catalog, &play(0, Some(0)))
        .unwrap()
        .state;
    assert!(local_ethereal(&doc, &state, 2));
    if !defend_in_hand {
        state = engine::apply_action(&state, &catalog, &play(2, None))
            .unwrap()
            .state;
    }

    // The native run arranges the piles here; the engine does it through the
    // document: the played Strike goes from Discard to the Draw pile.
    let mut arranged = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let discard = arranged.piles.get_mut("discard").unwrap();
    let at = discard
        .iter()
        .position(|card| card.id == "SCULPTING_STRIKE")
        .unwrap();
    let strike = discard.remove(at);
    arranged.piles.get_mut("draw").unwrap().insert(0, strike);
    // Re-admitted without Call of the Void. A mid-fight document holding a
    // live Call of the Void is not an admitted root today
    // (`PowerState(CallOfTheVoid)`, a mid-fight-root limit outside #3877),
    // so there the arranged document is read back into the same session.
    let (state, catalog) = if call {
        let state = HotBoundary::from_canonical(&arranged, &catalog).unwrap();
        (state, catalog)
    } else {
        load(&arranged)
    };

    let choosing = engine::apply_action(&state, &catalog, &play(3, None))
        .unwrap()
        .state;
    let transform_the_strike = engine::legal_actions(&choosing, &catalog)
        .iter()
        .copied()
        .find(|action| {
            let Action::Select { answer } = action else {
                return false;
            };
            engine::selected_card_uids(&choosing, &catalog, *answer).unwrap() == Some(vec![0])
        })
        .expect("Seance offers the Sculpting Strike in Draw");
    let after = engine::apply_action(&choosing, &catalog, &transform_the_strike)
        .unwrap()
        .state;

    // The state the sweep found stuck: it enumerates, projects and reloads.
    let mut buffer = engine::LegalActionBuffer::new();
    let legal = engine::legal_actions_checked(&after, &catalog, &mut buffer).unwrap();
    assert!(legal.contains(&Action::EndTurn));
    let reloaded_doc = HotBoundary::try_to_canonical(&after, &catalog).unwrap();
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&reloaded_doc).unwrap();
    let reloaded = HotBoundary::from_canonical(&reloaded_doc, &reloaded_catalog).unwrap();
    if !call {
        engine::admit(&reloaded_doc, &reloaded, &reloaded_catalog).unwrap();
    }
    assert!(
        engine::legal_actions_checked(&reloaded, &reloaded_catalog, &mut buffer)
            .unwrap()
            .contains(&Action::EndTurn),
        "the reloaded document needs no Sculpting Strike row"
    );

    let piles = [
        (PileId::Hand, "Hand"),
        (PileId::Draw, "DrawPile"),
        (PileId::Discard, "DiscardPile"),
        (PileId::Exhaust, "ExhaustPile"),
    ];
    let strike_live = piles
        .iter()
        .any(|(pile, _)| pile_uids(&after, *pile).contains(&0));
    let soul_in_draw = after.piles.get(PileId::Draw).as_slice().iter().any(|card| {
        catalog
            .spec(card.atom)
            .is_some_and(|spec| spec.identity.id == sts_sim::ids::CardId::Soul)
    });
    let ended = engine::apply_action(&after, &catalog, &Action::EndTurn)
        .unwrap()
        .state;
    let defend_pile = piles
        .iter()
        .find(|(pile, _)| pile_uids(&ended, *pile).contains(&2))
        .map(|(_, name)| name.to_string())
        .unwrap();
    (
        local_ethereal(&doc, &after, 2),
        strike_live,
        soul_in_draw,
        defend_pile,
    )
}

/// Seance turns the Sculpting Strike into a Soul and the marked Defend stays
/// Ethereal: it is in Hand at the end of the turn and exhausts.
#[test]
fn native_a_marked_card_survives_its_sculpting_strike_being_transformed() {
    let native = native_transform(NATIVE_SEANCE, 6);
    assert_eq!(native, (true, false, true, "ExhaustPile".to_string()));
    assert_eq!(engine_transform(false, true), native);
}

/// The same beside a live Call of the Void, the composition #3895's first
/// version still refused. Here the marked Defend was played before the
/// transform, so it is not in Hand at the end of the turn and is not
/// exhausted; the game deals it back the next turn still marked.
#[test]
fn native_the_mark_survives_beside_a_live_call_of_the_void() {
    let native = native_transform(NATIVE_SEANCE_CALL, 8);
    assert!(native.0 && !native.1 && native.2);
    let engine = engine_transform(true, false);
    assert_eq!(
        (engine.0, engine.1, engine.2),
        (native.0, native.1, native.2)
    );
    assert_ne!(engine.3, "ExhaustPile");
    assert_ne!(native.3, "ExhaustPile");
}

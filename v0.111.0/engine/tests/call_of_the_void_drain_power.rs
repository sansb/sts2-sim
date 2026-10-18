//! #3946 — a Drain Power Call of the Void hands its owner upgrades cards the
//! catalog knows, warm and from a cold reload.
//!
//! Call of the Void's listener hands its owner cards of the owner's pool at
//! each BeforeHandDraw, and a Necrobinder's pool holds Drain Power, whose play
//! upgrades Discard cards (`DrainPower/<OnPlay>d__3::MoveNext` RVA
//! `0x399f7c`, `CardCmd::Upgrade` IL_0149). The boundary closed the upgrade
//! successors for a Hello World or Calamity pool that holds one (#3877), not
//! for this one, so the root was admitted and the line then stopped at that
//! play: `mid-combat card identity "DEFEND_NECROBINDER"+1 was not
//! pre-interned` (the independent review of #3940: 30 such lines in 1,440).
//!
//! The root is a sweep root (`examples/composition_sweep`) with its
//! Generation stream overwritten so that the listener reaches a Drain Power
//! early, which is how the review found it. The line below is one of the
//! review's: on `main` before this change its Drain Power play refused.

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

/// The review driver's splitmix generator, for its stream words and picks.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        word ^ (word >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A Necrobinder with one Call of the Void against Cultists, run seed
/// `RVDRAIN{index}`, and every Generation stream word overwritten.
fn open(index: u64) -> (CanonicalStateV2, HotState, Catalog) {
    let character = sweep::Character::named("NECROBINDER").unwrap();
    let cards = vec!["CALL_OF_THE_VOID+0".to_string()];
    let root = sweep::custom_root(&cards, &[], &[], Some("CULTISTS_NORMAL")).unwrap();
    let mut save: Value = serde_json::from_str(&sweep::save_json(character, &root)).unwrap();
    save["rng"]["seed"] = Value::from(format!("RVDRAIN{index}"));
    let save = save.to_string();
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: Some("ENCOUNTER.CULTISTS_NORMAL"),
        node_type: Some("monster"),
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let RootOutcome::Opened(opening) = entry::build_root(&request, None) else {
        panic!("the synthetic root opens");
    };
    let mut doc: CanonicalStateV2 = opening.document.clone();
    let mut mix = Mix(0xABCD_0000 + index);
    for (key, stream) in doc.rng.iter_mut() {
        if key.contains("generation") {
            stream.words = [mix.next() | 1, mix.next(), mix.next(), mix.next()];
        }
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (doc, state, catalog)
}

/// Load a document as a cold copy: its catalog from the document alone.
fn cold(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog)
        .unwrap_or_else(|refusal| panic!("the engine's own state is not admitted: {refusal}"));
    (state, catalog)
}

/// Whether `action` plays a Drain Power in `state`'s Hand.
fn plays_drain_power(state: &HotState, catalog: &Catalog, action: &Action) -> bool {
    let Action::Play { uid, .. } = action else {
        return false;
    };
    HotBoundary::try_to_canonical(state, catalog).unwrap().piles["hand"]
        .iter()
        .any(|card| card.id == "DRAIN_POWER" && card.uid == Some(u64::from(*uid)))
}

/// The review's line: rollout `rollout` of root `index`, its picks drawn as
/// the driver drew them, run warm until its first generated Drain Power has
/// been played. Returns the root and the actions.
fn line_to_drain_power(index: u64, rollout: u64) -> (CanonicalStateV2, Vec<Action>) {
    let (doc, mut state, catalog) = open(index);
    let mut rng = Mix((77 + index) ^ rollout.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    // The driver drew its reload step first.
    rng.below(40);
    let mut actions = Vec::new();
    for _ in 0..160 {
        assert!(!state.history.over, "the line ends before a Drain Power");
        let legal = engine::legal_actions(&state, &catalog).to_vec();
        let mut pick = rng.below(legal.len() as u64) as usize;
        if format!("{:?}", legal[pick]).starts_with("EndTurn") && rng.below(3) != 0 {
            pick = rng.below(legal.len() as u64) as usize;
        }
        let action = legal[pick];
        let drain = plays_drain_power(&state, &catalog, &action);
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap_or_else(|refusal| panic!("warm line refuses at {action:?}: {refusal}"))
            .state;
        actions.push(action);
        if drain {
            return (doc, actions);
        }
    }
    panic!("no Drain Power in 160 actions");
}

/// Warm, the line plays its generated Drain Power and upgrades a Discard
/// card. Cold, a copy reloaded from the warm state before each action of the
/// line, so before the listener hands the Drain Power over as well as after,
/// plays the rest of the line to the same document.
#[test]
fn a_generated_drain_power_plays_warm_and_from_every_cold_reload() {
    let (doc, actions) = line_to_drain_power(23, 1);
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let mut warm = vec![HotBoundary::from_canonical(&doc, &catalog).unwrap()];
    for action in &actions {
        let next = engine::apply_action(warm.last().unwrap(), &catalog, action)
            .unwrap()
            .state;
        warm.push(next);
    }
    let end = HotBoundary::try_to_canonical(warm.last().unwrap(), &catalog).unwrap();
    let upgraded = |doc: &CanonicalStateV2| {
        doc.piles
            .values()
            .flatten()
            .filter(|card| card.id != "DRAIN_POWER" && card.upgrade > 0)
            .count()
    };
    assert_eq!(upgraded(&doc), 0);
    assert!(
        upgraded(&end) > 0,
        "the Drain Power upgraded a Discard card"
    );
    // The Drain Power was handed over, not dealt.
    assert!(
        doc.piles
            .values()
            .flatten()
            .all(|card| card.id != "DRAIN_POWER")
    );

    for (at, state) in warm[..actions.len()].iter().enumerate() {
        let reloaded = HotBoundary::try_to_canonical(state, &catalog).unwrap();
        let (mut cold_state, cold_catalog) = cold(&reloaded);
        for action in &actions[at..] {
            cold_state = engine::apply_action(&cold_state, &cold_catalog, action)
                .unwrap_or_else(|refusal| {
                    panic!("the copy reloaded before action {at} refuses at {action:?}: {refusal}")
                })
                .state;
        }
        assert_eq!(
            HotBoundary::try_to_canonical(&cold_state, &cold_catalog).unwrap(),
            end,
            "reloaded before action {at}"
        );
    }
}

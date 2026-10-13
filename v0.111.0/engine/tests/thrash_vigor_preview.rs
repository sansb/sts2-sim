//! #3877 — Thrash's candidate preview beside a Vigor its own attack kept.
//!
//! The composition sweep met `Thrash candidate preview beside live Vigor`
//! mid-line (`PATTER & THRASH`, `PREP_TIME & THRASH`, `TERRAFORMING & THRASH`,
//! `METAMORPHOSIS & PREP_TIME`): a Thrash played with Vigor whose hit ends the
//! combat. `Hook.AfterAttack` reaches no listener once the combat is ending,
//! so `VigorPower` keeps its amount and its `commandToModify`; Thrash's tail
//! still picks a Hand Attack and previews it through `Hook.ModifyDamage`, and
//! `VigorPower::ModifyDamageAdditive` (RVA `0xaa83c` IL_002a-IL_0051) answers
//! Zero for a card that is not the bound command's source.
//!
//! The `native_*` tests compare the engine with the game itself: the runs
//! under `tests/native/thrash_*.native.json`, recorded by
//! `tests/native/probe.py` from
//! `tests/native/issue3877_thrash_vigor_preview.spec.json` on the archived
//! v0.111.0 `sts2.dll` (headless harness). An Ironclad against
//! `ENCOUNTER.LOUSE_PROGENITOR_NORMAL` holds Patter, Thrash and a Strike.

#[allow(dead_code)]
#[path = "../examples/composition_sweep/sweep.rs"]
mod sweep;

use serde_json::Value;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::{Catalog, GameBuild},
    engine::{self, Action, SelectionRef},
    entry::{self, EntryInput, EntryRequest, RootOutcome},
    hot::{HotState, PileId},
};

const VIGOR_LETHAL: &str = include_str!("native/thrash_vigor_lethal.native.json");
const VIGOR_SURVIVES: &str = include_str!("native/thrash_vigor_survives.native.json");
const PLAIN_LETHAL: &str = include_str!("native/thrash_plain_lethal.native.json");

/// The end of the scenario, in terms both sides can state.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    vigor: i64,
    /// Thrash's `Damage` base: the printed 4 plus what it grew by.
    thrash_damage: String,
    thrash_pile: &'static str,
    strike_pile: &'static str,
    over: bool,
}

const PILES: [(&str, &str); 5] = [
    ("Hand", "hand"),
    ("DrawPile", "draw"),
    ("DiscardPile", "discard"),
    ("ExhaustPile", "exhaust"),
    ("PlayPile", "play"),
];

fn native_outcome(recorded: &str) -> Outcome {
    let all: Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(all["build"], "v0.111.0");
    let last = all["runs"][0]["trace"].as_array().unwrap().last().unwrap();
    assert_eq!(last["state"], "Finished");
    let snap = &last["snap"];
    let find = |id: &str| {
        PILES
            .iter()
            .find_map(|(native, ours)| {
                let at = snap["piles"][native]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|card| card == id)?;
                Some((*ours, snap["damage_bases"][native][at].clone()))
            })
            .unwrap()
    };
    let (thrash_pile, thrash_damage) = find("THRASH");
    Outcome {
        vigor: snap["powers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|power| power[0] == "VigorPower")
            .map_or(0, |power| power[1].as_i64().unwrap()),
        thrash_damage: thrash_damage.as_str().unwrap().to_string(),
        thrash_pile,
        strike_pile: find("STRIKE_IRONCLAD").0,
        over: snap["over"].as_bool().unwrap(),
    }
}

/// The sweep's synthetic Ironclad root against the Louse Progenitor with the
/// cards under test first in Hand, then exactly one Strike, nine Energy.
fn root(plays: &[&str], louse_hp: Option<i64>) -> (HotState, Catalog) {
    let character = sweep::Character::named("IRONCLAD").unwrap();
    let cards: Vec<String> = plays.iter().map(|id| id.to_string()).collect();
    let root = sweep::custom_root(&cards, &[], &[], Some("LOUSE_PROGENITOR_NORMAL")).unwrap();
    let save = sweep::save_json(character, &root);
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: Some("ENCOUNTER.LOUSE_PROGENITOR_NORMAL"),
        node_type: Some("monster"),
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let RootOutcome::Opened(opening) = entry::build_root(&request, None) else {
        panic!("the synthetic root opens");
    };
    let mut doc: CanonicalStateV2 = opening.document.clone();
    // The Hand becomes the cards under test and one Strike; everything else
    // is a Defend in the Draw pile, as the native scenario arranges it.
    let mut all: Vec<_> = ["hand", "draw"]
        .iter()
        .flat_map(|pile| doc.piles.get(*pile).cloned().unwrap_or_default())
        .collect();
    let mut hand = Vec::new();
    for id in plays.iter().copied().chain(["STRIKE_IRONCLAD"]) {
        let at = all.iter().position(|card| card.id == id).unwrap();
        hand.push(all.remove(at));
    }
    for card in &mut all {
        if card.id != "DEFEND_IRONCLAD" {
            card.id = "DEFEND_IRONCLAD".to_string();
        }
    }
    doc.piles.insert("hand".to_string(), hand);
    doc.piles.insert("draw".to_string(), all);
    doc.player.insert("energy".to_string(), 9.into());
    if let Some(hp) = louse_hp {
        doc.monsters[0].insert("hp".to_string(), hp.into());
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn engine_outcome(plays: &[&str], louse_hp: Option<i64>) -> Outcome {
    let (mut state, catalog) = root(plays, louse_hp);
    let hand: Vec<(u32, bool)> = state.piles.get(PileId::Hand).as_slice()[..plays.len()]
        .iter()
        .map(|card| (card.uid, catalog.spec(card.atom).unwrap().targeted))
        .collect();
    for (uid, targeted) in hand {
        let action = Action::Play {
            uid,
            target: targeted.then_some(0),
            selection: SelectionRef::NONE,
        };
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap()
            .state;
    }
    let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    let find = |id: &str| {
        PILES
            .iter()
            .find_map(|(_, ours)| {
                doc.piles
                    .get(*ours)?
                    .iter()
                    .find(|card| card.id == id)
                    .map(|card| (*ours, card.clone()))
            })
            .unwrap()
    };
    let (thrash_pile, thrash) = find("THRASH");
    // `physical_state[2]` is the retained growth, an exact fraction.
    let growth = thrash
        .physical_state
        .as_ref()
        .map_or((0, 1), |physical| match &physical[2] {
            Value::Object(fraction) => (
                fraction["__fraction__"][0].as_i64().unwrap(),
                fraction["__fraction__"][1].as_i64().unwrap(),
            ),
            other => (other.as_i64().unwrap(), 1),
        });
    assert_eq!(growth.1, 1, "a whole growth in these scenarios");
    Outcome {
        vigor: doc.player.get("vigor").and_then(Value::as_i64).unwrap_or(0),
        thrash_damage: (4 + growth.0).to_string(),
        thrash_pile,
        strike_pile: find("STRIKE_IRONCLAD").0,
        over: state.history.over,
    }
}

/// The play that refused. The first hit ends the combat, the Vigor is kept,
/// and Thrash grows by the Strike's 6: the kept Vigor adds nothing. The
/// Strike stays in Hand, because `CardCmd.Exhaust` is ending-gated.
#[test]
fn native_thrash_ending_the_combat_previews_without_its_kept_vigor() {
    let native = native_outcome(VIGOR_LETHAL);
    assert_eq!(
        (
            native.vigor,
            native.thrash_damage.as_str(),
            native.strike_pile,
            native.over
        ),
        (2, "10", "hand", true)
    );
    assert_eq!(engine_outcome(&["PATTER", "THRASH"], Some(1)), native);
}

/// The Louse survives: the attack consumes the Vigor, and the growth is the
/// same 6.
#[test]
fn native_thrash_that_does_not_end_the_combat_consumes_its_vigor() {
    let native = native_outcome(VIGOR_SURVIVES);
    assert_eq!(
        (
            native.vigor,
            native.thrash_damage.as_str(),
            native.strike_pile,
            native.over
        ),
        (0, "10", "exhaust", false)
    );
    assert_eq!(engine_outcome(&["PATTER", "THRASH"], None), native);
}

/// The control with no Vigor.
#[test]
fn native_thrash_ending_the_combat_without_vigor_matches() {
    let native = native_outcome(PLAIN_LETHAL);
    assert_eq!((native.vigor, native.thrash_damage.as_str()), (0, "10"));
    assert_eq!(engine_outcome(&["THRASH"], Some(1)), native);
}

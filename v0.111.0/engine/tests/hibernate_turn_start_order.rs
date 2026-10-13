//! #3877 — Hibernate's turn-start decrement takes its place in the player's
//! power list, and Hibernate is playable by one player.
//!
//! The composition sweep met `Hibernate AfterPlayerTurnStart order` at
//! admission for every root holding Hibernate beside another
//! `AfterPlayerTurnStart` player power (120 roots). The engine ran the
//! decrement after the whole ordered list; the game runs it where Hibernate
//! sits in `Creature.Powers`. `HibernatePower/<AfterPlayerTurnStart>d__6`
//! (RVA `0x33c790` IL_0033) awaits `PowerCmd.Decrement`, whose `ModifyAmount`
//! (`0x3f032c` IL_003a-003f) leaves at `IsEnding`: a Hibernate applied before
//! the power that ends the combat has decremented, one applied after it has
//! not.
//!
//! The `native_*` tests compare the engine with the game itself: the runs
//! under `tests/native/hibernate_*.native.json`, recorded by
//! `tests/native/probe.py` from
//! `tests/native/issue3877_hibernate_turn_start_order.spec.json` on the
//! archived v0.111.0 `sts2.dll` (headless harness). An Ironclad, alone,
//! against `ENCOUNTER.TOADPOLES_WEAK` plays Hibernate and Rolling Boulder in
//! both orders and ends the turn, with the Toadpoles at 1 HP (the Boulder
//! ends the combat) and at full HP. A Defect plays Hibernate at both levels.

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
    hot::HotState,
};

const BEFORE_LETHAL: &str = include_str!("native/hibernate_before_boulder_lethal.native.json");
const AFTER_LETHAL: &str = include_str!("native/hibernate_after_boulder_lethal.native.json");
const BEFORE: &str = include_str!("native/hibernate_before_boulder.native.json");
const AFTER: &str = include_str!("native/hibernate_after_boulder.native.json");
const DEFECT: &str = include_str!("native/hibernate_solo_defect.native.json");
const DEFECT_UPGRADED: &str = include_str!("native/hibernate_solo_defect_upgraded.native.json");

/// What both sides can state after each step.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    hibernate: i64,
    rolling_boulder: i64,
    /// The `AfterPlayerTurnStart` members among the player's powers, in
    /// application order.
    order: Vec<String>,
    frost: usize,
    orbs: usize,
    orb_slots: i64,
    block: i64,
    over: bool,
}

fn native_steps(recorded: &str) -> Vec<Step> {
    let all: Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(all["build"], "v0.111.0");
    let trace = all["runs"][0]["trace"].as_array().unwrap();
    trace[1..]
        .iter()
        .map(|entry| {
            assert_eq!(entry["state"], "Finished", "{}", entry["label"]);
            let snap = &entry["snap"];
            let power = |name: &str| {
                snap["powers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|power| power[0] == name)
                    .map_or(0, |power| power[1].as_i64().unwrap())
            };
            let orbs = snap["orbs"].as_array().unwrap();
            Step {
                hibernate: power("HibernatePower"),
                rolling_boulder: power("RollingBoulderPower"),
                order: snap["power_order"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| match name.as_str().unwrap() {
                        "HibernatePower" => "hibernate".to_string(),
                        "RollingBoulderPower" => "rolling_boulder".to_string(),
                        other => panic!("unexpected power {other}"),
                    })
                    .collect(),
                frost: orbs.iter().filter(|orb| *orb == "FrostOrb").count(),
                orbs: orbs.len(),
                orb_slots: snap["orb_slots"].as_i64().unwrap(),
                block: snap["block"].as_i64().unwrap(),
                over: snap["over"].as_bool().unwrap(),
            }
        })
        .collect()
}

/// The sweep's synthetic root for `character` against two Toadpoles, with the
/// first Hand cards replaced by `plays` and nine Energy, as the native
/// scenario arranges it.
fn root(character: &str, plays: &[(&str, i64)], toadpole_hp: Option<i64>) -> (HotState, Catalog) {
    let character = sweep::Character::named(character).unwrap();
    let cards: Vec<String> = plays
        .iter()
        .map(|(id, upgrade)| format!("{id}+{upgrade}"))
        .collect();
    let root = sweep::custom_root(&cards, &[], &[], Some("TOADPOLES_WEAK")).unwrap();
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
    let mut doc: CanonicalStateV2 = opening.document.clone();
    // Swap each card under test into the Hand from wherever the shuffle put it.
    for (slot, (id, upgrade)) in plays.iter().enumerate() {
        let is_it =
            |card: &sts_sim::canonical::CanonicalCardV2| card.id == *id && card.upgrade == *upgrade;
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
    if let Some(hp) = toadpole_hp {
        for monster in &mut doc.monsters {
            monster.insert("hp".to_string(), hp.into());
        }
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn step_of(state: &HotState, catalog: &Catalog) -> Step {
    let doc = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let int = |key: &str| doc.player.get(key).and_then(Value::as_i64).unwrap_or(0);
    let orbs: Vec<Value> = doc
        .player
        .get("orbs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Step {
        hibernate: int("hibernate"),
        rolling_boulder: int("rolling_boulder"),
        order: doc
            .player
            .get("after_player_turn_start_power_order")
            .and_then(Value::as_array)
            .map(|order| {
                order
                    .iter()
                    .map(|name| name.as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default(),
        frost: orbs
            .iter()
            .filter(|orb| orb.to_string().to_ascii_lowercase().contains("frost"))
            .count(),
        orbs: orbs.len(),
        orb_slots: int("orb_slots"),
        block: int("block"),
        over: state.history.over,
    }
}

/// Play the first `plays` Hand cards in order, then end the turn; one `Step`
/// after each, as the native trace has them.
fn engine_steps(character: &str, plays: &[(&str, i64)], toadpole_hp: Option<i64>) -> Vec<Step> {
    let (mut state, catalog) = root(character, plays, toadpole_hp);
    let uids: Vec<u32> = state.piles.get(sts_sim::hot::PileId::Hand).as_slice()[..plays.len()]
        .iter()
        .map(|card| card.uid)
        .collect();
    let mut steps = Vec::new();
    for uid in uids {
        let action = Action::Play {
            uid,
            target: None,
            selection: SelectionRef::NONE,
        };
        assert!(
            engine::legal_actions(&state, &catalog).contains(&action),
            "the play is offered to one player"
        );
        state = engine::apply_action(&state, &catalog, &action)
            .unwrap()
            .state;
        steps.push(step_of(&state, &catalog));
    }
    state = engine::apply_action(&state, &catalog, &Action::EndTurn)
        .unwrap()
        .state;
    steps.push(step_of(&state, &catalog));
    steps
}

const H: (&str, i64) = ("HIBERNATE", 0);
const B: (&str, i64) = ("ROLLING_BOULDER", 0);

/// Hibernate first: it decrements to nothing before the Boulder ends the
/// combat, and the power is gone.
#[test]
fn native_hibernate_before_a_lethal_boulder_has_decremented() {
    let native = native_steps(BEFORE_LETHAL);
    let last = native.last().unwrap();
    assert!(last.over && last.hibernate == 0 && last.order == ["rolling_boulder"]);
    assert_eq!(engine_steps("IRONCLAD", &[H, B], Some(1)), native);
}

/// Boulder first: the combat is ending when Hibernate's turn comes, and the
/// stack is still there. The engine used to refuse this root at admission,
/// and with the gate simply lifted it would have decremented here too.
#[test]
fn native_hibernate_after_a_lethal_boulder_keeps_its_stack() {
    let native = native_steps(AFTER_LETHAL);
    let last = native.last().unwrap();
    assert!(last.over && last.hibernate == 1);
    assert_eq!(last.order, ["rolling_boulder", "hibernate"]);
    assert_eq!(engine_steps("IRONCLAD", &[B, H], Some(1)), native);
}

/// Neither order matters while the combat goes on: Hibernate leaves, the
/// Boulder grows.
#[test]
fn native_hibernate_and_a_boulder_that_does_not_end_the_combat() {
    for (recorded, plays) in [(BEFORE, [H, B]), (AFTER, [B, H])] {
        let native = native_steps(recorded);
        let last = native.last().unwrap();
        assert!(!last.over && last.hibernate == 0 && last.rolling_boulder == 10);
        assert_eq!(engine_steps("IRONCLAD", &plays, None), native);
    }
}

/// One player may play it. An Ironclad has no orb slot: the first Frost makes
/// one, the second evokes it for 5 Block (`orbs::channel`). A Defect's three
/// slots hold a Lightning; Hibernate+ channels a third Frost and evokes that.
#[test]
fn native_hibernate_is_played_by_one_player() {
    let ironclad = native_steps(BEFORE);
    assert_eq!(
        (
            ironclad[0].hibernate,
            ironclad[0].frost,
            ironclad[0].orb_slots,
            ironclad[0].block
        ),
        (1, 1, 1, 5)
    );
    for (recorded, upgrade, frost) in [(DEFECT, 0, 2), (DEFECT_UPGRADED, 1, 3)] {
        let native = native_steps(recorded);
        assert_eq!(
            (native[0].hibernate, native[0].frost, native[0].orbs),
            (1, frost, 3)
        );
        assert_eq!(native[1].hibernate, 0);
        assert_eq!(
            engine_steps("DEFECT", &[("HIBERNATE", upgrade)], None),
            native
        );
    }
}

/// #3914, met through this change: a root with Hibernate beside Entropy is
/// admitted now, and a parked turn-start Entropy choice has to reload. On
/// main a live Entropy had no arm in admission's player-power validity match,
/// so the reload's ActionReplay predecessor was "not admitted" (the sweep's
/// lone `ENTROPY+0` root failed its round trip). These are the sweep's own
/// roots and seeds, with every parked state round-tripped.
#[test]
fn a_parked_turn_start_entropy_choice_reloads_alone_and_beside_hibernate() {
    let play = sweep::Play {
        rollouts: 12,
        park_round_trips: true,
        ..sweep::Play::default()
    };
    for (character, cards) in [
        ("IRONCLAD", vec!["ENTROPY+0"]),
        ("DEFECT", vec!["ENTROPY+0", "HIBERNATE+0"]),
    ] {
        let character = sweep::Character::named(character).unwrap();
        let cards: Vec<String> = cards.into_iter().map(str::to_string).collect();
        let root = sweep::custom_root(&cards, &[], &[], None).unwrap();
        let entropy: Vec<_> = sweep::run_root(character, &root, &play)
            .into_iter()
            .filter(|finding| finding.stage == "roundtrip")
            .collect();
        assert!(entropy.is_empty(), "{}: {entropy:?}", character.name);
    }
}

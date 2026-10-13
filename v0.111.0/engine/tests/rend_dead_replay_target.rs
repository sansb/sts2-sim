//! #3877 — a replayed Rend whose target died in the first body is a no-op,
//! not a refusal.
//!
//! The composition sweep met `monster index N is absent or dead` on legal
//! lines: Glam on a Rend, and Echo Form beside one. Rend's body
//! (`Rend/<OnPlay>d__3::MoveNext` RVA `0x3b70b0`) only null-checks
//! `cardPlay.Target` (IL_0026-IL_0030) and runs one `AttackCommand`
//! (IL_0036-IL_0077). `OnPlayWrapper` (`0x31b8d0`) gives a replayed body the
//! same frozen target, and `AttackCommand.Execute`'s `IsAlive` filter
//! (`0x3f19c0` IL_0168-IL_0196) leaves a dead one no receiver.
//!
//! The fixture is the document before the play, as the sweep's root
//! `SILENT / GLAM@REND` reached it: a Toadpole at 8 HP with Thorns 2, a
//! second Toadpole, and the Glam Rend in Hand. The `native_*` tests compare
//! the engine with the game itself.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, LegalActionBuffer, SelectionRef},
    hot::{HotState, PileId},
};

const FIXTURE: &str = include_str!("../fixtures/issue3877_glam_rend_lethal_first_body.json");
const REND_UID: u32 = 7;

fn load(first_toadpole_hp: i64) -> (HotState, Catalog) {
    let mut doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    doc.monsters[0].insert("hp".to_string(), first_toadpole_hp.into());
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn rend(target: u8) -> Action {
    Action::Play {
        uid: REND_UID,
        target: Some(target),
        selection: SelectionRef::NONE,
    }
}

/// Rend deals 10 with no debuff on the target. At 8 HP the first body kills;
/// the Glam replay then has no receiver. Thorns answered the one hit that
/// landed, the card is discarded once, and the fight goes on.
#[test]
fn a_glam_replay_of_a_rend_that_killed_its_target_lands_on_nobody() {
    let (entry, catalog) = load(8);
    let hp_before = entry.hp;
    let state = engine::apply_action(&entry, &catalog, &rend(0))
        .unwrap()
        .state;
    assert!(!state.history.over);
    assert!(state.monsters[0].hp <= 0);
    assert_eq!(state.monsters[1].hp, entry.monsters[1].hp);
    assert_eq!(state.hp, hp_before - 2, "Thorns 2 answered the one hit");
    let discard: Vec<u32> = state
        .piles
        .get(PileId::Discard)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect();
    assert_eq!(discard.iter().filter(|uid| **uid == REND_UID).count(), 1);
    let mut buffer = LegalActionBuffer::new();
    assert!(
        !engine::legal_actions_checked(&state, &catalog, &mut buffer)
            .unwrap()
            .is_empty()
    );
}

/// The control: a target that survives the first body takes both, and Thorns
/// answers both.
#[test]
fn a_glam_replay_of_a_rend_hits_a_living_target_twice() {
    let (entry, catalog) = load(26);
    let state = engine::apply_action(&entry, &catalog, &rend(0))
        .unwrap()
        .state;
    assert_eq!(state.monsters[0].hp, 26 - 20);
    assert_eq!(state.hp, entry.hp - 4);
}

// ---------------------------------------------------------------------------
// Native witnesses: the engine against the game.
// ---------------------------------------------------------------------------

/// The game's own results, recorded by `tests/native/
/// issue3877_rend_dead_replay_target.py` on the archived v0.111.0 `sts2.dll`
/// through the headless harness: a Silent plays one Rend on the first
/// Toadpole (Thorns 2) of `ENCOUNTER.TOADPOLES_WEAK`, with the replay coming
/// from Glam, Echo Form or One-Two Punch.
const NATIVE: &str = include_str!("native/issue3877_rend_dead_replay_target.json");

/// What a play of Rend leaves behind, in terms both sides can state.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    target_dead: bool,
    /// HP the target lost (`None` once it is dead: native removes the corpse).
    target_damage: Option<i64>,
    other_damage: i64,
    player_hp_lost: i64,
    rend_in_discard: usize,
    over: bool,
}

fn native_outcome(case: &str) -> Outcome {
    let all: serde_json::Value = serde_json::from_str(NATIVE).unwrap();
    let run = &all[case];
    assert_eq!(run["build"]["version"], "v0.111.0");
    assert_eq!(run["play"], "Finished");
    let hp = |list: &serde_json::Value, id: i64| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|monster| monster["id"] == id)
            .map(|monster| monster["hp"].as_i64().unwrap())
    };
    let (before, after) = (&run["monsters_before"], &run["monsters_after"]);
    let target_after = hp(after, 1);
    Outcome {
        target_dead: target_after.is_none(),
        target_damage: target_after.map(|left| hp(before, 1).unwrap() - left),
        other_damage: hp(before, 2).unwrap() - hp(after, 2).unwrap(),
        player_hp_lost: run["player_hp_before"].as_i64().unwrap()
            - run["player_hp_after"].as_i64().unwrap(),
        rend_in_discard: run["discard"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|id| *id == "CARD.REND")
            .count(),
        over: run["over"].as_bool().unwrap(),
    }
}

/// The engine's play of the same Rend. `source` picks where the replay comes
/// from: the fixture's Glam, or a player power installed in its place.
fn engine_outcome(source: &str, first_toadpole_hp: i64) -> Outcome {
    let mut doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    doc.monsters[0].insert("hp".to_string(), first_toadpole_hp.into());
    if source != "glam" {
        let hand = doc.piles.get_mut("hand").unwrap();
        hand[0].enchantment = None;
        doc.player.insert(source.to_string(), 1.into());
        // The power object's row in the side-end ledger, as a live fight
        // carries it.
        doc.player.insert(
            "after_side_turn_end_power_order".to_string(),
            serde_json::json!([[source, 0]]),
        );
        doc.player
            .insert("next_after_side_turn_end_power_uid".to_string(), 1.into());
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let entry = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &entry, &catalog).unwrap();
    let state = engine::apply_action(&entry, &catalog, &rend(0))
        .unwrap()
        .state;
    let target_dead = state.monsters[0].hp <= 0;
    Outcome {
        target_dead,
        target_damage: (!target_dead)
            .then(|| i64::from(entry.monsters[0].hp) - i64::from(state.monsters[0].hp)),
        other_damage: i64::from(entry.monsters[1].hp) - i64::from(state.monsters[1].hp),
        player_hp_lost: i64::from(entry.hp) - i64::from(state.hp),
        rend_in_discard: state
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .filter(|card| card.uid == REND_UID)
            .count(),
        over: state.history.over,
    }
}

/// Glam: the first body kills, the replay starts and finishes a second play
/// (native logs two `CardPlayStartedEntry`) and damages nobody; Thorns
/// answers once.
#[test]
fn native_glam_replay_onto_a_dead_target_matches() {
    let native = native_outcome("glam_lethal");
    assert!(native.target_dead);
    assert_eq!(native.player_hp_lost, 2);
    assert_eq!(engine_outcome("glam", 8), native);
}

#[test]
fn native_glam_replay_onto_a_living_target_matches() {
    let native = native_outcome("glam_survives");
    assert_eq!(native.target_damage, Some(20));
    assert_eq!(engine_outcome("glam", 25), native);
}

/// One-Two Punch replays through the same `OnPlayWrapper` loop.
#[test]
fn native_one_two_punch_replay_onto_a_dead_target_matches() {
    assert_eq!(
        engine_outcome("one_two_punch", 8),
        native_outcome("punch_lethal")
    );
    assert_eq!(
        engine_outcome("one_two_punch", 25),
        native_outcome("punch_survives")
    );
}

/// The native Echo Form runs have the same outcome as Glam's. The engine side
/// is not replayed here: Echo Form reads the first card of a turn, and the
/// fixture is rooted after a Strike was played.
#[test]
fn native_echo_form_replay_has_the_glam_outcome() {
    assert_eq!(native_outcome("echo_lethal"), native_outcome("glam_lethal"));
    assert_eq!(
        native_outcome("echo_survives"),
        native_outcome("glam_survives")
    );
    assert_eq!(
        native_outcome("plain_lethal"),
        native_outcome("glam_lethal"),
        "one body alone already leaves this state"
    );
}

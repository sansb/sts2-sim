//! #2669: native AfterEnergyReset execution over the certified ledger.
//!
//! When the unified acquisition ledger is determined (explicit checkpoint
//! fact or uniquely inferred legacy order), the turn-start reset walk runs
//! every ordinary listener in ledger order with native command gates:
//! GainStars/GainEnergy/Channel skip while ending, Remove does not, and
//! Decrement returns at IsEnding. Actual owner death disables later owner
//! callbacks; legacy-unknown roots keep the split walks and their terminal
//! guard.
//!
//! Beyond the BlackHole/Genesis matrix: full-queue terminal evokes on both
//! ledger certifications, post-terminal turn-start draws, owner death with
//! and without the Illusion veto or Lizard Tail prevention, the Spinner
//! loop break, mid-combat ledger attach/stack order, and no new targets
//! after terminal.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, Event, SelectionRef},
    hot::HotState,
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("hp".into(), json!(50));
    doc.player.insert("max_hp".into(), json!(50));
    doc.player.insert("player_phase".into(), json!(3));
    doc.player.insert("exact_piles".into(), json!(true));
    doc.player.insert("turn".into(), json!(2));
    doc.player.remove("cards_drawn_combat");
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc.piles = serde_json::from_value(json!({
        "hand": [],
        "draw": [],
        "discard": [],
    }))
    .unwrap();
    doc.player.insert("next_card_uid".into(), json!(1));
    doc
}

fn load(doc: &CanonicalStateV2) -> (HotState, Catalog) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn end_turn(doc: &CanonicalStateV2) -> (CanonicalStateV2, HotState, Catalog) {
    let (wire, next, catalog, _) = end_turn_events(doc);
    (wire, next, catalog)
}

fn end_turn_events(doc: &CanonicalStateV2) -> (CanonicalStateV2, HotState, Catalog, Vec<Event>) {
    let (state, catalog) = load(doc);
    let transition = engine::apply_action(&state, &catalog, &Action::EndTurn).unwrap();
    let wire = HotBoundary::try_to_canonical(&transition.state, &catalog).unwrap();
    (wire, transition.state, catalog, transition.events)
}

/// Cold roundtrip plus admission for live states; projection equality only
/// once combat is over.
fn cold(next: &HotState, catalog: &Catalog) -> CanonicalStateV2 {
    let wire = HotBoundary::try_to_canonical(next, catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let back = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&back, &rebuilt).unwrap(),
        wire,
        "cold publication preserves the ended walk"
    );
    if !next.history.over {
        engine::admit(&wire, &back, &rebuilt).unwrap();
    }
    wire
}

fn player(wire: &CanonicalStateV2, key: &str) -> serde_json::Value {
    wire.player.get(key).cloned().unwrap_or(json!(null))
}

/// BlackHole plus Genesis executes around every other peer in both ledger
/// orders: stars granted, BlackHole damage applied, one-shot stacks cleared,
/// durations decremented, and the expiry removals reflected in the ledger.
#[test]
fn black_hole_genesis_executes_around_every_peer_in_both_orders() {
    // (peer key, peer amount, extra player setup, energy delta, peer end state)
    for (peer, amount) in [
        ("star_next_turn", 2),
        ("energy_next_turn", 3),
        ("radiance", 2),
        ("lightning_rod", 1),
        ("spinner", 2),
    ] {
        for order in [vec!["genesis", peer], vec![peer, "genesis"]] {
            let mut doc = entry();
            doc.player.insert("black_hole".into(), json!(2));
            // BlackHole listens AfterCardPlayed: its acquisition row must
            // name a live carrier uid from the shared allocator.
            doc.player.insert(
                "after_card_played_power_order".into(),
                json!([["black_hole", 0]]),
            );
            doc.player
                .insert("next_after_side_turn_end_power_uid".into(), json!(1));
            doc.player.insert("genesis".into(), json!(2));
            doc.player.insert(peer.into(), json!(amount));
            doc.player
                .insert("after_energy_reset_order".into(), json!(order.clone()));
            if peer == "star_next_turn" {
                doc.player
                    .insert("star_energy_reset_order".into(), json!(order.clone()));
            } else {
                doc.player
                    .insert("star_energy_reset_order".into(), json!(["genesis"]));
            }
            if peer == "lightning_rod" || peer == "spinner" {
                doc.player
                    .insert("orb_energy_reset_order".into(), json!([peer]));
                doc.player.insert("orb_slots".into(), json!(3));
            }
            let (wire, _next, _catalog) = end_turn(&doc);
            let order_label = order.join("+");
            // Genesis granted (plus the peer's own grant for StarNextTurn)
            // and BlackHole answered with 2 damage.
            let expected_stars = if peer == "star_next_turn" { 4 } else { 2 };
            assert_eq!(
                player(&wire, "stars"),
                json!(expected_stars),
                "{order_label}"
            );
            // BlackHole answers every star gain, so the StarNextTurn peer
            // deals its damage twice.
            let expected_hp = if peer == "star_next_turn" { 96 } else { 98 };
            assert_eq!(wire.monsters[0]["hp"], json!(expected_hp), "{order_label}");
            // No duration or expiry surprises per peer.
            match peer {
                "star_next_turn" => {
                    assert_eq!(
                        player(&wire, "star_next_turn"),
                        json!(null),
                        "{order_label}"
                    );
                    assert_eq!(
                        player(&wire, "after_energy_reset_order"),
                        json!(["genesis"]),
                        "{order_label}"
                    );
                }
                "energy_next_turn" => {
                    assert_eq!(player(&wire, "energy"), json!(6), "{order_label}");
                    assert_eq!(
                        player(&wire, "energy_next_turn"),
                        json!(null),
                        "{order_label}"
                    );
                    assert_eq!(
                        player(&wire, "after_energy_reset_order"),
                        json!(["genesis"]),
                        "{order_label}"
                    );
                }
                "radiance" => {
                    assert_eq!(player(&wire, "energy"), json!(4), "{order_label}");
                    assert_eq!(player(&wire, "radiance"), json!(1), "{order_label}");
                    assert_eq!(
                        player(&wire, "after_energy_reset_order"),
                        json!(order.clone()),
                        "{order_label}"
                    );
                }
                "lightning_rod" => {
                    assert_eq!(player(&wire, "lightning_rod"), json!(null), "{order_label}");
                    assert_eq!(
                        player(&wire, "orbs"),
                        json!([["LIGHTNING", null]]),
                        "{order_label}"
                    );
                    assert_eq!(
                        player(&wire, "after_energy_reset_order"),
                        json!(["genesis"]),
                        "{order_label}"
                    );
                }
                "spinner" => {
                    assert_eq!(player(&wire, "spinner"), json!(2), "{order_label}");
                    assert_eq!(
                        player(&wire, "orbs"),
                        json!([["GLASS", 4], ["GLASS", 4]]),
                        "{order_label}"
                    );
                    assert_eq!(
                        player(&wire, "after_energy_reset_order"),
                        json!(order.clone()),
                        "{order_label}"
                    );
                }
                _ => unreachable!(),
            }
        }
    }
}

/// A BlackHole lethal mid-walk ends the fight but not the frozen suffix:
/// later gains and channels are ineffective, Remove still clears the
/// one-shot stacks, durations are retained, relic flags reset without
/// grants, and the Late summon is skipped.
#[test]
fn black_hole_lethal_still_runs_the_ending_suffix_once() {
    let mut doc = entry();
    doc.player.insert("black_hole".into(), json!(6));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["black_hole", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert("energy_next_turn".into(), json!(3));
    doc.player.insert("radiance".into(), json!(1));
    doc.player.insert("lightning_rod".into(), json!(1));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["genesis", "energy_next_turn", "radiance", "lightning_rod"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    doc.player
        .insert("orb_energy_reset_order".into(), json!(["lightning_rod"]));
    doc.player.insert("orb_slots".into(), json!(1));
    doc.player
        .insert("fake_tea_set_charged".into(), json!(true));
    doc.player.insert("art_of_war".into(), json!(true));
    doc.player.insert("bound_phylactery".into(), json!(true));
    doc.player
        .insert("art_of_war_current_attack".into(), json!(true));
    doc.player.insert(
        "relics_entering".into(),
        json!([
            "RELIC.BURNING_BLOOD",
            "RELIC.ART_OF_WAR",
            "RELIC.FAKE_VENERABLE_TEA_SET",
            "RELIC.BOUND_PHYLACTERY",
        ]),
    );
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":5,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    let (wire, next, _catalog) = end_turn(&doc);
    // The Genesis gain landed, then BlackHole ended the fight.
    assert_eq!(player(&wire, "stars"), json!(2));
    assert_eq!(player(&wire, "over"), json!(true));
    assert!(wire.monsters[0]["hp"].as_i64().unwrap() <= 0);
    // EnergyNextTurn was removed without its grant: the base reset (3) is
    // intact on the hot state, and the wire omits energy because 3 is the
    // canonical default.
    assert_eq!(next.energy, 3);
    assert_eq!(player(&wire, "energy"), json!(null));
    assert_eq!(player(&wire, "energy_next_turn"), json!(null));
    // Radiance kept its duration without granting.
    assert_eq!(player(&wire, "radiance"), json!(1));
    // LightningRod never channeled and kept its duration; the empty orb
    // bar is omitted from the wire.
    assert_eq!(player(&wire, "lightning_rod"), json!(1));
    assert_eq!(player(&wire, "orbs"), json!(null));
    // Relic flags reset without grants (false is the wire default, so a
    // reset latch reads back null); the Late summon never fired.
    assert_eq!(player(&wire, "art_of_war_last_attack"), json!(null));
    assert_eq!(player(&wire, "art_of_war_current_attack"), json!(null));
    assert_eq!(player(&wire, "fake_tea_set_charged"), json!(null));
    assert_eq!(player(&wire, "pet"), json!(null));
    // Expiry removals are reflected; retained listeners keep their places.
    assert_eq!(
        player(&wire, "after_energy_reset_order"),
        json!(["genesis", "radiance", "lightning_rod"])
    );
}

/// A full-queue terminal evoke finishes the entered LightningRod suffix
/// (the evoke kills, a new Lightning enqueues) while the later Spinner
/// channel is skipped and both durations are retained. Identical on the
/// explicit ledger and the uniquely inferred legacy order.
#[test]
fn full_queue_terminal_evoke_completes_entered_suffix() {
    for explicit in [true, false] {
        let mut doc = entry();
        doc.player.insert("lightning_rod".into(), json!(2));
        doc.player.insert("spinner".into(), json!(1));
        if explicit {
            doc.player.insert(
                "after_energy_reset_order".into(),
                json!(["lightning_rod", "spinner"]),
            );
        }
        doc.player.insert(
            "orb_energy_reset_order".into(),
            json!(["lightning_rod", "spinner"]),
        );
        doc.player.insert("orb_slots".into(), json!(1));
        doc.player
            .insert("orbs".into(), json!([["LIGHTNING", null]]));
        doc.monsters = serde_json::from_value(json!([
            {"kind":"TOADPOLE","hp":5,"max_hp":100,"loop_pos":2}
        ]))
        .unwrap();
        let (wire, next, catalog) = end_turn(&doc);
        let label = if explicit { "explicit" } else { "inferred" };
        assert_eq!(player(&wire, "over"), json!(true), "{label}");
        assert!(wire.monsters[0]["hp"].as_i64().unwrap() <= 0, "{label}");
        // Decrement returned at IsEnding: both durations retained.
        assert_eq!(player(&wire, "lightning_rod"), json!(2), "{label}");
        assert_eq!(player(&wire, "spinner"), json!(1), "{label}");
        // The entered channel evoked the old Lightning and enqueued a new
        // one; the skipped Spinner channel queued no Glass.
        assert_eq!(
            player(&wire, "orbs"),
            json!([["LIGHTNING", null]]),
            "{label}"
        );
        if explicit {
            assert_eq!(
                player(&wire, "after_energy_reset_order"),
                json!(["lightning_rod", "spinner"]),
                "{label}"
            );
        } else {
            assert!(
                !wire.player.contains_key("after_energy_reset_order"),
                "{label}: inferred ledgers stay off the wire"
            );
        }
        cold(&next, &catalog);
    }
}

/// A lethal mid-walk does not start the turn-start draw: with a full draw
/// pile and Infinite Blades live, the hand stays empty, the draw pile is
/// untouched, no Shivs mint, and no combat-draw counter opens.
#[test]
fn post_terminal_turn_start_draws_nothing() {
    let mut doc = entry();
    doc.player.insert("black_hole".into(), json!(6));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["black_hole", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player.insert("genesis".into(), json!(2));
    doc.player
        .insert("after_energy_reset_order".into(), json!(["genesis"]));
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    doc.player.insert("infinite_blades".into(), json!(2));
    doc.player.insert(
        "before_hand_draw_power_order".into(),
        json!(["infinite_blades"]),
    );
    doc.piles = serde_json::from_value(json!({
        "hand": [],
        "draw": [
            {"id":"STRIKE_IRONCLAD","uid":0,"upgrade":0},
            {"id":"DEFEND_IRONCLAD","uid":1,"upgrade":0},
            {"id":"DEFEND_IRONCLAD","uid":2,"upgrade":0},
            {"id":"DEFEND_IRONCLAD","uid":3,"upgrade":0},
            {"id":"DEFEND_IRONCLAD","uid":4,"upgrade":0},
            {"id":"DEFEND_IRONCLAD","uid":5,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":5,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    let (wire, next, catalog) = end_turn(&doc);
    assert_eq!(player(&wire, "over"), json!(true));
    assert!(wire.monsters[0]["hp"].as_i64().unwrap() <= 0);
    assert!(wire.piles["hand"].is_empty());
    assert_eq!(wire.piles["draw"].len(), 6);
    assert!(
        wire.piles.values().flatten().all(|card| card.id != "SHIV"),
        "no Shivs mint after terminal"
    );
    assert!(!wire.player.contains_key("cards_drawn_combat"));
    cold(&next, &catalog);
}

fn owner_fixture(text: &'static str) -> CanonicalStateV2 {
    serde_json::from_str(text).unwrap()
}

/// Owner death mid-walk skips later callbacks and detaches the Type1
/// listeners: the entered rod evoke finished (a monster took damage) while
/// Genesis never granted and both attachments are gone.
#[test]
fn owner_death_mid_walk_skips_later_callbacks_and_cleans_up() {
    let mut doc = owner_fixture(include_str!("../fixtures/ea6_reset_owner_death.json"));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["lightning_rod", "genesis"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    let (wire, next, catalog) = end_turn(&doc);
    assert_eq!(player(&wire, "over"), json!(true));
    assert_eq!(
        player(&wire, "hp"),
        json!(null),
        "dead owners project no hp"
    );
    assert_eq!(next.hp, 0);
    assert_eq!(player(&wire, "stars"), json!(null), "genesis never granted");
    assert_eq!(player(&wire, "genesis"), json!(null), "detached");
    assert_eq!(player(&wire, "lightning_rod"), json!(null), "detached");
    assert!(
        wire.monsters
            .iter()
            .any(|monster| monster["hp"].as_i64().unwrap() < monster["max_hp"].as_i64().unwrap()),
        "the entered evoke finished its suffix"
    );
    cold(&next, &catalog);
}

/// Under the Illusion veto the attachments survive while the hooks stay
/// inert: Genesis keeps its stacks but grants nothing, and LightningRod
/// keeps its duration because the decrement gate sees the deactivation.
#[test]
fn illusion_veto_preserves_attachments_but_hooks_stay_inert() {
    let mut doc = owner_fixture(include_str!("../fixtures/ea6_reset_owner_illusion.json"));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["lightning_rod", "genesis"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    let (wire, next, catalog) = end_turn(&doc);
    assert_eq!(player(&wire, "over"), json!(true));
    assert_eq!(
        player(&wire, "hp"),
        json!(null),
        "dead owners project no hp"
    );
    assert_eq!(next.hp, 0);
    // Loop-head skip: the later callback never ran (0 is wire-omitted).
    assert_eq!(player(&wire, "stars"), json!(null));
    assert_eq!(player(&wire, "genesis"), json!(2), "veto kept the stack");
    // Decrement gate: deactivation (not ending) retained the duration.
    assert_eq!(player(&wire, "lightning_rod"), json!(2));
    assert_eq!(
        player(&wire, "after_energy_reset_order"),
        json!(["lightning_rod", "genesis"])
    );
    cold(&next, &catalog);
}

/// A prevented death (Lizard Tail) never deactivates the hooks, so the walk
/// continues: Genesis grants and LightningRod decrements live.
#[test]
fn prevented_death_keeps_walking() {
    let mut doc = owner_fixture(include_str!("../fixtures/ea6_reset_owner_lizard_tail.json"));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["lightning_rod", "genesis"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    let (wire, next, catalog) = end_turn(&doc);
    assert!(
        !wire.player.contains_key("over"),
        "combat continues after prevention"
    );
    assert_eq!(player(&wire, "hp"), json!(50), "lizard tail healed half");
    assert_eq!(player(&wire, "stars"), json!(2));
    assert_eq!(player(&wire, "lightning_rod"), json!(1));
    assert_eq!(player(&wire, "genesis"), json!(2));
    cold(&next, &catalog);
}

/// The Spinner loop breaks when its first channel kills the owner: the
/// iter-1 evoke drops the weak toadpole, Gremlin Horn draws into a
/// Hellraiser Strike counterattack, and the reflected Thorns kill the owner
/// before iteration two. The later Rod and Genesis callbacks never run even
/// though the veto preserves their attachments.
#[test]
fn spinner_loop_breaks_on_mid_walk_owner_death() {
    let mut doc = owner_fixture(include_str!("../fixtures/ea6_reset_owner_illusion.json"));
    doc.player.insert("spinner".into(), json!(2));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["spinner", "lightning_rod", "genesis"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    doc.player.insert(
        "orb_energy_reset_order".into(),
        json!(["spinner", "lightning_rod"]),
    );
    let (wire, next, catalog, events) = end_turn_events(&doc);
    assert_eq!(player(&wire, "over"), json!(true));
    assert_eq!(
        player(&wire, "hp"),
        json!(null),
        "dead owners project no hp"
    );
    assert_eq!(next.hp, 0);
    let hits = events
        .iter()
        .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
        .count();
    assert_eq!(
        hits, 2,
        "iter-1 evoke plus the Strike counterattack; no second evoke, no rod evoke"
    );
    assert_eq!(player(&wire, "stars"), json!(null));
    assert_eq!(player(&wire, "spinner"), json!(2), "veto kept the stack");
    assert_eq!(player(&wire, "lightning_rod"), json!(2));
    assert_eq!(player(&wire, "genesis"), json!(2));
    assert_eq!(
        player(&wire, "after_energy_reset_order"),
        json!(["spinner", "lightning_rod", "genesis"])
    );
    cold(&next, &catalog);
}

/// A mid-combat attach appends to the ledger end (reapply order) while
/// stacking never duplicates it; the generated listener then walks and
/// cold-reloads exactly.
#[test]
fn mid_combat_attach_appends_and_stacking_does_not_duplicate() {
    // Reapply: RefineBlade attaches a fresh EnergyNextTurn at the end.
    let mut doc = entry();
    doc.player.insert("genesis".into(), json!(2));
    doc.player
        .insert("after_energy_reset_order".into(), json!(["genesis"]));
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    doc.piles = serde_json::from_value(json!({
        "hand": [{"id":"REFINE_BLADE","uid":0,"upgrade":0}],
        "draw": [],
        "discard": [],
    }))
    .unwrap();
    let (state, catalog) = load(&doc);
    let played = engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
    .state;
    assert_eq!(
        played.powers.value(sts_sim::ids::PowerId::EnergyNextTurn),
        1
    );
    let wire = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    assert_eq!(
        wire.player["after_energy_reset_order"],
        json!(["genesis", "energy_next_turn"])
    );
    // The generated listener walks at turn start, then cold-reloads.
    let (wire, next, catalog) = end_turn(&wire);
    assert_eq!(next.energy, 4, "base 3 plus the generated grant");
    assert_eq!(player(&wire, "energy_next_turn"), json!(null));
    cold(&next, &catalog);

    // Stack: RefineBlade onto a live EnergyNextTurn stacks the amount but
    // keeps the single ledger entry.
    let mut doc = entry();
    doc.player.insert("energy_next_turn".into(), json!(2));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["energy_next_turn"]),
    );
    doc.piles = serde_json::from_value(json!({
        "hand": [{"id":"REFINE_BLADE","uid":0,"upgrade":0}],
        "draw": [],
        "discard": [],
    }))
    .unwrap();
    let (state, catalog) = load(&doc);
    let played = engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
    .state;
    assert_eq!(
        played.powers.value(sts_sim::ids::PowerId::EnergyNextTurn),
        3
    );
    let wire = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    assert_eq!(
        wire.player["after_energy_reset_order"],
        json!(["energy_next_turn"])
    );
    cold(&played, &catalog);
}

/// After a terminal mid-walk, the later Rod callback never channels: no
/// new orb, no new targets, exactly the one lethal damage event.
#[test]
fn no_new_targets_after_terminal() {
    let mut doc = entry();
    doc.player.insert("black_hole".into(), json!(6));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["black_hole", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.player.insert("genesis".into(), json!(2));
    doc.player.insert("lightning_rod".into(), json!(1));
    doc.player.insert(
        "after_energy_reset_order".into(),
        json!(["genesis", "lightning_rod"]),
    );
    doc.player
        .insert("star_energy_reset_order".into(), json!(["genesis"]));
    doc.player
        .insert("orb_energy_reset_order".into(), json!(["lightning_rod"]));
    doc.player.insert("orb_slots".into(), json!(1));
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":5,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    let (wire, next, catalog, events) = end_turn_events(&doc);
    assert_eq!(player(&wire, "over"), json!(true));
    assert_eq!(player(&wire, "stars"), json!(2));
    assert_eq!(player(&wire, "lightning_rod"), json!(1));
    assert_eq!(player(&wire, "orbs"), json!(null));
    let hits = events
        .iter()
        .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
        .count();
    assert_eq!(hits, 1, "only the lethal BlackHole answer targeted");
    cold(&next, &catalog);
}

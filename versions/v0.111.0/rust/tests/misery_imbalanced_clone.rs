//! #2647 slice B2 — `Misery` cloning `ImbalancedPower` at root level.
//!
//! The unit witnesses live beside the code they pin; this file carries the
//! one chain that only a public root can show: admit a Bowlbug roster, play
//! Misery on the Rock, cold reload, let the clone's carrier deal a fully
//! blocked attack, cold reload again, and watch it resume the parked move its
//! own listener parked.
use serde_json::{Value, json};
use sts_sim::{boundary::HotBoundary, canonical::CanonicalStateV2, engine};

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

/// A `build_bowlbugs_weak`-shaped root: the Rock in slot 0 with its intrinsic
/// `ImbalancedPower`, a Bowlbug Egg beside it, and a Misery in hand.
///
/// Block is set high enough that the Egg's Bite is fully blocked, which is
/// `ImbalancedPower/<AfterDamageGiven>d__4::MoveNext` `0x33cd60` IL_0033's
/// only precondition once the clone has landed.
fn bowlbug_root() -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    document.monsters = vec![
        serde_json::from_value(json!({"kind": "BOWLBUG_ROCK", "hp": 46, "max_hp": 46})).unwrap(),
        serde_json::from_value(
            json!({"kind": "BOWLBUG_EGG", "hp": 24, "max_hp": 24, "slot": 1, "uid": 1}),
        )
        .unwrap(),
    ];
    document.player.insert("block".to_owned(), json!(40));
    let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
    piles["hand"] = json!([{"id": "MISERY", "uid": 0, "upgrade": 0}]);
    document.piles = serde_json::from_value(piles).unwrap();
    document
}

fn hydrate(document: &CanonicalStateV2) -> (sts_sim::catalog::Catalog, sts_sim::hot::HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    (catalog, state)
}

/// Project, hydrate from the projection, and assert the reload reproduces the
/// state the ENGINE built — not merely that the document round-trips.
///
/// The roster is compared structurally rather than through a second
/// projection: #2710's round-2 lesson is that a self-consistent layout bug
/// survives `project -> hydrate -> project` untouched, and the ledger is
/// exactly the field that bug would live in. `HotState` as a whole is not
/// comparable across the two catalogs, whose card atoms are interned in
/// different orders, so the document equality below carries the rest.
fn cold_reload(state: &sts_sim::hot::HotState, catalog: &sts_sim::catalog::Catalog) {
    let projected = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let (reload_catalog, restored) = hydrate(&projected);
    engine::admit(&projected, &restored, &reload_catalog).unwrap();
    assert_eq!(
        restored.monsters, state.monsters,
        "the cold-reloaded roster is not what the engine built",
    );
    assert_eq!(
        HotBoundary::try_to_canonical(&restored, &reload_catalog).unwrap(),
        projected,
    );
}

#[test]
fn a_bowlbug_root_clones_imbalanced_and_the_carrier_generically_stuns() {
    let document = bowlbug_root();
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(
        state.monsters[1]
            .misery_debuff_order
            .attachments()
            .is_empty(),
        "the Egg starts with no attachment",
    );

    // Play Misery on the Rock. `0x3ad358` IL_003e-IL_009c froze the Rock's
    // one Type-2 power before the attack; IL_0320-IL_035b then clones it onto
    // every other hittable enemy.
    let played = engine::apply_action_into(
        &state,
        &catalog,
        &engine::Action::Play {
            uid: 0,
            target: Some(0),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(played.monsters[0].hp, 39);
    assert_eq!(
        played.monsters[1].misery_debuff_order.attachments().len(),
        1,
    );
    cold_reload(&played, &catalog);

    // End the turn. The Egg bites into 40 Block, so
    // `ImbalancedPower/<AfterDamageGiven>d__4::MoveNext` `0x33cd60` reaches
    // IL_0068-IL_006f and awaits `CreatureCmd::Stun(Owner, null)` — the arm
    // slice A could not reach and B1 could only construct.
    let ended =
        engine::apply_action_into(&played, &catalog, &engine::Action::EndTurn, &mut Vec::new())
            .unwrap();
    assert_eq!(
        ended.monsters[1].override_state,
        sts_sim::hot::MonsterOverride::Stunned,
    );
    // The Egg's generated loop is the single `BITE` row, so `StunInternal`
    // `0x11d7cc` IL_0031-IL_0055's `StateLog.Last()` is that move and the
    // parked telegraph is `EggBite`.
    assert_eq!(
        ended.monsters[1].forced_follow_up,
        sts_sim::hot::MonsterFollowUp::EggBite,
    );
    // The Rock's own arm is the latch, never this one.
    assert_ne!(
        ended.monsters[0].override_state,
        sts_sim::hot::MonsterOverride::Stunned,
    );
    cold_reload(&ended, &catalog);

    // Resume from the COLD copy: the stun consumes one enemy action and the
    // parked telegraph performs next.
    let projected = HotBoundary::try_to_canonical(&ended, &catalog).unwrap();
    let (reload_catalog, restored) = hydrate(&projected);
    let resumed = engine::apply_action_into(
        &restored,
        &reload_catalog,
        &engine::Action::EndTurn,
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        resumed.monsters[1].override_state,
        sts_sim::hot::MonsterOverride::None,
        "the stun performed and cleared",
    );
    assert_eq!(
        resumed.monsters[1].forced_follow_up,
        sts_sim::hot::MonsterFollowUp::None,
    );
    cold_reload(&resumed, &reload_catalog);
}

/// The roster-level half: a root whose Misery reader could reach an
/// unrepresentable recipient refuses at ADMISSION, not late inside a search.
#[test]
fn a_bowlbug_root_with_an_unrepresentable_recipient_refuses_at_admission() {
    for (kind, uid) in [("TERROR_EEL", 1u32), ("TOADPOLE", 1)] {
        let mut document = bowlbug_root();
        document.monsters[1] = serde_json::from_value(
            json!({"kind": kind, "hp": 24, "max_hp": 24, "slot": 1, "uid": uid}),
        )
        .unwrap();
        let (catalog, state) = hydrate(&document);
        assert!(
            engine::admit(&document, &state, &catalog)
                .unwrap_err()
                .contains(engine::MissingCapability::ArgumentShape(
                    "Misery Imbalanced clone recipient"
                )),
            "{kind} must refuse at admission",
        );
    }

    // Thorns on an otherwise eligible recipient, for #2647 §5.4's reason.
    let mut thorny = bowlbug_root();
    thorny.monsters[1].insert("thorns".to_owned(), json!(2));
    let (catalog, state) = hydrate(&thorny);
    assert!(
        engine::admit(&thorny, &state, &catalog)
            .unwrap_err()
            .contains(engine::MissingCapability::ArgumentShape(
                "Misery Imbalanced clone recipient"
            )),
    );

    // The control: the same roster with no Misery in the catalog admits, so
    // the refusal really is the reader's and not the roster's.
    let mut no_misery = bowlbug_root();
    no_misery.monsters[1] = serde_json::from_value(
        json!({"kind": "TOADPOLE", "hp": 24, "max_hp": 24, "slot": 1, "uid": 1}),
    )
    .unwrap();
    let mut piles: Value = serde_json::to_value(&no_misery.piles).unwrap();
    piles["hand"] = json!([{"id": "STRIKE_IRONCLAD", "uid": 0, "upgrade": 0}]);
    no_misery.piles = serde_json::from_value(piles).unwrap();
    let (catalog, state) = hydrate(&no_misery);
    engine::admit(&no_misery, &state, &catalog).unwrap();
}

/// #3404: an Unsettling Lamp beside a carrier admits. The clone's applier is
/// a monster, so the Lamp's given-side multiplier is gated on
/// `ICombatState.ContainsCreature(applier)` (`0x3efbac` IL_01d3-IL_01ec),
/// which the clone now reads from the applier row
/// (`damage::apply_card_monster_imbalanced_clone`). Played on the Rock, the
/// Misery copies only the monster-applied Imbalanced, which can neither latch
/// the Lamp (`0x9d4fc` IL_0032-IL_003f) nor be doubled by an unlatched one
/// (`0x9d5a8` IL_000c-IL_0012): the Egg gets 1, the Lamp stays available, and
/// the successor reloads cold and admits.
#[test]
fn a_bowlbug_root_with_an_unsettling_lamp_admits_and_clones_undoubled() {
    let mut document = bowlbug_root();
    document.player.insert("lamp".to_owned(), json!(true));
    document.player.insert(
        "relics_entering".to_owned(),
        json!(["RELIC.BURNING_BLOOD", "RELIC.UNSETTLING_LAMP"]),
    );
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(state.fanouts.unsettling_lamp_available());
    let played = engine::apply_action_into(
        &state,
        &catalog,
        &engine::Action::Play {
            uid: 0,
            target: Some(0),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let attachments: Vec<_> = played.monsters[1]
        .misery_debuff_order
        .attachments()
        .map(|record| record.amount)
        .collect();
    assert_eq!(attachments, [1]);
    assert!(played.fanouts.unsettling_lamp_available());
    cold_reload(&played, &catalog);
}

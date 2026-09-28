//! #2693 S1 — monster Strength as an ordered attachment, at root level.
//!
//! The unit witnesses live beside the code they pin. This file carries the
//! one chain only a public root can show, and it is the chain the issue was
//! filed on: a Toadpole with Malaise and Misery in hand, Malaise played so the
//! monster ends at `Strength -3` / `Weak 3`, that exact output projected and
//! **cold reloaded**, and Misery then played from the cold copy.
//!
//! Before this stage the reload refused `Misery power acquisition order`,
//! because `misery_scalar_state_is_exact` rejected a negative `Strength`
//! categorically. What it rejected was *unrecorded provenance*, and Malaise
//! now records it: `Malaise/<OnPlay>d__9::MoveNext` `0x3ab428` IL_00fc-IL_0109
//! applies `-powerAmount` Strength with `card.Owner.Player.Creature` as the
//! applier, before it proceeds to Weak — so the ledger ends
//! `[Strength row, Weak token]`, in that order, and that order is what
//! `Misery` replays.
use serde_json::{Value, json};
use sts_sim::{boundary::HotBoundary, canonical::CanonicalStateV2, engine};

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

/// The #2693 reproducer's shape: one Toadpole, Malaise and Misery in hand.
///
/// `rest` adds extra monsters, so the same root serves the single-target case
/// (which only has to admit and replay) and the two-monster case (where the
/// clone actually executes).
fn malaise_root(rest: &[Value]) -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let mut monsters = vec![
        serde_json::from_value(json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100})).unwrap(),
    ];
    for extra in rest {
        monsters.push(serde_json::from_value(extra.clone()).unwrap());
    }
    document.monsters = monsters;
    let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
    piles["hand"] = json!([
        {"id": "MALAISE", "uid": 0, "upgrade": 0},
        {"id": "MISERY", "uid": 1, "upgrade": 0},
    ]);
    document.piles = serde_json::from_value(piles).unwrap();
    document
}

fn hydrate(document: &CanonicalStateV2) -> (sts_sim::catalog::Catalog, sts_sim::hot::HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    (catalog, state)
}

/// Project, hydrate the projection, admit it, and assert the reload is the
/// roster the ENGINE built rather than a re-hydration of its own output
/// (#2710's round-2 lesson). Returns the cold pair so a caller can go on
/// replaying from it.
fn cold_reload(
    state: &sts_sim::hot::HotState,
    catalog: &sts_sim::catalog::Catalog,
) -> (
    CanonicalStateV2,
    sts_sim::catalog::Catalog,
    sts_sim::hot::HotState,
) {
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
    (projected, reload_catalog, restored)
}

fn play(
    state: &sts_sim::hot::HotState,
    catalog: &sts_sim::catalog::Catalog,
    uid: u32,
    target: usize,
) -> sts_sim::hot::HotState {
    engine::apply_action_into(
        state,
        catalog,
        &engine::Action::Play {
            uid,
            target: Some(target as u8),
            selection: engine::SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap()
}

/// The issue's own reproducer, end to end.
#[test]
fn the_public_malaise_root_cold_reloads_and_then_admits_misery() {
    let document = malaise_root(&[]);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    let after_malaise = play(&state, &catalog, 0, 0);
    let monster = &after_malaise.monsters[0];
    assert_eq!(monster.powers.value(sts_sim::ids::PowerId::Strength), -3);
    assert_eq!(monster.powers.value(sts_sim::ids::PowerId::Weak), 3);
    // Strength is applied BEFORE Weak, so the row leads the ledger — the
    // order a later `Misery` replays, and the order a recipient's Artifact
    // would consume against.
    assert_eq!(
        monster.misery_debuff_order.placed_attachments(),
        vec![(
            0,
            sts_sim::hot::AttachmentRecord {
                power: sts_sim::hot::AttachedPowerModel::Strength,
                applier: sts_sim::hot::Applier::Player,
                amount: -3,
            }
        )],
    );
    assert_eq!(
        monster
            .misery_debuff_order
            .as_slice()
            .copied()
            .collect::<Vec<_>>(),
        vec![sts_sim::hot::MiseryToken::Weak],
    );

    // This is the exact step #2693 was filed on: projecting that output and
    // loading it refused `Misery power acquisition order`.
    let (projected, cold_catalog, cold) = cold_reload(&after_malaise, &catalog);
    assert_eq!(
        projected.monsters[0]["power_attachments"],
        json!([["strength", "player", 0, -3, 0]]),
    );

    // ...and Misery now plays from the cold copy. With one monster the
    // post-hit `HittableEnemies` walk (`0x3ad358` IL_01ed) has no recipient
    // but the source snapshot still has to be exact, which is what refused.
    let warm = play(&after_malaise, &catalog, 1, 0);
    let cold_played = play(&cold, &cold_catalog, 1, 0);
    assert_eq!(
        HotBoundary::try_to_canonical(&warm, &catalog).unwrap(),
        HotBoundary::try_to_canonical(&cold_played, &cold_catalog).unwrap(),
        "the warm and cold replays diverged",
    );
    assert_eq!(warm.monsters[0].hp, 93);
    cold_reload(&cold_played, &cold_catalog);
}

/// The same root with a second monster, so the clone actually executes.
#[test]
fn a_malaised_source_clones_its_negative_strength_onto_every_other_enemy() {
    let document = malaise_root(&[
        json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 1, "uid": 1}),
        json!({"kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 2, "uid": 2}),
    ]);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    let after_malaise = play(&state, &catalog, 0, 0);
    let (_, cold_catalog, cold) = cold_reload(&after_malaise, &catalog);

    let played = play(&cold, &cold_catalog, 1, 0);
    // Recipient-major (`0x3ad358` IL_0236-IL_03c3): each other living monster
    // takes the whole frozen walk. The Strength copy is a FRESH attach there,
    // so `set_Applier` `0x3efbac` IL_013d writes the applier the clone
    // carried — the player, from the source's own row.
    for index in 1..=2 {
        let recipient = &played.monsters[index];
        assert_eq!(recipient.powers.value(sts_sim::ids::PowerId::Strength), -3);
        assert_eq!(recipient.powers.value(sts_sim::ids::PowerId::Weak), 3);
        assert_eq!(
            recipient.misery_debuff_order.placed_attachments(),
            vec![(
                0,
                sts_sim::hot::AttachmentRecord {
                    power: sts_sim::hot::AttachedPowerModel::Strength,
                    applier: sts_sim::hot::Applier::Player,
                    amount: -3,
                }
            )],
            "recipient {index}",
        );
    }
    // The source grows no second instance from its own clone.
    assert_eq!(
        played.monsters[0].misery_debuff_order.attachments().len(),
        1,
    );
    cold_reload(&played, &cold_catalog);
}

/// The upkeep gate: a fight that cannot reach `Misery` records no provenance
/// and projects byte-identically to the pre-#2693 engine.
///
/// This is the half of coordinator ruling 1 that a census cannot show. The
/// two roots differ only in whether a `MISERY` card is in hand, and the
/// Malaise they both play is the same command; what differs is the projected
/// document, and only by the presence of the row.
#[test]
fn a_fight_without_misery_records_no_strength_provenance() {
    let mut without = malaise_root(&[]);
    let mut piles: Value = serde_json::to_value(&without.piles).unwrap();
    piles["hand"] = json!([{"id": "MALAISE", "uid": 0, "upgrade": 0}]);
    without.piles = serde_json::from_value(piles).unwrap();

    let (catalog, state) = hydrate(&without);
    engine::admit(&without, &state, &catalog).unwrap();
    assert!(!state.fanouts.misery_attachment_upkeep());
    let played = play(&state, &catalog, 0, 0);
    assert_eq!(
        played.monsters[0]
            .powers
            .value(sts_sim::ids::PowerId::Strength),
        -3,
    );
    assert!(
        played.monsters[0]
            .misery_debuff_order
            .attachments()
            .is_empty(),
    );
    let projected = HotBoundary::try_to_canonical(&played, &catalog).unwrap();
    assert!(
        !projected.monsters[0].contains_key("power_attachments"),
        "a Misery-free fight must project the pre-#2693 bytes",
    );
    // ...and it still admits and replays, which is the thing that must not
    // regress: unrecorded provenance beside a NEGATIVE Strength refuses only
    // where a reader exists, and here there is none.
    let (reload_catalog, restored) = hydrate(&projected);
    engine::admit(&projected, &restored, &reload_catalog).unwrap();
    assert_eq!(restored.monsters, played.monsters);

    // The control: the identical root WITH Misery reachable does record it.
    let with = malaise_root(&[]);
    let (with_catalog, with_state) = hydrate(&with);
    assert!(with_state.fanouts.misery_attachment_upkeep());
    let with_played = play(&with_state, &with_catalog, 0, 0);
    assert_eq!(
        HotBoundary::try_to_canonical(&with_played, &with_catalog)
            .unwrap()
            .monsters[0]["power_attachments"],
        json!([["strength", "player", 0, -3, 0]]),
    );
}

/// An enemy-applied Strength row is distinguishable from a player-applied
/// one at root level, and the engine builds it that way.
///
/// `RitualPower/<AfterSideTurnEnd>d__11::MoveNext` `0x342a94` IL_0052-IL_0071
/// is `Apply<StrengthPower>(ctx, Owner, Amount, /*applier*/ Owner, null, 0)`,
/// so the row records `Applier::Monster(owner uid)` — not the player, and not
/// the null a relic would pass.
#[test]
fn a_ritual_monster_records_itself_as_the_applier() {
    let mut document = malaise_root(&[]);
    document.monsters[0].insert("kind".to_owned(), json!("DAMP_CULTIST"));
    document.monsters[0].insert("ritual".to_owned(), json!(2));
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    assert!(state.fanouts.misery_attachment_upkeep());

    // The Cultist's own turn applies Ritual, which sets
    // `RitualPower::WasJustAppliedByEnemy` (`AfterApplied` `0xa6d23`), and
    // `<AfterSideTurnEnd>d__11` IL_0038-IL_0047 spends exactly that flag on
    // the first side end. The SECOND side end is the first one that applies
    // Strength, and the third restacks it.
    let mut ended = state;
    let mut amounts = Vec::new();
    for _ in 0..3 {
        ended =
            engine::apply_action_into(&ended, &catalog, &engine::Action::EndTurn, &mut Vec::new())
                .unwrap();
        amounts.push(
            ended.monsters[0]
                .powers
                .value(sts_sim::ids::PowerId::Strength),
        );
    }
    assert_eq!(amounts, vec![0, 8, 16], "Ritual ticks after its fresh turn");

    let monster = &ended.monsters[0];
    assert_eq!(
        monster.misery_debuff_order.placed_attachments(),
        vec![(
            0,
            sts_sim::hot::AttachmentRecord {
                power: sts_sim::hot::AttachedPowerModel::Strength,
                applier: sts_sim::hot::Applier::Monster(monster.uid),
                amount: 16,
            }
        )],
        "an enemy self-buff records the enemy, not the player, and restacks \
         in place rather than growing a second position",
    );
    cold_reload(&ended, &catalog);

    // A positive row is not in `Misery`'s dictionary (`0x3ad30a` reads
    // `TypeForCurrentAmount`, and `0x83a94` IL_0013-IL_003e is 2 only while
    // the amount is negative), so playing Misery here copies nothing from it
    // and the row keeps its position and its enemy applier.
    let played = play(&ended, &catalog, 1, 0);
    assert_eq!(
        played.monsters[0]
            .powers
            .value(sts_sim::ids::PowerId::Strength),
        16,
    );
    assert_eq!(
        played.monsters[0]
            .misery_debuff_order
            .placed_attachments()
            .len(),
        1,
    );
}

/// A recipient's single Artifact consumes on whichever copy lands first, so
/// Strength-before-Weak and Weak-before-Strength end in observably different
/// states. That is what makes the ledger position load-bearing rather than
/// bookkeeping.
#[test]
fn the_recipients_artifact_consumes_on_whichever_copy_the_ledger_orders_first() {
    let document = malaise_root(&[json!({
        "kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 1, "uid": 1, "artifact": 1
    })]);
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();

    // Malaise applies Strength then Weak, so the Strength copy is first and
    // the Artifact spends itself on it; the Weak copy lands.
    let played = play(&play(&state, &catalog, 0, 0), &catalog, 1, 0);
    let recipient = &played.monsters[1];
    assert_eq!(recipient.powers.value(sts_sim::ids::PowerId::Artifact), 0);
    assert_eq!(recipient.powers.value(sts_sim::ids::PowerId::Strength), 0);
    assert_eq!(recipient.powers.value(sts_sim::ids::PowerId::Weak), 3);
    assert!(
        recipient.misery_debuff_order.attachments().is_empty(),
        "a blocked copy never attaches (`ApplyInternal` `0x84012` IL_0001-IL_000e)",
    );
    cold_reload(&played, &catalog);
}

/// #2727 — an entering Strength whose applier was never observed refuses at
/// admission wherever Sleight of Flesh can read it.
///
/// `damage::materialize_entering_strength_provenance` records such an instance
/// as `Applier::Unknown`: its position is determined (nothing else is on the
/// ledger) and its amount is the scalar, but *who applied it* is simply absent
/// from the evidence. That was exact for everything #2693 needed — the
/// singleton lookup `PowerCmd::FindExistingInstanceForStacking` `0x1338d8`
/// IL_0058 resolves by model id, and the only applier readers were the two
/// given-side modifiers the Lamp rule already refuses beside.
///
/// Sleight of Flesh is a new reader, and it is not satisfiable either way.
/// `SleightOfFleshPower/<AfterPowerAmountChanged>d__6::MoveNext` `0x344dc4`
/// IL_0067-IL_0075 fires only when the `Misery` copy's applier is the player,
/// and an entering `Strength -3` is exactly as consistent with the player's
/// own Malaise before the checkpoint as with an `initial_powers` or ascension
/// write. So the root refuses by name rather than guessing — I5.
///
/// Three controls, so the refusal cannot be the roster's, the deck's or the
/// Strength's: the same root without Sleight of Flesh admits and projects the
/// `unknown` row, the same root whose row IS recorded as the player admits
/// with Sleight of Flesh in hand, and the same root without Misery admits
/// because the ledger then has no reader at all.
#[test]
fn an_unknown_entering_strength_refuses_beside_a_reachable_sleight_of_flesh() {
    let with_sleight = |strength: Value| {
        let mut document = malaise_root(&[json!({
            "kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 1, "uid": 1
        })]);
        document.monsters[0].insert("strength".to_owned(), strength);
        let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
        piles["hand"] = json!([
            {"id": "MALAISE", "uid": 0, "upgrade": 0},
            {"id": "MISERY", "uid": 1, "upgrade": 0},
            {"id": "SLEIGHT_OF_FLESH", "uid": 2, "upgrade": 0},
        ]);
        document.piles = serde_json::from_value(piles).unwrap();
        document
    };

    // Control one: the same entering Strength with no Sleight of Flesh
    // anywhere in the closure admits, and the row really is `unknown`.
    let mut quiet = malaise_root(&[json!({
        "kind": "TOADPOLE", "hp": 100, "max_hp": 100, "slot": 1, "uid": 1
    })]);
    quiet.monsters[0].insert("strength".to_owned(), json!(-3));
    let (catalog, state) = hydrate(&quiet);
    engine::admit(&quiet, &state, &catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .monsters[0]["power_attachments"],
        json!([["strength", "unknown", 0, -3, 0]]),
        "S4 materialized the unobserved applier",
    );

    // The same root with Sleight of Flesh reachable refuses, by name.
    let document = with_sleight(json!(-3));
    let (catalog, state) = hydrate(&document);
    assert!(
        engine::admit(&document, &state, &catalog)
            .unwrap_err()
            .contains(engine::MissingCapability::ArgumentShape(
                "Misery Strength clone unknown applier"
            )),
    );

    // A POSITIVE entering Strength refuses too: the row mirrors the live
    // scalar, so anything that drives it negative later makes the very same
    // copy Type-2 and Sleight-eligible, and the applier stays unknown through
    // the `ModifyAmount` (`0x3efbac` IL_013d is the fresh-attach path only).
    let positive = with_sleight(json!(4));
    let (catalog, state) = hydrate(&positive);
    assert!(
        engine::admit(&positive, &state, &catalog)
            .unwrap_err()
            .contains(engine::MissingCapability::ArgumentShape(
                "Misery Strength clone unknown applier"
            )),
    );

    // Control two: recorded provenance admits with Sleight of Flesh in hand.
    let mut recorded = with_sleight(json!(-3));
    recorded.monsters[0].insert(
        "power_attachments".to_owned(),
        json!([["strength", "player", 0, -3, 0]]),
    );
    let (catalog, state) = hydrate(&recorded);
    engine::admit(&recorded, &state, &catalog).unwrap();

    // Control three: no Misery in the closure, so no clone can carry it.
    let mut no_misery = with_sleight(json!(-3));
    let mut piles: Value = serde_json::to_value(&no_misery.piles).unwrap();
    piles["hand"] = json!([
        {"id": "MALAISE", "uid": 0, "upgrade": 0},
        {"id": "SLEIGHT_OF_FLESH", "uid": 2, "upgrade": 0},
    ]);
    no_misery.piles = serde_json::from_value(piles).unwrap();
    let (catalog, state) = hydrate(&no_misery);
    engine::admit(&no_misery, &state, &catalog).unwrap();
}

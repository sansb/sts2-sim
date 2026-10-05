//! #3649: Rupture registers a played card only on its owner's own side.
//!
//! `RupturePower::BeforeCardPlayed` RVA `0xa6e9c` (v0.111.0 `sts2.dll`,
//! SHA-256 `9cb4f1ad…fbf12b4`) returns at IL_0042 unless
//! `CombatState.CurrentSide == Owner.Side` (IL_002a-IL_0040), before
//! `playedCards.Add(card, 0)` at IL_0048-IL_005a. A card played on the enemy
//! side has no entry, so its own HP loss is an unregistered `cardSource` for
//! `<AfterDamageReceived>d__9` `0x342fec`, which leaves on the same side test
//! (IL_0046-IL_005c), and `<AfterCardPlayed>d__10` `0x342ec4` finds nothing
//! to release (`Remove` false at IL_005a, leave at IL_0061).
//!
//! The enemy-side play is the one #3622 found (`tests/inferno_side_gate.rs`):
//! a monster's hit arms Centennial Puzzle, whose Draw runs inside that
//! attack, and Hellraiser AutoPlays each drawn Strike. Here every Strike
//! carries Corrupted, whose OnPlay costs the owner 2 HP with the card as
//! `cardSource` (`Corrupted/<OnPlay>d__5::MoveNext` RVA `0x3881bc`
//! IL_001d-IL_0047). That loss reaches the play frame's Rupture accumulator
//! (`apply_enchantment_on_play`), which reads the registration bit and
//! nothing else.
//!
//! The fixture is the KD13JGCDPB3U Toadpoles entry with Rupture and
//! Hellraiser put in the opening hand, Centennial Puzzle owned and armed, the
//! Injury on top of the draw pile replaced by a Strike, and Corrupted on
//! every Strike. Ending turn one then runs, in order: Toadpole 0 buffs
//! Thorns, Toadpole 1 attacks, the Puzzle draws three Strikes on the enemy
//! side, and the turn-two hand draw plays a fourth Strike on the player side.
//!
//! The live engine logs the same two shapes (headless harness, build
//! v0.111.0 `41cef1ea`, seed `PROBE3649A`, Ironclad A0 against
//! `TOADPOLES_WEAK`, deck of Rupture, Hellraiser and twelve Corrupted
//! Strikes, Centennial Puzzle): each of the three Strikes drawn during the
//! Toadpole's attack logs its 9 damage and its own 2 HP loss and no
//! `PowerReceived`, and each Strike of the next hand draw is followed, after
//! its `CardPlayFinished` row, by `PowerReceived STRENGTH_POWER 1`.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, Subject},
    exact_solve_v1::ExactSolveActionV1,
    ids::PowerId,
    solo_v1::admit_exact_solve,
};

#[test]
fn a_corrupted_strike_played_on_the_enemy_side_banks_no_rupture_strength() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["hand"][0]["id"] = "RUPTURE".into();
    value["piles"]["hand"][3]["id"] = "HELLRAISER".into();
    value["piles"]["draw"][0]["id"] = "STRIKE_IRONCLAD".into();
    // Every Strike, so the physical siblings stay one payload class.
    for pile in ["hand", "draw"] {
        for card in value["piles"][pile].as_array_mut().unwrap() {
            if card["id"] == "STRIKE_IRONCLAD" {
                card["enchantment"] = serde_json::json!(["CORRUPTED", 0]);
            }
        }
    }
    value["player"]["relics_entering"]
        .as_array_mut()
        .unwrap()
        .push("RELIC.CENTENNIAL_PUZZLE".into());
    value["player"]["puzzle"] = true.into();
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();

    let line = r#"[{"kind":"play","uid":0},{"kind":"play","uid":3},{"kind":"end"}]"#;
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut events = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
    }

    // Each play, each owner HP loss and each Strength grant, in order.
    let trace: Vec<_> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::CardPlayed { .. }
                    | Event::PlayerDamaged { .. }
                    | Event::PowerChanged {
                        subject: Subject::Player,
                        power: PowerId::Strength,
                        ..
                    }
            )
        })
        .collect();
    assert!(
        matches!(
            trace.as_slice(),
            [
                // Enemy side: Toadpole 1's hit, then the three Puzzle-drawn
                // Strikes. Two enter Toadpole 0 (Thorns 2, taken first); all
                // three pay Corrupted's 2. No Strength for any of it.
                Event::PlayerDamaged { hp_lost: 8, .. },
                Event::CardPlayed { uid: 5, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::CardPlayed { uid: 6, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::CardPlayed { uid: 7, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                // Player side: the turn-two hand draw plays one more Strike
                // into Toadpole 0. Rupture answers the Thorns loss at once
                // (a null card source), and the registered card's Corrupted
                // loss is released at its AfterCardPlayed.
                Event::CardPlayed { uid: 12, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PowerChanged { amount: 1, .. },
                Event::PlayerDamaged { hp_lost: 2, .. },
                Event::PowerChanged { amount: 2, .. },
            ]
        ),
        "{trace:?}"
    );
    assert_eq!(state.powers.value(PowerId::Strength), 2);
    assert_eq!(state.hp, 42);
    assert!(state.player_side_active);
}

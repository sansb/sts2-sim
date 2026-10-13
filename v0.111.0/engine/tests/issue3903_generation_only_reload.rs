//! Every park reached from an admitted root reloads cold, when the root
//! only REACHES the Hellraiser chain through a generator (#3903).
//!
//! The roots here hold Game Piece or Iron Club (alone, together, and beside
//! Gremlin Horn or Centennial Puzzle), selecting Strikes, and two Power
//! Potions: no Hellraiser card and no Hellraiser power. They are built from a
//! synthetic save through the crate's public path, as the composition sweep
//! builds its roots, so the generator carries real provenance. A scripted
//! line uses a potion, takes Hellraiser from its offer, plays it and then
//! plays the fight; every choice it meets is branched over.
//!
//! PR #3882's second review found that an admission wall keyed on "the root
//! holds the chain" admitted such a root and refused the mid-fight state
//! once Hellraiser was live. Every ActionReplay receipt minted after that
//! named a predecessor admission no longer took, so the park applied warm
//! and failed to load cold (`ActionReplay predecessor is not admitted`).
//! This test fails on that defect: it reloads every park from its canonical
//! document alone, admits it, and requires the reloaded state to offer the
//! same answers and reach the same successors.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::{Catalog, GameBuild};
use sts_sim::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101;
use sts_sim::engine::{self, Action};
use sts_sim::entry::{self, EntryInput, EntryRequest, RootOutcome};
use sts_sim::hot::HotState;

const POWERS: [&str; 2] = ["HELLRAISER", "INFLAME"];

/// Gremlin Horn's own runtime refusals (`engine::hook_action`, #3387), which
/// are on `main` and involve neither relic: a line that meets one ends
/// there. Any other refusal fails the test.
const HORN_OWN: [&str; 6] = [
    "deferred hook action choice resolved without a prompt",
    "Gremlin Horn Draw choice outside a player action",
    "second deferred hook action",
    "choice beside a deferred hook action",
    "deferred hook action at combat end",
    "deferred hook action choice source moved",
];

/// The Horn's own refusal `error` names, if it is one.
fn horn_own(error: &impl std::fmt::Debug) -> Option<&'static str> {
    let text = format!("{error:?}");
    HORN_OWN.into_iter().find(|name| text.contains(name))
}

/// The synthetic save: the Ironclad starter kit, `relics` after Burning
/// Blood, the selecting Strikes and Powers the chain needs, and two Power
/// Potions. `generation` reseeds the combat card-generation stream.
fn save(relics: &[&str], generation: u64) -> String {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/entry_builder_v1.json")).unwrap();
    let mut save = fixture["cases"][0]["save"].clone();
    let mut deck = Vec::new();
    for id in ["STRIKE_IRONCLAD", "DEFEND_IRONCLAD"] {
        for _ in 0..4 {
            deck.push(json!({"floor_added_to_deck": 1, "id": format!("CARD.{id}")}));
        }
    }
    for id in [
        "INFLAME",
        "INFLAME",
        "INFLAME",
        "SEEKER_STRIKE",
        "SEEKER_STRIKE",
        "SCULPTING_STRIKE",
        "SCULPTING_STRIKE",
    ] {
        deck.push(json!({"floor_added_to_deck": 1, "id": format!("CARD.{id}")}));
    }
    let relic_rows: Vec<Value> = std::iter::once("BURNING_BLOOD")
        .chain(relics.iter().copied())
        .map(|relic| {
            let mut row = json!({"floor_added_to_deck": 1, "id": format!("RELIC.{relic}")});
            if relic == "IRON_CLUB" {
                row["props"] = json!({"ints": [{"name": "CardsPlayed", "value": 0}]});
            }
            row
        })
        .collect();
    let epochs: Vec<String> = UNLOCK_EPOCH_UNIVERSE_V1101
        .iter()
        .map(|epoch| format!("EPOCH.{epoch}"))
        .collect();
    save["rng"]["rngs"]["combat_card_generation"]["s0"] = json!(generation);
    let player = &mut save["players"][0];
    player["max_hp"] = json!(80);
    player["current_hp"] = json!(80);
    player["deck"] = Value::Array(deck);
    player["relics"] = Value::Array(relic_rows);
    player["potions"] = json!([
        {"id": "POTION.POWER_POTION", "slot_index": 0},
        {"id": "POTION.POWER_POTION", "slot_index": 1},
    ]);
    player["unlock_state"]["unlocked_epochs"] = json!(epochs);
    serde_json::to_string(&save).unwrap()
}

/// Reload a canonical document alone and admit it.
fn admitted(document: &CanonicalStateV2) -> Result<(HotState, Catalog), String> {
    document.validate_schema().map_err(|e| e.to_string())?;
    let catalog = HotBoundary::catalog_from_canonical(document).map_err(|e| e.to_string())?;
    let state = HotBoundary::from_canonical(document, &catalog).map_err(|e| e.to_string())?;
    engine::admit(document, &state, &catalog).map_err(|refusal| {
        refusal
            .missing()
            .map(|capability| capability.to_string())
            .collect::<Vec<_>>()
            .join(" ;; ")
    })?;
    Ok((state, catalog))
}

fn root(relics: &[&str], generation: u64) -> (HotState, Catalog) {
    let save = save(relics, generation);
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: None,
        node_type: None,
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let opening = match entry::build_root(&request, None) {
        RootOutcome::Opened(opening) => opening,
        RootOutcome::Refused(refusal) => panic!("{relics:?}: entry {refusal}"),
        RootOutcome::EntryOnly(_, refusal) => panic!("{relics:?}: opening {refusal}"),
    };
    admitted(&opening.document).unwrap_or_else(|error| panic!("{relics:?}: {error}"))
}

fn document(state: &HotState, catalog: &Catalog) -> CanonicalStateV2 {
    HotBoundary::try_to_canonical(state, catalog).unwrap()
}

fn digest(state: &HotState, catalog: &Catalog) -> String {
    document(state, catalog).differential_digest()
}

fn pile_ids(document: &CanonicalStateV2, pile: &str) -> Vec<(u64, String)> {
    document.piles[pile]
        .iter()
        .filter_map(|card| Some((card.uid?, card.id.clone())))
        .collect()
}

fn holds(document: &CanonicalStateV2, id: &str) -> bool {
    document
        .piles
        .values()
        .any(|pile| pile.iter().any(|card| card.id == id))
}

/// The callers of the Draw frames a parked document holds, bottom first.
fn draw_callers(document: &CanonicalStateV2) -> Vec<String> {
    document
        .continuations
        .iter()
        .filter_map(|frame| frame.fields.get("caller")?.as_str().map(str::to_string))
        .collect()
}

#[derive(Default)]
struct Seen {
    /// Parks by the caller of their innermost Draw frame.
    parks: BTreeMap<String, usize>,
    /// Lines ended by one of the Horn's own refusals, by name.
    horn_refusals: BTreeMap<&'static str, usize>,
    states: usize,
}

/// The cold-reload contract for one parked state; returns its successors.
fn reload_park(state: &HotState, catalog: &Catalog, at: &str, seen: &mut Seen) -> Vec<HotState> {
    let wire = document(state, catalog);
    let (loaded, loaded_catalog) = admitted(&wire)
        .unwrap_or_else(|error| panic!("{at}: a warm park does not load cold: {error}"));
    assert_eq!(
        document(&loaded, &loaded_catalog).canonical_json(),
        wire.canonical_json(),
        "{at}"
    );
    let caller = draw_callers(&wire)
        .last()
        .cloned()
        .unwrap_or_else(|| "none".to_string());
    *seen.parks.entry(caller).or_default() += 1;
    let answers = engine::legal_actions(state, catalog);
    assert!(!answers.is_empty(), "{at}: a park with no answer");
    assert_eq!(
        engine::legal_actions(&loaded, &loaded_catalog),
        answers,
        "{at}"
    );
    answers
        .iter()
        .filter_map(|answer| {
            let warm = engine::apply_action(state, catalog, answer);
            let cold = engine::apply_action(&loaded, &loaded_catalog, answer);
            let warm = match warm {
                Ok(next) => next.state,
                Err(error) => {
                    let name = horn_own(&error)
                        .unwrap_or_else(|| panic!("{at} {answer:?} warm: {error:?}"));
                    // The reloaded park refuses the same answer the same way.
                    assert_eq!(cold.as_ref().err().and_then(horn_own), Some(name), "{at}");
                    *seen.horn_refusals.entry(name).or_default() += 1;
                    return None;
                }
            };
            let cold = cold
                .unwrap_or_else(|error| panic!("{at} {answer:?} cold: {error:?}"))
                .state;
            assert_eq!(
                digest(&cold, &loaded_catalog),
                digest(&warm, catalog),
                "{at} {answer:?}"
            );
            Some(warm)
        })
        .collect()
}

/// The scripted action at a state with no choice pending: a potion, then a
/// Power, then the first other card, then the end of the turn.
fn scripted(state: &HotState, catalog: &Catalog) -> Option<Action> {
    let actions = engine::legal_actions(state, catalog);
    if let Some(potion) = actions
        .iter()
        .find(|action| matches!(action, Action::UsePotion { .. }))
    {
        return Some(*potion);
    }
    let hand = pile_ids(&document(state, catalog), "hand");
    let is_power = |uid: u32| {
        hand.iter()
            .any(|(held, id)| *held == u64::from(uid) && POWERS.contains(&id.as_str()))
    };
    actions
        .iter()
        .find(|action| matches!(action, Action::Play { uid, .. } if is_power(*uid)))
        .or_else(|| actions.first())
        .copied()
}

/// Play the scripted line from `state`, branching over every answer of
/// every choice until `budget` states have been visited.
fn play(state: HotState, catalog: &Catalog, at: String, budget: &mut usize, seen: &mut Seen) {
    let mut work = vec![(state, at, 0usize)];
    while let Some((mut state, at, mut steps)) = work.pop() {
        loop {
            seen.states += 1;
            if state.history.over || steps > 90 || *budget == 0 {
                break;
            }
            *budget -= 1;
            steps += 1;
            if state.pending.is_some() {
                let successors = reload_park(&state, catalog, &at, seen);
                // A potion's offer: keep to the Hellraiser pick when there
                // is one, so the line reaches the chain.
                let hellraiser = successors
                    .iter()
                    .position(|next| holds(&document(next, catalog), "HELLRAISER"));
                let before = holds(&document(&state, catalog), "HELLRAISER");
                match hellraiser {
                    Some(pick) if !before => state = successors[pick].clone(),
                    _ => {
                        let mut successors = successors.into_iter().enumerate();
                        let Some((_, first)) = successors.next() else {
                            break;
                        };
                        for (index, next) in successors {
                            work.push((next, format!("{at}/{steps}:{index}"), steps));
                        }
                        state = first;
                    }
                }
                continue;
            }
            let Some(action) = scripted(&state, catalog) else {
                panic!("{at}: stuck after {steps} steps");
            };
            state = match engine::apply_action(&state, catalog, &action) {
                Ok(next) => next.state,
                Err(error) => {
                    let name = horn_own(&error)
                        .unwrap_or_else(|| panic!("{at} step {steps} {action:?}: {error:?}"));
                    assert!(at.contains("GREMLIN_HORN"), "{at}: {name}");
                    *seen.horn_refusals.entry(name).or_default() += 1;
                    break;
                }
            };
        }
    }
}

/// A generation seed under which the first Power Potion offers Hellraiser.
fn hellraiser_seed() -> u64 {
    (1..200)
        .find(|&generation| {
            let (state, catalog) = root(&["GAME_PIECE"], generation);
            let Some(potion) = engine::legal_actions(&state, &catalog)
                .into_iter()
                .find(|action| matches!(action, Action::UsePotion { .. }))
            else {
                return false;
            };
            let offer = engine::apply_action(&state, &catalog, &potion)
                .unwrap()
                .state;
            engine::legal_actions(&offer, &catalog)
                .iter()
                .any(|answer| {
                    engine::apply_action(&offer, &catalog, answer)
                        .is_ok_and(|next| holds(&document(&next.state, &catalog), "HELLRAISER"))
                })
        })
        .expect("some generation seed offers Hellraiser")
}

#[test]
fn every_park_reached_from_a_generation_only_root_reloads_cold() {
    let generation = hellraiser_seed();
    let mut total = Seen::default();
    for relics in [
        &["GAME_PIECE"][..],
        &["IRON_CLUB"],
        &["GAME_PIECE", "IRON_CLUB"],
        &["IRON_CLUB", "GAME_PIECE"],
        &["GAME_PIECE", "GREMLIN_HORN"],
        &["IRON_CLUB", "GREMLIN_HORN"],
        &["GAME_PIECE", "IRON_CLUB", "GREMLIN_HORN"],
        &["IRON_CLUB", "CENTENNIAL_PUZZLE"],
    ] {
        let (state, catalog) = root(relics, generation);
        let wire = document(&state, &catalog);
        // The root reaches the chain and holds none of it.
        assert!(!holds(&wire, "HELLRAISER"), "{relics:?}");
        let mut seen = Seen::default();
        let mut budget = 2_500;
        play(
            state,
            &catalog,
            format!("{relics:?}"),
            &mut budget,
            &mut seen,
        );
        eprintln!(
            "{relics:?}: {} states, parks by innermost Draw {:?}, Horn's own refusals {:?}",
            seen.states, seen.parks, seen.horn_refusals
        );
        for (caller, count) in seen.parks {
            *total.parks.entry(caller).or_default() += count;
        }
        total.states += seen.states;
    }
    eprintln!("total: {} states, parks {:?}", total.states, total.parks);
    // Not vacuous: both relics' Draws parked, with Hellraiser generated.
    for caller in ["game_piece", "iron_club"] {
        assert!(
            total.parks.get(caller).copied().unwrap_or(0) > 0,
            "no {caller} park was reached: {:?}",
            total.parks
        );
    }
}

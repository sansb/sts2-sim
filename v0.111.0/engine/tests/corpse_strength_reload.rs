//! #3644 — a corpse that kept its Strength reloads as the corpse the engine
//! built.
//!
//! Both death cleanups reset a dying monster's attachment ledger and keep its
//! `StrengthPower` scalar, so in a fight that can reach `Misery` the engine
//! projects a dead monster as `strength: N` with no `power_attachments` row.
//! Hydration used to record an `unknown` row for it
//! (`engine::damage::materialize_entering_strength_provenance`), so the
//! reloaded state was not the projected one. Nothing reads a corpse's row, so
//! no play changed; what broke was the reload itself:
//!
//! * the document did not project back to itself, and
//! * a parked ActionReplay receipt whose predecessor held such a corpse
//!   refused on load, because `boundary::authenticate_action_replay` replays
//!   the root action from the *hydrated* predecessor and compares the whole
//!   document: "ActionReplay transcript does not reproduce the complete
//!   canonical state".
//!
//! The release census projected twelve such documents, in four fights. Two of
//! them are stored fixtures and are replayed here.

use std::path::Path;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action, SelectionRef};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hot::HotState;

const TOADPOLES: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

fn hydrate(document: &CanonicalStateV2) -> (Catalog, HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    (catalog, state)
}

fn play(state: &HotState, catalog: &Catalog, uid: u32, target: Option<u8>) -> HotState {
    engine::apply_action(
        state,
        catalog,
        &Action::Play {
            uid,
            target,
            selection: SelectionRef::new(None),
        },
    )
    .unwrap_or_else(|error| {
        panic!(
            "play {uid}: {error}; legal: {:?}",
            engine::legal_actions(state, catalog)
        )
    })
    .state
}

/// Two Toadpoles, the first one Strike away from death and holding Strength.
/// Misery in hand makes the ledger live; Prepared parks a selection.
fn root() -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(TOADPOLES).unwrap();
    document.monsters[0].insert("hp".to_owned(), json!(5));
    document.monsters[0].insert("strength".to_owned(), json!(3));
    let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
    piles["hand"] = json!([
        {"id": "STRIKE_IRONCLAD", "uid": 0, "upgrade": 0},
        {"id": "PREPARED", "uid": 1, "upgrade": 0},
        {"id": "MISERY", "uid": 2, "upgrade": 0},
        {"id": "DEFEND_IRONCLAD", "uid": 3, "upgrade": 0},
    ]);
    document.piles = serde_json::from_value(piles).unwrap();
    document
}

#[test]
fn a_corpse_that_kept_its_strength_reloads_as_the_engine_built_it() {
    let document = root();
    let (catalog, state) = hydrate(&document);
    engine::admit(&document, &state, &catalog).unwrap();
    // The living monster's entering Strength is recorded, as before.
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap()
            .monsters[0]["power_attachments"],
        json!([["strength", "unknown", 0, 3, 0]]),
    );

    let killed = play(&state, &catalog, 0, Some(0));
    let projected = HotBoundary::try_to_canonical(&killed, &catalog).unwrap();
    // The shape under test, read off the projection so the test cannot pass
    // by the monster surviving or the death wiping its Strength.
    assert!(projected.monsters[0]["hp"].as_i64().unwrap() <= 0);
    assert_eq!(projected.monsters[0]["strength"], json!(3));
    assert!(!projected.monsters[0].contains_key("power_attachments"));

    // Warm == cold: the hydrated roster is the one the engine built, and the
    // document projects back to itself.
    let (cold_catalog, cold) = hydrate(&projected);
    engine::admit(&projected, &cold, &cold_catalog).unwrap();
    assert_eq!(cold.monsters, killed.monsters);
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        projected,
    );

    // A selection parked beside that corpse carries a receipt whose
    // predecessor is `projected`. It loads, and replays to the same document.
    let parked = play(&killed, &catalog, 1, None);
    let parked_document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
    assert!(parked_document.player.contains_key("pending"));
    assert_eq!(
        parked_document.continuations[0].fields["predecessor"],
        serde_json::to_value(&projected).unwrap(),
        "the receipt's predecessor is the corpse document",
    );
    let (parked_catalog, reloaded) = hydrate(&parked_document);
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &parked_catalog).unwrap(),
        parked_document,
    );
}

/// Replay a stored fixture and reload the documents it projects.
///
/// `parked` are the action indices whose result is a parked receipt beside a
/// corpse holding Strength: the documents the census could not reload. Each
/// must load. Every other step that loads must project back to itself; a
/// step that still refuses for another named reason (#3644's other rows, for
/// example `fa8417ca31149b55` step 29, "Misery power acquisition order") is
/// outside this test.
fn parked_corpse_documents_reload(fixture: &str, parked: &[usize]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights")
        .join(fixture);
    let entry: CanonicalStateV2 =
        serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).unwrap()).unwrap();
    let line: Value =
        serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
    let (catalog, mut state) = hydrate(&entry);
    engine::admit(&entry, &state, &catalog).unwrap();

    let actions = line["actions"].as_array().unwrap();
    let digests = line["step_digests"].as_array().unwrap();
    let mut events = Vec::new();
    let mut reloaded_beside_a_corpse = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        let wire: ExactSolveActionV1 = serde_json::from_value(action.clone()).unwrap();
        let action = Action::try_from(wire).unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .unwrap_or_else(|error| panic!("{fixture} step {index}: {error}"));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(
            Value::String(document.differential_digest()),
            digests[index],
            "{fixture} step {index}",
        );
        let reload = HotBoundary::catalog_from_canonical(&document).and_then(|catalog| {
            HotBoundary::from_canonical(&document, &catalog).map(|state| (catalog, state))
        });
        let (reload_catalog, reloaded) = match reload {
            Ok(pair) => pair,
            Err(error) if !parked.contains(&index) => {
                assert!(
                    !error.to_string().contains("does not reproduce"),
                    "{fixture} step {index}: {error}",
                );
                continue;
            }
            Err(error) => panic!("{fixture} step {index}: {error}"),
        };
        assert_eq!(
            HotBoundary::try_to_canonical(&reloaded, &reload_catalog).unwrap(),
            document,
            "{fixture} step {index}",
        );
        let corpse_holds_strength = document.monsters.iter().any(|monster| {
            monster["hp"].as_i64().unwrap() <= 0
                && monster.contains_key("strength")
                && !monster.contains_key("power_attachments")
        });
        if corpse_holds_strength
            && document.player.contains_key("pending")
            && !document.continuations.is_empty()
        {
            reloaded_beside_a_corpse.push(index);
        }
    }
    // A regenerated fixture that no longer reaches the shape fails here
    // instead of passing vacuously.
    assert_eq!(reloaded_beside_a_corpse, parked, "{fixture}");
}

#[test]
fn the_fixtures_that_park_beside_a_strength_corpse_reload() {
    parked_corpse_documents_reload("f4051bdad0c87668", &[8]);
    parked_corpse_documents_reload("fa8417ca31149b55", &[21, 25]);
}

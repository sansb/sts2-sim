//! #3662 — a document holding Trash to Treasure in
//! `player.local_generated_power_order` reloads.
//!
//! The engine registers Trash to Treasure in the generated-card listener
//! order when the power is first applied, and the boundary projects that
//! order. The loader's allowed family named only Arsenal, Pillar of Creation
//! and Smokestack, so every document projected after the play refused on
//! load: "listener is outside this hook family or repeats". The release
//! census projected 436 such documents, in thirteen Defect fights. Four of
//! them are stored fixtures and are replayed here.

use std::path::Path;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hot::HotState;

fn hydrate(document: &CanonicalStateV2) -> (Catalog, HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    (catalog, state)
}

/// What one fixture's replay established, from action index `first` on.
#[derive(Debug, PartialEq, Eq)]
struct Reloads {
    /// Documents carrying Trash to Treasure in the order.
    documents: usize,
    /// Of those, the ones that loaded and projected back to themselves.
    reloaded: usize,
    /// Of those, the ones whose reloaded state reached the next stored step
    /// digest under the catalog rebuilt from the document.
    continued: usize,
    /// The distinct orders read, in first-seen order.
    orders: Vec<Value>,
}

/// Replay a stored fixture; from action index `first` on, every projected
/// document carries Trash to Treasure in the order (alone, or ahead of a
/// Smokestack acquired later in `f25de514675098c8`).
///
/// Each such document must load, project back to itself, and continue: the
/// next recorded action, applied to the reloaded state under the catalog
/// rebuilt from that document, reaches the stored step digest. A document
/// that refused for one of #3644's other named reasons (a parked receipt
/// whose predecessor is not admitted) would be outside this test; none of
/// these four fixtures holds one, and the counts returned are pinned by the
/// caller so the test cannot pass by skipping.
fn trash_to_treasure_documents_reload(fixture: &str, first: usize) -> Reloads {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights")
        .join(fixture);
    let entry: CanonicalStateV2 =
        serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).unwrap()).unwrap();
    let line: Value =
        serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
    let (catalog, mut state) = hydrate(&entry);
    engine::admit(&entry, &state, &catalog).unwrap();

    let actions: Vec<Action> = line["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            let wire: ExactSolveActionV1 = serde_json::from_value(action.clone()).unwrap();
            Action::try_from(wire).unwrap()
        })
        .collect();
    let digests = line["step_digests"].as_array().unwrap();
    let mut events = Vec::new();
    let mut seen = Reloads {
        documents: 0,
        reloaded: 0,
        continued: 0,
        orders: Vec::new(),
    };
    for (index, action) in actions.iter().enumerate() {
        events.clear();
        state = engine::apply_action_into(&state, &catalog, action, &mut events)
            .unwrap_or_else(|error| panic!("{fixture} step {index}: {error}"));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(
            Value::String(document.differential_digest()),
            digests[index],
            "{fixture} step {index}",
        );
        let order = document.player.get("local_generated_power_order");
        if index < first {
            assert_eq!(order, None, "{fixture} step {index}");
            continue;
        }
        // A regenerated fixture that no longer reaches the shape fails here
        // instead of passing vacuously.
        let order = order.unwrap_or_else(|| panic!("{fixture} step {index}: no order"));
        assert!(
            order
                .as_array()
                .unwrap()
                .contains(&json!(["trash_to_treasure"])),
            "{fixture} step {index}: {order}",
        );
        if !seen.orders.contains(order) {
            seen.orders.push(order.clone());
        }
        seen.documents += 1;
        let reload = HotBoundary::catalog_from_canonical(&document).and_then(|catalog| {
            HotBoundary::from_canonical(&document, &catalog).map(|state| (catalog, state))
        });
        let (reload_catalog, reloaded) = match reload {
            Ok(pair) => pair,
            Err(error) => {
                let text = error.to_string();
                assert!(
                    !text.contains("local_generated_power_order")
                        && text.contains("ActionReplay predecessor is not admitted"),
                    "{fixture} step {index}: {text}",
                );
                continue;
            }
        };
        assert_eq!(
            HotBoundary::try_to_canonical(&reloaded, &reload_catalog).unwrap(),
            document,
            "{fixture} step {index}",
        );
        seen.reloaded += 1;
        let Some(next) = actions.get(index + 1) else {
            continue;
        };
        if engine::admit(&document, &reloaded, &reload_catalog).is_err() {
            continue;
        }
        events.clear();
        let continued = engine::apply_action_into(&reloaded, &reload_catalog, next, &mut events)
            .unwrap_or_else(|error| panic!("{fixture} step {index} continued: {error}"));
        let continued = HotBoundary::try_to_canonical(&continued, &reload_catalog).unwrap();
        assert_eq!(
            Value::String(continued.differential_digest()),
            digests[index + 1],
            "{fixture} step {index}: the reloaded state continues to another digest",
        );
        seen.continued += 1;
    }
    seen
}

#[test]
fn the_fixtures_that_hold_trash_to_treasure_in_the_order_reload() {
    let alone = json!([["trash_to_treasure"]]);
    let ahead_of_smokestack = json!([["trash_to_treasure"], ["smokestack"]]);
    // (fixture, first action index, documents, orders). Every document
    // reloads, and every one but the last continues to the next digest.
    for (fixture, first, documents, orders) in [
        (
            "f25de514675098c8",
            3,
            19,
            vec![alone.clone(), ahead_of_smokestack.clone()],
        ),
        (
            "f41a3d66c3383f12",
            26,
            22,
            vec![alone.clone(), ahead_of_smokestack.clone()],
        ),
        ("fa1ff66e17a54ebd", 27, 30, vec![alone.clone()]),
        ("fd41bbfb8f5a0529", 2, 24, vec![alone.clone()]),
    ] {
        assert_eq!(
            trash_to_treasure_documents_reload(fixture, first),
            Reloads {
                documents,
                reloaded: documents,
                continued: documents - 1,
                orders,
            },
            "{fixture}",
        );
    }
}

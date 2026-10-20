//! The action-replay receipt keeps its predecessor hot (#3499 option 4) and
//! renders it only at the boundary. These witnesses pin that the rendered
//! receipt is byte-identical to the one the eager mint wrote, that it does
//! not drift while the receipt persists, and that every receipt-carrying
//! document loads back to a state that renders the same document.

use std::path::Path;

use serde_json::Value;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine;
use sts_sim::hidden::{self, PolicyRng};
use sts_sim::hot::HotState;

fn rendered_predecessor(document: &CanonicalStateV2) -> Option<&Value> {
    document
        .continuations
        .iter()
        .find(|frame| frame.frame_type == "ActionReplayFrame")?
        .fields
        .get("predecessor")
}

fn eager_predecessor(state: &HotState, catalog: &Catalog) -> Value {
    // What `install_parked_action_replay_root` stored before option 4: the
    // predecessor's canonical JSON, read back as the document embeds it.
    let document = HotBoundary::try_to_canonical(state, catalog).unwrap();
    serde_json::from_str(&document.canonical_json()).unwrap()
}

/// Seeded walks over every eval fixture. At each step that mints a receipt,
/// the receipt rendered from the hot predecessor equals the eager projection
/// of the state the action was taken from; at every later step that keeps it,
/// the rendering is unchanged; and the document loads and re-renders exactly.
#[test]
fn the_lazily_rendered_receipt_is_the_eager_one() {
    let fights = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights");
    let mut ids: Vec<String> = std::fs::read_dir(&fights)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    ids.sort();
    let (mut mints, mut kept, mut loads, mut fixtures) = (0usize, 0, 0, 0);
    let (mut unrenderable, mut unadmitted) = (0usize, 0usize);
    let mut legal = engine::LegalActionBuffer::new();
    let mut events = Vec::new();
    for id in &ids {
        let Ok(bytes) = std::fs::read(fights.join(id).join("entry.canonical.json")) else {
            continue;
        };
        let document: CanonicalStateV2 = serde_json::from_slice(&bytes).unwrap();
        let Ok(catalog) = HotBoundary::catalog_from_canonical(&document) else {
            continue;
        };
        let Ok(root) = HotBoundary::from_canonical(&document, &catalog) else {
            continue;
        };
        if engine::admit(&document, &root, &catalog).is_err() {
            continue;
        }
        fixtures += 1;
        for walk in 0..3u64 {
            let mut rng = PolicyRng(walk ^ u64::from_str_radix(&id[1..], 16).unwrap());
            let mut state = root.clone();
            let mut receipt: Option<Value> = None;
            // Eager projections of every state a receipt could have been
            // minted from since the last rendering: a receipt state whose
            // outer document refuses (a lineage the boundary cannot write,
            // unrelated to the receipt) is stepped over, not lost.
            let mut sources: Vec<Value> = Vec::new();
            for _ in 0..60 {
                if state.history.over {
                    break;
                }
                let actions = engine::legal_actions_into(&state, &catalog, &mut legal);
                if actions.is_empty() {
                    break;
                }
                let action = actions[rng.below(actions.len())];
                let Ok(next) = engine::apply_action_into(&state, &catalog, &action, &mut events)
                else {
                    break;
                };
                if !hidden::has_replay_receipt(&next) {
                    receipt = None;
                    sources.clear();
                    state = next;
                    continue;
                }
                if !hidden::has_replay_receipt(&state) || sources.is_empty() || receipt.is_some() {
                    // Any step under a receipt may pop it and mint anew.
                    sources.push(eager_predecessor(&state, &catalog));
                }
                let rendered = match HotBoundary::try_to_canonical(&next, &catalog) {
                    Ok(rendered) => rendered,
                    Err(refusal) => {
                        let text = format!("{refusal:?}");
                        assert!(
                            !text.contains("ActionReplay") && !text.contains("predecessor"),
                            "{id}: the receipt itself refused to render: {text}"
                        );
                        unrenderable += 1;
                        state = next;
                        continue;
                    }
                };
                let lazy = rendered_predecessor(&rendered)
                    .unwrap_or_else(|| panic!("{id}: no rendered receipt"))
                    .clone();
                if receipt.as_ref() == Some(&lazy) {
                    kept += 1;
                } else {
                    // A new receipt: minted from a state this walk stood on
                    // under the eager projection, never from anything else.
                    assert!(
                        sources.contains(&lazy),
                        "{id}: the rendered receipt differs from every eager mint"
                    );
                    mints += 1;
                }
                sources.clear();
                let reloaded_catalog = HotBoundary::catalog_from_canonical(&rendered).unwrap();
                let reloaded = match HotBoundary::from_canonical(&rendered, &reloaded_catalog) {
                    Ok(reloaded) => reloaded,
                    Err(refusal) => {
                        // A mid-turn predecessor the entry admission does not
                        // accept refuses to load on main too (#3644); any
                        // other reload refusal is this change's.
                        let text = format!("{refusal:?}");
                        assert!(
                            text.contains("ActionReplay predecessor is not admitted"),
                            "{id}: receipt reloads: {text}"
                        );
                        unadmitted += 1;
                        receipt = Some(lazy);
                        state = next;
                        continue;
                    }
                };
                assert_eq!(
                    HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog)
                        .unwrap()
                        .canonical_json(),
                    rendered.canonical_json(),
                    "{id}: a loaded receipt re-renders differently"
                );
                loads += 1;
                receipt = Some(lazy);
                state = next;
            }
        }
    }
    eprintln!(
        "lazy receipt witness: {fixtures} fixtures, {mints} mints, \
         {kept} steps keeping a receipt, {loads} reloads, \
         {unrenderable} receipt states whose outer document refuses, \
         {unadmitted} whose predecessor is not admitted on reload"
    );
    assert!(mints > 1_000, "only {mints} receipts minted");
    assert!(kept >= 5, "only {kept} kept-receipt steps");
    assert!(loads > 1_000, "only {loads} receipt documents reloaded");
}

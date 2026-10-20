//! Witnesses for the information-set search (#3436).

use std::path::Path;

use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine;
use sts_sim::hidden::{self, DrawKnowledge, PolicyRng};
use sts_sim::hot::{HotState, PileId};
use sts_sim::ismcts::{self, Information, IsmctsParams, IsmctsResult};

fn fixture(id: &str) -> (HotState, Catalog) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights")
        .join(id)
        .join("entry.canonical.json");
    let document: CanonicalStateV2 = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let root = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &root, &catalog).unwrap();
    (root, catalog)
}

fn digest(result: &IsmctsResult) -> Vec<(String, u64, String)> {
    result
        .root
        .iter()
        .map(|stat| {
            (
                format!("{:?}", stat.action),
                stat.visits,
                format!("{:.12}", stat.mean),
            )
        })
        .collect()
}

/// The hidden search is a function of the observation alone: replacing the
/// true draw order and every stream's words cannot change a single statistic,
/// while the clairvoyant search over the same states does see them.
///
/// Seapunk is a short legacy-route fight. The two action-replay fights park
/// selections inside the tree, whose receipts embed the predecessor's whole
/// state — draw order and words included — so they also witness that a
/// receipt-carrying information set is keyed without its hidden parts.
#[test]
fn the_hidden_search_cannot_see_the_true_draw_order_or_rng() {
    for (id, iterations) in [
        ("f156e43104f3752c", 3_000),
        ("f232d07ae4f96a5b", 1_500),
        ("f960d49055bd158f", 1_500),
    ] {
        let (root, catalog) = fixture(id);
        let other = hidden::sample_world(&root, &DrawKnowledge::none(), &mut PolicyRng(99));
        assert_ne!(other, root, "{id}: the alternative true world differs");
        let run = |state: &HotState, information| {
            ismcts::search(
                state,
                &DrawKnowledge::none(),
                &catalog,
                &IsmctsParams::new(information, iterations, 5),
            )
            .unwrap()
        };
        let a = run(&root, Information::Hidden);
        let b = run(&other, Information::Hidden);
        assert!(a.iterations == iterations && !a.root.is_empty());
        assert_eq!(
            digest(&a),
            digest(&b),
            "{id}: the hidden search saw the true world"
        );
        let c = run(&root, Information::Clairvoyant);
        let d = run(&other, Information::Clairvoyant);
        assert_ne!(
            digest(&c),
            digest(&d),
            "{id}: control: the clairvoyant search reads the true world"
        );
        eprintln!(
            "{id}: {} nodes, {} keyed by sanitized receipt, refusals {:?}",
            a.nodes, a.receipt_nodes, a.refusals
        );
        if id != "f156e43104f3752c" {
            assert!(
                a.receipt_nodes > 0,
                "{id}: no receipt-carrying node reached"
            );
        }
    }
}

/// Every position the knowledge tracker claims is known must hold in every
/// world of the information set. Over the eval fixtures that hold a
/// top-placing card, each step that leaves knowledge is re-applied from a
/// counterfactual world (unknown draw order reshuffled, every stream's words
/// replaced). Where the counterfactual step leaves the player the same
/// observation (draw positions aside), the claimed cards must sit at the same
/// places. A claim that
/// depended on hidden RNG — a random insert taken for a deterministic one —
/// fails here.
#[test]
fn claimed_draw_positions_hold_in_counterfactual_worlds() {
    const TOP_PLACERS: [&str; 6] = [
        "\"HEADBUTT\"",
        "\"SHINING_STRIKE\"",
        "\"THINKING_AHEAD\"",
        "\"GLIMMER\"",
        "\"COSMIC_INDIFFERENCE\"",
        "\"PHOTON_CUT\"",
    ];
    let fights = Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights");
    let mut ids: Vec<String> = std::fs::read_dir(&fights)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|id| {
            std::fs::read_to_string(fights.join(id).join("entry.canonical.json"))
                .is_ok_and(|text| TOP_PLACERS.iter().any(|card| text.contains(card)))
        })
        .collect();
    ids.sort();
    let (mut claims, mut checked, mut diverged, mut refusals) = (0usize, 0usize, 0usize, 0usize);
    let mut legal = engine::LegalActionBuffer::new();
    let mut events = Vec::new();
    for id in &ids {
        let path = fights.join(id).join("entry.canonical.json");
        let document: CanonicalStateV2 =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let Ok(catalog) = HotBoundary::catalog_from_canonical(&document) else {
            continue;
        };
        let Ok(root) = HotBoundary::from_canonical(&document, &catalog) else {
            continue;
        };
        if engine::admit(&document, &root, &catalog).is_err() {
            continue;
        }
        for salt in 0..4u64 {
            let mut rng = PolicyRng(salt ^ u64::from_str_radix(&id[1..], 16).unwrap());
            let mut state = root.clone();
            let mut know = DrawKnowledge::none();
            for _ in 0..80 {
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
                let next_know = match hidden::observe_step(&catalog, &state, &know, &next, &events)
                {
                    Ok(next_know) => next_know,
                    Err(_) => {
                        refusals += 1;
                        break;
                    }
                };
                if !next_know.is_empty() {
                    claims += 1;
                    for world in 0..3u64 {
                        let alternative =
                            hidden::sample_world(&state, &know, &mut PolicyRng(world + 1));
                        let mut alt_events = Vec::new();
                        let Ok(alt_next) = engine::apply_action_into(
                            &alternative,
                            &catalog,
                            &action,
                            &mut alt_events,
                        ) else {
                            continue;
                        };
                        let Ok(alt_know) = hidden::observe_step(
                            &catalog,
                            &alternative,
                            &know,
                            &alt_next,
                            &alt_events,
                        ) else {
                            continue;
                        };
                        // Only a world the player cannot tell apart after the
                        // step must agree: a random target that kills a
                        // different monster is a different observation.
                        if hidden::info_key(&alt_next, &DrawKnowledge::none())
                            != hidden::info_key(&next, &DrawKnowledge::none())
                        {
                            diverged += 1;
                            continue;
                        }
                        checked += 1;
                        assert!(
                            hidden::check_knowledge(&alt_next, &next_know).is_ok(),
                            "{id}: a claimed position moved in a counterfactual world: \
                             {next_know:?} vs {alt_know:?}"
                        );
                    }
                }
                state = next;
                know = next_know;
            }
        }
    }
    eprintln!(
        "knowledge witness: {} fixtures, {claims} steps with knowledge, \
         {checked} counterfactual checks, {diverged} observably different, \
         {refusals} refusals",
        ids.len()
    );
    assert!(checked > 20, "the witness checked only {checked} claims");
}

/// Root parallelism (#3436): several independent trees, their root statistics
/// summed. Every iteration is accounted for, the merge is deterministic for a
/// fixed thread count, and each tree is itself hidden, so the parallel search
/// is as blind to the true world as one tree.
#[test]
fn root_parallel_trees_sum_and_stay_blind() {
    let (root, catalog) = fixture("f156e43104f3752c");
    let other = hidden::sample_world(&root, &DrawKnowledge::none(), &mut PolicyRng(99));
    let mut params = IsmctsParams::new(Information::Hidden, 2_001, 5);
    params.threads = 4;
    let a = ismcts::search(&root, &DrawKnowledge::none(), &catalog, &params).unwrap();
    assert_eq!(a.iterations, 2_001);
    assert_eq!(a.root.iter().map(|stat| stat.visits).sum::<u64>(), 2_001);
    let again = ismcts::search(&root, &DrawKnowledge::none(), &catalog, &params).unwrap();
    assert_eq!(
        digest(&a),
        digest(&again),
        "deterministic for a fixed thread count"
    );
    let b = ismcts::search(&other, &DrawKnowledge::none(), &catalog, &params).unwrap();
    assert_eq!(
        digest(&a),
        digest(&b),
        "the parallel search saw the true world"
    );
}

/// The fast identity of a receipt-carrying state (#3436: the rest of the
/// state's representative plus the receipt's action and answers, paired with
/// the search node its predecessor was) must partition exactly as the slow
/// sanitized-document key does. Several worlds of one root's information set
/// walk the same action line together; wherever they all carry a liftable
/// receipt — so share an origin by construction — any two must agree on the
/// fast parts exactly when they agree on the slow key.
#[test]
fn fast_receipt_identity_partitions_as_the_document_key_does() {
    let mut checked = 0usize;
    let (mut receipts, mut lifted, mut stopped) = (0usize, 0usize, 0usize);
    for id in ["f232d07ae4f96a5b", "f960d49055bd158f", "fdeafdfe845c3ff1"] {
        let (root, catalog) = fixture(id);
        let mut legal = engine::LegalActionBuffer::new();
        let mut events = Vec::new();
        for walk in 0..6u64 {
            // Worlds that differ only below the half of the draw pile the
            // first turns do not reach: the same observations along the
            // walk, different hidden order inside every receipt.
            let mut worlds: Vec<HotState> = (0..4u64)
                .map(|w| {
                    let mut world = root.clone();
                    let draw = world.piles.get_mut(PileId::Draw).make_mut();
                    let half = draw.len() / 2;
                    let mut shuffle = PolicyRng(w * 31 + walk);
                    for i in (half + 1..draw.len()).rev() {
                        let j = half + shuffle.below(i - half + 1);
                        draw.swap(i, j);
                    }
                    world
                })
                .collect();
            let mut rng = PolicyRng(walk ^ 0xC0FFEE);
            for _ in 0..40 {
                if worlds[0].history.over {
                    break;
                }
                let actions = engine::legal_actions_into(&worlds[0], &catalog, &mut legal).to_vec();
                if actions.is_empty() {
                    break;
                }
                let action = actions[rng.below(actions.len())];
                let stepped: Vec<_> = worlds
                    .iter()
                    .map(|w| engine::apply_action_into(w, &catalog, &action, &mut events))
                    .collect();
                if stepped.iter().any(Result::is_err) {
                    stopped += 1;
                    break;
                }
                worlds = stepped.into_iter().map(Result::unwrap).collect();
                let know = DrawKnowledge::none();
                if hidden::has_replay_receipt(&worlds[0]) {
                    receipts += 1;
                    if hidden::sole_receipt_parts(&worlds[0], &know).is_some() {
                        lifted += 1;
                    }
                }
                let parts: Vec<_> = worlds
                    .iter()
                    .map(|w| hidden::sole_receipt_parts(w, &know))
                    .collect();
                if parts.iter().all(Option::is_some) {
                    let slow: Vec<_> = worlds
                        .iter()
                        .map(|w| hidden::receipt_aware_key(w, &know, &catalog).unwrap())
                        .collect();
                    for a in 0..worlds.len() {
                        for b in a + 1..worlds.len() {
                            checked += 1;
                            assert_eq!(
                                parts[a] == parts[b],
                                slow[a] == slow[b],
                                "{id}: the fast and slow receipt identities partition differently"
                            );
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "fast receipt identity: {checked} pairs checked; {receipts} receipt steps, \
         {lifted} lifted, {stopped} walks stopped by a divergent world"
    );
    assert!(checked > 50, "only {checked} receipt pairs reached");
    let _ = (receipts, lifted, stopped);
}

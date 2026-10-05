//! #3660 — a receipt minted under the session's catalog reloads when the
//! predecessor's own catalog has lost `requires_action_replay`.
//!
//! A session builds one catalog, from the fight's entry document, and keeps
//! it. A parked receipt stores the pre-action document, and the loader
//! replays the root action under a catalog rebuilt from that predecessor.
//! The rebuilt closure is narrower (cards have left the live piles), so a
//! derived bit can be true in the session and false in the rebuild.
//!
//! `requires_action_replay` is the bit that decides how a lone turn-start
//! hand choice (Tools of the Trade) parks: rooted with it, rootless without
//! it. Thirteen census documents, in four fights, carried a receipt the
//! replay did not mint, and refused: "ActionReplay transcript does not
//! reproduce the complete canonical state". None of the four is a stored
//! fixture, so the shape is built here from a public root.

use std::path::Path;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action, SelectionAnswer, SelectionRef};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hot::HotState;

const TOADPOLES: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

fn hydrate(document: &CanonicalStateV2) -> (Catalog, HotState) {
    let catalog = HotBoundary::catalog_from_canonical(document).unwrap();
    let state = HotBoundary::from_canonical(document, &catalog).unwrap();
    engine::admit(document, &state, &catalog).unwrap();
    (catalog, state)
}

fn apply(state: &HotState, catalog: &Catalog, action: &Action) -> HotState {
    engine::apply_action(state, catalog, action)
        .unwrap_or_else(|error| {
            panic!(
                "{action:?}: {error}; legal: {:?}",
                engine::legal_actions(state, catalog)
            )
        })
        .state
}

fn play(uid: u32, target: Option<u8>) -> Action {
    Action::Play {
        uid,
        target,
        selection: SelectionRef::new(None),
    }
}

fn project(state: &HotState, catalog: &Catalog) -> CanonicalStateV2 {
    HotBoundary::try_to_canonical(state, catalog).unwrap()
}

/// Tools of the Trade, a Pommel Strike and a Glam Hologram. The Hologram
/// selects and its enchantment replays it, which makes the entry closure
/// replay-capable; it exhausts itself, and the closure of what is left is
/// not. This is the census shape (`f6e36db39b363b7e`: an enchanted Hologram
/// and Tools of the Trade were the identities only the session held).
fn root() -> CanonicalStateV2 {
    let mut document: CanonicalStateV2 = serde_json::from_str(TOADPOLES).unwrap();
    for monster in &mut document.monsters {
        monster.insert("hp".to_owned(), json!(60));
        monster.insert("max_hp".to_owned(), json!(60));
    }
    let mut piles: Value = serde_json::to_value(&document.piles).unwrap();
    piles["hand"] = json!([
        {"id": "TOOLS_OF_THE_TRADE", "uid": 0, "upgrade": 0},
        {"id": "POMMEL_STRIKE", "uid": 1, "upgrade": 0},
        {"id": "HOLOGRAM", "uid": 2, "upgrade": 0, "enchantment": ["GLAM", 1]},
    ]);
    piles["draw"] = json!(
        (3..12)
            .map(|uid| json!({"id": "DEFEND_IRONCLAD", "uid": uid, "upgrade": 0}))
            .collect::<Vec<_>>()
    );
    piles["discard"] = json!([]);
    piles.as_object_mut().unwrap().remove("exhaust");
    document.piles = serde_json::from_value(piles).unwrap();
    document
        .player
        .insert("next_card_uid".to_owned(), json!(12));
    document
}

/// The session's side of the shape: its catalog, the state before the
/// EndTurn, that state's document (the receipt's predecessor), and the rooted
/// park the EndTurn reaches.
struct Scenario {
    catalog: Catalog,
    predecessor: CanonicalStateV2,
    parked: HotState,
    parked_document: CanonicalStateV2,
}

fn scenario() -> Scenario {
    let entry = root();
    let (catalog, state) = hydrate(&entry);
    // Tools of the Trade becomes a power, Pommel Strike goes to Discard, and
    // Hologram takes it back and exhausts itself.
    let mut warm = apply(&state, &catalog, &play(0, None));
    warm = apply(&warm, &catalog, &play(1, Some(0)));
    warm = apply(&warm, &catalog, &play(2, None));
    while warm.pending.is_some() {
        let answer = engine::legal_actions(&warm, &catalog)[0];
        warm = apply(&warm, &catalog, &answer);
    }
    let predecessor = project(&warm, &catalog);
    assert!(predecessor.continuations.is_empty());
    assert_eq!(predecessor.player["tools_of_the_trade"], json!(1));
    assert!(
        serde_json::to_value(&predecessor.piles).unwrap()["exhaust"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["id"] == "HOLOGRAM"),
        "the selector has left the live piles",
    );
    // The session parks the next turn's Tools of the Trade choice under a
    // receipt whose predecessor is that document.
    let parked = apply(&warm, &catalog, &Action::EndTurn);
    let parked_document = project(&parked, &catalog);
    assert!(parked_document.player.contains_key("pending"));
    assert_eq!(parked_document.continuations.len(), 1, "the receipt");
    assert_eq!(
        parked_document.continuations[0].fields["predecessor"],
        serde_json::to_value(&predecessor).unwrap(),
    );
    Scenario {
        catalog,
        predecessor,
        parked,
        parked_document,
    }
}

fn load(document: &CanonicalStateV2) -> Result<(Catalog, HotState), String> {
    HotBoundary::catalog_from_canonical(document)
        .and_then(|catalog| {
            HotBoundary::from_canonical(document, &catalog).map(|state| (catalog, state))
        })
        .map_err(|error| error.to_string())
}

/// A document with its continuation store removed: the game state alone.
fn game_state(document: &CanonicalStateV2) -> CanonicalStateV2 {
    let mut stripped = document.clone();
    stripped.continuations.clear();
    stripped
}

const MISMATCH: &str = "ActionReplay transcript does not reproduce the complete canonical state";

#[test]
fn a_turn_start_choice_receipt_reloads_after_the_closure_lost_its_selector() {
    let Scenario {
        catalog,
        parked,
        parked_document,
        ..
    } = scenario();

    // The receipt loads, is admitted, and projects back to itself.
    let (reload_catalog, reloaded) = hydrate(&parked_document);
    assert_eq!(project(&reloaded, &reload_catalog), parked_document);

    // And it continues as the session does: every answer, then the turn's
    // plays and the next turn's park, reach the session's own documents.
    let mut warm = parked;
    let mut cold = reloaded;
    let mut steps = 0;
    for _ in 0..3 {
        while warm.pending.is_some() {
            let answer = engine::legal_actions(&warm, &catalog)[0];
            assert!(matches!(
                answer,
                Action::Select {
                    answer: SelectionAnswer::OptionIndex(_) | SelectionAnswer::CardUid(_)
                }
            ));
            warm = apply(&warm, &catalog, &answer);
            cold = apply(&cold, &reload_catalog, &answer);
            assert_eq!(project(&cold, &reload_catalog), project(&warm, &catalog));
            steps += 1;
        }
        let action = engine::legal_actions(&warm, &catalog)[0];
        warm = apply(&warm, &catalog, &action);
        cold = apply(&cold, &reload_catalog, &action);
        assert_eq!(project(&cold, &reload_catalog), project(&warm, &catalog));
        warm = apply(&warm, &catalog, &Action::EndTurn);
        cold = apply(&cold, &reload_catalog, &Action::EndTurn);
        let warm_document = project(&warm, &catalog);
        assert_eq!(project(&cold, &reload_catalog), warm_document);
        // Each later park is rooted too, and reloads.
        assert_eq!(warm_document.continuations.len(), 1);
        let (again_catalog, again) = hydrate(&warm_document);
        assert_eq!(project(&again, &again_catalog), warm_document);
        steps += 2;
    }
    assert!(steps >= 9);
}

/// The case the rooted replay deliberately accepts, pinned.
///
/// Take the predecessor as the entry of a fight of its own. That session
/// never had a selector, parks the EndTurn rootless, and never emits the
/// rooted document. The loader now accepts the rooted document all the same:
/// one (predecessor, action, answers) authenticates two documents. That is
/// sound only because the two are one game state under two continuation
/// stores, which the loader checks on every such load and this test checks
/// here, and because both continue to the same game states.
#[test]
fn the_rooted_and_the_rootless_park_of_one_transcript_are_one_game_state() {
    let Scenario {
        predecessor,
        parked_document: rooted_document,
        ..
    } = scenario();

    // The narrow session: rooted at the predecessor, rootless at the park.
    let (narrow_catalog, narrow) = hydrate(&predecessor);
    let mut rootless = apply(&narrow, &narrow_catalog, &Action::EndTurn);
    let rootless_document = project(&rootless, &narrow_catalog);
    assert!(rootless_document.continuations.is_empty(), "no receipt");
    assert_eq!(rooted_document.continuations.len(), 1, "the receipt");

    // Everything but the continuation store is equal, the pending selection
    // included.
    assert_eq!(game_state(&rooted_document), game_state(&rootless_document));
    assert!(rooted_document.player.contains_key("pending"));

    // Both load. The rootless one is the narrow session's own document; the
    // rooted one is the deliberately accepted case.
    let (rootless_catalog, reloaded_rootless) = hydrate(&rootless_document);
    assert_eq!(
        project(&reloaded_rootless, &rootless_catalog),
        rootless_document
    );
    let (rooted_catalog, mut rooted) = hydrate(&rooted_document);
    assert_eq!(project(&rooted, &rooted_catalog), rooted_document);

    // And they continue to equal game states: the answer, a play and the
    // EndTurn, for three turns, each side under its own catalog.
    let mut steps = 0;
    for _ in 0..3 {
        while rootless.pending.is_some() {
            let answer = engine::legal_actions(&rootless, &narrow_catalog)[0];
            assert_eq!(engine::legal_actions(&rooted, &rooted_catalog)[0], answer);
            rootless = apply(&rootless, &narrow_catalog, &answer);
            rooted = apply(&rooted, &rooted_catalog, &answer);
            steps += 1;
        }
        // With the choice answered neither side holds a continuation, and
        // the documents are equal outright.
        assert_eq!(
            project(&rooted, &rooted_catalog),
            project(&rootless, &narrow_catalog)
        );
        let action = engine::legal_actions(&rootless, &narrow_catalog)[0];
        for end in [action, Action::EndTurn] {
            rootless = apply(&rootless, &narrow_catalog, &end);
            rooted = apply(&rooted, &rooted_catalog, &end);
            assert_eq!(
                game_state(&project(&rooted, &rooted_catalog)),
                game_state(&project(&rootless, &narrow_catalog)),
            );
            steps += 1;
        }
    }
    assert!(steps >= 9);
}

/// Forgeries aimed at the rooted replay. Each takes the rooted document,
/// which loads, changes one thing, and must refuse.
#[test]
fn a_forged_receipt_still_refuses_under_the_rooted_replay() {
    let Scenario {
        parked_document, ..
    } = scenario();
    load(&parked_document).unwrap();

    // A game field: neither transcript reproduces it.
    let mut forged = parked_document.clone();
    let block = forged
        .player
        .get("block")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    forged.player.insert("block".to_owned(), json!(block + 1));
    let error = load(&forged).unwrap_err();
    assert!(error.contains(MISMATCH), "{error}");

    // The receipt's answers: an answer the transcript never consumed. The
    // replay runs past this park, so no transcript reproduces the document.
    let mut forged = parked_document.clone();
    forged.continuations[0].fields.insert(
        "answers".to_owned(),
        json!([{"kind": "option_index", "value": 0}]),
    );
    let error = load(&forged).unwrap_err();
    assert!(
        error.contains("ActionReplay transcript does not end exactly parked"),
        "{error}"
    );

    // The receipt's action: a play of a card the predecessor holds, and a
    // potion it does not.
    for action in [
        json!({"kind": "play", "uid": 1, "target": 0, "selection_uid": null}),
        json!({"kind": "use_potion", "slot": 0, "target": null}),
    ] {
        let mut forged = parked_document.clone();
        forged.continuations[0]
            .fields
            .insert("action".to_owned(), action.clone());
        let error = load(&forged).unwrap_err();
        assert!(
            error.contains("ActionReplay root action is not exactly legal"),
            "{action}: {error}"
        );
    }

    // The receipt's predecessor: one game field changed inside it. Both
    // transcripts start from the forged predecessor and neither reaches the
    // document.
    let mut forged = parked_document.clone();
    let hp = forged.continuations[0].fields["predecessor"]["player"]["hp"]
        .as_i64()
        .unwrap();
    forged.continuations[0]
        .fields
        .get_mut("predecessor")
        .unwrap()["player"]["hp"] = json!(hp - 1);
    let error = load(&forged).unwrap_err();
    assert!(error.contains(MISMATCH), "{error}");
}

/// A document as an engine without `player.session_bookkeeping` wrote it:
/// the record removed, from the document and from a receipt's predecessor.
fn before_the_record(document: &CanonicalStateV2) -> CanonicalStateV2 {
    let mut stripped = document.clone();
    stripped.player.remove("session_bookkeeping");
    for frame in &mut stripped.continuations {
        if let Some(player) = frame
            .fields
            .get_mut("predecessor")
            .and_then(|predecessor| predecessor.get_mut("player"))
            .and_then(Value::as_object_mut)
        {
            player.remove("session_bookkeeping");
        }
    }
    stripped
}

/// One stored fixture replayed in a session, with every document it
/// projects.
struct Replayed {
    actions: Vec<Action>,
    digests: Vec<Value>,
    documents: Vec<CanonicalStateV2>,
}

fn replay_fixture(fixture: &str) -> Replayed {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../eval/fights")
        .join(fixture);
    let entry: CanonicalStateV2 =
        serde_json::from_slice(&std::fs::read(dir.join("entry.canonical.json")).unwrap()).unwrap();
    let line: Value =
        serde_json::from_slice(&std::fs::read(dir.join("human_line.json")).unwrap()).unwrap();
    let (catalog, mut state) = hydrate(&entry);
    let actions: Vec<Action> = line["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| {
            let wire: ExactSolveActionV1 = serde_json::from_value(action.clone()).unwrap();
            Action::try_from(wire).unwrap()
        })
        .collect();
    let digests = line["step_digests"].as_array().unwrap().clone();
    let mut documents = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        state = apply(&state, &catalog, action);
        let document = project(&state, &catalog);
        assert_eq!(
            Value::String(document.differential_digest()),
            digests[index],
            "{fixture} step {index}",
        );
        documents.push(document);
    }
    Replayed {
        actions,
        digests,
        documents,
    }
}

/// Reload `document` (the one projected at step `from`, or that document as
/// an older engine wrote it) and apply the rest of the recorded line to it,
/// under the catalog rebuilt from that one document. `Ok` is the number of
/// later steps that reached the session's document (all of them); `Err`
/// names the first step that did not.
///
/// With `ignoring_the_record` both sides are compared as
/// [`before_the_record`] leaves them, so a document that never carried the
/// record is not counted as different for that alone. Without it the
/// comparison is the stored step digest.
fn continue_from(
    replayed: &Replayed,
    from: usize,
    document: &CanonicalStateV2,
    ignoring_the_record: bool,
) -> Result<usize, String> {
    let (catalog, mut state) = load(document)?;
    for index in from + 1..replayed.actions.len() {
        state = engine::apply_action(&state, &catalog, &replayed.actions[index])
            .map_err(|error| format!("step {index}: {error}"))?
            .state;
        let projected = project(&state, &catalog);
        let same = if ignoring_the_record {
            before_the_record(&projected) == before_the_record(&replayed.documents[index])
        } else {
            Value::String(projected.differential_digest()) == replayed.digests[index]
        };
        if !same {
            return Err(format!("step {index}: another document"));
        }
    }
    Ok(replayed.actions.len() - from - 1)
}

/// #3660's second instance, and the wider symptom behind it.
///
/// Both fights open with the whole pool reachable, so the session counts
/// started plays for Normality, and both lose that closure mid-fight. Every
/// document the session projects records `normality_count`, so a catalog
/// rebuilt from any one of them still counts:
///
/// * the receipt parked on Scavenge (the two documents #3660 measured)
///   reloads;
/// * EVERY document of the line, parked or not, reloaded on its own,
///   continues through the rest of the recorded line to the stored digests.
///
/// Without the record (the same documents as an engine before it wrote
/// them) the parked receipt refuses, and an ordinary mid-fight document
/// loads and then continues to other digests: the count stops advancing.
#[test]
fn a_document_reloaded_at_any_step_keeps_the_sessions_normality_count() {
    for (fixture, parked_index, session_count) in
        [("fae227072bbaa5c9", 9, 4), ("f5dab8732a800775", 21, 3)]
    {
        let replayed = replay_fixture(fixture);
        let steps = replayed.actions.len();

        // The parked receipt #3660 measured.
        let parked = &replayed.documents[parked_index];
        assert!(!parked.continuations.is_empty(), "{fixture}");
        assert_eq!(
            parked.player["normality_card_plays_started_this_turn"],
            json!(session_count),
            "{fixture}",
        );
        let (catalog, state) = load(parked).unwrap_or_else(|error| panic!("{fixture}: {error}"));
        assert_eq!(&project(&state, &catalog), parked, "{fixture}");
        let error = load(&before_the_record(parked)).unwrap_err();
        assert!(error.contains(MISMATCH), "{fixture}: {error}");

        // Every document, reloaded on its own, continues to the stored
        // digests; and the ordinary ones diverge without the record.
        let mut diverged_without_the_record = Vec::new();
        for (index, document) in replayed.documents.iter().enumerate() {
            assert!(
                document.player["session_bookkeeping"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("normality_count")),
                "{fixture} step {index}",
            );
            let (catalog, state) =
                load(document).unwrap_or_else(|error| panic!("{fixture} step {index}: {error}"));
            assert_eq!(
                &project(&state, &catalog),
                document,
                "{fixture} step {index}"
            );
            assert_eq!(
                continue_from(&replayed, index, document, false),
                Ok(steps - index - 1),
                "{fixture} step {index}",
            );
            if document.continuations.is_empty()
                && continue_from(&replayed, index, &before_the_record(document), true).is_err()
            {
                diverged_without_the_record.push(index);
            }
        }
        // The wider symptom: ordinary documents that an engine without the
        // record reloads and then continues differently.
        assert!(
            !diverged_without_the_record.is_empty(),
            "{fixture}: no ordinary document diverges without the record",
        );
        eprintln!(
            "{fixture}: {steps} steps; ordinary documents diverging without the record: {diverged_without_the_record:?}"
        );
    }
}

/// The Misery ledger, the third bit, from a public root.
///
/// The session reaches Misery (it is in Hand), so it keeps the monsters'
/// attachment ledger. Burning Pact exhausts the Misery; the closure of what
/// is left does not reach it. Fight Me then raises a monster's Strength, which
/// the session restacks in the ledger. A document reloaded between the two
/// plays records `misery_ledger` and keeps the ledger as the session does;
/// without the record the reloaded state drops the row.
#[test]
fn a_document_reloaded_after_misery_left_the_closure_keeps_the_ledger() {
    let mut entry: CanonicalStateV2 = serde_json::from_str(TOADPOLES).unwrap();
    entry.monsters[0].insert("strength".to_owned(), json!(3));
    let mut piles: Value = serde_json::to_value(&entry.piles).unwrap();
    piles["hand"] = json!([
        {"id": "MISERY", "uid": 0, "upgrade": 0},
        {"id": "BURNING_PACT", "uid": 1, "upgrade": 0},
        {"id": "FIGHT_ME", "uid": 2, "upgrade": 0},
    ]);
    piles["draw"] = json!(
        (3..12)
            .map(|uid| json!({"id": "DEFEND_IRONCLAD", "uid": uid, "upgrade": 0}))
            .collect::<Vec<_>>()
    );
    piles["discard"] = json!([]);
    piles.as_object_mut().unwrap().remove("exhaust");
    entry.piles = serde_json::from_value(piles).unwrap();
    entry.player.insert("next_card_uid".to_owned(), json!(12));

    let (catalog, state) = hydrate(&entry);
    // Burning Pact names the card it exhausts in the play itself.
    let warm = apply(
        &state,
        &catalog,
        &Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::new(Some(0)),
        },
    );
    assert!(warm.pending.is_none());
    let between = project(&warm, &catalog);
    assert!(between.continuations.is_empty());
    assert!(
        serde_json::to_value(&between.piles).unwrap()["exhaust"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["id"] == "MISERY"),
        "Misery has left the live piles",
    );
    assert_eq!(
        between.player["session_bookkeeping"],
        json!(["misery_ledger"])
    );

    let session = project(&apply(&warm, &catalog, &play(2, Some(0))), &catalog);
    // Fight Me raised the Strength from 3 to 4, and the ledger row with it.
    assert_eq!(session.monsters[0]["strength"], json!(4));
    assert_eq!(
        session.monsters[0]["power_attachments"],
        json!([["strength", "unknown", 0, 4, 0]])
    );

    // With the record: the reloaded state plays Fight Me to the session's
    // document.
    let (cold_catalog, cold) = hydrate(&between);
    assert_eq!(project(&cold, &cold_catalog), between);
    assert_eq!(
        project(
            &apply(&cold, &cold_catalog, &play(2, Some(0))),
            &cold_catalog
        ),
        session,
    );

    // Without it: the same play, another document. The ledger is the whole
    // difference besides the record itself.
    let old = before_the_record(&between);
    let (old_catalog, old_state) = load(&old).unwrap();
    let mut diverged = project(
        &engine::apply_action(&old_state, &old_catalog, &play(2, Some(0)))
            .unwrap()
            .state,
        &old_catalog,
    );
    assert!(!diverged.player.contains_key("session_bookkeeping"));
    assert_ne!(
        diverged.monsters[0].get("power_attachments"),
        session.monsters[0].get("power_attachments"),
    );
    diverged.monsters[0].insert(
        "power_attachments".to_owned(),
        session.monsters[0]["power_attachments"].clone(),
    );
    assert_eq!(diverged, before_the_record(&session));
}

/// What one read of `Catalog::requires_action_replay` can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BitRead {
    /// Accepts or refuses a root, an action or a stack; writes nothing.
    Refusal,
    /// Chooses the replay-aware transaction or the ordinary one for the same
    /// action, and with it whether a park is rooted.
    Path,
    /// Chooses how a play is driven: a frozen-batch parent frame is pushed
    /// only with the bit. The one known difference in game state under it,
    /// a Rupture batch lost to the parent's cursor writes, is fixed (#3679):
    /// with the bit forced on, every census fight replays as it does
    /// without. That is a measurement over the capture corpus, not a proof,
    /// and the loader does not rely on this read being neutral.
    Driver,
    /// The loader's own test of the two catalogs.
    Loader,
}

/// Every production read of the bit, by file and enclosing function.
///
/// The rooted replay in `boundary::authenticate_action_replay` (#3660) may
/// only supply another continuation store for a game state the predecessor's
/// own catalog already authenticated, and it checks that on every load, so
/// it does not need any read below to be neutral. This table is the record
/// of what each read does, and the scan fails when a read is added, moved or
/// removed without being classified here.
const BIT_READS: &[(&str, &str, usize, BitRead)] = &[
    (
        "boundary.rs",
        "rooted_replay_is_eligible",
        2,
        BitRead::Loader,
    ),
    // Root admission. Three of these relax a refusal when the bit is set
    // (the direct-draw frozen-batch wall in `admit`, the Sly cardinality and
    // future-writer walls, the Distilled suspending-child wall), so admission
    // under the bit is not a subset of admission without it.
    ("engine/admission.rs", "admit", 5, BitRead::Refusal),
    (
        "engine/admission.rs",
        "unsupported_sly_frozen_batch_future_is_reachable",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_hand_selection_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_generated_offer_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_discard_retrieval_child_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_hand_exhaust_suffix_child_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_discard_hand_cap_retrieval_child_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "rooted_sly_hidden_daggers_child_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/admission.rs",
        "distilled_recursive_source_closure_is_exact",
        1,
        BitRead::Refusal,
    ),
    ("engine/mod.rs", "legal_actions_into", 2, BitRead::Refusal),
    ("engine/mod.rs", "apply_action_into", 1, BitRead::Path),
    (
        "engine/mod.rs",
        "apply_action_into_replay",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/mod.rs",
        "service_void_form_end_turn",
        1,
        BitRead::Path,
    ),
    (
        "engine/play.rs",
        "play_card_with_work_inner",
        1,
        BitRead::Driver,
    ),
    (
        "engine/play.rs",
        "persisted_card_play_stack_is_exact_inner",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/play.rs",
        "foregone_before_hand_draw_stack_is_exact",
        1,
        BitRead::Refusal,
    ),
    (
        "engine/play.rs",
        "replay_rooted_stack_is_exact",
        2,
        BitRead::Refusal,
    ),
    (
        "engine/play.rs",
        "restricted_frozen_auto_batch_stack_is_exact",
        1,
        BitRead::Refusal,
    ),
];

/// The production reads of `needle` in one source file, by enclosing
/// function. A `#[cfg(test)]` module, a comment line and the accessor's own
/// definition are not reads.
fn production_reads(source: &str, needle: &str) -> Vec<(String, usize)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut reads: Vec<(String, usize)> = Vec::new();
    let mut in_test_module = false;
    let mut function = String::new();
    for (index, line) in lines.iter().enumerate() {
        let opens_module = (line.starts_with("mod ")
            || line.starts_with("pub(crate) mod ")
            || line.starts_with("pub(super) mod "))
            && line.ends_with('{');
        if opens_module {
            in_test_module = index > 0 && lines[index - 1].starts_with("#[cfg(test)]");
        } else if line.starts_with('}') {
            in_test_module = false;
        }
        if in_test_module {
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = ["pub(crate) fn ", "pub(super) fn ", "pub fn ", "fn "]
            .iter()
            .find_map(|prefix| trimmed.strip_prefix(prefix))
        {
            function = rest
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap()
                .to_owned();
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let count = line.matches(needle).count();
        if count == 0 {
            continue;
        }
        match reads.iter_mut().find(|(name, _)| *name == function) {
            Some((_, total)) => *total += count,
            None => reads.push((function.clone(), count)),
        }
    }
    reads
}

#[test]
fn every_read_of_requires_action_replay_is_classified() {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    let mut found: Vec<(String, String, usize)> = Vec::new();
    for path in files {
        let name = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // The accessor and the builder live here; neither is a read.
        if name == "catalog.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        for (function, count) in production_reads(&source, ".requires_action_replay()") {
            found.push((name.clone(), function, count));
        }
    }
    let mut expected: Vec<(String, String, usize)> = BIT_READS
        .iter()
        .map(|(file, function, count, _)| ((*file).to_owned(), (*function).to_owned(), *count))
        .collect();
    expected.sort();
    found.sort();
    assert_eq!(
        found, expected,
        "a read of requires_action_replay was added, moved or removed: classify it in BIT_READS",
    );
    // The reads that are not a pure accept-or-refuse, named so that a new
    // one cannot join them unnoticed.
    let not_refusal: Vec<(&str, BitRead)> = BIT_READS
        .iter()
        .filter(|(_, _, _, class)| *class != BitRead::Refusal)
        .map(|(_, function, _, class)| (*function, *class))
        .collect();
    assert_eq!(
        not_refusal,
        [
            ("rooted_replay_is_eligible", BitRead::Loader),
            ("apply_action_into", BitRead::Path),
            ("service_void_form_end_turn", BitRead::Path),
            ("play_card_with_work_inner", BitRead::Driver),
        ],
    );
    // The scan sees through its own exclusions: a test module and a comment
    // are skipped, a second read in one function is counted.
    let sample = "fn a() {\n    x.requires_action_replay();\n    // y.requires_action_replay()\n    \
                  z.requires_action_replay() && w.requires_action_replay()\n}\n#[cfg(test)]\nmod \
                  tests {\n    fn b() {\n        x.requires_action_replay();\n    }\n}\nfn c() \
                  {\n    x.requires_action_replay()\n}\n";
    assert_eq!(
        production_reads(sample, ".requires_action_replay()"),
        [("a".to_owned(), 3), ("c".to_owned(), 1)],
    );
}

/// The one place a forged `normality_count` turns an exact result into a
/// refusal. `play::note_normality_card_play_started` refuses a card body
/// started on the enemy side ("Normality enemy-side CardPlayStarted"), and
/// it only looks when the catalog keeps the count. An admitted fight does
/// start card bodies on the enemy side: a monster's hit arms Centennial
/// Puzzle, its Draw runs inside the attack, and Hellraiser plays the drawn
/// Strikes (`tests/rupture_side_gate.rs`, #3632). The same root plays
/// exactly without the record and refuses by name with it.
///
/// A session whose own closure holds Normality refuses here too, with or
/// without the record: the record adds no refusal a truthful session lacks.
#[test]
fn a_forged_normality_count_refuses_an_enemy_side_card_body() {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["hand"][0]["id"] = "RUPTURE".into();
    value["piles"]["hand"][3]["id"] = "HELLRAISER".into();
    value["piles"]["draw"][0]["id"] = "STRIKE_IRONCLAD".into();
    value["player"]["relics_entering"]
        .as_array_mut()
        .unwrap()
        .push("RELIC.CENTENNIAL_PUZZLE".into());
    value["player"]["puzzle"] = true.into();
    let plain: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    assert!(!plain.player.contains_key("session_bookkeeping"));
    let mut forged = plain.clone();
    forged
        .player
        .insert("session_bookkeeping".to_owned(), json!(["normality_count"]));

    let line = [play(0, None), play(3, None), Action::EndTurn];
    let run = |document: &CanonicalStateV2| {
        let (catalog, mut state) = hydrate(document);
        for (index, action) in line.iter().enumerate() {
            state = engine::apply_action(&state, &catalog, action)
                .map_err(|refusal| (index, refusal.to_string()))?
                .state;
        }
        Ok::<_, (usize, String)>(project(&state, &catalog))
    };
    // Without the record: exact, through the enemy-side plays.
    let exact = run(&plain).unwrap();
    assert!(!exact.player.contains_key("session_bookkeeping"));
    // With it: both plays on the player side are counted and exact, and the
    // EndTurn that starts a card body on the enemy side refuses by name.
    let (index, refusal) = run(&forged).unwrap_err();
    assert_eq!(index, 2, "{refusal}");
    assert!(
        refusal.contains("Normality enemy-side CardPlayStarted"),
        "{refusal}"
    );
}

/// What one read of a bookkeeping bit does (#3660).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BookkeepingRead {
    /// Keeps or skips a ledger row, the count, or a uid assignment. The
    /// scalars and every rule of play are the same either way.
    Write,
    /// A short-circuit ahead of a pile scan that finds nothing without the
    /// card.
    Scan,
    /// Accepts or refuses. `temp_strength_wrapper_on_retained_owner` refuses
    /// a nested death when the ledger cannot say whether a wrapper is
    /// already attached; `note_normality_card_play_started` refuses a card
    /// body started on the enemy side when it keeps the count.
    WriteAndRefusal,
    /// Mirrors the catalog bit into the state at hydration.
    Mirror,
}

/// Every production read of the three bookkeeping bits and of the state
/// flag that mirrors the first, by file and enclosing function.
///
/// `player.session_bookkeeping` rests on these reads being bookkeeping: a
/// recorded bit may add a write or move a refusal and must never change an
/// admitted outcome. The two `WriteAndRefusal` rows are the refusals; each
/// has a test of its own. The scan fails when a read is added, moved or
/// removed without being classified here.
const BOOKKEEPING_READS: &[(&str, &str, &str, usize, BookkeepingRead)] = &[
    (
        ".misery_is_reachable()",
        "boundary.rs",
        "from_canonical",
        1,
        BookkeepingRead::Mirror,
    ),
    (
        ".has_normality()",
        "engine/play.rs",
        "normality_blocks_card_plays",
        1,
        BookkeepingRead::Scan,
    ),
    (
        ".has_normality()",
        "engine/play.rs",
        "note_normality_card_play_started",
        1,
        BookkeepingRead::WriteAndRefusal,
    ),
    (
        ".has_batch139_self_return()",
        "engine/cards.rs",
        "self_return_turn_is_reachable",
        1,
        BookkeepingRead::Scan,
    ),
    (
        ".has_batch139_self_return()",
        "engine/cards.rs",
        "freeze_self_return_before_hand_draw",
        1,
        BookkeepingRead::Write,
    ),
    // Hydration: an `unknown` row for a living monster's entering Strength.
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "materialize_entering_strength_provenance",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "temp_strength_wrapper_on_retained_owner",
        1,
        BookkeepingRead::WriteAndRefusal,
    ),
    // Each of the rest hands the flag to a ledger writer
    // (`write_monster_strength`, `write_monster_self_strength`,
    // `write_monster_temp_strength_wrapper`,
    // `unwind_monster_temp_strength_wrappers`,
    // `clear_monster_temp_strength_wrappers`), whose scalar result does not
    // depend on it.
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "dispatch_monster_damage_result",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "powers_after_damage_given",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "finish_monster_death_body",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "corpse_slug_ravenous_after_death",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "finish_secondary_death_cascade",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "monster_attack_player_positive_results_apply",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "apply_monster_strength_delta_after_type_two_gate",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "apply_monster_temp_strength_wrapper_after_type_two_gate",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/damage.rs",
        "apply_potion_temporary_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/monsters.rs",
        "crab_rage_after_death",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/monsters.rs",
        "fur_coat_after_opponent_added",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/play.rs",
        "apply_test_subject_enrage",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "engine/turn.rs",
        "run_enemy_phase_inner",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/boss.rs",
        "apply",
        3,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/boss.rs",
        "ponder",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/boss.rs",
        "soul_siphon",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/elite.rs",
        "ento_spit_exact",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "attack_steal",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "attack_strength_branch",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "axebot_bootup",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "louse_curl_grow",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "possess_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/normal.rs",
        "weak_player_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/shared.rs",
        "attack_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/shared.rs",
        "block_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/shared.rs",
        "add_monster_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/shared.rs",
        "wriggle",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "moves/spawned.rs",
        "buff_team_strength",
        1,
        BookkeepingRead::Write,
    ),
    (
        ".misery_attachment_upkeep()",
        "steps/templates.rs",
        "strength_enemy",
        1,
        BookkeepingRead::Write,
    ),
];

fn production_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    walk(&src, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path
                .strip_prefix(&src)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, std::fs::read_to_string(&path).unwrap())
        })
        .collect()
}

#[test]
fn every_read_of_a_bookkeeping_bit_is_classified() {
    let sources = production_sources();
    let mut needles: Vec<&str> = BOOKKEEPING_READS.iter().map(|row| row.0).collect();
    needles.sort_unstable();
    needles.dedup();
    assert_eq!(
        needles,
        [
            ".has_batch139_self_return()",
            ".has_normality()",
            ".misery_attachment_upkeep()",
            ".misery_is_reachable()",
        ]
    );
    let mut found: Vec<(String, String, String, usize)> = Vec::new();
    for (name, source) in &sources {
        // The accessors and the builder live in these two; neither reads.
        if matches!(name.as_str(), "catalog.rs" | "hot.rs") {
            continue;
        }
        for needle in &needles {
            for (function, count) in production_reads(source, needle) {
                found.push(((*needle).to_owned(), name.clone(), function, count));
            }
        }
    }
    let mut expected: Vec<(String, String, String, usize)> = BOOKKEEPING_READS
        .iter()
        .map(|(needle, file, function, count, _)| {
            (
                (*needle).to_owned(),
                (*file).to_owned(),
                (*function).to_owned(),
                *count,
            )
        })
        .collect();
    expected.sort();
    found.sort();
    assert_eq!(
        found, expected,
        "a read of a bookkeeping bit was added, moved or removed: classify it in BOOKKEEPING_READS",
    );
    assert_eq!(found.iter().map(|row| row.3).sum::<usize>(), 38);
    // The reads that can move a refusal, named so a new one cannot join
    // them unnoticed.
    let refusals: Vec<&str> = BOOKKEEPING_READS
        .iter()
        .filter(|row| row.4 == BookkeepingRead::WriteAndRefusal)
        .map(|row| row.2)
        .collect();
    assert_eq!(
        refusals,
        [
            "note_normality_card_play_started",
            "temp_strength_wrapper_on_retained_owner"
        ]
    );
    // The flag reaches the ledger writers as a parameter. Inside them it is
    // read at exactly these places, each of which keeps, restacks or
    // abandons a ledger row and returns the same scalars either way.
    let damage = &sources
        .iter()
        .find(|(name, _)| name == "engine/damage.rs")
        .unwrap()
        .1;
    let production = damage.split("\n#[cfg(test)]\nmod tests {").next().unwrap();
    let branches = |needle: &str| production.matches(needle).count();
    assert_eq!(branches("if !upkeep || previous == updated {"), 1);
    assert_eq!(branches("    if upkeep {\n"), 3);
    assert_eq!(branches("    if !upkeep {\n"), 1);
    assert_eq!(
        branches(
            "} else if upkeep && super::monsters::temp_strength_provenance_is_recorded(monster) {"
        ),
        1
    );
    assert_eq!(
        production.matches("upkeep &&").count()
            + production.matches("!upkeep").count()
            + production.matches("if upkeep").count(),
        7,
        "a new branch on the ledger flag in engine/damage.rs: read it and classify it here",
    );
}

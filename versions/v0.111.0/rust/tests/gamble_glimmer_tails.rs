//! #2667: Calculated Gamble and Glimmer own their nested Draw returns.
//!
//! Gamble's paired discard/draw freezes the hand count, so the parked Draw
//! record carries the frozen count plus the Sly siblings in discard order;
//! the Sly batch replays over the live-resolved objects only after the Draw
//! fully returns. Glimmer's exact-one selector freezes from the fresh live
//! Hand after its awaited Draw returns — never before, and never mislabeled
//! as an inner Draw suspension.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::HotState,
    ids::PowerId,
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!(
        {"hp":100,"max_hp":100,"player_phase":3,"next_card_uid":10}
    ))
    .unwrap();
    doc.monsters = serde_json::from_value(json!(
        [{"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}]
    ))
    .unwrap();
    doc
}

fn with_hellraiser(doc: &mut CanonicalStateV2) {
    doc.player.insert("hellraiser".into(), json!(1));
    doc.player.insert(
        "after_side_turn_end_power_order".into(),
        json!([["hellraiser", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
}

fn with_stratagem(doc: &mut CanonicalStateV2) {
    doc.player.insert("stratagem".into(), json!(1));
}

fn play(doc: &CanonicalStateV2, uid: u32) -> engine::Transition {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    engine::apply_action(
        &state,
        &catalog,
        &Action::Play {
            uid,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
    .unwrap()
}

/// Cold roundtrip plus admission, mirroring the owned-draw-tail witnesses.
fn wire(doc: &CanonicalStateV2, state: &HotState) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let wire = HotBoundary::try_to_canonical(state, &catalog).unwrap();
    let rebuilt = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &catalog).unwrap();
    assert_eq!(cold, *state, "cold publication preserves exact state");
    if !state.history.over {
        let rebuilt_state = HotBoundary::from_canonical(&wire, &rebuilt).unwrap();
        engine::admit(&wire, &rebuilt_state, &rebuilt).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt_state, &rebuilt).unwrap(),
            wire
        );
    }
    wire
}

fn cold_apply(parked: &CanonicalStateV2, action: Action) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(parked).unwrap();
    let state = HotBoundary::from_canonical(parked, &catalog).unwrap();
    engine::admit(parked, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    wire(parked, &next)
}

/// Drive every legal answer from a parked state to a terminal wire.
fn terminals(parked: &CanonicalStateV2) -> Vec<CanonicalStateV2> {
    let mut frontier = vec![parked.clone()];
    let mut done = Vec::new();
    while let Some(current) = frontier.pop() {
        if !current.player.contains_key("pending") {
            done.push(current);
            continue;
        }
        let catalog = HotBoundary::catalog_from_canonical(&current).unwrap();
        let state = HotBoundary::from_canonical(&current, &catalog).unwrap();
        engine::admit(&current, &state, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert!(!choices.is_empty(), "a parked state offers an answer");
        for choice in choices {
            frontier.push(cold_apply(&current, choice));
        }
    }
    done
}

fn hand_uids(doc: &CanonicalStateV2) -> Vec<u32> {
    doc.piles["hand"]
        .iter()
        .map(|card| card.uid.unwrap() as u32)
        .collect()
}

/// The parked Gamble Draw's frozen program from the cold wire.
fn gamble_program(frame: &sts_sim::canonical::CanonicalFrameV2) -> &serde_json::Value {
    frame
        .fields
        .get("caller_locals")
        .expect("a CardPlay-owned Draw carries caller_locals")
}

fn parked_gamble_frame(parked: &CanonicalStateV2) -> &sts_sim::canonical::CanonicalFrameV2 {
    parked
        .continuations
        .iter()
        .find(|frame| {
            frame
                .fields
                .get("caller")
                .and_then(|caller| caller.as_str())
                == Some("none")
        })
        .unwrap_or_else(|| panic!("a parked Gamble Draw is on the stack"))
}

#[test]
fn gamble_sync_sly_batch_runs_in_frozen_discard_order() {
    let mut doc = entry();
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"CALCULATED_GAMBLE","uid":0,"upgrade":0},
            {"id":"REFLEX","uid":1,"upgrade":0},
            {"id":"ABRASIVE","uid":2,"upgrade":0}
        ],
        "draw": (3..9).map(|uid|json!({"id":"DEFEND_SILENT","uid":uid,"upgrade":0})).collect::<Vec<_>>(),
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    assert!(
        !result.state.pending.is_some(),
        "an unobstructed paired Draw completes inline"
    );
    // The paired Draw moves 3 and 4; the replayed Sly batch moves 5 and 6
    // after it — never interleaved, never twice.
    let drawn: Vec<u32> = result
        .events
        .iter()
        .filter_map(|event| match event {
            engine::Event::CardDrawn { uid } => Some(*uid),
            _ => None,
        })
        .collect();
    assert_eq!(drawn, [3, 4, 5, 6], "paired draws precede the Sly redraw");
    assert_eq!(
        result.state.powers.value(PowerId::Dexterity),
        1,
        "the replayed Abrasive grants Dexterity exactly once"
    );
    assert_eq!(
        result.state.powers.value(PowerId::Thorns),
        4,
        "the replayed Abrasive grants Thorns exactly once"
    );
    let published = wire(&doc, &result.state);
    assert_eq!(hand_uids(&published), [3, 4, 5, 6]);
}

#[test]
fn gamble_hellraiser_suspend_parks_paired_and_replays_sly_once() {
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"CALCULATED_GAMBLE","uid":0,"upgrade":0},
            {"id":"ABRASIVE","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":2,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":7,"upgrade":0}
        ],
        // The Strike arrives third: two drawn cards sit in hand when its
        // nested Select runs, so the selection parks instead of completing.
        "draw": [
            {"id":"DEFEND_SILENT","uid":4,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"SCULPTING_STRIKE","uid":3,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Hellraiser selection parks the paired Draw"
    );
    let frame = parked_gamble_frame(&parked);
    let program = gamble_program(frame);
    assert_eq!(
        program[0],
        json!("gamble_paired"),
        "the program rides caller_locals"
    );
    assert_eq!(
        program[1],
        json!(3),
        "the frozen count is the three discards"
    );
    assert_eq!(
        program[2].as_array().unwrap().len(),
        1,
        "only the Sly sibling is captured"
    );
    assert_eq!(
        program[2][0][0],
        json!(1),
        "the sibling is the discarded Abrasive"
    );
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        assert!(!terminal.player.contains_key("pending"));
        assert!(terminal.continuations.is_empty());
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        assert_eq!(
            state.powers.value(PowerId::Dexterity),
            1,
            "the resumed batch grants Dexterity exactly once"
        );
        assert_eq!(
            state.powers.value(PowerId::Thorns),
            4,
            "the resumed batch grants Thorns exactly once"
        );
    }
}

#[test]
fn gamble_stratagem_reshuffle_resolves_sly_from_live_piles() {
    let mut doc = entry();
    with_stratagem(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"CALCULATED_GAMBLE","uid":0,"upgrade":0},
            {"id":"ABRASIVE","uid":1,"upgrade":0}
        ],
        "draw": [],
        "discard": [
            {"id":"DEFEND_SILENT","uid":7,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":8,"upgrade":0}
        ],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Stratagem reshuffle parks the paired Draw"
    );
    let frame = parked_gamble_frame(&parked);
    assert_eq!(gamble_program(frame)[1], json!(1));
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        assert!(!terminal.player.contains_key("pending"));
        assert!(terminal.continuations.is_empty());
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        // The Abrasive crossed Discard into the reshuffled Draw pile mid
        // suspend; the resume resolves it live and plays it exactly once.
        assert_eq!(state.powers.value(PowerId::Dexterity), 1);
        assert_eq!(state.powers.value(PowerId::Thorns), 4);
    }
}

#[test]
fn gamble_repeat_plays_keep_independent_programs() {
    let mut doc = entry();
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"CALCULATED_GAMBLE","uid":0,"upgrade":0},
            {"id":"REFLEX","uid":1,"upgrade":0}
        ],
        "draw": [
            {"id":"CALCULATED_GAMBLE","uid":9,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":3,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":4,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    // First play discards one, draws the second Gamble, and replays Reflex.
    let first = play(&doc, 0);
    assert!(!first.state.pending.is_some());
    let midst = wire(&doc, &first.state);
    assert_eq!(hand_uids(&midst), [9, 3, 4]);
    // The drawn second Gamble plays with its own frozen count of two.
    let second = play(&midst, 9);
    assert!(!second.state.pending.is_some());
    let published = wire(&midst, &second.state);
    assert_eq!(hand_uids(&published), [5, 6]);
}

#[test]
fn glimmer_sync_draw_then_select_moves_to_draw_top_both_levels() {
    for level in 0..=1 {
        let mut doc = entry();
        doc.piles = serde_json::from_value(json!({
            "hand": [
                {"id":"GLIMMER","uid":0,"upgrade":level},
                {"id":"STRIKE_SILENT","uid":1,"upgrade":0}
            ],
            "draw": (2..8).map(|uid|json!({"id":"DEFEND_SILENT","uid":uid,"upgrade":0})).collect::<Vec<_>>(),
            "discard": [],
        }))
        .unwrap();
        let result = play(&doc, 0);
        let parked = wire(&doc, &result.state);
        assert!(
            parked.player.contains_key("pending"),
            "level {level}: the exact-one selector parks after the Draw returns"
        );
        assert_eq!(
            parked.player["pending"][1],
            json!(0),
            "level {level}: the parked selection belongs to the Glimmer owner"
        );
        let catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
        let state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert_eq!(choices.len(), 3 + level as usize + 1, "level {level}");
        let before = hand_uids(&parked);
        let answered = cold_apply(&parked, choices[0]);
        let after = hand_uids(&answered);
        assert_eq!(after.len(), before.len() - 1, "level {level}");
        let moved = before
            .iter()
            .find(|uid| !after.contains(uid))
            .copied()
            .unwrap();
        assert_eq!(
            answered.piles["draw"].first().unwrap().uid.unwrap() as u32,
            moved,
            "level {level}: the chosen card lands on top of the Draw pile"
        );
    }
}

#[test]
fn glimmer_hellraiser_inner_suspend_is_not_the_outer_selection() {
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"GLIMMER","uid":0,"upgrade":0},
            {"id":"STRIKE_SILENT","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":8,"upgrade":0}
        ],
        "draw": [
            {"id":"SCULPTING_STRIKE","uid":2,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":3,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":4,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    eprintln!(
        "DEBUG glimmer pending={} pend={:?} events={:?}",
        result.state.pending.is_some(),
        result.state.pending.as_deref().map(|_| ()),
        result
            .events
            .iter()
            .map(|event| match event {
                engine::Event::CardDrawn { uid } => format!("Drawn{uid}"),
                engine::Event::CardResolved { uid, .. } => format!("Resolved{uid}"),
                engine::Event::CardPlayed { uid, .. } => format!("Played{uid}"),
                _ => "?".to_owned(),
            })
            .collect::<Vec<_>>()
    );
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Hellraiser selection parks the awaited Draw"
    );
    assert_ne!(
        parked.player["pending"][1],
        json!(0),
        "the inner suspension is never mislabeled as the outer Glimmer selection"
    );
    // Answer the inner selection: only after the Draw fully returns does
    // the Glimmer selector itself park.
    let mut current = parked;
    for _ in 0..8 {
        if current.player["pending"][1] == json!(0) {
            break;
        }
        let catalog = HotBoundary::catalog_from_canonical(&current).unwrap();
        let state = HotBoundary::from_canonical(&current, &catalog).unwrap();
        engine::admit(&current, &state, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert!(!choices.is_empty());
        current = cold_apply(&current, choices[0]);
    }
    assert_eq!(
        current.player["pending"][1],
        json!(0),
        "the Glimmer selector parks after its Draw returns"
    );
    let done = terminals(&current);
    assert!(!done.is_empty());
    for terminal in &done {
        assert!(!terminal.player.contains_key("pending"));
        assert!(terminal.continuations.is_empty());
    }
}

#[test]
fn glimmer_stratagem_suspend_then_select_moves_to_draw_top() {
    let mut doc = entry();
    with_stratagem(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"GLIMMER","uid":0,"upgrade":0},
            {"id":"STRIKE_SILENT","uid":1,"upgrade":0}
        ],
        "draw": [],
        "discard": [
            {"id":"DEFEND_SILENT","uid":7,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":8,"upgrade":0}
        ],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Stratagem reshuffle parks the awaited Draw"
    );
    assert_ne!(
        parked.player["pending"][1],
        json!(0),
        "the reshuffle suspension is never mislabeled as the Glimmer selection"
    );
    let mut current = parked;
    for _ in 0..8 {
        if current.player["pending"][1] == json!(0) {
            break;
        }
        let catalog = HotBoundary::catalog_from_canonical(&current).unwrap();
        let state = HotBoundary::from_canonical(&current, &catalog).unwrap();
        engine::admit(&current, &state, &catalog).unwrap();
        let choices = engine::legal_actions(&state, &catalog);
        assert!(!choices.is_empty());
        current = cold_apply(&current, choices[0]);
    }
    assert_eq!(
        current.player["pending"][1],
        json!(0),
        "the Glimmer selector parks after its Draw returns"
    );
    let before = hand_uids(&current);
    let catalog = HotBoundary::catalog_from_canonical(&current).unwrap();
    let state = HotBoundary::from_canonical(&current, &catalog).unwrap();
    let choices = engine::legal_actions(&state, &catalog);
    assert!(!choices.is_empty());
    let answered = cold_apply(&current, choices[0]);
    let after = hand_uids(&answered);
    assert_eq!(after.len() + 1, before.len());
    let moved = before
        .iter()
        .find(|uid| !after.contains(uid))
        .copied()
        .unwrap();
    assert_eq!(
        answered.piles["draw"].first().unwrap().uid.unwrap() as u32,
        moved
    );
}

#[test]
fn gamble_paired_malformed_programs_refuse_at_the_boundary() {
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"CALCULATED_GAMBLE","uid":0,"upgrade":0},
            {"id":"ABRASIVE","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":2,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":7,"upgrade":0}
        ],
        "draw": [
            {"id":"DEFEND_SILENT","uid":4,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"SCULPTING_STRIKE","uid":3,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    let frame = parked_gamble_frame(&parked);
    let program = gamble_program(frame).clone();
    let sibling = program[2][0].clone();
    let cases: Vec<(&str, serde_json::Value)> = vec![
        ("unknown tag", json!(["gambleXXXX", 2, [[1, 0]]])),
        ("zero count", json!(["gamble_paired", 0, [[1, 0]]])),
        (
            "count mismatch",
            json!(["gamble_paired", 4, [sibling.clone()]]),
        ),
        (
            "duplicate sibling",
            json!(["gamble_paired", 3, [sibling.clone(), sibling.clone()]]),
        ),
        ("unknown uid", json!(["gamble_paired", 3, [[999, 0]]])),
        (
            "atom mismatch",
            json!([
                "gamble_paired",
                2,
                [[
                    sibling[0].clone(),
                    (sibling[1].as_u64().unwrap() + 1) % 65536
                ]]
            ]),
        ),
        (
            "siblings exceed count",
            json!(["gamble_paired", 1, [sibling.clone(), [2, 0]]]),
        ),
    ];
    for (name, locals) in cases {
        let mut forged = parked.clone();
        let index = forged
            .continuations
            .iter()
            .position(|candidate| {
                candidate
                    .fields
                    .get("caller")
                    .and_then(|caller| caller.as_str())
                    == Some("none")
            })
            .unwrap();
        forged.continuations[index]
            .fields
            .insert("caller_locals".to_owned(), locals);
        let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
        assert!(
            HotBoundary::from_canonical(&forged, &catalog).is_err(),
            "{name} refuses at the boundary"
        );
    }
}

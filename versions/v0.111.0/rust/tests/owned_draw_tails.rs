//! #2665: BigBang, EscapePlan, and Expertise own their Draw tails.
//!
//! Each body issues its Draw as an authenticated CardPlay-owned tail, so a
//! Hellraiser selection or Stratagem reshuffle can suspend it mid-command.
//! The suffix resumes after the Draw returns: BigBang's Stars/Energy/Forge,
//! EscapePlan's first-returned Skill read, and Expertise's per-object Retain
//! over retained, removed, and moved references alike.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::HotState,
};

fn entry() -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":10
    }))
    .unwrap();
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
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

/// Cold roundtrip plus admission, mirroring the Scrape witnesses.
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

fn cold_apply(parked: &CanonicalStateV2, action: Action) -> (CanonicalStateV2, Vec<engine::Event>) {
    let catalog = HotBoundary::catalog_from_canonical(parked).unwrap();
    let state = HotBoundary::from_canonical(parked, &catalog).unwrap();
    engine::admit(parked, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    (wire(parked, &next), events)
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
            frontier.push(cold_apply(&current, choice).0);
        }
    }
    done
}

#[test]
fn big_bang_ordinary_tail_runs_stars_energy_forge_both_levels() {
    for level in 0..=1 {
        let mut doc = entry();
        doc.piles = serde_json::from_value(json!({
            "hand": [{"id":"BIG_BANG","uid":0,"upgrade":level}],
            "draw": [{"id":"DEFEND_SILENT","uid":1,"upgrade":0}],
            "discard": [],
        }))
        .unwrap();
        let result = play(&doc, 0);
        assert!(!result.state.history.over);
        assert_eq!(result.state.stars, 1, "level {level}");
        assert_eq!(result.state.energy, 4, "level {level}");
        assert!(
            result
                .state
                .piles
                .get(sts_sim::hot::PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 0),
            "BigBang exhausts"
        );
        wire(&doc, &result.state);
    }
}

#[test]
fn big_bang_hellraiser_suspend_resumes_the_suffix() {
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"BIG_BANG","uid":0,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "draw": [{"id":"SCULPTING_STRIKE","uid":1,"upgrade":0}],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Hellraiser selection parks the owned Draw"
    );
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        assert!(!terminal.player.contains_key("pending"));
        assert!(terminal.continuations.is_empty());
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        assert_eq!(state.stars, 1, "the resumed tail grants Stars");
        assert_eq!(state.energy, 4, "the resumed tail grants Energy");
    }
}

#[test]
fn escape_plan_reads_first_returned_skill_both_levels() {
    for level in 0..=1 {
        // Skill drawn: powered block lands.
        let mut doc = entry();
        doc.piles = serde_json::from_value(json!({
            "hand": [{"id":"ESCAPE_PLAN","uid":0,"upgrade":level}],
            "draw": [{"id":"DEFEND_SILENT","uid":1,"upgrade":0}],
            "discard": [],
        }))
        .unwrap();
        let result = play(&doc, 0);
        assert_eq!(
            result.state.block,
            if level == 0 { 3 } else { 5 },
            "level {level}"
        );
        wire(&doc, &result.state);

        // Attack drawn: no block.
        let mut doc = entry();
        doc.piles = serde_json::from_value(json!({
            "hand": [{"id":"ESCAPE_PLAN","uid":0,"upgrade":level}],
            "draw": [{"id":"SCULPTING_STRIKE","uid":1,"upgrade":0}],
            "discard": [],
        }))
        .unwrap();
        let result = play(&doc, 0);
        assert_eq!(result.state.block, 0, "level {level}");
        wire(&doc, &result.state);
    }
}

#[test]
fn escape_plan_hellraiser_suspend_reads_the_returned_attack() {
    // The drawn Strike leaves Hand under Hellraiser AutoPlay; the resumed
    // read still sees the returned object (an Attack), not the hand tail.
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"ESCAPE_PLAN","uid":0,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "draw": [{"id":"SCULPTING_STRIKE","uid":1,"upgrade":0}],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Hellraiser selection parks the owned Draw"
    );
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        assert_eq!(state.block, 0, "the returned Strike is not a Skill");
    }
}

#[test]
fn escape_plan_stratagem_reshuffle_resumes_the_skill_read() {
    let mut doc = entry();
    doc.player.insert("stratagem".into(), json!(1));
    doc.piles = serde_json::from_value(json!({
        "hand": [{"id":"ESCAPE_PLAN","uid":0,"upgrade":1}],
        "draw": [],
        "discard": [
            {"id":"DEFEND_SILENT","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":2,"upgrade":0}
        ],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Stratagem selection parks the reshuffling Draw"
    );
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        assert_eq!(state.block, 5, "the returned Skill grants block");
    }

    // A forged `requested` count refuses at reload or admission: the record
    // re-derives one Draw from the EscapePlan program.
    let mut forged = parked.clone();
    let owner = forged
        .continuations
        .iter_mut()
        .find(|frame| frame.frame_type == "DrawFrame")
        .expect("a parked Draw carries its record");
    owner.fields.insert("requested".to_owned(), json!(2));
    let refused = HotBoundary::catalog_from_canonical(&forged).and_then(|catalog| {
        HotBoundary::from_canonical(&forged, &catalog).map(|state| (catalog, state))
    });
    assert!(
        refused.is_err()
            || refused
                .is_ok_and(|(catalog, state)| engine::admit(&forged, &state, &catalog).is_err()),
        "an inflated requested count cannot authenticate"
    );
}

#[test]
fn expertise_retains_returned_objects_both_levels() {
    for level in 0..=1 {
        let count = if level == 0 { 2 } else { 3 };
        let mut doc = entry();
        doc.piles = serde_json::from_value(json!({
            "hand": [{"id":"EXPERTISE","uid":0,"upgrade":level}],
            "draw": (1..=count)
                .map(|uid| json!({"id":"DEFEND_SILENT","uid":uid,"upgrade":0}))
                .collect::<Vec<_>>(),
            "discard": [],
        }))
        .unwrap();
        let result = play(&doc, 0);
        assert!(result.state.exact_piles);
        for uid in 1..=count {
            assert!(
                result.state.card_states.get(uid).transient_retain,
                "level {level} retains uid {uid}"
            );
        }
        wire(&doc, &result.state);
    }
}

#[test]
fn expertise_hellraiser_consumed_card_keeps_its_retain() {
    // The drawn Strike is consumed by Hellraiser AutoPlay (moved to
    // Discard); the resumed suffix still retains its uid alongside the
    // Hand-retained sibling.
    let mut doc = entry();
    with_hellraiser(&mut doc);
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"EXPERTISE","uid":0,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "draw": [
            {"id":"SCULPTING_STRIKE","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":2,"upgrade":0}
        ],
        "discard": [],
    }))
    .unwrap();
    let result = play(&doc, 0);
    let parked = wire(&doc, &result.state);
    assert!(
        parked.player.contains_key("pending"),
        "the Hellraiser selection parks the owned Draw"
    );
    let done = terminals(&parked);
    assert!(!done.is_empty());
    for terminal in &done {
        assert!(!terminal.player.contains_key("pending"));
        assert!(terminal.continuations.is_empty());
        let catalog = HotBoundary::catalog_from_canonical(terminal).unwrap();
        let state = HotBoundary::from_canonical(terminal, &catalog).unwrap();
        assert!(
            state.card_states.get(1).transient_retain,
            "the moved Strike keeps its Retain"
        );
        assert!(
            state.card_states.get(2).transient_retain,
            "the retained sibling keeps its Retain"
        );
    }
}

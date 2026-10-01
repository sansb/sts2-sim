//! Rooted Tools-of-the-Trade Sly batches take the generic cursor-aware frame
//! walk (#2671, PR 1).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
//! `ToolsOfTheTradePower/<AfterPlayerTurnStart>d__5::MoveNext` (RVA
//! `0x349cb4`) builds ONE `CardSelectorPrefs(prompt, Amount)` at
//! `IL_003c`-`IL_0053`, awaits ONE `CardSelectCmd::FromHandForDiscard` at
//! `IL_005a`, materialises the answer once at `IL_00b9`, and awaits ONE plural
//! `CardCmd::Discard` at `IL_00d0`. `CardCmd/<Discard>d__3::MoveNext` (RVA
//! `0x3e01ac`) forwards at `IL_0023` to `DiscardAndDraw` (RVA `0x3e0274`),
//! which freezes `<slyCards>5__4` before any movement (`IL_00f6`-`IL_0109`)
//! and then walks that frozen list (`IL_02cc`-`IL_02d8`), awaiting each
//! `CardCmd::AutoPlay` (`IL_02fd`) before its `MoveNext` (`IL_0360`-`IL_0365`).
//! The batch cursor is therefore an ordinary per-child position, not the
//! legacy first-child-only quotient, so the rooted aggregate is authenticated
//! by the same per-frame walk that already owns every other rooted Sly parent.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, MissingCapability},
};

/// A replay-capable end-of-turn document whose next hand carries one Sly
/// child. `I Am Invincible` forces the fight to require an ActionReplay root WITHOUT
/// being a Sly-discard card parent, so `parent_specs` stays empty and a live
/// singleton Tools listener is the ONLY Sly-batch source. The draw pile is
/// deliberately deeper than the turn-start draw so the discard survives.
fn entry(sly_id: &str, amount: i64) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player.insert("next_card_uid".to_owned(), json!(20));
    doc.player.insert("master_planner".to_owned(), json!(1));
    doc.player.insert(
        "after_card_played_power_order".to_owned(),
        json!([["master_planner", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
    doc.player
        .insert("tools_of_the_trade".to_owned(), json!(amount));
    doc.player.insert(
        "after_player_turn_start_power_order".to_owned(),
        json!(["tools_of_the_trade"]),
    );
    doc.monsters.truncate(1);
    doc.monsters[0].insert("hp".to_owned(), json!(100));
    doc.monsters[0].insert("max_hp".to_owned(), json!(100));
    doc.piles = serde_json::from_value(json!({
        "hand": [
            {"id":"I_AM_INVINCIBLE","upgrade":0,"uid":0},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":2},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":3}
        ],
        "draw": [
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":10},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":11},
            {"id":sly_id,"upgrade":0,"uid":12,"local_keywords":["Sly"]},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":13},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":14},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":15},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":16},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":17}
        ],
        "discard": [
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":5},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":6}
        ]
    }))
    .unwrap();
    doc
}

fn cold_apply(doc: &CanonicalStateV2, action: Action) -> (CanonicalStateV2, Vec<Event>) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    (
        HotBoundary::try_to_canonical(&next, &catalog).unwrap(),
        events,
    )
}

fn reload_admitted(doc: &CanonicalStateV2) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        *doc
    );
}

fn cold_refuses(doc: &CanonicalStateV2) {
    match HotBoundary::catalog_from_canonical(doc).and_then(|catalog| {
        HotBoundary::from_canonical(doc, &catalog).map(|state| (catalog, state))
    }) {
        Err(_) => {}
        Ok((catalog, state)) => assert!(engine::admit(doc, &state, &catalog).is_err()),
    }
}

fn played(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::CardPlayed { uid, .. } => Some(*uid),
            _ => None,
        })
        .collect()
}

/// The Tools discard answer that picks exactly `uid`.
fn tools_pick(doc: &CanonicalStateV2, uid: u32) -> Action {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| {
            let Action::Select { answer } = *action else {
                return false;
            };
            engine::selected_card_uids(&state, &catalog, answer)
                .ok()
                .flatten()
                .as_deref()
                == Some(&[uid][..])
        })
        .unwrap_or_else(|| panic!("no Tools answer picks {uid}"))
}

fn frame_types(doc: &CanonicalStateV2) -> Vec<String> {
    doc.continuations
        .iter()
        .map(|frame| frame.frame_type.clone())
        .collect()
}

fn uids(doc: &CanonicalStateV2, pile: &str) -> Vec<u32> {
    doc.piles
        .get(pile)
        .map(|cards| {
            cards
                .iter()
                .map(|card| card.uid.unwrap() as u32)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn child_answer(doc: &CanonicalStateV2) -> Action {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| matches!(action, Action::Select { .. }))
        .expect("the parked Sly child offers its own selection")
}

/// The rooted four-frame aggregate the legacy first-child quotient refused.
///
/// `Hologram` selects from the Discard pile, which is exactly the pile the
/// legacy proof's `first_selector_excludes` required the child NOT to read, so
/// before this slice the public Tools answer refused outright. The generic
/// walk binds ownership at `batch.cursor - 1` instead, and the generic driver
/// resumes it and returns once to the listener chain.
#[test]
fn rooted_tools_discard_selecting_sly_child_parks_reloads_and_completes() {
    let doc = entry("HOLOGRAM", 1);
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    assert_eq!(frame_types(&choice), vec!["ActionReplayFrame".to_owned()]);
    reload_admitted(&choice);

    let (parked, events) = cold_apply(&choice, tools_pick(&choice, 12));
    assert_eq!(
        played(&events),
        vec![12],
        "the Sly child plays exactly once"
    );
    assert_eq!(
        frame_types(&parked),
        vec![
            "ActionReplayFrame".to_owned(),
            "TurnStartHandChoiceFrame".to_owned(),
            "FrozenAutoBatchFrame".to_owned(),
            "CardPlayFrame".to_owned(),
        ],
        "the rooted Tools aggregate parks at its own selector"
    );
    assert!(parked.player.contains_key("pending"));
    assert_eq!(uids(&parked, "play"), vec![12]);
    // Cold reload of the newly routed aggregate is byte-identical.
    reload_admitted(&parked);

    let (completed, events) = cold_apply(&parked, child_answer(&parked));
    assert!(completed.continuations.is_empty(), "{completed:?}");
    assert!(!completed.player.contains_key("pending"));
    assert!(
        played(&events).is_empty(),
        "resuming the child replays nothing"
    );
    // The listener chain advances exactly once: the turn after the Tools
    // listener began is entered a single time.
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::TurnBegan { .. }))
            .count(),
        1
    );
    // The child's own body finished under the generic driver: Hologram
    // exhausted itself and the answered Discard card is back in Hand.
    assert_eq!(uids(&completed, "exhaust"), vec![12]);
    assert!(uids(&completed, "play").is_empty());
    assert!(uids(&parked, "discard").contains(&2));
    assert!(uids(&completed, "hand").contains(&2));
    assert!(!uids(&completed, "discard").contains(&2));
}

/// A Hand-selecting Sly child is one the legacy quotient already accepted at
/// cursor one. Its whole trajectory must be unchanged by the reroute.
#[test]
fn rooted_tools_hand_selecting_sly_child_is_unchanged() {
    let doc = entry("PURITY", 1);
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, 12));
    assert_eq!(played(&events), vec![12]);
    assert_eq!(
        frame_types(&parked),
        vec![
            "ActionReplayFrame".to_owned(),
            "TurnStartHandChoiceFrame".to_owned(),
            "FrozenAutoBatchFrame".to_owned(),
            "CardPlayFrame".to_owned(),
        ]
    );
    reload_admitted(&parked);

    let (completed, events) = cold_apply(&parked, child_answer(&parked));
    assert!(completed.continuations.is_empty());
    assert!(!completed.player.contains_key("pending"));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::TurnBegan { .. }))
            .count(),
        1
    );
}

/// Forged ownership on the newly routed aggregate still refuses atomically,
/// and a rooted Tools aggregate cannot lose its replay root.
#[test]
fn rooted_tools_aggregate_refuses_forged_ownership_atomically() {
    let doc = entry("HOLOGRAM", 1);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, _) = cold_apply(&choice, tools_pick(&choice, 12));

    let batch_frame = |doc: &mut CanonicalStateV2, key: &str, value: serde_json::Value| {
        doc.continuations
            .iter_mut()
            .find(|frame| frame.frame_type == "FrozenAutoBatchFrame")
            .unwrap()
            .fields
            .insert(key.to_owned(), value);
    };

    let mut forged_cursor = parked.clone();
    batch_frame(&mut forged_cursor, "cursor", json!(2));
    cold_refuses(&forged_cursor);

    let mut forged_source = parked.clone();
    batch_frame(&mut forged_source, "source", json!("Draw pile flip"));
    cold_refuses(&forged_source);

    let mut missing_root = parked.clone();
    assert_eq!(
        missing_root.continuations.remove(0).frame_type,
        "ActionReplayFrame"
    );
    cold_refuses(&missing_root);

    let mut vanished_child = parked.clone();
    vanished_child
        .piles
        .get_mut("play")
        .unwrap()
        .retain(|card| card.uid != Some(12));
    cold_refuses(&vanished_child);

    // The original document is untouched by every refusal above.
    reload_admitted(&parked);
}

/// The wall pin PR 1 left standing, moved by #2671 PR 2.
///
/// **Before:** this document — a Tools listener at amount two with two
/// CERTIFIED (`HOLOGRAM`) selecting Sly siblings — refused outright, because the
/// replay branch's `tools_plural_parent_reachable && effective_sly.len() > 1`
/// disjunct was a blanket cardinality wall that never looked at the children.
///
/// **Now:** that disjunct is gone and the batch is authenticated per child, so
/// the certified pair admits. The negative half moves onto the *child grammar*:
/// swapping either sibling for an uncertified row still refuses with
/// `ContinuationFrame`. The document and the fixture are otherwise unchanged, so
/// the pair pins exactly what the narrowing did and what it did not.
#[test]
fn plural_tools_roots_admit_only_certified_children() {
    let plural = |second: &str| {
        let mut doc = entry("HOLOGRAM", 2);
        let card = doc
            .piles
            .get_mut("draw")
            .unwrap()
            .iter_mut()
            .find(|card| card.uid == Some(14))
            .unwrap();
        card.id = second.to_owned();
        card.local_keywords = vec!["Sly".to_owned()];
        doc
    };

    let certified = plural("HOLOGRAM");
    let catalog = HotBoundary::catalog_from_canonical(&certified).unwrap();
    let state = HotBoundary::from_canonical(&certified, &catalog).unwrap();
    engine::admit(&certified, &state, &catalog)
        .expect("a plural batch of certified children admits since #2671 PR 2");

    let uncertified = plural("HEADBUTT");
    let catalog = HotBoundary::catalog_from_canonical(&uncertified).unwrap();
    let state = HotBoundary::from_canonical(&uncertified, &catalog).unwrap();
    let refusal = engine::admit(&uncertified, &state, &catalog)
        .expect_err("an uncertified plural sibling must still refuse");
    assert!(
        refusal.contains(MissingCapability::ContinuationFrame),
        "{refusal:?}"
    );
}

/// The same document with the Sly marker removed admits, so the refusal above
/// is attributable to the uncertified *child*, not to the fixture.
#[test]
fn the_uncertified_child_document_admits_once_it_is_not_sly() {
    let mut doc = entry("HEADBUTT", 1);
    doc.piles
        .get_mut("draw")
        .unwrap()
        .iter_mut()
        .find(|card| card.uid == Some(12))
        .unwrap()
        .local_keywords
        .clear();
    reload_admitted(&doc);
}

/// Every certified family stays clear of the Sly/Tools wall under a live
/// singleton Tools listener. The new gate must narrow to exactly the
/// uncertified rows; if it caught a certified family it would be a regression
/// dressed up as caution.
///
/// The assertion is the absence of `ContinuationFrame` — the capability this
/// wall surfaces — rather than blanket admission, because some of these rows
/// need document state this minimal fixture deliberately does not carry
/// (`DISCOVERY` and `QUASAR` refuse on `Discovery generation provenance`,
/// which is a different gate and not this slice's business). The uncertified
/// control above asserts the positive side of the same capability, so the pair
/// is not vacuous.
#[test]
fn singleton_tools_with_each_certified_child_family_clears_the_sly_wall() {
    for id in [
        "HOLOGRAM",
        "COSMIC_INDIFFERENCE",
        "DREDGE",
        "HIDDEN_DAGGERS",
        "BRAND",
        "SCAVENGE",
        "DISCOVERY",
        "QUASAR",
        "PURITY",
    ] {
        let doc = entry(id, 1);
        let catalog = HotBoundary::catalog_from_canonical(&doc)
            .unwrap_or_else(|error| panic!("{id}: catalog refused: {error:?}"));
        let state = HotBoundary::from_canonical(&doc, &catalog)
            .unwrap_or_else(|error| panic!("{id}: hydration refused: {error:?}"));
        if let Err(refusal) = engine::admit(&doc, &state, &catalog) {
            assert!(
                !refusal.contains(MissingCapability::ContinuationFrame),
                "{id}: certified child was caught by the Sly/Tools wall: {refusal:?}"
            );
        }
    }
}

/// `GLIMMER` is certified by `cardplay_no_result_draw_can_suspend`, which is a
/// CATALOG-conditional predicate: the owned Draw tail is only a suspension
/// point when some Draw hook in the fight can actually park. The new gate
/// therefore has to track the predicate, not the card name — Glimmer is caught
/// in a fight with no parking Draw hook and cleared in one with Stratagem live.
/// That asymmetry is the sharpest available evidence that `tools_parent_live`
/// routes through `replay_child_program_is_exact` rather than blanket-refusing.
#[test]
fn glimmer_is_walled_without_a_parking_draw_hook_and_cleared_with_one() {
    let without = entry("GLIMMER", 1);
    let catalog = HotBoundary::catalog_from_canonical(&without).unwrap();
    let state = HotBoundary::from_canonical(&without, &catalog).unwrap();
    let refusal = engine::admit(&without, &state, &catalog)
        .expect_err("Glimmer's Draw tail cannot be certified with no parking hook");
    assert!(
        refusal.contains(MissingCapability::ContinuationFrame),
        "{refusal:?}"
    );

    let mut with = entry("GLIMMER", 1);
    with.player.insert("stratagem".to_owned(), json!(1));
    let catalog = HotBoundary::catalog_from_canonical(&with).unwrap();
    let state = HotBoundary::from_canonical(&with, &catalog).unwrap();
    if let Err(refusal) = engine::admit(&with, &state, &catalog) {
        assert!(
            !refusal.contains(MissingCapability::ContinuationFrame),
            "a live Stratagem certifies Glimmer's Draw tail: {refusal:?}"
        );
    }
}

/// The admission gate this slice had to add, and the reason it is not
/// deferrable to PR 2.
///
/// `unsupported_sly_frozen_batch_future_is_reachable` used to take its early
/// `return false` whenever no Sly-discard card parent and no *plural* Tools
/// writer were reachable, so a live singleton Tools listener's current child
/// was never measured against `replay_child_program_is_exact`. Before #2671 the
/// late first-child continuation proof refused such a child mid-play; once the
/// rooted aggregate takes the generic frame walk that late refusal is gone, so
/// an UNCERTIFIED selecting Sly child would have been admitted by absence of a
/// check. `tools_parent_live` is that check.
///
/// `HEADBUTT` is a real reachable row outside the certified families: it is a
/// selecting Sly-capable child (`autoplay_child_requires_suspension` on the
/// Discard pile) that satisfies none of the nine `replay_child_program_is_exact`
/// disjuncts. It is one of exactly ten such rows on this build (Abundance,
/// Graveblast, Headbutt, Neow's Fury and Seeker Strike, both upgrades) — the
/// measurement that disproves "the certified families are the whole reachable
/// selecting-Sly set".
///
/// The assertion names the CAPABILITY rather than "some refusal": this fixture
/// also trips `master planner local Sly provenance` on every uncertified row
/// (they are not legitimate Master Planner targets), so a bare `is_err()` check
/// would pass on the pre-gate tree too and witness nothing. Measured: the bare
/// form passed on `16542be8`; this form fails there.
#[test]
fn singleton_tools_with_an_uncertified_selecting_sly_child_refuses_at_admission() {
    let doc = entry("HEADBUTT", 1);
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    let refusal = engine::admit(&doc, &state, &catalog)
        .expect_err("an uncertified selecting Sly child under live Tools must refuse");
    assert!(
        refusal.contains(MissingCapability::ContinuationFrame),
        "{refusal:?}"
    );
}

/// The non-replay branch decision, pinned rather than argued.
///
/// A rootless fight has no generic frame walk: `persisted_card_play_stack_is_exact`
/// routes a rootless stack to `restricted_frozen_auto_batch_stack_is_exact`,
/// whose Tools arm opens with
/// `has_replay_root != catalog.requires_action_replay() -> refuse` and which
/// additionally demands cursor one and `first_selector_excludes(Discard)`. So
/// the live singleton listener is judged in that branch by the legacy narrow
/// `invalid_child`, and a Discard-selecting family that the ROOTED slices
/// certified must still refuse there. `SHADOW_STEP` is what makes the fixture
/// replay-capable, so dropping it is what makes the fight rootless.
#[test]
fn a_rootless_singleton_tools_refuses_a_discard_selecting_child() {
    let mut doc = entry("HOLOGRAM", 1);
    doc.piles
        .get_mut("hand")
        .unwrap()
        .retain(|card| card.uid != Some(0));
    cold_refuses(&doc);

    // The same rootless document with a legacy-narrow child is unaffected: the
    // branch refuses the grammar, not the listener.
    let mut narrow = entry("PURITY", 1);
    narrow
        .piles
        .get_mut("hand")
        .unwrap()
        .retain(|card| card.uid != Some(0));
    reload_admitted(&narrow);
}

/// Cross-slice composition with the concurrent Misery lane (#2712, now on
/// `main`). A document that reaches admission through the Tools/Sly path AND
/// carries a malformed monster `power_attachments` row must still refuse on the
/// ledger, so neither lane's gate can mask the other's.
#[test]
fn a_tools_path_document_still_refuses_a_malformed_attachment_ledger() {
    let mut doc = entry("HOLOGRAM", 1);
    doc.monsters[0].insert(
        "power_attachments".to_owned(),
        json!([["weak", "player", 0, 0, 0]]),
    );
    cold_refuses(&doc);

    // A well-formed ledger is refused by the standing #2693 wall rather than
    // silently admitted, and the Tools path does not change that either.
    let mut populated = entry("HOLOGRAM", 1);
    populated.monsters[0].insert(
        "power_attachments".to_owned(),
        json!([["weak", "player", 0, 2, 0]]),
    );
    cold_refuses(&populated);
}

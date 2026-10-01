//! Plural Tools-of-the-Trade Sly batches are authenticated per child (#2671,
//! PR 2).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//!
//! `ToolsOfTheTradePower/<AfterPlayerTurnStart>d__5::MoveNext` RVA `0x349cb4`
//! builds ONE `CardSelectorPrefs(DiscardSelectionPrompt, Amount)`
//! (`IL_003c`-`IL_0053`) and awaits ONE `CardSelectCmd::FromHandForDiscard`
//! (`IL_005a`). `CardSelectorPrefs::.ctor` RVA `0x1397d4` delegates to
//! `0x1397f8`, which writes `MinSelect = MaxSelect = Amount`
//! (`IL_005c`-`IL_0065`), so the screen is one exactly-`Amount` multi-card
//! selection rather than `Amount` repeated single selections. The answer is
//! materialised once, in answer order, at `IL_00b9`; `IL_00bf`-`IL_00c7` ends
//! the listener when the answer is empty; `IL_00d0` awaits ONE plural
//! `CardCmd::Discard`.
//!
//! `CardCmd/<Discard>d__3::MoveNext` RVA `0x3e01ac` forwards at `IL_0023` to
//! `CardCmd::DiscardAndDraw(ctx, cards, 0)` RVA `0x3e0274`. Its discard loop is
//! ONE body per card: `IL_00f6` reads `CardModel::get_IsSlyThisTurn` for that
//! card, `IL_0109` appends it to `<slyCards>5__4`, `IL_011d` awaits
//! `CardPileCmd::Add`, `IL_018e` records `CombatHistory::CardDiscarded` and
//! `IL_01a5` awaits `Hook::AfterCardDiscarded`. Sly membership is therefore
//! captured PER CARD immediately before that card's own move, not batch-wide
//! before the loop. After the loop the frozen `<slyCards>5__4` enumerator
//! (`IL_02cc`-`IL_02d8`) awaits each `CardCmd::AutoPlay` (`IL_02fd`) before its
//! `MoveNext` (`IL_0360`-`IL_0365`): child *k*'s screen opens only once child
//! *k-1* has fully completed.
//!
//! `CardCmd/<AutoPlay>d__0::MoveNext` RVA `0x3df9d4` re-gates every entry on
//! `CombatManager::IsOverOrEnding` (`IL_0051`-`IL_005d`) and a dead owner
//! (`IL_006d`-`IL_007e`), so a terminal earlier child silently suppresses every
//! later sibling. `CardModel::get_IsSlyThisTurn` RVA `0x7cd5d` is
//! `Keywords.Contains(6) || HasSingleTurnSly`.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, Event, MissingCapability},
};

/// A replay-capable end-of-turn document. `I Am Invincible` forces the fight to
/// require an ActionReplay root WITHOUT being a Sly-discard card parent, so
/// `parent_specs` stays empty and the Tools listener is the only Sly-batch
/// source. The eight-card draw pile becomes a seven-card hand at turn start
/// (uids 10..=16), and `children` overwrites chosen draw slots with Sly rows.
fn entry(children: &[(u32, &str)], amount: i64) -> CanonicalStateV2 {
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
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":12},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":13},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":14},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":15},
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":16},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":17}
        ],
        "discard": [
            {"id":"STRIKE_IRONCLAD","upgrade":0,"uid":5},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":6}
        ]
    }))
    .unwrap();
    for (uid, id) in children.iter().copied() {
        set_child(&mut doc, uid, id, 0, true);
    }
    doc
}

/// Overwrite one draw slot, optionally marking it Sly.
fn set_child(doc: &mut CanonicalStateV2, uid: u32, id: &str, upgrade: i64, sly: bool) {
    let card = doc
        .piles
        .get_mut("draw")
        .unwrap()
        .iter_mut()
        .find(|card| card.uid == Some(u64::from(uid)))
        .unwrap_or_else(|| panic!("draw slot {uid} exists"));
    card.id = id.to_owned();
    card.upgrade = upgrade;
    card.local_keywords = if sly {
        vec!["Sly".to_owned()]
    } else {
        Vec::new()
    };
}

fn catalog_state(doc: &CanonicalStateV2) -> (sts_sim::catalog::Catalog, sts_sim::hot::HotState) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    (catalog, state)
}

fn cold_apply(doc: &CanonicalStateV2, action: Action) -> (CanonicalStateV2, Vec<Event>) {
    let (catalog, state) = catalog_state(doc);
    engine::admit(doc, &state, &catalog).unwrap();
    let mut events = Vec::new();
    let next = engine::apply_action_into(&state, &catalog, &action, &mut events).unwrap();
    (
        HotBoundary::try_to_canonical(&next, &catalog).unwrap(),
        events,
    )
}

/// Cold reload is admitted and byte-identical to the state the engine built.
fn reload_admitted(doc: &CanonicalStateV2) {
    let (catalog, state) = catalog_state(doc);
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

fn refusal(doc: &CanonicalStateV2) -> engine::AdmissionRefusal {
    let (catalog, state) = catalog_state(doc);
    engine::admit(doc, &state, &catalog).expect_err("document must refuse")
}

/// True when the document clears the Sly/Tools temporal wall specifically. The
/// wall is the only producer of `ContinuationFrame` in these fixtures, and
/// naming the capability keeps the sweep below from passing on an unrelated
/// refusal.
fn clears_the_sly_wall(doc: &CanonicalStateV2) -> bool {
    let Ok(catalog) = HotBoundary::catalog_from_canonical(doc) else {
        return false;
    };
    let Ok(state) = HotBoundary::from_canonical(doc, &catalog) else {
        return false;
    };
    match engine::admit(doc, &state, &catalog) {
        Ok(()) => true,
        Err(refusal) => !refusal.contains(MissingCapability::ContinuationFrame),
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

fn turns_began(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, Event::TurnBegan { .. }))
        .count()
}

/// The Tools discard answer that picks exactly `uids`, in that order.
fn tools_pick(doc: &CanonicalStateV2, uids: &[u32]) -> Action {
    let (catalog, state) = catalog_state(doc);
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
                == Some(uids)
        })
        .unwrap_or_else(|| panic!("no Tools answer picks {uids:?}"))
}

fn frame_types(doc: &CanonicalStateV2) -> Vec<String> {
    doc.continuations
        .iter()
        .map(|frame| frame.frame_type.clone())
        .collect()
}

fn batch_field(doc: &CanonicalStateV2, key: &str) -> serde_json::Value {
    doc.continuations
        .iter()
        .find(|frame| frame.frame_type == "FrozenAutoBatchFrame")
        .expect("a frozen batch frame")
        .fields
        .get(key)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
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

/// The parked child's own answer that selects exactly `uid`.
fn pick_uid(doc: &CanonicalStateV2, uid: u32) -> Action {
    let (catalog, state) = catalog_state(doc);
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
        .unwrap_or_else(|| panic!("no child answer picks {uid}"))
}

fn child_answer(doc: &CanonicalStateV2) -> Action {
    let (catalog, state) = catalog_state(doc);
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| matches!(action, Action::Select { .. }))
        .expect("the parked Sly child offers its own selection")
}

const PARKED_AGGREGATE: [&str; 4] = [
    "ActionReplayFrame",
    "TurnStartHandChoiceFrame",
    "FrozenAutoBatchFrame",
    "CardPlayFrame",
];

fn assert_parked_aggregate(doc: &CanonicalStateV2) {
    assert_eq!(frame_types(doc), PARKED_AGGREGATE.map(str::to_owned));
    assert!(doc.player.contains_key("pending"));
}

// ---------------------------------------------------------------------------
// 1. The batch executes as native does: one selection, ordered discard, strictly
//    sequential AutoPlay, one return to the listener chain.
// ---------------------------------------------------------------------------

/// Three selecting Sly siblings suspend at cursors 1, 2 and 3 in ANSWER order,
/// each pause is cold-reloadable against the state the engine built, and the
/// `AfterPlayerTurnStart` listener chain advances exactly once at the end.
///
/// The answer deliberately is not the hand order: picking `[16, 12, 14]` out of
/// a hand ordered `10..=16` witnesses `IL_00b9`'s "materialised in answer
/// order", and `FromHand`'s manual branch preserving `GetSelectedCards` order
/// verbatim.
#[test]
fn plural_tools_suspends_its_sly_siblings_at_each_cursor_in_answer_order() {
    let doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM"), (16, "HOLOGRAM")], 3);
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    reload_admitted(&choice);

    let (first, events) = cold_apply(&choice, tools_pick(&choice, &[16, 12, 14]));
    assert_eq!(
        played(&events),
        vec![16],
        "the answer's first Sly pick runs"
    );
    assert_parked_aggregate(&first);
    assert_eq!(batch_field(&first, "cursor"), json!(1));
    assert_eq!(uids(&first, "play"), vec![16]);
    reload_admitted(&first);

    let (second, events) = cold_apply(&first, child_answer(&first));
    assert_eq!(played(&events), vec![12], "the second sibling follows");
    assert_parked_aggregate(&second);
    assert_eq!(batch_field(&second, "cursor"), json!(2));
    reload_admitted(&second);

    let (third, events) = cold_apply(&second, child_answer(&second));
    assert_eq!(played(&events), vec![14], "the third sibling follows");
    assert_parked_aggregate(&third);
    assert_eq!(batch_field(&third, "cursor"), json!(3));
    reload_admitted(&third);

    let (done, events) = cold_apply(&third, child_answer(&third));
    assert!(done.continuations.is_empty(), "{done:?}");
    assert!(!done.player.contains_key("pending"));
    assert!(
        played(&events).is_empty(),
        "the last resume replays nothing"
    );
    assert_eq!(turns_began(&events), 1, "the listener chain returns once");
    // Every sibling exhausted itself exactly once, in answer order.
    assert_eq!(uids(&done, "exhaust"), vec![16, 12, 14]);
}

/// Selection ORDER is a distinct, exactly replayable outcome, not a set that
/// collapses. `CardSelectorPrefs` is one exactly-`Amount` selector and
/// `FromHand`'s manual answer is preserved verbatim, so both orders of the same
/// pair are legal and produce different trajectories.
#[test]
fn plural_tools_answer_orders_are_distinct_and_both_replayable() {
    let doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);

    let (catalog, state) = catalog_state(&choice);
    let answers: Vec<Vec<u32>> = engine::legal_actions(&state, &catalog)
        .into_iter()
        .filter_map(|action| {
            let Action::Select { answer } = action else {
                return None;
            };
            engine::selected_card_uids(&state, &catalog, answer)
                .ok()
                .flatten()
                .map(|uids| uids.to_vec())
        })
        .collect();
    // P(7, 2) ordered picks out of a seven-card hand, not C(7, 2) = 21.
    assert_eq!(answers.len(), 42, "{answers:?}");
    assert!(answers.contains(&vec![12, 14]));
    assert!(answers.contains(&vec![14, 12]));

    let (forward, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(played(&events), vec![12]);
    reload_admitted(&forward);
    let (reverse, events) = cold_apply(&choice, tools_pick(&choice, &[14, 12]));
    assert_eq!(played(&events), vec![14]);
    reload_admitted(&reverse);
    assert_ne!(
        forward, reverse,
        "the two orders must not collapse to one state"
    );

    // Both orders run to completion, mirrored.
    let finish = |mut doc: CanonicalStateV2, expected: [u32; 2]| {
        let (next, events) = cold_apply(&doc, child_answer(&doc));
        assert_eq!(played(&events), vec![expected[1]]);
        doc = next;
        let (done, _) = cold_apply(&doc, child_answer(&doc));
        assert!(done.continuations.is_empty());
        assert_eq!(uids(&done, "exhaust"), expected.to_vec());
    };
    finish(forward, [12, 14]);
    finish(reverse, [14, 12]);
}

/// A selecting child that is NOT first: the earlier sibling is a non-selecting
/// Sly card, which autoplays inline, and the later selecting sibling opens its
/// own screen only afterwards. Exactly-once play accounting for both.
#[test]
fn plural_tools_selecting_child_after_an_earlier_sibling_completes_once() {
    let mut doc = entry(&[(14, "HOLOGRAM")], 2);
    // An INTRINSICALLY Sly row (`spec.sly`) is an effective-Sly entry that never
    // suspends, and it needs no local keyword, so the earlier sibling is not
    // itself a Master Planner provenance question.
    set_child(&mut doc, 12, "ABRASIVE", 0, false);
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(
        played(&events),
        vec![12, 14],
        "the inline sibling resolves, then the selecting one opens its screen"
    );
    assert_parked_aggregate(&parked);
    assert_eq!(batch_field(&parked, "cursor"), json!(2));
    reload_admitted(&parked);

    let (done, events) = cold_apply(&parked, child_answer(&parked));
    assert!(done.continuations.is_empty());
    assert!(
        played(&events).is_empty(),
        "resuming the later sibling replays nothing"
    );
    assert_eq!(turns_began(&events), 1);
}

/// Sly membership is captured PER CARD inside the discard loop. A batch whose
/// picks mix Sly and non-Sly cards discards both and autoplays only the Sly
/// ones, in the answer's relative order — and the non-Sly pick is genuinely
/// discarded rather than skipped.
#[test]
fn plural_tools_captures_sly_membership_per_entry_in_answer_order() {
    let mut doc = entry(&[(12, "HOLOGRAM"), (16, "HOLOGRAM")], 3);
    set_child(&mut doc, 14, "DEFEND_IRONCLAD", 0, false);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);

    // Non-Sly card sandwiched between two Sly children.
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14, 16]));
    assert_eq!(played(&events), vec![12]);
    // All three picks left the hand; only the two Sly ones can autoplay.
    assert!(!uids(&parked, "hand").contains(&14));
    assert!(uids(&parked, "discard").contains(&14));
    reload_admitted(&parked);
    let (second, events) = cold_apply(&parked, child_answer(&parked));
    assert_eq!(
        played(&events),
        vec![16],
        "the non-Sly pick is not autoplayed"
    );
    let (done, _) = cold_apply(&second, child_answer(&second));
    assert!(done.continuations.is_empty());
    assert_eq!(uids(&done, "exhaust"), vec![12, 16]);

    // Zero Sly among the picks: the listener discards and returns with no batch.
    let mut inert = entry(&[], 2);
    set_child(&mut inert, 12, "DEFEND_IRONCLAD", 0, false);
    set_child(&mut inert, 14, "DEFEND_IRONCLAD", 0, false);
    let (choice, _) = cold_apply(&inert, Action::EndTurn);
    let (done, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert!(done.continuations.is_empty(), "{done:?}");
    assert!(played(&events).is_empty());
    assert_eq!(turns_began(&events), 1);
    assert!(uids(&done, "discard").contains(&12));
    assert!(uids(&done, "discard").contains(&14));
}

// ---------------------------------------------------------------------------
// 2. Ending the fight mid-batch: the per-entry `AutoPlay` gate and the per-card
//    `AfterCardDiscarded` hook.
// ---------------------------------------------------------------------------

/// `CardCmd/<AutoPlay>d__0::MoveNext` RVA `0x3df9d4` re-checks
/// `CombatManager::IsOverOrEnding` at `IL_0051`-`IL_005d` for EVERY entry of
/// the frozen Sly list, so a terminal earlier child silently suppresses each
/// later sibling. `RICOCHET` is intrinsically Sly (`spec.sly`) and lethal here.
#[test]
fn plural_tools_terminal_child_suppresses_every_later_sibling() {
    let mut doc = entry(&[(14, "HOLOGRAM")], 2);
    set_child(&mut doc, 12, "RICOCHET", 0, false);
    doc.monsters[0].insert("hp".to_owned(), json!(1));
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (done, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(
        played(&events),
        vec![12],
        "the later Sly sibling must not play once the fight is over"
    );
    assert!(
        done.continuations.is_empty(),
        "no aggregate is parked: {done:?}"
    );
    assert!(!done.player.contains_key("pending"));
    // The suppressed sibling is still where the discard left it, not exhausted.
    assert!(!uids(&done, "exhaust").contains(&14));

    // The identical document with a live monster runs both children, so the
    // suppression above is attributable to the ending, not to the fixture.
    let mut alive = entry(&[(14, "HOLOGRAM")], 2);
    set_child(&mut alive, 12, "RICOCHET", 0, false);
    let (choice, _) = cold_apply(&alive, Action::EndTurn);
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(played(&events), vec![12, 14]);
    assert_parked_aggregate(&parked);
}

/// The whole point of the per-card capture: `Hook::AfterCardDiscarded`
/// (`IL_01a5`) runs BETWEEN two entries' `IL_00f6` Sly reads, and one of the two
/// listeners that exist on this build can end the fight from inside it.
///
/// Native census on the archived DLL: exactly four `AfterCardDiscarded` methods
/// exist — `AbstractModel` RVA `0x79fc2` (`return Task.CompletedTask`),
/// `Tingsha` RVA `0x9c82c` (async body `Tingsha/<AfterCardDiscarded>d__4`
/// `0x3326e0`: `CombatTargets.NextItem<Creature>` at `IL_0094`, then
/// `CreatureCmd::Damage` at `IL_00c8`), `ToughBandages` RVA `0x9cd64` (async
/// body `0x332e2c`: `CreatureCmd::GainBlock` at `IL_0082`), and the dispatcher
/// `Hook::AfterCardDiscarded` RVA `0x10337c`. NEITHER listener writes
/// `CardModel::Keywords` or `_hasSingleTurnSly`, so no `AfterCardDiscarded`
/// body can grant or remove a later entry's Sly. Liveness is the one thing it
/// can change: Tingsha's damage can end the fight, after which
/// `Hook/<IterateCombatHookListeners>d__0::MoveNext` RVA `0x3d3bc0`
/// `IL_0028`-`IL_0042` yields zero listeners for every remaining entry.
#[test]
fn plural_tools_after_card_discarded_hook_can_end_the_batch_between_entries() {
    let mut doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    doc.player.insert(
        "relics_entering".to_owned(),
        json!(["RELIC.BURNING_BLOOD", "RELIC.TINGSHA"]),
    );
    doc.player.insert("tingsha".to_owned(), json!(true));
    doc.monsters[0].insert("hp".to_owned(), json!(3));
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (done, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert!(
        played(&events).is_empty(),
        "the fight ended inside the discard loop, before any AutoPlay: {:?}",
        played(&events)
    );
    assert!(done.continuations.is_empty(), "{done:?}");
    // The first pick moved and fired the hook; the second never moved.
    assert!(uids(&done, "discard").contains(&12));
    assert!(uids(&done, "hand").contains(&14));

    // With more monster hp the same relic does not end the fight, and the batch
    // runs normally — so the assertions above are about the ending.
    let mut survives = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    survives.player.insert(
        "relics_entering".to_owned(),
        json!(["RELIC.BURNING_BLOOD", "RELIC.TINGSHA"]),
    );
    survives.player.insert("tingsha".to_owned(), json!(true));
    let (choice, _) = cold_apply(&survives, Action::EndTurn);
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(played(&events), vec![12]);
    assert_parked_aggregate(&parked);
    reload_admitted(&parked);
}

// ---------------------------------------------------------------------------
// 3. Batch membership is frozen: a child moving a sibling does not change who
//    plays, or how often.
// ---------------------------------------------------------------------------

/// `<slyCards>5__4` holds object references captured at `IL_0109`, and
/// `CardCmd::AutoPlay` at `IL_02fd` replays them with no re-check of pile or
/// Sly. So a `Hologram` child that pulls the NEXT sibling out of the shared
/// discard pile does not remove it from the batch: it still plays exactly once,
/// at its own cursor, now from Hand.
#[test]
fn plural_tools_hologram_retrieving_a_later_sibling_still_plays_it_once() {
    let doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, _) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));

    // Both picks are already in Discard when the first child's screen opens.
    assert!(uids(&parked, "discard").contains(&14));
    let retrieve_later = pick_uid(&parked, 14);
    let (mid, events) = cold_apply(&parked, retrieve_later);
    assert!(
        !uids(&mid, "discard").contains(&14),
        "the later sibling left Discard for Hand"
    );
    assert_eq!(
        played(&events),
        vec![14],
        "and it still plays exactly once, at its own cursor"
    );
    assert_eq!(
        uids(&mid, "play"),
        vec![14],
        "the retrieved sibling is the active child"
    );
    assert_parked_aggregate(&mid);
    assert_eq!(batch_field(&mid, "cursor"), json!(2));
    reload_admitted(&mid);

    let (done, events) = cold_apply(&mid, child_answer(&mid));
    assert!(done.continuations.is_empty());
    assert!(played(&events).is_empty(), "no extra play on the way out");
    assert_eq!(turns_began(&events), 1);
    assert_eq!(uids(&done, "exhaust"), vec![12, 14]);
}

/// The mirror case: retrieving an ALREADY COMPLETED earlier sibling adds no
/// play. The batch cursor is past it, so it simply sits in Hand.
#[test]
fn plural_tools_retrieving_a_completed_earlier_sibling_adds_no_extra_play() {
    let mut doc = entry(&[(14, "HOLOGRAM")], 2);
    // `TACTICIAN` is intrinsically Sly and its natural result route is Discard,
    // so the later Hologram sibling can see it on its own retrieval screen.
    set_child(&mut doc, 12, "TACTICIAN", 0, false);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    assert_eq!(played(&events), vec![12, 14]);
    assert_eq!(batch_field(&parked, "cursor"), json!(2));
    assert!(uids(&parked, "discard").contains(&12));

    let (done, events) = cold_apply(&parked, pick_uid(&parked, 12));
    assert!(
        played(&events).is_empty(),
        "the completed sibling must not play again: {:?}",
        played(&events)
    );
    assert!(uids(&done, "hand").contains(&12));
    assert!(done.continuations.is_empty());
    assert_eq!(turns_began(&events), 1);
}

// ---------------------------------------------------------------------------
// 4. The listener chain.
// ---------------------------------------------------------------------------

/// "Return to the Tools power suffix" is the `AfterPlayerTurnStart` listener
/// chain: there is no body suffix after the Discard await (`0x349cb4` returns at
/// `IL_012a`). A later Tyranny listener opens exactly once, after the WHOLE
/// plural batch, and the Tools listener never re-fires.
#[test]
fn plural_tools_returns_to_the_listener_chain_once_without_repeating_earlier_listeners() {
    let mut doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    doc.player.insert("tyranny".to_owned(), json!(1));
    doc.player.insert(
        "after_player_turn_start_power_order".to_owned(),
        json!(["tools_of_the_trade", "tyranny"]),
    );
    reload_admitted(&doc);

    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (first, _) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
    let (second, _) = cold_apply(&first, child_answer(&first));
    assert_eq!(batch_field(&second, "cursor"), json!(2));
    let (after_batch, events) = cold_apply(&second, child_answer(&second));

    // Tyranny's own screen is the next thing that happens, exactly once, and
    // the turn has not begun yet because the chain is not finished.
    assert_eq!(turns_began(&events), 0, "the chain is not done yet");
    assert_eq!(
        frame_types(&after_batch),
        vec!["ActionReplayFrame".to_owned()],
        "the whole Tools aggregate popped"
    );
    let pending = after_batch
        .player
        .get("pending")
        .and_then(|value| value.as_array())
        .expect("a pending listener selection")
        .clone();
    assert_eq!(pending[0], json!("turn_start_hand_choice_select"));
    assert_eq!(
        pending[2],
        json!("tyranny"),
        "the NEXT listener opened, not a repeat of Tools"
    );
    assert_eq!(
        pending[4],
        json!(2),
        "the listener cursor advanced by exactly one"
    );
    reload_admitted(&after_batch);

    let answer = child_answer(&after_batch);
    let (done, events) = cold_apply(&after_batch, answer);
    assert_eq!(
        turns_began(&events),
        1,
        "the chain finishes once, with no second Tools listener"
    );
    assert!(done.continuations.is_empty());
}

// ---------------------------------------------------------------------------
// 5. What still refuses, and what the dropped disjunct was protecting.
// ---------------------------------------------------------------------------

/// Review target 4: anything the batch can reach that is not exact refuses at
/// ADMISSION, not mid-play. `HEADBUTT` is one of the ten reachable selecting Sly
/// rows outside `replay_child_program_is_exact`; a plural batch that could
/// contain it never becomes a root.
#[test]
fn plural_tools_still_refuses_an_uncertified_sly_child_at_admission() {
    for children in [
        [(12, "HEADBUTT"), (14, "HOLOGRAM")],
        [(12, "HOLOGRAM"), (14, "HEADBUTT")],
    ] {
        let doc = entry(&children, 2);
        assert!(
            refusal(&doc).contains(MissingCapability::ContinuationFrame),
            "{children:?} escaped the narrowed wall"
        );
    }
    // Attribution control: the same pair with both children certified admits.
    let certified = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    reload_admitted(&certified);
}

/// Review target 1. Dropping the `tools_plural_parent_reachable && (...)`
/// disjunct also drops `future_generated_play_count_exceeds_one` and
/// `future_restricted_batch_result_route_changes` for Tools-owned children —
/// they only ever reached the replay branch through `invalid_child`.
///
/// The justification is uniformity, and it is checkable two ways. The replay
/// branch ALREADY omits both for every card-row Sly parent (they sit behind
/// `tools_plural_parent_reachable` and nothing else), and
/// `unsupported_direct_draw_frozen_batch_future_is_reachable` — the sibling wall
/// that does apply them — is skipped wholesale when
/// `catalog.requires_action_replay()`. In both cases the reason is the same: the
/// generic rooted driver persists a real per-frame stack, so replay counts and
/// result routes are represented rather than quotiented.
///
/// This makes the omission non-vacuous for Tools: every writer that would have
/// set one of the two predicates is reachable here, and the plural batch parks,
/// cold-reloads and completes with both children played exactly once.
#[test]
fn plural_tools_child_replay_and_result_route_writers_are_owned_by_the_driver() {
    for writer in [
        // future_generated_play_count_exceeds_one
        "BURST",
        "ECHO_FORM",
        "ONE_TWO_PUNCH",
        "SIGNAL_BOOST",
        // future_restricted_batch_result_route_changes
        "REBOUND",
        "NOSTALGIA",
        "CORRUPTION",
        "FERAL",
    ] {
        let mut doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
        set_child(&mut doc, 16, writer, 0, false);
        assert!(
            clears_the_sly_wall(&doc),
            "{writer} re-walled the certified plural batch"
        );
        let (choice, _) = cold_apply(&doc, Action::EndTurn);
        let (parked, events) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));
        assert_eq!(played(&events), vec![12], "{writer}");
        reload_admitted(&parked);
        let (mid, events) = cold_apply(&parked, child_answer(&parked));
        assert_eq!(played(&events), vec![14], "{writer}");
        reload_admitted(&mid);
        let (done, _) = cold_apply(&mid, child_answer(&mid));
        assert!(done.continuations.is_empty(), "{writer}: {done:?}");
        assert_eq!(uids(&done, "exhaust"), vec![12, 14], "{writer}");
    }
}

/// Every family the sibling slices certified survives being a plural sibling, at
/// both upgrades, paired with a different certified family. The assertion is the
/// absence of the Sly/Tools capability rather than blanket admission, because
/// some rows need document state this minimal fixture deliberately does not
/// carry (`DISCOVERY`/`QUASAR` refuse on `Discovery generation provenance`,
/// a different gate). The uncertified control above asserts the positive side of
/// the same capability, so the pair is not vacuous.
#[test]
fn plural_tools_admits_every_supported_child_family_at_both_upgrades() {
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
        for upgrade in [0, 1] {
            let mut doc = entry(&[(12, "HOLOGRAM")], 2);
            set_child(&mut doc, 14, id, upgrade, true);
            assert!(
                clears_the_sly_wall(&doc),
                "{id}+{upgrade} was caught by the Sly/Tools wall as a plural sibling"
            );
        }
    }
}

/// Forged ownership on a parked plural aggregate refuses atomically, and the
/// original document is untouched by every refusal.
#[test]
fn plural_tools_refuses_forged_cursor_order_and_substitution_atomically() {
    let doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, _) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));

    let batch_frame = |doc: &mut CanonicalStateV2, key: &str, value: serde_json::Value| {
        doc.continuations
            .iter_mut()
            .find(|frame| frame.frame_type == "FrozenAutoBatchFrame")
            .unwrap()
            .fields
            .insert(key.to_owned(), value);
    };

    // The cursor names the wrong child.
    let mut forged_cursor = parked.clone();
    batch_frame(&mut forged_cursor, "cursor", json!(2));
    cold_refuses(&forged_cursor);

    // Past the end of the batch.
    let mut overrun = parked.clone();
    batch_frame(&mut overrun, "cursor", json!(3));
    cold_refuses(&overrun);

    // The frozen entries are reordered, so `cursor - 1` no longer names the
    // live CardPlay.
    let mut reordered = parked.clone();
    let entries = batch_field(&parked, "entries");
    let mut rows = entries.as_array().cloned().expect("batch entries");
    rows.reverse();
    batch_frame(&mut reordered, "entries", serde_json::Value::Array(rows));
    cold_refuses(&reordered);

    // The batch source is forged away from SlyDiscard.
    let mut forged_source = parked.clone();
    batch_frame(&mut forged_source, "source", json!("Draw pile flip"));
    cold_refuses(&forged_source);

    // The replay root is removed.
    let mut orphan = parked.clone();
    assert_eq!(
        orphan.continuations.remove(0).frame_type,
        "ActionReplayFrame"
    );
    cold_refuses(&orphan);

    // The active child's physical card vanishes.
    let mut vanished = parked.clone();
    vanished
        .piles
        .get_mut("play")
        .unwrap()
        .retain(|card| card.uid != Some(12));
    cold_refuses(&vanished);

    // The later sibling — a frozen batch entry that has not run yet — vanishes.
    let mut vanished_sibling = parked.clone();
    vanished_sibling
        .piles
        .get_mut("discard")
        .unwrap()
        .retain(|card| card.uid != Some(14));
    cold_refuses(&vanished_sibling);

    reload_admitted(&parked);
}

/// The frozen-parent snapshot is the authority for batch membership, and the
/// producer reads Sly live per entry. The two cannot disagree on this build (no
/// `AfterCardDiscarded` listener writes Sly — see the census above), but the
/// comparison is what makes that fail-closed rather than assumed: forging the
/// parent's frozen Sly marker so `captured_sly != batch.entries()` refuses.
#[test]
fn plural_tools_batch_disagreeing_with_the_frozen_parent_snapshot_fails_closed() {
    let doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    let (choice, _) = cold_apply(&doc, Action::EndTurn);
    let (parked, _) = cold_apply(&choice, tools_pick(&choice, &[12, 14]));

    let mut forged = parked.clone();
    let parent = forged
        .continuations
        .iter_mut()
        .find(|frame| frame.frame_type == "TurnStartHandChoiceFrame")
        .expect("the Tools parent frame");
    let mut rows = parent
        .fields
        .get("selected")
        .and_then(|value| value.as_array().cloned())
        .expect("frozen parent selection");
    // Drop the later sibling from the parent's frozen entry list while leaving
    // it in the batch: `captured_sly` shrinks, the batch does not.
    rows.truncate(1);
    parent
        .fields
        .insert("selected".to_owned(), serde_json::Value::Array(rows));
    cold_refuses(&forged);

    reload_admitted(&parked);
}

/// A rootless fight has no generic frame walk, so the legacy rootless
/// restrictions still bite: plural Tools without an `ActionReplay` root refuses.
#[test]
fn rootless_plural_tools_remains_refused() {
    let mut doc = entry(&[(12, "HOLOGRAM"), (14, "HOLOGRAM")], 2);
    doc.piles
        .get_mut("hand")
        .unwrap()
        .retain(|card| card.uid != Some(0));
    cold_refuses(&doc);
}

/// Native `FromHand` auto-answers when the filtered hand is no larger than
/// `MinSelect` (`0x3e7568` `IL_0153`-`IL_017f`), and `IL_00bf`-`IL_00c7` of the
/// Tools listener ends it outright on an empty answer. Neither opens a screen,
/// and neither is changed by this slice.
#[test]
fn tools_hand_at_or_below_amount_auto_answers_without_a_screen() {
    // Amount well above the hand: the whole hand is the answer, no screen opens,
    // and the plural batch runs straight through from the auto answer.
    let mut wide = entry(&[(12, "HOLOGRAM"), (10, "HOLOGRAM")], 9);
    wide.piles.get_mut("draw").unwrap().truncate(3);
    let (parked, events) = cold_apply(&wide, Action::EndTurn);
    assert_eq!(
        played(&events),
        vec![10],
        "the auto-answered batch autoplays in hand order"
    );
    assert_parked_aggregate(&parked);
    assert_eq!(batch_field(&parked, "cursor"), json!(1));
    // The pending selection belongs to the CHILD, not to a Tools screen.
    let pending = parked
        .player
        .get("pending")
        .and_then(|value| value.as_array())
        .expect("a pending child selection");
    assert_ne!(pending[0], json!("turn_start_hand_choice_select"));
    reload_admitted(&parked);

    let (second, events) = cold_apply(&parked, child_answer(&parked));
    assert_eq!(played(&events), vec![12]);
    let (done, _) = cold_apply(&second, child_answer(&second));
    assert!(done.continuations.is_empty());
    assert_eq!(uids(&done, "exhaust"), vec![10, 12]);
}

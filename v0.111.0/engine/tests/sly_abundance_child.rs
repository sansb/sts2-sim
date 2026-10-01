//! Abundance / Abundance+ as a rooted Sly generated-offer child (#2715).
//!
//! Native authority is `sts2.dll` v0.111.0, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//!
//! `Abundance/<OnPlay>d__6::MoveNext` RVA `0x3888ec` is
//! `Discovery/<OnPlay>d__4::MoveNext` RVA `0x399254` with three local edits.
//! Both read `CardPoolModel::GetUnlockedCards(unlockState,
//! CardMultiplayerConstraint)` (Abundance `IL_0027`-`IL_005d`, Discovery
//! `IL_0033`-`IL_0063`), push the literal `3` (Abundance `IL_0081`, Discovery
//! `IL_0068`), draw through `CardFactory::GetDistinctForCombat` RVA `0x112878`
//! off `RunRngSet::get_CombatCardGeneration` (Abundance `IL_0097`, Discovery
//! `IL_007e`), tolerate a null answer without minting (Abundance `IL_014b`,
//! Discovery `IL_0103`), and finish with `CardModel::SetToFreeThisTurn` plus
//! `CardPileCmd::AddGeneratedCardToCombat(card, 2 /* Hand */, owner,
//! 1 /* bottom */)` RVA `0x1305a0` (Abundance `IL_014e`-`IL_015c`, Discovery
//! `IL_0106`-`IL_0114`).
//!
//! The three edits: a Power filter (`Where` at `IL_005d`-`IL_007c`, predicate
//! `Abundance/<>c::<OnPlay>b__6_0` RVA `0x3888de`, `get_Type` `ceq` 3); every
//! offer upgraded once (`CardCmd::Upgrade` at `IL_00b8` inside the
//! `IL_00a2`-`IL_00c4` foreach); and `canSkip = 0` at `IL_00e7` where
//! Discovery passes `1` at `IL_0096`. In
//! `CardSelectCmd/<FromChooseACardScreen>d__16::MoveNext` RVA `0x3e58c0` that
//! argument is read once, at `IL_0224`, and only to hand to
//! `NChooseACardSelectionScreen::ShowScreen` (`IL_0229`).
//!
//! None of it is source-sensitive: `CardCmd/<AutoPlay>d__0::MoveNext` RVA
//! `0x3df9d4` moves the card to the Play pile (`CardPileCmd::Add` at
//! `IL_04b7`) and then enters the very same `CardModel::OnPlayWrapper`
//! (`IL_062a`) with the same `choiceContext` a manual play uses. An Abundance
//! auto-played out of a Sly discard opens the identical screen and takes the
//! identical result route.
//!
//! `Abundance::OnUpgrade` RVA `0xd7480` only calls
//! `CardEnergyCost::UpgradeBy(-1)`, so L0 and L1 share one body, and
//! `Abundance::get_CanBeGeneratedInCombat` RVA `0xd7475` returns false, so the
//! Power pool cannot contain Abundance itself.
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, MissingCapability},
};

/// The rows this build leaves uncertified once Abundance is certified: the
/// residue of PR #2713's 154-certified / 10-uncertified selecting-Sly census.
/// Scope item 4 of #2715 — they must stay refused everywhere Abundance is now
/// admitted, or the widening is not the one that was reviewed.
const STILL_UNCERTIFIED: [&str; 4] = ["GRAVEBLAST", "HEADBUTT", "NEOWS_FURY", "SEEKER_STRIKE"];

/// A replay-capable end-of-turn document, the shape `tools_sly_routing.rs` and
/// `tools_plural_sly.rs` already use. `I Am Invincible` forces the fight to
/// require an ActionReplay root WITHOUT itself being a Sly-discard card
/// parent, so `parent_specs` stays empty and the Tools listener is the only
/// Sly-batch source. The eight-card draw pile becomes the next hand.
fn entry(children: &[(u32, &str, i64)], amount: i64) -> CanonicalStateV2 {
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
    for (uid, id, upgrade) in children.iter().copied() {
        set_child(&mut doc, uid, id, upgrade, true);
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

/// Assert the Sly/frozen-batch wall does not fire.
///
/// The assertion names the CAPABILITY rather than blanket admission, exactly
/// as `tools_sly_routing.rs::singleton_tools_with_each_certified_child_family_clears_the_sly_wall`
/// does: this minimal fixture deliberately does not carry the document state
/// Abundance's own generation-provenance gate wants, and that is a different
/// gate and not this slice's business. The `STILL_UNCERTIFIED` controls below
/// assert the positive side of the same capability on the same fixtures, so
/// the pair is not vacuous.
#[track_caller]
fn clears_the_sly_wall(doc: &CanonicalStateV2, what: &str) {
    let catalog = HotBoundary::catalog_from_canonical(doc)
        .unwrap_or_else(|error| panic!("{what}: catalog refused: {error:?}"));
    let state = HotBoundary::from_canonical(doc, &catalog)
        .unwrap_or_else(|error| panic!("{what}: hydration refused: {error:?}"));
    if let Err(refusal) = engine::admit(doc, &state, &catalog) {
        assert!(
            !refusal.contains(MissingCapability::ContinuationFrame),
            "{what}: caught by the Sly frozen-batch wall: {refusal:?}"
        );
    }
}

#[track_caller]
fn hits_the_sly_wall(doc: &CanonicalStateV2, what: &str) {
    let catalog = HotBoundary::catalog_from_canonical(doc)
        .unwrap_or_else(|error| panic!("{what}: catalog refused: {error:?}"));
    let state = HotBoundary::from_canonical(doc, &catalog)
        .unwrap_or_else(|error| panic!("{what}: hydration refused: {error:?}"));
    let Err(refusal) = engine::admit(doc, &state, &catalog) else {
        panic!("{what}: must not admit");
    };
    assert!(
        refusal.contains(MissingCapability::ContinuationFrame),
        "{what}: must refuse for ContinuationFrame specifically, got {refusal:?}"
    );
}

/// **The measured wall.** This is the shape the 2026-09-22 EA6 frontier
/// measurement found on floors 30/31/35/38/39/40 with the Entropy draft
/// applied: `invalid_future_child` fired because `Abundance`/`Abundance+` was
/// the ONE reachable spec satisfying `hand_trick_child` and failing
/// `replay_child_program_is_exact`. Hand Trick applies single-turn Sly through
/// `CardCmd::ApplySingleTurnSly` (`0x12fe38` `IL_000d`), so a reachable Hand
/// Trick plus a reachable Abundance is a future Sly writer/child pair even
/// when nothing is Sly right now.
///
/// MUTATION CONTROL: on `origin/main` this test fails at both levels —
/// `rooted_sly_generated_offer_is_exact` is a three-way `matches!` that omits
/// `AbundanceExact`, so `invalid_future_child` holds and admission returns
/// `ContinuationFrame`.
#[test]
fn abundance_as_a_reachable_hand_trick_child_clears_the_sly_wall() {
    for upgrade in [0, 1] {
        // Nothing is Sly here. The wall is a *reachability* claim: Hand Trick
        // can make the Abundance Sly later, and a live Tools listener can then
        // freeze it into a batch.
        let mut doc = entry(&[], 1);
        set_child(&mut doc, 12, "HAND_TRICK", 0, false);
        set_child(&mut doc, 13, "ABUNDANCE", upgrade, false);
        clears_the_sly_wall(&doc, &format!("Abundance+{upgrade} as a Hand Trick child"));
    }

    // There is deliberately no uncertified control for THIS arm. The four
    // rows in `STILL_UNCERTIFIED` are all Attacks (`is_skill: false`) and both
    // future-child arms require `is_skill`, so none of them can be a Hand
    // Trick child — which is precisely why Abundance was the unique offender
    // the frontier measurement found. That fact is pinned derivedly by
    // `admission.rs::every_reachable_future_sly_child_is_certified`; the
    // controls for these four live on the current-Sly fixtures below, where
    // they are reachable and do fire.
}

/// Abundance as the CURRENT local-Sly child of a live singleton Tools
/// listener — the #2713 routing fixture, with Abundance in the Sly slot. Here
/// the wall is `invalid_current_program`, the other half of the predicate.
///
/// MUTATION CONTROL: fails on `origin/main` at both levels.
#[test]
fn abundance_as_a_live_singleton_tools_child_clears_the_sly_wall() {
    for upgrade in [0, 1] {
        let doc = entry(&[(12, "ABUNDANCE", upgrade)], 1);
        clears_the_sly_wall(
            &doc,
            &format!("Abundance+{upgrade} under a live singleton Tools listener"),
        );
    }
    for id in STILL_UNCERTIFIED {
        let doc = entry(&[(12, id, 0)], 1);
        hits_the_sly_wall(&doc, &format!("{id} under a live singleton Tools listener"));
    }
}

/// Abundance beside a second certified child under a PLURAL Tools batch — the
/// composition #2716 made admissible, which #2715's issue body asks for by
/// name. Both children are current local-Sly rows in the same frozen batch, so
/// `selecting_sly` has two members and `invalid_current_program` measures each
/// of them.
///
/// MUTATION CONTROL: fails on `origin/main` for every pairing.
#[test]
fn plural_tools_admits_abundance_beside_a_second_certified_child() {
    for sibling in ["HOLOGRAM", "DREDGE", "SPLASH", "ABUNDANCE"] {
        for upgrade in [0, 1] {
            let doc = entry(&[(12, "ABUNDANCE", upgrade), (13, sibling, 0)], 2);
            clears_the_sly_wall(
                &doc,
                &format!("plural Tools: Abundance+{upgrade} beside {sibling}"),
            );
        }
    }
    // One uncertified sibling still refuses the whole plural batch: the
    // predicate is per-child, so certifying Abundance must not launder the
    // card next to it.
    for id in STILL_UNCERTIFIED {
        let doc = entry(&[(12, "ABUNDANCE", 0), (13, id, 0)], 2);
        hits_the_sly_wall(&doc, &format!("plural Tools: Abundance beside {id}"));
    }
}

/// The non-Tools regression witness — folded-in item (5) of #2715.
///
/// Every fixture above rests on a Tools listener, so all of them reach the
/// wall through `tools_parent_live`/`tools_plural_parent_reachable`. This one
/// deletes the listener and the power entirely and puts a CARD-ROW Sly parent
/// (Hidden Daggers) in hand instead, so `parent_specs` is what defeats
/// `unsupported_sly_frozen_batch_future_is_reachable`'s early `return false`.
///
/// The Hidden Daggers parent matters for a second reason: without it this
/// fixture takes that early return and the test is vacuous. It was, in its
/// first draft — it passed under the mutation control, which is what caught
/// it. The `hits_the_sly_wall` control below is what keeps it honest, and the
/// fuller non-Tools coverage is the rooted Hidden Daggers battery in
/// `admission.rs`, which carries no Tools row at all.
#[test]
fn abundance_clears_the_sly_wall_with_no_tools_listener_in_the_fight() {
    fn card_row_parent_fixture(child: &str, upgrade: i64) -> CanonicalStateV2 {
        let mut doc = entry(&[(12, child, upgrade)], 0);
        doc.player.remove("tools_of_the_trade");
        doc.player.remove("after_player_turn_start_power_order");
        // A card-row discard-to-Sly parent, in hand, is now the only Sly-batch
        // source in the fight.
        doc.piles.get_mut("hand").unwrap()[1].id = "HIDDEN_DAGGERS".to_owned();
        doc
    }

    for upgrade in [0, 1] {
        clears_the_sly_wall(
            &card_row_parent_fixture("ABUNDANCE", upgrade),
            &format!("Abundance+{upgrade} under a card-row parent, no Tools in the fight"),
        );
    }
    for id in STILL_UNCERTIFIED {
        hits_the_sly_wall(
            &card_row_parent_fixture(id, 0),
            &format!("{id} under a card-row parent, no Tools in the fight"),
        );
    }
}

/// Abundance prints `exhausts: true` at BOTH levels, which is why Master
/// Planner cannot make it a *future* Sly child.
///
/// `future_master_planner_selecting_sly_child_is_reachable` requires
/// `!spec.exhausts`: Master Planner writes local Sly in `AfterCardPlayed`
/// (`MasterPlannerPower::AfterCardPlayed` RVA `0xa492c` `IL_0052`), before
/// result routing, so a printed-Exhaust Skill takes the marker and then leaves
/// the Hand/Draw/Discard domain permanently — no discard-to-Sly parent can
/// ever collect it. Abundance therefore reaches the wall through the Hand
/// Trick arm only, and the form in which Master Planner does matter for it is
/// an ALREADY-live local-Sly marker with the power live, which is the second
/// half of this test.
///
/// Pinned rather than argued, because if a future build makes Abundance
/// non-exhausting the Master Planner arm silently acquires a new child and the
/// reasoning in `#2715`'s walk stops holding.
#[test]
fn abundance_exhausts_so_master_planner_owns_it_only_as_a_live_marker() {
    for upgrade in [0, 1] {
        let row = sts_sim::content_tables::card_row(sts_sim::ids::CardId::Abundance, upgrade)
            .expect("Abundance has a row at both levels");
        assert!(
            row.exhausts,
            "Abundance+{upgrade} must print Exhaust; if it stops, \
             `future_master_planner_selecting_sly_child_is_reachable` gains it as a \
             future child and #2715's Hand-Trick-only reachability claim is stale"
        );
        assert!(
            row.selects && row.is_skill && row.playable,
            "Abundance+{upgrade} must stay a playable selecting Skill"
        );
    }

    // The live-marker form: Master Planner is live (the fixture sets it), the
    // Abundance already carries local Sly, and the batch is published by the
    // Tools listener the marker fed.
    for upgrade in [0, 1] {
        let doc = entry(&[(12, "ABUNDANCE", upgrade)], 1);
        assert_eq!(
            doc.player.get("master_planner").and_then(|v| v.as_i64()),
            Some(1),
            "the fixture must keep Master Planner live or this witness is mislabelled"
        );
        clears_the_sly_wall(
            &doc,
            &format!("Abundance+{upgrade} as a live Master-Planner-marked child"),
        );
    }
}

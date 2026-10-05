//! EA6YVY5X1QM1: real captured starts and completed native action checkpoints.
use serde_json::{Value, json};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::{boundary::HotBoundary, canonical::CanonicalStateV2, engine};

#[test]
fn early_floors_match_recorded_native_actions_with_clash_and_parse() {
    for (root, steps, checkpoints, final_hp) in [
        (
            include_str!("../../eval/search/ea6-admission-v1/floor6.canonical.json"),
            include_str!("../../eval/search/ea6-admission-v1/floor6-recorded-line.json"),
            13,
            36,
        ),
        (
            include_str!("../../eval/search/ea6-admission-v1/floor7.canonical.json"),
            include_str!("../../eval/search/ea6-admission-v1/floor7-recorded-line.json"),
            9,
            29,
        ),
    ] {
        let entry: CanonicalStateV2 = serde_json::from_str(root).unwrap();
        let steps: Vec<Value> = serde_json::from_str(steps).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
        let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
        engine::admit(&entry, &state, &catalog).unwrap();
        let mut checked = 0;
        for (i, step) in steps.iter().enumerate() {
            let wire: ExactSolveActionV1 = serde_json::from_value(step["action"].clone()).unwrap();
            let action: engine::Action = wire.try_into().unwrap();
            state = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap();
            let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(
                projected.differential_digest(),
                step["digest_after"],
                "step {i}"
            );
            let reload_catalog = HotBoundary::catalog_from_canonical(&projected).unwrap();
            let restored = HotBoundary::from_canonical(&projected, &reload_catalog).unwrap();
            assert_eq!(
                HotBoundary::try_to_canonical(&restored, &reload_catalog).unwrap(),
                projected
            );
            if let Some(native) = step.get("native") {
                checked += 1;
                assert_eq!(
                    json!({"hp":state.hp,"max_hp":state.max_hp,"block":state.block,
                    "energy":state.energy,"turn":state.turn}),
                    native["player"],
                    "step {i}"
                );
                let monsters: Vec<_> = state
                    .monsters
                    .iter()
                    .filter(|m| m.hp > 0)
                    .map(|m| json!([m.kind.as_str(), m.hp, m.max_hp, m.block]))
                    .collect();
                assert_eq!(json!(monsters), native["monsters"], "step {i}");
                let doc = serde_json::to_value(&projected).unwrap();
                assert_eq!(
                    doc["player"].get("ally").unwrap_or(&Value::Null),
                    &native["ally"],
                    "step {i} Osty"
                );
                assert_eq!(doc["rng"], native["rng"], "step {i} RNG");
                for (pile, expected) in native["piles"].as_object().unwrap() {
                    let cards: Vec<_> = doc["piles"][pile]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| json!([c["id"], c.get("upgrade").cloned().unwrap_or(json!(0))]))
                        .collect();
                    assert_eq!(json!(cards), *expected, "step {i} {pile}");
                }
            }
        }
        assert_eq!(checked, checkpoints);
        assert_eq!(state.hp, final_hp);
        assert!(state.history.over);
    }
}

#[test]
fn floor31_skill_potion_matches_native_necrobinder_choice_and_rng() {
    // This isolates the potion while the full fight's later card/listener
    // frontier is still refused. It deliberately does not claim admission.
    let fixture: Value = serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor31-skill-potion.json"
    ))
    .unwrap();
    let entry: CanonicalStateV2 = serde_json::from_value(fixture["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    let selecting = engine::apply_action_into(
        &state,
        &catalog,
        &engine::Action::UsePotion {
            slot: 0,
            target: None,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let wire: ExactSolveActionV1 =
        serde_json::from_value(json!({"kind":"select","answer":{"kind":"option_index","index":1}}))
            .unwrap();
    let action: engine::Action = wire.try_into().unwrap();
    let after = engine::apply_action_into(&selecting, &catalog, &action, &mut Vec::new()).unwrap();
    let doc =
        serde_json::to_value(HotBoundary::try_to_canonical(&after, &catalog).unwrap()).unwrap();
    assert_eq!(
        json!({"hp":after.hp,"max_hp":after.max_hp,"block":after.block,"energy":after.energy,"turn":after.turn}),
        fixture["native"]["player"]
    );
    assert_eq!(doc["rng"], fixture["native"]["rng"]);
    for (pile, expected) in fixture["native"]["piles"].as_object().unwrap() {
        let cards: Vec<_> = doc["piles"][pile]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| json!([c["id"], c.get("upgrade").cloned().unwrap_or(json!(0))]))
            .collect();
        assert_eq!(json!(cards), *expected, "{pile}");
    }
    assert!(
        doc["piles"]["hand"]
            .as_array()
            .unwrap()
            .iter()
            .any(|card| card["id"] == "PULL_AGGRO")
    );
}

#[test]
fn floor20_abundance_line_matches_native_choices_resources_piles_and_rng() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor20-abundance-native.json"
    ))
    .unwrap();
    let entry: CanonicalStateV2 = serde_json::from_value(fixture["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let mut state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    let mut checked = 0;
    for (i, step) in fixture["steps"].as_array().unwrap().iter().enumerate() {
        let wire: ExactSolveActionV1 = serde_json::from_value(step["action"].clone()).unwrap();
        let action: engine::Action = wire.try_into().unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new()).unwrap();
        let projected = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(
            projected.differential_digest(),
            step["digest_after"],
            "step {i}"
        );
        let doc = serde_json::to_value(&projected).unwrap();
        if let Some(native) = step.get("native") {
            checked += 1;
            assert_eq!(
                json!({"hp":state.hp,"max_hp":state.max_hp,"block":state.block,
                "energy":state.energy,"turn":state.turn}),
                native["player"],
                "step {i}"
            );
            let monsters: Vec<_> = state
                .monsters
                .iter()
                .filter(|m| m.hp > 0)
                .map(|m| json!([m.kind.as_str(), m.hp, m.max_hp, m.block]))
                .collect();
            assert_eq!(json!(monsters), native["monsters"], "step {i}");
            assert_eq!(
                doc["player"].get("ally").unwrap_or(&Value::Null),
                &native["ally"],
                "step {i}"
            );
            assert_eq!(doc["rng"], native["rng"], "step {i}");
            for (pile, expected) in native["piles"].as_object().unwrap() {
                let cards: Vec<_> = doc["piles"][pile]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| json!([c["id"], c.get("upgrade").cloned().unwrap_or(json!(0))]))
                    .collect();
                assert_eq!(json!(cards), *expected, "step {i} {pile}");
            }
        }
    }
    assert_eq!(checked, 15);
    assert!(state.history.over);
    assert_eq!(state.hp, 50);
}

/// #2647 B2 — the Bowlbug root that motivated the issue no longer refuses
/// `Misery power acquisition order`.
///
/// This is the acceptance measurement for the whole slice, read rather than
/// predicted: the roster is a Bowlbug Rock plus an Egg and a Nectar, and the
/// Rock's intrinsic `ImbalancedPower` was the sole reason Misery's reader
/// rejected the entire roster. The fixture still does **not** admit — its
/// remaining refusals were the Entropy frontier (#2637) and the generated-card
/// provenance frontier, and since #3122 are only the synthetic fixture's
/// unrecorded AfterEnergyReset order, none of which share code with this
/// slice — so the
/// assertion is exactly the disappearance of the Misery refusal plus a pin on
/// what is left, so a later stage cannot quietly trade one for another.
#[test]
fn floor31_no_longer_refuses_the_misery_imbalanced_reader() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor31-skill-potion.json"
    ))
    .unwrap();
    let entry: CanonicalStateV2 = serde_json::from_value(fixture["entry"].clone()).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let state = HotBoundary::from_canonical(&entry, &catalog).unwrap();

    // The roster really is the one #2647 names, so this cannot pass vacuously.
    let kinds: Vec<_> = state
        .monsters
        .iter()
        .map(|monster| monster.kind.as_str())
        .collect();
    assert_eq!(kinds, ["BOWLBUG_ROCK", "BOWLBUG_EGG", "BOWLBUG_NECTAR"]);

    let refusal = engine::admit(&entry, &state, &catalog).unwrap_err();
    let mut remaining: Vec<_> = refusal
        .missing()
        .map(|capability| format!("{capability:?}"))
        .collect();
    remaining.sort();
    // #2637 removed exactly one name from this list, and the direction matters:
    // the root still REFUSES, and it still refuses the Entropy frontier by
    // name. `Entropy recursive transform closure` was a redundant second name
    // for the same gap.
    //
    // This fixture publishes `reward_card_pool: Necrobinder` and **no**
    // `entropy_card_pool`, so `catalog::entropy_transform_provenance_is_exact`
    // is false and `engine::turn::entropy_transform_pool` refuses — the
    // transform provably cannot execute here, before and after #2637. The
    // closure walk therefore no longer interns the five-class union, and
    // `boundary::entropy_catalog_closure_is_exact` no longer demands it back;
    // demanding a union for a mechanic that cannot run is what produced the
    // second name. `Entropy transform pool provenance`, the name that
    // describes the ACTUAL gap, is untouched and still asserted below — which
    // is the condition #2637 attaches to this gate: every state where Entropy
    // is reachable but the provenance is inexact still refuses at admission,
    // by name.
    //
    // The removed name is not dead: it still fires when the closure IS
    // expanded but the catalog is missing part of the union, and when no owner
    // is named at all. `engine::admission::tests::
    // entropy_transform_provenance_agrees_across_all_four_consumers` pins both.
    //
    // #2946 removed three more names, in the same direction: Calamity,
    // Discovery and Jackpot read the OWNER's CharacterCardPool through
    // `GetUnlockedCards(Owner.UnlockState, ..)` and never `entropy_card_pool`,
    // so this Necrobinder root's absent Entropy owner no longer refuses them
    // (`engine::cards::owner_pool_generation_provenance_is_exact`). Entropy and
    // Abundance DO still require it, and still refuse here by name.
    //
    // #3122 removed those last two, in the same direction again. Entropy's
    // transform pool is keyed by the ORIGINAL card (`GetDefaultTransformationOptions`
    // `0x112960` IL_000c-IL_0047) and Abundance reads the owner's
    // CharacterCardPool (`0x3888ec` IL_0027-IL_0058); neither reads
    // `entropy_card_pool`, so an absent one no longer refuses them (a
    // contradictory one still does). What remains is the synthetic fixture's
    // unrecorded AfterEnergyReset order, which is not a generation name — the
    // same residue `spectrum_shift_closes_the_full_colorless_pool_and_pins_live_order`
    // documents for its own synthetic root.
    assert_eq!(
        remaining,
        ["ArgumentShape(\"terminal star/orb AfterEnergyReset order\")"],
    );
}

#[test]
fn floor12_admits_the_captured_abundance_power_composition() {
    let entry: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor12.canonical.json"
    ))
    .unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&entry).unwrap();
    let state = HotBoundary::from_canonical(&entry, &catalog).unwrap();
    engine::admit(&entry, &state, &catalog).unwrap();
    let projected = without_the_bookkeeping_record(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        "floor 12",
    );
    assert_eq!(projected, entry);
}

/// These roots were captured before `player.session_bookkeeping` existed
/// (#3660). Each loads without it, derives the bookkeeping from its closure
/// and projects with the record added. That record is the only difference.
fn without_the_bookkeeping_record(
    mut projected: CanonicalStateV2,
    floor: &str,
) -> CanonicalStateV2 {
    // Floor 12's closure reaches Misery and neither Normality nor a
    // self-return card; the later floors reach the whole owner pool.
    let expected = if matches!(floor, "floor 12") {
        serde_json::json!(["misery_ledger"])
    } else {
        serde_json::json!(["misery_ledger", "normality_count", "self_return_uids"])
    };
    assert_eq!(
        projected.player.remove("session_bookkeeping"),
        Some(expected),
        "{floor}"
    );
    projected
}

/// The six floors #2637 is about, as committed roots.
///
/// Until this landing pass the headline claim — "floors 30/31/35/38/39/40
/// admit with zero refusals, and their interned closures are N ids / M specs"
/// — rested on roots that lived only in a scratch directory. An independent
/// verifier could reproduce none of it from the repo, and neither could CI.
/// These six files are those roots, regenerated with this checkout's own
/// projector (`rust_review.build_root` over the uploaded run and its per-floor
/// save) and byte-identical to the set the cycle-2 measurement used.
///
/// Three things are asserted per floor, and each is a claim the PR body makes:
///
/// 1. `engine::admit` returns `Ok(())` — zero refusals, not "only the expected
///    ones". This is what the whole slice is for, and it is the shape of
///    assertion that would have caught the origin-domain narrowing if one of
///    these decks had held a non-combat-generable origin.
/// 2. The cold projection round-trips, so admission is not resting on a state
///    the boundary cannot re-emit.
/// 3. The interned closure size, pinned exactly. The closure is what this
///    slice widened, and an unpinned size is a number nobody can check: a
///    change that shrank the probe and the real closure together would move
///    both sides at once and fail nothing else.
///
/// Admission is not a replay certificate and this test does not claim one —
/// the uploaded bundle carries no recorded action line for these six floors.
#[test]
fn the_six_entropy_floors_admit_with_zero_refusals_and_pinned_closures() {
    // #2561 widened the Entropy closure by the 17-curse `CurseCardPool`;
    // every floor now interns all of them (488 ids), each still admitting.
    for (floor, root, atoms, ids) in [
        (
            30u32,
            include_str!("../../eval/search/ea6-admission-v1/floor30.canonical.json"),
            958usize,
            488usize,
        ),
        (
            31,
            include_str!("../../eval/search/ea6-admission-v1/floor31.canonical.json"),
            958,
            488,
        ),
        (
            35,
            include_str!("../../eval/search/ea6-admission-v1/floor35.canonical.json"),
            960,
            488,
        ),
        (
            38,
            include_str!("../../eval/search/ea6-admission-v1/floor38.canonical.json"),
            960,
            488,
        ),
        (
            39,
            include_str!("../../eval/search/ea6-admission-v1/floor39.canonical.json"),
            960,
            488,
        ),
        (
            40,
            include_str!("../../eval/search/ea6-admission-v1/floor40.canonical.json"),
            962,
            488,
        ),
    ] {
        let entry: CanonicalStateV2 = serde_json::from_str(root).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&entry)
            .unwrap_or_else(|refusal| panic!("floor {floor}: catalog refused: {refusal:?}"));
        let state = HotBoundary::from_canonical(&entry, &catalog)
            .unwrap_or_else(|refusal| panic!("floor {floor}: boundary refused: {refusal:?}"));
        if let Err(refusal) = engine::admit(&entry, &state, &catalog) {
            panic!("floor {floor}: admission refused: {refusal:?}");
        }
        let projected = without_the_bookkeeping_record(
            HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
            &format!("floor {floor}"),
        );
        assert_eq!(
            projected, entry,
            "floor {floor}: cold projection did not round-trip"
        );
        let mut distinct: Vec<&str> = catalog
            .specs()
            .map(|spec| spec.identity.id.as_str())
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            (catalog.atom_count(), distinct.len()),
            (atoms, ids),
            "floor {floor}: interned closure size moved"
        );
    }
}

/// Floor 33, `ENCOUNTER.KAISER_CRAB_BOSS` — the one floor in #2637's scope
/// that still refused after the Entropy landing.
///
/// Its root is built by the same `rust_review.build_root` over the same
/// uploaded run and that floor's own start save as the six above, and it is
/// asserted the same three ways: zero refusals, a round-tripping cold
/// projection, and an exactly pinned interned closure. The roster assertion
/// is what keeps it from passing vacuously — before #2654 this document did
/// not survive `from_canonical` at all, because `player.kaiser_facing` and
/// `monsters[].crab_rage` were unmodeled fields.
///
/// Admission is not a replay certificate and this test does not claim one:
/// the uploaded bundle carries no recorded action line for floor 33, and the
/// local capture corpus holds no `.mcr` for this seed.
#[test]
fn floor33_kaiser_crab_admits_with_zero_refusals_and_a_pinned_closure() {
    let root = include_str!("../../eval/search/ea6-admission-v1/floor33.canonical.json");
    let entry: CanonicalStateV2 = serde_json::from_str(root).unwrap();
    let roster: Vec<&str> = entry
        .monsters
        .iter()
        .map(|monster| monster["kind"].as_str().unwrap())
        .collect();
    assert_eq!(roster, ["CRUSHER", "ROCKET"]);
    assert_eq!(entry.player["kaiser_facing"], json!(0));
    assert!(
        entry
            .monsters
            .iter()
            .all(|monster| monster["crab_rage"] == json!(true))
    );

    let catalog = HotBoundary::catalog_from_canonical(&entry)
        .unwrap_or_else(|refusal| panic!("floor 33: catalog refused: {refusal:?}"));
    let state = HotBoundary::from_canonical(&entry, &catalog)
        .unwrap_or_else(|refusal| panic!("floor 33: boundary refused: {refusal:?}"));
    if let Err(refusal) = engine::admit(&entry, &state, &catalog) {
        panic!("floor 33: admission refused: {refusal:?}");
    }
    let projected = without_the_bookkeeping_record(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        "floor 33",
    );
    assert_eq!(
        projected, entry,
        "floor 33: cold projection did not round-trip"
    );

    let mut distinct: Vec<&str> = catalog
        .specs()
        .map(|spec| spec.identity.id.as_str())
        .collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        (catalog.atom_count(), distinct.len()),
        (958usize, 488usize),
        "floor 33: interned closure size moved"
    );
}

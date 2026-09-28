//! Kaiser Crab facing, back attacks and the sibling death lifecycle (#2654).
//!
//! Authority: archived v0.111.0 `sts2.dll`, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//!
//! `SurroundedPower` is a PLAYER power: `Rocket/<AfterAddedToRoom>d__29::MoveNext`
//! (RVA `0x367c94`) applies it to `CombatState.GetOpponentsOf(rocket)` at
//! IL_00ae. Its `ModifyDamageMultiplicative` (`0xa8bc4`) returns exactly 3/2
//! when the receiver is that player and the dealer holds the marker matching
//! the current facing, its `BeforeCardPlayed` (`0x34750c`) and
//! `BeforePotionUsed` (`0x34760c`) both run `UpdateDirection` (`0x347a5c`) on
//! a non-null target, and its `AfterDeath` (`0x3473a8`) re-faces onto the sole
//! surviving side. `CrabRagePower.AfterDeath` (`0x337f54`) gives a live
//! sibling Strength 6 then Block 99 and removes itself.
use serde_json::{Value, json};
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
};

const CRUSHER_HP: i64 = 219;
const ROCKET_HP: i64 = 209;

fn toadpoles() -> CanonicalStateV2 {
    serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap()
}

/// The real captured floor-33 root, EA6YVY5X1QM1 node 32.
fn floor33() -> CanonicalStateV2 {
    serde_json::from_str(include_str!(
        "../../eval/search/ea6-admission-v1/floor33.canonical.json"
    ))
    .unwrap()
}

/// The validated Crusher/Rocket pair on the Ironclad starter fixture, so a
/// witness can choose its own hand without disturbing the captured root.
fn kaiser(facing: i64) -> CanonicalStateV2 {
    let mut doc = toadpoles();
    doc.monsters[0] = serde_json::from_value(json!({
        "kind": "CRUSHER", "hp": CRUSHER_HP, "max_hp": CRUSHER_HP, "crab_rage": true,
    }))
    .unwrap();
    doc.monsters[1] = serde_json::from_value(json!({
        "kind": "ROCKET", "hp": ROCKET_HP, "max_hp": ROCKET_HP, "crab_rage": true,
        "slot": 1, "uid": 1,
    }))
    .unwrap();
    doc.player.insert("kaiser_facing".into(), json!(facing));
    doc
}

fn set_hp(doc: &mut CanonicalStateV2, index: usize, hp: i64) {
    doc.monsters[index].insert("hp".into(), json!(hp));
}

fn hand(doc: &mut CanonicalStateV2, ids: &[&str]) {
    let pile = doc.piles.get_mut("hand").unwrap();
    for (index, id) in ids.iter().enumerate() {
        pile[index].id = (*id).into();
    }
}

fn player_int(doc: &CanonicalStateV2, key: &str, default: i64) -> i64 {
    doc.player
        .get(key)
        .and_then(Value::as_i64)
        .unwrap_or(default)
}

fn monster_int(doc: &CanonicalStateV2, index: usize, key: &str, default: i64) -> i64 {
    doc.monsters[index]
        .get(key)
        .and_then(Value::as_i64)
        .unwrap_or(default)
}

fn load(doc: &CanonicalStateV2) -> (sts_sim::catalog::Catalog, sts_sim::hot::HotState) {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    (catalog, state)
}

/// Apply one action and prove the successor survives a cold reload: every new
/// field has to reach the wire and come back, or the exact-search key (which
/// IS `try_to_canonical`) would collapse two distinct facings.
fn act_and_reload(doc: &CanonicalStateV2, action: &Action) -> CanonicalStateV2 {
    let (catalog, state) = load(doc);
    let next = engine::apply_action_into(&state, &catalog, action, &mut Vec::new()).unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    if !next.history.over {
        engine::admit(&wire, &cold, &cold_catalog).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire,
        "cold reload did not reproduce the projected document"
    );
    wire
}

fn play(doc: &CanonicalStateV2, uid: u32, target: Option<u8>) -> CanonicalStateV2 {
    act_and_reload(
        doc,
        &Action::Play {
            uid,
            target,
            selection: SelectionRef::NONE,
        },
    )
}

fn refusal(doc: &CanonicalStateV2) -> String {
    let catalog = match HotBoundary::catalog_from_canonical(doc) {
        Ok(catalog) => catalog,
        Err(refusal) => return format!("{refusal:?}"),
    };
    let state = match HotBoundary::from_canonical(doc, &catalog) {
        Ok(state) => state,
        Err(refusal) => return format!("{refusal:?}"),
    };
    match engine::admit(doc, &state, &catalog) {
        Ok(()) => panic!("expected a refusal, got admission"),
        Err(refusal) => format!("{refusal:?}"),
    }
}

// --------------------------------------------------------------------------
// The live back-attack multiplier
// --------------------------------------------------------------------------

/// Both directions of `SurroundedPower::ModifyDamageMultiplicative` `0xa8bc4`,
/// end to end through a real enemy turn.
///
/// Crusher's telegraph at `loop_pos` 0 is `THRASH_MOVE` 14x1 and Rocket's is
/// `TARGETING_RETICLE_MOVE` 4x1 (`content_tables::LOOPS`). At facing 0 the
/// switch takes the `BackAttackLeftPower` arm (IL_0035-IL_0044) and Crusher
/// alone is multiplied; at facing 1 it takes the `BackAttackRightPower` arm
/// (IL_0046-IL_0055) and Rocket alone is. The factor is the
/// `Decimal(15, 0, 0, false, 1)` at IL_0057 — 3/2, not a rounded 1.5.
#[test]
fn each_facing_multiplies_exactly_the_crab_behind_the_player() {
    for (facing, expected) in [(0, 21 + 4), (1, 14 + 6)] {
        let doc = kaiser(facing);
        let hp_before = player_int(&doc, "hp", 0);
        let next = act_and_reload(&doc, &Action::EndTurn);
        assert_eq!(
            hp_before - player_int(&next, "hp", 0),
            expected,
            "facing {facing}: 14 Thrash + 4 Reticle, one of them at 3/2",
        );
        assert_eq!(
            player_int(&next, "kaiser_facing", -1),
            facing,
            "facing {facing}: an untargeted enemy turn never re-faces",
        );
    }
}

/// The factor is per HIT, not per command, and it is frozen inside the same
/// snapshot as Strength: `Hook.ModifyDamageInternal` `0x106aa0` folds every
/// multiplicative listener (IL_0098) once per `CreatureCmd.Damage`, and
/// `AttackCommand` issues one Damage per hit.
///
/// Crusher's `BUG_STING_MOVE` is 7x2. At facing 0 each hit takes 3/2 of
/// `7 + Strength`, and the two hits are truncated independently.
#[test]
fn a_multi_hit_back_attack_multiplies_every_hit_after_strength() {
    let mut doc = kaiser(0);
    doc.monsters[0].insert("loop_pos".into(), json!(2));
    doc.monsters[0].insert("strength".into(), json!(1));
    doc.monsters[1].insert("loop_pos".into(), json!(4)); // RECHARGE_MOVE: no attack
    let hp_before = player_int(&doc, "hp", 0);

    let next = act_and_reload(&doc, &Action::EndTurn);

    assert_eq!(
        hp_before - player_int(&next, "hp", 0),
        24,
        "two hits of floor((7 + 1) * 3/2) = 12, not one command-wide factor",
    );
}

/// The factor composes with the ordinary receiver-side terms in the native
/// fold order and truncates ONCE, at the end. Player Vulnerable is 3/2
/// (`VulnerablePower::ModifyDamageMultiplicative` `0xaae4c`), so a Rocket
/// `TARGETING_RETICLE_MOVE` of 4 at facing 1 is 4 * 3/2 * 3/2 = 9 exactly,
/// which a per-factor truncation would have rendered as 6 * 3/2 = 9 by luck
/// and a 5 Thrash would not.
#[test]
fn the_back_factor_composes_with_vulnerable_under_one_truncation() {
    let mut doc = kaiser(1);
    doc.player.insert("player_vuln".into(), json!(3));
    doc.monsters[0].insert("loop_pos".into(), json!(1)); // ENLARGING_STRIKE 4x1
    let hp_before = player_int(&doc, "hp", 0);

    let next = act_and_reload(&doc, &Action::EndTurn);

    // Crusher 4 * 3/2 (Vulnerable only) = 6; Rocket 4 * 3/2 * 3/2 = 9.
    assert_eq!(hp_before - player_int(&next, "hp", 0), 15);
}

/// Osty absorbs the enlarged hit, not the base one: the multiplier is entirely
/// upstream of the block/pet/player split that `CreatureCmd.<Damage>d__12`
/// publishes, so a back attack spills past the ally that would have eaten it.
///
/// Rocket `PRECISION_BEAM_MOVE` is 20x1. At facing 1 that is 30; 4 Block and a
/// 7 HP Osty absorb 11, and the player takes the remaining 19 rather than
/// being fully covered by the 24 the unmultiplied hit would have met.
#[test]
fn osty_absorbs_the_multiplied_hit_and_the_split_keeps_its_original_result() {
    let mut doc = kaiser(1);
    doc.player.insert("ally".into(), json!(["OSTY", 7, 7]));
    doc.player.insert("block".into(), json!(4));
    doc.monsters[0].insert("loop_pos".into(), json!(3)); // ADAPT_MOVE: buff only
    doc.monsters[1].insert("loop_pos".into(), json!(1));
    let hp_before = player_int(&doc, "hp", 0);

    let next = act_and_reload(&doc, &Action::EndTurn);

    assert_eq!(
        next.player.get("ally"),
        None,
        "Osty takes its seven and dies"
    );
    assert_eq!(
        hp_before - player_int(&next, "hp", 0),
        19,
        "30 - 4 Block - 7 Osty, not 20 - 4 - 7",
    );
}

// --------------------------------------------------------------------------
// Facing writers: targeted cards and potions
// --------------------------------------------------------------------------

/// `SurroundedPower/<BeforeCardPlayed>d__11::MoveNext` `0x34750c`: a targeted
/// card runs `UpdateDirection(cardPlay.Target)`, and only the arm matching the
/// current facing flips. Targeting the crab the player already faces is a
/// no-op, which is why attacking the same crab twice never earns a bonus.
#[test]
fn a_targeted_card_faces_its_target_and_only_flips_the_matching_arm() {
    for (facing, target, expected) in [
        (0, 0u8, 1), // facing 0 + Crusher (BackAttackLeft) -> 1
        (0, 1, 0),   // facing 0 + Rocket: the left arm does not match
        (1, 1, 0),   // facing 1 + Rocket (BackAttackRight) -> 0
        (1, 0, 1),   // facing 1 + Crusher: the right arm does not match
    ] {
        let doc = kaiser(facing);
        let next = play(&doc, 0, Some(target));
        assert_eq!(
            player_int(&next, "kaiser_facing", -1),
            expected,
            "facing {facing} targeting slot {target}",
        );
    }
}

/// The same listener returns at IL_001d-IL_002a on a null `CardPlay.Target`,
/// so an untargeted card leaves the facing alone however many enemies exist.
#[test]
fn an_untargeted_card_never_moves_the_facing() {
    let mut doc = kaiser(0);
    hand(&mut doc, &["DEFEND_IRONCLAD"]);
    let next = play(&doc, 0, None);
    assert_eq!(player_int(&next, "kaiser_facing", -1), 0);
    assert_eq!(player_int(&next, "block", 0), 5);
}

/// `SurroundedPower/<BeforePotionUsed>d__12::MoveNext` `0x34760c` takes the
/// identical `UpdateDirection`, with one extra `CombatManager.IsInProgress`
/// gate the card listener does not carry.
#[test]
fn a_targeted_potion_faces_its_target_and_an_untargeted_one_does_not() {
    let mut doc = kaiser(0);
    doc.player
        .insert("potions".into(), json!(["BEETLE_JUICE", "BLOCK_POTION"]));
    doc.player.insert(
        "potion_slots".into(),
        json!(["BEETLE_JUICE", "BLOCK_POTION"]),
    );
    doc.player
        .insert("fully_unlocked_potion_pool".into(), json!(true));

    let targeted = act_and_reload(
        &doc,
        &Action::UsePotion {
            slot: 0,
            target: Some(0),
        },
    );
    assert_eq!(
        player_int(&targeted, "kaiser_facing", -1),
        1,
        "Beetle Juice targets Crusher at facing 0",
    );
    assert_eq!(monster_int(&targeted, 0, "shrink", 0), 4);

    let untargeted = act_and_reload(
        &doc,
        &Action::UsePotion {
            slot: 1,
            target: None,
        },
    );
    assert_eq!(player_int(&untargeted, "kaiser_facing", -1), 0);
    assert_eq!(player_int(&untargeted, "block", 0), 12);
}

/// The RESUMED body takes the same `SurroundedPower.BeforeCardPlayed` walk.
///
/// Native runs the hook once per generated `CardPlay`, and a suspended play's
/// `StartBody` continuation is a body that has not yet run it — which is why
/// `play.rs` calls `update_kaiser_facing` on both paths, the inline replay loop
/// and the resumed `StartBody` (reading the retained target through
/// `persisted_target_index`). Without a witness that arm was the only reachable
/// production call site in this slice with no test crossing it.
///
/// Scrape is a targeted card that parks on a selection, so the play is cut in
/// half exactly where the resume happens. Three things are pinned: the inline
/// body's flip survives into the PARKED wire document (facing is keyed state
/// that has to round-trip across a suspension, with a live `pending` frame);
/// the resume does not refuse; and the completed play leaves the facing where
/// one `UpdateDirection` per CardPlay leaves it — the retained target is the
/// crab the player now faces, so the second walk correctly finds no matching
/// arm and writes nothing.
#[test]
fn a_suspended_and_resumed_card_play_takes_the_same_facing_walk() {
    let mut doc = kaiser(0);
    doc.player.insert("next_card_uid".into(), json!(5));
    doc.player.insert("master_planner".into(), json!(1));
    doc.player.insert(
        "after_card_played_power_order".into(),
        json!([["master_planner", 0]]),
    );
    doc.player
        .insert("next_after_side_turn_end_power_uid".into(), json!(1));
    doc.piles = serde_json::from_value(json!({
        "hand": [{"id":"SCRAPE","upgrade":0,"uid":0}],
        "draw": [
            {"id":"HOLOGRAM","upgrade":0,"uid":1,"local_keywords":["Sly"]},
            {"id":"DEFEND_IRONCLAD","upgrade":0,"uid":2,"local_keywords":["Sly"]},
            {"id":"DEFEND_SILENT","upgrade":0,"uid":3},
            {"id":"STRIKE_DEFECT","upgrade":0,"uid":4}
        ],
        "discard": []
    }))
    .unwrap();

    // Facing 0 targeting Crusher takes the `BackAttackLeftPower` arm.
    let parked = play(&doc, 0, Some(0));
    assert!(
        parked.player.contains_key("pending"),
        "Scrape must park on its selection, or this witnesses nothing",
    );
    assert_eq!(
        player_int(&parked, "kaiser_facing", -1),
        1,
        "the inline body's flip survives the suspension on the wire",
    );

    let done = act_and_reload(&parked, &select_uid(&parked, 2));
    assert!(!done.player.contains_key("pending"));
    assert_eq!(
        player_int(&done, "kaiser_facing", -1),
        1,
        "the resumed StartBody ran the hook and its retained target no longer \
         matches the facing-1 arm, so it wrote nothing",
    );
}

/// The public selector for one card uid, used by the suspend/resume witness.
fn select_uid(doc: &CanonicalStateV2, wanted: u32) -> Action {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::legal_actions(&state, &catalog)
        .into_iter()
        .find(|action| {
            let Action::Select { answer } = action else {
                return false;
            };
            engine::selected_card_uids(&state, &catalog, *answer)
                .unwrap()
                .is_some_and(|uids| uids == [wanted])
        })
        .unwrap_or_else(|| panic!("missing public selector for uid {wanted}"))
}

/// A dead target flips nothing. Native reaches that through power removal
/// rather than through an HP test: `Creature.RemoveAllPowersAfterDeath`
/// `0x11dbac` drops every power whose `ShouldPowerBeRemovedAfterOwnerDeath` is
/// the base `0x840dd`, and `BackAttackLeftPower` does not override it — so a
/// dead Crusher no longer satisfies the facing-0 arm.
///
/// The public action boundary closes the manual half of it outright — a
/// manual play at a dead slot is `BadTarget`, never a facing write — and the
/// surviving arm is the one an automatic play could still reach, which the
/// `hp > 0` guard in `update_kaiser_facing` covers.
#[test]
fn a_corpse_can_neither_be_targeted_nor_re_expose_the_survivor() {
    let mut doc = kaiser(1);
    set_hp(&mut doc, 0, 4);
    hand(&mut doc, &["STRIKE_IRONCLAD", "STRIKE_IRONCLAD"]);

    let after_kill = play(&doc, 0, Some(0));
    assert!(monster_int(&after_kill, 0, "hp", 1) <= 0);
    assert_eq!(
        player_int(&after_kill, "kaiser_facing", -1),
        0,
        "AfterDeath faces the sole surviving Rocket",
    );

    let (catalog, state) = load(&after_kill);
    let error = engine::apply_action_into(
        &state,
        &catalog,
        &Action::Play {
            uid: 1,
            target: Some(0),
            selection: SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}").contains("BadTarget"),
        "a corpse is not a legal manual target: {error:?}",
    );

    // The live target is still the one that cannot flip the facing back:
    // Rocket at facing 0 takes the `BackAttackLeftPower` arm and misses.
    let after_live_target = play(&after_kill, 1, Some(1));
    assert_eq!(player_int(&after_live_target, "kaiser_facing", -1), 0);
}

// --------------------------------------------------------------------------
// Death: the re-face and the sibling rage
// --------------------------------------------------------------------------

/// Killing either crab re-faces onto the survivor and hands that survivor the
/// complete `CrabRagePower.AfterDeath` payload once: Strength 6, then Block
/// 99, then the instance removes itself (`0x337f54` IL_0062-IL_0154).
#[test]
fn killing_either_crab_rages_the_survivor_once_and_faces_it() {
    for (victim, survivor, facing_before, facing_after) in [(0usize, 1usize, 1, 0), (1, 0, 0, 1)] {
        let mut doc = kaiser(facing_before);
        set_hp(&mut doc, victim, 4);
        hand(&mut doc, &["STRIKE_IRONCLAD"]);

        let next = play(&doc, 0, Some(victim as u8));

        assert!(monster_int(&next, victim, "hp", 1) <= 0);
        assert_eq!(
            monster_int(&next, survivor, "strength", 0),
            6,
            "victim {victim}: Strength 6",
        );
        assert_eq!(
            monster_int(&next, survivor, "block", 0),
            99,
            "victim {victim}: Block 99",
        );
        assert_eq!(
            next.monsters[survivor].get("crab_rage"),
            None,
            "victim {victim}: the power removed itself",
        );
        assert_eq!(
            next.monsters[victim].get("crab_rage"),
            None,
            "victim {victim}: the corpse's own instance went with it",
        );
        assert_eq!(
            player_int(&next, "kaiser_facing", -1),
            facing_after,
            "victim {victim}: the survivor is now in front",
        );
    }
}

/// The re-faced survivor can never back-attack: whichever crab lives, the
/// facing that `AfterDeath` installs is the one whose arm that crab does not
/// satisfy. Rocket's `LASER_MOVE` is 35x1, and 35 is what it deals.
#[test]
fn the_survivor_of_a_death_can_no_longer_reach_the_players_back() {
    let mut doc = kaiser(1);
    set_hp(&mut doc, 0, 4);
    doc.monsters[1].insert("loop_pos".into(), json!(3));
    hand(&mut doc, &["STRIKE_IRONCLAD"]);

    let after_kill = play(&doc, 0, Some(0));
    assert_eq!(player_int(&after_kill, "kaiser_facing", -1), 0);
    let hp_before = player_int(&after_kill, "hp", 0);

    let next = act_and_reload(&after_kill, &Action::EndTurn);

    // 35 Laser + the 6 Strength CrabRage just granted = 41, taken whole: the
    // player has no Block, and facing 0 does not satisfy Rocket's
    // `BackAttackRightPower` arm, so there is no 3/2 anywhere. 61 would be the
    // number if the death had left the survivor behind the player.
    assert_eq!(hp_before - player_int(&next, "hp", 0), 41);
}

/// **The #2655 unlock.** A killing AoE commits HP to BOTH crabs in phase 1, so
/// the CrabRage hook drained in phase 3 finds no live sibling and grants
/// nothing: `PowerCmd.Apply` `0x3efbac` IL_0061 returns on a target whose
/// `CanReceivePowers` is false, and `CreatureCmd.GainBlock` `0x3eaec0`
/// IL_0046 returns zero on a dead creature.
///
/// Under the pre-#2655 per-target commit/death loop the first crab's death
/// would have resolved while the second still stood at full HP, handing it
/// Block 99 before its own damage landed — and the AoE would not have killed
/// it. This test fails loudly if that ordering ever comes back.
#[test]
fn a_killing_aoe_commits_both_crabs_before_either_rage_can_shield_the_other() {
    let mut doc = kaiser(0);
    set_hp(&mut doc, 0, 4);
    set_hp(&mut doc, 1, 4);
    hand(&mut doc, &["THUNDERCLAP"]);

    let next = play(&doc, 0, None);

    assert!(monster_int(&next, 0, "hp", 1) <= 0);
    assert!(
        monster_int(&next, 1, "hp", 1) <= 0,
        "the second crab took its own committed damage, unshielded",
    );
    for index in 0..2 {
        assert_eq!(
            monster_int(&next, index, "block", 0),
            0,
            "monster {index}: no Block 99 on a same-batch corpse",
        );
        assert_eq!(
            monster_int(&next, index, "strength", 0),
            0,
            "monster {index}: no Strength 6 on a same-batch corpse",
        );
    }
    assert_eq!(
        player_int(&next, "kaiser_facing", -1),
        0,
        "the SurroundedPower instance outlives the roster it was installed for",
    );
}

/// A player-Thorns retaliation kill takes the identical death walk: the rage
/// and the re-face are properties of the death, not of who dealt it.
#[test]
fn a_thorns_retaliation_kill_rages_the_sibling_and_re_faces() {
    let mut doc = kaiser(0);
    doc.player.insert("thorns".into(), json!(9));
    set_hp(&mut doc, 0, 4);
    doc.monsters[1].insert("loop_pos".into(), json!(4)); // Rocket RECHARGE: no attack

    let next = act_and_reload(&doc, &Action::EndTurn);

    assert!(
        monster_int(&next, 0, "hp", 1) <= 0,
        "Thrash retaliates into 9 Thorns",
    );
    assert_eq!(monster_int(&next, 1, "strength", 0), 6);
    assert_eq!(monster_int(&next, 1, "block", 0), 99);
    assert_eq!(
        player_int(&next, "kaiser_facing", -1),
        0,
        "Rocket carries BackAttackRight, so the facing-0 arm does not match",
    );
}

/// A death the player's own pet absorbs nothing of, but which combat-ends the
/// fight, still runs the hooks in the same order — and a prevented PLAYER
/// death does not disturb the crabs at all. Lizard Tail revives the player at
/// the end of the enemy turn; the roster, the facing and both rage instances
/// are exactly where the turn left them.
#[test]
fn a_prevented_player_death_leaves_the_whole_kaiser_lifecycle_untouched() {
    let mut doc = kaiser(0);
    doc.player.insert("hp".into(), json!(8));
    doc.player
        .insert("relics_entering".into(), json!(["RELIC.LIZARD_TAIL"]));
    doc.player.insert("lizard_tail_used".into(), json!(0));

    let next = act_and_reload(&doc, &Action::EndTurn);

    assert!(player_int(&next, "hp", 0) > 0, "Lizard Tail prevented it");
    assert_eq!(next.player["lizard_tail_used"], 1);
    assert_eq!(player_int(&next, "kaiser_facing", -1), 0);
    assert!(next.monsters.iter().all(|m| m["crab_rage"] == json!(true)));
}

// --------------------------------------------------------------------------
// State, keys and cold reload
// --------------------------------------------------------------------------

/// `kaiser_facing` and `crab_rage` are keyed state: the exact-search and memo
/// key IS `try_to_canonical` (`exact_dfs::memo_key`), so two states that
/// differ only in facing must project differently, and both must reload.
#[test]
fn facing_and_rage_are_keyed_state_that_survives_a_cold_reload() {
    let zero = kaiser(0);
    let one = kaiser(1);
    let (catalog_zero, state_zero) = load(&zero);
    let (catalog_one, state_one) = load(&one);
    let wire_zero = HotBoundary::try_to_canonical(&state_zero, &catalog_zero).unwrap();
    let wire_one = HotBoundary::try_to_canonical(&state_one, &catalog_one).unwrap();
    assert_ne!(
        wire_zero.canonical_json(),
        wire_one.canonical_json(),
        "the two facings must not share a search key",
    );
    assert_eq!(wire_zero, zero);
    assert_eq!(wire_one, one);

    // The absent sentinel elides, exactly as Python's projector elides a
    // dataclass field at its default.
    let plain = toadpoles();
    let (catalog, state) = load(&plain);
    let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert_eq!(wire.player.get("kaiser_facing"), None);
    assert!(wire.monsters.iter().all(|m| m.get("crab_rage").is_none()));
}

/// The captured floor-33 root, re-asserted from this slice's own file so a
/// change to the mechanic that still admitted but projected differently could
/// not pass silently.
#[test]
fn the_captured_floor33_root_admits_and_round_trips() {
    let doc = floor33();
    let (catalog, state) = load(&doc);
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        doc
    );
    assert_eq!(player_int(&doc, "kaiser_facing", -1), 0);
    assert_eq!(monster_int(&doc, 0, "max_hp", 0), CRUSHER_HP);
    assert_eq!(monster_int(&doc, 1, "max_hp", 0), ROCKET_HP);
}

// --------------------------------------------------------------------------
// Retained walls — one witness each
// --------------------------------------------------------------------------

/// Facing outside the two-member `SurroundedPower/Direction` enum is
/// unrepresentable at the boundary, not silently clamped.
#[test]
fn a_facing_outside_the_native_direction_enum_refuses_by_name() {
    for bad in [2, -2, 7] {
        let mut doc = kaiser(0);
        doc.player.insert("kaiser_facing".into(), json!(bad));
        let text = refusal(&doc);
        assert!(
            text.contains("kaiser_facing"),
            "facing {bad} refused as {text}",
        );
    }
}

/// A facing with no Crusher/Rocket in the roster has no installer — the only
/// `Apply<SurroundedPower>` in the build is `0x367c94` IL_00ae — so it is
/// forged rather than merely unrepresented.
#[test]
fn a_facing_without_the_kaiser_roster_refuses_by_name() {
    let mut doc = toadpoles();
    doc.player.insert("kaiser_facing".into(), json!(0));
    assert!(
        refusal(&doc).contains("Kaiser Crab Surrounded lifecycle"),
        "{}",
        refusal(&doc),
    );
}

/// `CrabRagePower` has the same single pair of installers, so no other kind
/// may carry one.
#[test]
fn crab_rage_on_a_non_crab_refuses_at_the_boundary() {
    let mut doc = toadpoles();
    doc.monsters[0].insert("crab_rage".into(), json!(true));
    let text = refusal(&doc);
    assert!(text.contains("crab_rage"), "{text}");
}

/// Every spoofed or malformed variant of the fixed roster keeps the one
/// precise `Kaiser Crab Surrounded lifecycle` refusal. Each row is a distinct
/// clause of `_validated_kaiser_roster` (L10589-L10628).
#[test]
fn spoofed_and_malformed_kaiser_rosters_stay_refused_by_name() {
    type Mutation = fn(&mut CanonicalStateV2);
    let mutations: [(&str, Mutation); 8] = [
        ("swapped order", |doc| {
            doc.monsters.swap(0, 1);
            doc.monsters[0].remove("slot");
            doc.monsters[0].remove("uid");
            doc.monsters[1].insert("slot".into(), json!(1));
            doc.monsters[1].insert("uid".into(), json!(1));
        }),
        ("solo Crusher", |doc| {
            doc.monsters.truncate(1);
        }),
        ("three monsters", |doc| {
            let mut third = doc.monsters[1].clone();
            third.insert("slot".into(), json!(2));
            third.insert("uid".into(), json!(2));
            doc.monsters.push(third);
        }),
        ("wrong slots", |doc| {
            doc.monsters[1].insert("slot".into(), json!(2));
        }),
        ("wrong uids", |doc| {
            doc.monsters[1].insert("uid".into(), json!(4));
        }),
        ("mixed ascension band", |doc| {
            doc.monsters[1].insert("max_hp".into(), json!(199));
            doc.monsters[1].insert("hp".into(), json!(199));
        }),
        ("live crab missing its rage", |doc| {
            doc.monsters[1].remove("crab_rage");
        }),
        ("corpse retaining its rage", |doc| {
            doc.monsters[1].insert("hp".into(), json!(0));
        }),
    ];
    for (name, mutate) in mutations {
        let mut doc = kaiser(0);
        mutate(&mut doc);
        let text = refusal(&doc);
        assert!(
            text.contains("Kaiser Crab Surrounded lifecycle"),
            "{name}: refused as {text}",
        );
    }
}

/// `SurroundedPower.AfterDeath` `0x3473a8` IL_0020-IL_0028 returns on a
/// prevented removal, and `CrabRagePower.AfterDeath` `0x337f54` has no such
/// test — so the two would disagree if a crab's removal could ever be
/// prevented. It cannot: a prevented monster removal in this engine is the
/// illusion / Stock / Test Subject revive machinery, every arm of which is
/// kind-specific, and the `_ => monster.revive_stage == 0` default refuses any
/// other kind that carries revive state.
///
/// This test is what keeps that reasoning honest. If a crab ever became able
/// to hold revive state, it fails first, and the `wasRemovalPrevented` arm
/// must then be derived rather than argued away.
#[test]
fn a_crab_cannot_carry_revive_state_so_its_removal_is_never_prevented() {
    for index in 0..2 {
        let mut doc = kaiser(0);
        doc.monsters[index].insert("hp".into(), json!(0));
        doc.monsters[index].insert("revive_stage".into(), json!(1));
        doc.monsters[index].remove("crab_rage");
        let text = refusal(&doc);
        assert!(
            text.contains("illusion revive lifecycle"),
            "monster {index}: refused as {text}",
        );
    }
}

/// Only the A8+ HP pair admits; the below-A8 pair refuses by name.
///
/// The two HP getters are `GetValueIfAscension(8, …)`
/// (`Crusher::get_MinInitialHp` `0xb1e50`, `Rocket` `0xbc8f8`), but every move
/// constant these two spend is gated one ascension HIGHER — `ThrashDamage`
/// `0xb1e6a` is `GetValueIfAscension(9, 14, 12)`, and Bug Sting `0xb1e82`,
/// Adapt `0xb1e90`, Guarded Strike `0xb1e9b`, Reticle `0xbc912`, Precision
/// Beam `0xbc91d`, Laser `0xbc92a` and Charge Up `0xbc937` are the same shape.
/// `content_tables::LOOPS` carries the A9+ tier alone, so a `(209, 199)`
/// roster proves ascension < 8, hence < 9, hence that every move argument is
/// the wrong tier.
///
/// This test is the one that matters: the assertion is not merely that the
/// document refuses, but that it refuses BEFORE an end-turn could spend a
/// wrong-tier move. The commented number is what the engine would have dealt.
#[test]
fn the_below_a8_hp_pair_refuses_because_its_move_tier_is_wrong() {
    let mut doc = kaiser(0);
    for (index, hp) in [(0usize, 209), (1usize, 199)] {
        doc.monsters[index].insert("max_hp".into(), json!(hp));
        doc.monsters[index].insert("hp".into(), json!(hp));
    }
    let text = refusal(&doc);
    assert!(
        text.contains("Kaiser Crab Surrounded lifecycle"),
        "below-A8 pair refused as {text}",
    );

    // The A8+ pair is the one that admits, and Crusher's Thrash at facing 0 is
    // the A9+ 14 taken at 3/2 = 21. Below A8 native spends 12, for 18 — which
    // is exactly the number admitting the low band would have silently got
    // wrong, because nothing downstream re-derives the tier.
    let admitted = kaiser(0);
    let next = act_and_reload(&admitted, &Action::EndTurn);
    assert_eq!(
        player_int(&admitted, "hp", 0) - player_int(&next, "hp", 0),
        21 + 4,
    );
}

/// A mixed band is not a band: one crab at each tier refuses too, so the pin
/// cannot be satisfied by an HP that merely looks plausible for one of them.
#[test]
fn a_mixed_ascension_band_pair_refuses_by_name() {
    for (index, hp) in [(0usize, 209), (1usize, 199)] {
        let mut doc = kaiser(0);
        doc.monsters[index].insert("max_hp".into(), json!(hp));
        doc.monsters[index].insert("hp".into(), json!(hp));
        assert!(
            refusal(&doc).contains("Kaiser Crab Surrounded lifecycle"),
            "monster {index} at {hp}",
        );
    }
}

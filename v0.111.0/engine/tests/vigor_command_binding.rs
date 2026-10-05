//! #2696: Vigor stays bound to the attack command that took it while a
//! Hellraiser Strike is AutoPlayed inside that command.
//!
//! `VigorPower::BeforeAttack` RVA `0xaa7ac` (v0.111.0 `sts2.dll`, SHA-256
//! `9cb4f1ad…fbf12b4`) binds the first eligible powered `AttackCommand`
//! (IL_0076-IL_0078) and does not rebind while one is bound
//! (IL_0033-IL_0040). `ModifyDamageAdditive` RVA `0xaa83c` answers zero to a
//! different non-null card source (IL_0031-IL_0051), and
//! `<AfterAttack>d__8::MoveNext` RVA `0x34a824` consumes only for the bound
//! command (IL_0024-IL_0032). A card Hellraiser AutoPlays inside the bound
//! command (`HellraiserPower/<AfterCardDrawnEarly>d__7::MoveNext` RVA
//! `0x33c1a8` IL_011e-IL_012e) is a different card, so it reads no Vigor and
//! consumes none.
//!
//! Every fixture is the KD13JGCDPB3U Toadpoles entry with the piles, the
//! Toadpoles' HP and the player's powers and relics replaced. Each passes
//! `admit_exact_solve` and `engine::admit` and is driven through
//! `apply_action_into`.
//!
//! The live engine agrees (headless harness, build v0.111.0 `41cef1ea`,
//! Ironclad against `TOADPOLES_WEAK`, Akabeko's Vigor 8, Hellraiser played
//! from hand); each test names its probe.
use serde_json::{Value, json};
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, Event, Subject},
    exact_solve_v1::ExactSolveActionV1,
    hot::HotState,
    ids::PowerId,
    solo_v1::admit_exact_solve,
};

const STRIKE: &str = "STRIKE_IRONCLAD";

fn card(id: &str, uid: u32) -> Value {
    json!({"id": id, "uid": uid, "upgrade": 0})
}

/// Vigor 5, Hellraiser 1, the Toadpoles at 5 and 100 HP, `hand` (uids from
/// 0) in Hand and `draw` (the following uids) in the Draw pile.
fn root(relics: &[&str], hand: &[&str], draw: &[&str]) -> Value {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    let player = &mut value["player"];
    player["hp"] = 100.into();
    player["max_hp"] = 100.into();
    player["vigor"] = 5.into();
    player["hellraiser"] = 1.into();
    player["after_side_turn_end_power_order"] = json!([["hellraiser", 0]]);
    player["next_after_side_turn_end_power_uid"] = 1.into();
    for relic in relics {
        player["relics_entering"]
            .as_array_mut()
            .unwrap()
            .push(format!("RELIC.{relic}").into());
        match *relic {
            "GREMLIN_HORN" => player["gremlin_horn"] = true.into(),
            "CENTENNIAL_PUZZLE" => player["puzzle"] = true.into(),
            other => panic!("{other}"),
        }
    }
    let mut uid = 0;
    let mut pile = |ids: &[&str]| -> Value {
        ids.iter()
            .map(|id| {
                uid += 1;
                card(id, uid - 1)
            })
            .collect::<Vec<_>>()
            .into()
    };
    let hand = pile(hand);
    let draw = pile(draw);
    value["piles"]["hand"] = hand;
    value["piles"]["draw"] = draw;
    value["player"]["next_card_uid"] = uid.into();
    value["monsters"][0]["hp"] = 5.into();
    value["monsters"][1]["hp"] = 100.into();
    value["monsters"][1]["max_hp"] = 100.into();
    value
}

struct Loaded {
    state: HotState,
    catalog: Catalog,
}

fn load(value: Value) -> Loaded {
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();
    Loaded { state, catalog }
}

/// Apply `line`, returning the last state and every event in order.
fn run(loaded: &Loaded, line: &str) -> (HotState, Vec<Event>) {
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut state = loaded.state.clone();
    let mut all = Vec::new();
    let mut events = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        events.clear();
        state = engine::apply_action_into(&state, &loaded.catalog, &action, &mut events)
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
        all.extend(events.iter().cloned());
    }
    (state, all)
}

/// Project, load cold, admit, and project again: the document is a fixed
/// point of the boundary. (A cold catalog interns its atoms in document
/// order, so the two hot states are compared through their projection.)
fn cold(state: &HotState, catalog: &Catalog) -> (Loaded, CanonicalStateV2) {
    let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    engine::admit(&wire, &cold, &cold_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
    (
        Loaded {
            state: cold,
            catalog: cold_catalog,
        },
        wire,
    )
}

fn assert_cold_equal(state: &HotState, catalog: &Catalog) {
    cold(state, catalog);
}

fn hp(state: &HotState, uid: u32) -> i32 {
    state
        .monsters
        .iter()
        .find(|monster| monster.uid == uid)
        .unwrap()
        .hp
}

/// `(uid, unblocked)` of every monster result, in order.
fn hits(events: &[Event]) -> Vec<(u32, i32)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::MonsterDamaged { uid, unblocked, .. } => Some((*uid, *unblocked)),
            _ => None,
        })
        .collect()
}

/// The projected Gigantification stack count and bound latch.
fn gigantification(state: &HotState, catalog: &Catalog) -> (i64, bool) {
    let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let field = |key: &str| wire.player.get(key).cloned().unwrap_or(Value::Null);
    (
        field("gigantification").as_i64().unwrap_or(0),
        field("gigantification_bound").as_bool().unwrap_or(false),
    )
}

fn vigor_writes(events: &[Event]) -> Vec<i32> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::PowerChanged {
                subject: Subject::Player,
                power: PowerId::Vigor,
                amount,
            } => Some(*amount),
            _ => None,
        })
        .collect()
}

const PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE: &str = r#"[{"kind":"play","uid":0,"target":0}]"#;

/// The issue's reproduction. The outer Strike kills the 5 HP Toadpole with
/// its Vigor, Gremlin Horn draws the other Strike, Hellraiser plays it, and
/// that Strike is a different card: 6, not 11. `main` left the survivor at 89.
///
/// Live engine, seed `PROBE2696A`: the kill, the drawn Strike's 6 on the
/// survivor, then one `PowerReceived VigorPower -8`; the survivor at 94.
#[test]
fn a_hellraiser_strike_drawn_inside_a_vigor_attack_reads_no_vigor() {
    let loaded = load(root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]));
    let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert_eq!(hits(&events), [(0, 11), (1, 6)]);
    assert_eq!(hp(&state, 1), 94);
    assert_eq!(state.powers.value(PowerId::Vigor), 0);
    assert_cold_equal(&state, &loaded.catalog);
}

/// The nested Strike consumes nothing, and the outer command consumes once,
/// after the nested card has resolved. `main` wrote Vigor twice: once in the
/// nested command's AfterAttack and once in the outer one.
#[test]
fn the_nested_strike_consumes_nothing_and_the_outer_command_consumes_once() {
    let loaded = load(root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]));
    let (_, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert_eq!(vigor_writes(&events), [0]);
    let position = |wanted: &dyn Fn(&Event) -> bool| events.iter().position(wanted).unwrap();
    let nested_resolved = position(&|event| matches!(event, Event::CardResolved { uid: 1, .. }));
    let consumed = position(&|event| {
        matches!(
            event,
            Event::PowerChanged {
                power: PowerId::Vigor,
                ..
            }
        )
    });
    let outer_resolved = position(&|event| matches!(event, Event::CardResolved { uid: 0, .. }));
    assert!(nested_resolved < consumed && consumed < outer_resolved);
}

/// A multi-hit outer command keeps its Vigor for the hits after the nested
/// Strike. Whirlwind at X = 3: the first hit kills the weak Toadpole and
/// deals 10 to the other, the drawn Strike deals 6, and the two remaining
/// hits deal 10 each.
///
/// Live engine, seed `PROBE2696B2` (Vigor 8, an Energy Potion first): 13, the
/// Strike's 6, 13, 13, then one `PowerReceived VigorPower -8`; the survivor
/// at 55 of 100.
#[test]
fn a_multi_hit_command_keeps_its_vigor_after_a_nested_strike() {
    let loaded = load(root(&["GREMLIN_HORN"], &["WHIRLWIND"], &[STRIKE]));
    let (state, events) = run(&loaded, r#"[{"kind":"play","uid":0}]"#);
    assert_eq!(
        hits(&events),
        [(0, 10), (1, 10), (1, 6), (1, 10), (1, 10)],
        "{events:?}"
    );
    assert_eq!(hp(&state, 1), 64);
    assert_eq!(vigor_writes(&events), [0]);
    assert_cold_equal(&state, &loaded.catalog);
}

/// The binding ends with its command. Without Hellraiser the Horn only draws
/// the second Strike; played next, it is an ordinary first command with no
/// Vigor left to take. Unchanged from `main`.
#[test]
fn a_later_command_is_not_nested_and_finds_the_vigor_consumed() {
    let mut value = root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]);
    let player = value["player"].as_object_mut().unwrap();
    player.remove("hellraiser");
    player.remove("after_side_turn_end_power_order");
    player.remove("next_after_side_turn_end_power_uid");
    let loaded = load(value);
    let (state, events) = run(
        &loaded,
        r#"[{"kind":"play","uid":0,"target":0},{"kind":"play","uid":1,"target":1}]"#,
    );
    assert_eq!(hits(&events), [(0, 11), (1, 6)]);
    assert_eq!(vigor_writes(&events), [0]);
    assert_eq!(hp(&state, 1), 94);
}

/// The binding is released with its command. The first Strike binds and
/// consumes; Patter then applies Vigor again; the second Strike is a
/// different card and must bind as a first command. A binding that outlived
/// the first Strike would answer it zero (6, not 8) and leave the Vigor.
///
/// Native: the consumed power was removed (`Creature::RemovePowerInternal`
/// RVA `0x11db0b`), so `PowerCmd/<Apply>d__1`1::MoveNext` RVA `0x3ef988`
/// finds nothing to stack on (IL_006c) and installs a fresh `ToMutable()`
/// instance (IL_0084) whose `Data.commandToModify` is null
/// (`PowerModel::DeepCloneFields` RVA `0x8406d` IL_001b-IL_0020).
#[test]
fn vigor_gained_after_a_bound_command_binds_the_next_command() {
    let mut value = root(&[], &[STRIKE, "PATTER", STRIKE], &["DEFEND_IRONCLAD"]);
    let player = value["player"].as_object_mut().unwrap();
    player.remove("hellraiser");
    player.remove("after_side_turn_end_power_order");
    player.remove("next_after_side_turn_end_power_uid");
    value["monsters"][0]["hp"] = 26.into();
    let loaded = load(value);
    let (state, events) = run(
        &loaded,
        r#"[{"kind":"play","uid":0,"target":0},{"kind":"play","uid":1},{"kind":"play","uid":2,"target":0}]"#,
    );
    // Patter grants Vigor 2.
    assert_eq!(hits(&events), [(0, 11), (0, 8)]);
    assert_eq!(vigor_writes(&events), [0, 2, 0]);
    assert_eq!(state.powers.value(PowerId::Vigor), 0);
    assert_cold_equal(&state, &loaded.catalog);
}

/// A zero-hit command still binds and consumes: Whirlwind at X = 0 deals
/// nothing and takes the Vigor. `AttackCommand/<Execute>d__90::MoveNext` RVA
/// `0x3f19c0` runs BeforeAttack (IL_00d8) and AfterAttack (IL_0845) around a
/// hit loop of any length. Unchanged from `main`.
#[test]
fn a_zero_hit_command_binds_and_consumes() {
    let mut value = root(&["GREMLIN_HORN"], &["WHIRLWIND", STRIKE], &[STRIKE]);
    value["player"]["energy"] = 0.into();
    let loaded = load(value);
    let (state, events) = run(&loaded, r#"[{"kind":"play","uid":0}]"#);
    assert!(hits(&events).is_empty());
    assert_eq!(vigor_writes(&events), [0]);
    assert_eq!(state.powers.value(PowerId::Vigor), 0);
}

/// The nested Strike ends the combat, so the outer command's AfterAttack
/// reaches no listener and the Vigor stays
/// (`Hook/<IterateCombatHookListeners>d__0` RVA `0x3d3bc0` IL_0028-0042).
///
/// Live engine, seed `PROBE2696A` with the second Toadpole at 6 HP: the
/// player still owns `VigorPower 8` when the combat is over.
#[test]
fn a_nested_strike_that_ends_the_combat_leaves_the_vigor() {
    let mut value = root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]);
    value["monsters"][1]["hp"] = 6.into();
    let loaded = load(value);
    let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert_eq!(hits(&events), [(0, 11), (1, 6)]);
    assert!(state.history.over);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::CombatOver { player_won: true }))
    );
    assert!(vigor_writes(&events).is_empty());
    assert_eq!(state.powers.value(PowerId::Vigor), 5);
}

/// The nested Strike's own target has Thorns and the retaliation kills the
/// owner. The nested hit still lands, without Vigor, and neither command's
/// AfterAttack consumes: the owner's hooks are gone. `main` landed 11.
///
/// Live engine, seed `PROBE2696A` with the player at 2 HP and Thorns 2 on
/// the second Toadpole: the Thorns result kills the player and the second
/// Toadpole ends at 94 of 100. No `PowerReceived` row follows.
#[test]
fn a_nested_strike_whose_thorns_kills_the_owner_lands_without_vigor() {
    let mut value = root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]);
    value["player"]["hp"] = 2.into();
    value["monsters"][1]["thorns"] = 2.into();
    let loaded = load(value);
    let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert_eq!(hits(&events), [(0, 11), (1, 6)]);
    assert_eq!(hp(&state, 1), 94);
    assert_eq!(state.hp, 0);
    assert!(state.history.over);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::CombatOver { player_won: false }))
    );
    assert!(vigor_writes(&events).is_empty());
}

/// Gigantification follows the same contract and already did: the nested
/// Strike is not the bound command's card (`GigantificationPower::
/// ModifyDamageMultiplicative` RVA `0xa2ebc` IL_0045-IL_005c), is not bound
/// itself (`BeforeAttack` RVA `0xa2e3c` IL_005c-IL_0069), and the outer
/// command's AfterAttack takes one stack (`<AfterAttack>d__8::MoveNext` RVA
/// `0x33b668` IL_0024-IL_003c).
///
/// Live engine, seed `PROBE2696D`, one and two Gigantification Potions: the
/// drawn Strike's result is 6 and one `PowerReceived GigantificationPower -1`
/// follows; with two stacks one is left.
#[test]
fn gigantification_multiplies_the_bound_command_only_and_takes_one_stack() {
    for stacks in [1, 2] {
        let mut value = root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]);
        value["monsters"][0]["hp"] = 18.into();
        let player = value["player"].as_object_mut().unwrap();
        player.remove("vigor");
        player.insert("gigantification".into(), stacks.into());
        let loaded = load(value);
        let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
        assert_eq!(hits(&events), [(0, 18), (1, 6)], "{stacks}");
        assert_eq!(hp(&state, 1), 94);
        assert_eq!(
            gigantification(&state, &loaded.catalog),
            (stacks - 1, false)
        );
        assert_cold_equal(&state, &loaded.catalog);
    }
}

/// Vigor and Gigantification bound to the same outer command: 3 x (6 + 5)
/// on the target, a bare 6 from the nested Strike.
#[test]
fn vigor_and_gigantification_stay_with_the_same_outer_command() {
    let mut value = root(&["GREMLIN_HORN"], &[STRIKE], &[STRIKE]);
    value["monsters"][0]["hp"] = 26.into();
    value["player"]["gigantification"] = 1.into();
    let loaded = load(value);
    let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert_eq!(hits(&events), [(0, 33), (1, 6)]);
    assert_eq!(state.powers.value(PowerId::Vigor), 0);
    assert_eq!(gigantification(&state, &loaded.catalog), (0, false));
}

/// The other way a command nests: the target's Thorns costs the player HP
/// inside the outer hit, Centennial Puzzle draws three cards there, and
/// Hellraiser plays each Strike before the outer hit commits
/// (`ThornsPower/<BeforeDamageReceived>d__4` RVA `0x349954`,
/// `CentennialPuzzle/<AfterDamageReceived>d__10` RVA `0x321758`). Each nested
/// Strike deals 6 wherever it lands; the outer hit then lands its 11.
/// `main` gave the first nested Strike the Vigor as well.
///
/// Live engine, seed `PROBE2696G` (Vigor 8, Thorns 2 on the target): three
/// nested results of 6, then the outer 14, then one
/// `PowerReceived VigorPower -8`.
#[test]
fn strikes_drawn_by_the_puzzle_inside_a_thorns_hit_read_no_vigor() {
    let mut value = root(
        &["CENTENNIAL_PUZZLE"],
        &[STRIKE],
        &[STRIKE, STRIKE, STRIKE, "DEFEND_IRONCLAD"],
    );
    value["monsters"][0]["hp"] = 26.into();
    value["monsters"][0]["thorns"] = 2.into();
    let loaded = load(value);
    let (state, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    let hits = hits(&events);
    assert_eq!(hits.len(), 4, "{events:?}");
    assert!(hits[..3].iter().all(|(_, unblocked)| *unblocked == 6));
    assert_eq!(hits[3], (0, 11));
    assert_eq!(vigor_writes(&events), [0]);
    assert_eq!(state.powers.value(PowerId::Vigor), 0);
    assert_cold_equal(&state, &loaded.catalog);
}

/// A selecting Strike drawn by the Horn: Seeker Strike attacks (nested, no
/// Vigor) and then begins its choice, which native defers to a queued hook
/// action behind the enclosing play. So the outer command has consumed its
/// Vigor by the time the choice is published, and the parked state loads
/// cold. Every answer then finishes the Seeker Strike without touching
/// Vigor again.
#[test]
fn a_selecting_strike_under_the_horn_parks_after_the_outer_command_consumed() {
    let loaded = load(root(
        &["GREMLIN_HORN"],
        &[STRIKE],
        &["SEEKER_STRIKE", "DEFEND_IRONCLAD", "BASH", STRIKE],
    ));
    let (parked, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert!(parked.pending.is_some(), "{events:?}");
    let hits = hits(&events);
    // Seeker Strike's DamageVar is 9 (`get_CanonicalVars` RVA `0xea710`).
    assert_eq!(hits, [(0, 11), (1, 9)]);
    assert_eq!(vigor_writes(&events), [0]);
    assert_eq!(parked.powers.value(PowerId::Vigor), 0);
    assert_cold_equal(&parked, &loaded.catalog);

    let answers = engine::legal_actions(&parked, &loaded.catalog);
    assert!(!answers.is_empty());
    for answer in answers {
        let mut events = Vec::new();
        let done =
            engine::apply_action_into(&parked, &loaded.catalog, &answer, &mut events).unwrap();
        assert!(done.pending.is_none());
        assert!(vigor_writes(&events).is_empty());
        assert_eq!(hp(&done, 1), 91);
        assert_cold_equal(&done, &loaded.catalog);
    }
}

/// A selecting Strike drawn by the Puzzle parks INSIDE the outer command:
/// the Puzzle's Draw is awaited by the hit, so the choice is shown with the
/// outer hit uncommitted and the Vigor still bound. The park is carried by
/// the action's replay receipt, which re-executes the play with the answer;
/// the binding is rebuilt by that re-execution, hot or from a cold load of
/// the parked document.
#[test]
fn a_selecting_strike_under_the_puzzle_parks_inside_the_bound_command() {
    let mut value = root(
        &["CENTENNIAL_PUZZLE"],
        &[STRIKE],
        &[
            "SEEKER_STRIKE",
            "DEFEND_IRONCLAD",
            "BASH",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
            "DEFEND_IRONCLAD",
        ],
    );
    value["monsters"][0]["hp"] = 26.into();
    value["monsters"][0]["thorns"] = 2.into();
    let loaded = load(value);
    let (parked, events) = run(&loaded, PLAY_THE_STRIKE_AT_THE_WEAK_TOADPOLE);
    assert!(parked.pending.is_some(), "{events:?}");
    // Parked mid-command: the bound Vigor is still there, as in native.
    assert_eq!(parked.powers.value(PowerId::Vigor), 5);
    let (reloaded, _) = cold(&parked, &loaded.catalog);

    let answers = engine::legal_actions(&parked, &loaded.catalog);
    assert!(!answers.is_empty());
    assert_eq!(
        answers,
        engine::legal_actions(&reloaded.state, &reloaded.catalog)
    );
    for answer in answers {
        let mut events = Vec::new();
        let done =
            engine::apply_action_into(&parked, &loaded.catalog, &answer, &mut events).unwrap();
        let mut cold_events = Vec::new();
        let cold_done = engine::apply_action_into(
            &reloaded.state,
            &reloaded.catalog,
            &answer,
            &mut cold_events,
        )
        .unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&done, &loaded.catalog).unwrap(),
            HotBoundary::try_to_canonical(&cold_done, &reloaded.catalog).unwrap()
        );
        assert_eq!(events, cold_events);
        assert!(done.pending.is_none(), "{events:?}");
        assert_eq!(done.powers.value(PowerId::Vigor), 0);
        // The outer hit landed with its Vigor, after the nested Seeker
        // Strike landed without.
        let left: i32 = done.monsters.iter().map(|monster| monster.hp).sum();
        assert_eq!(
            26 + 100 - left,
            11 + 9,
            "outer 6 + 5 plus Seeker Strike's bare 9"
        );
        assert_cold_equal(&done, &loaded.catalog);
    }
}

//! #3671: Lethality is read per damage instance, not once per attack command.
//!
//! `LethalityPower::ModifyDamageMultiplicative` RVA `0xa4634` (v0.111.0
//! `sts2.dll`, SHA-256 `9cb4f1ad…fbf12b4`) counts the owner's Attack
//! `CardPlayStarted` rows of this turn (IL_0066-IL_0086, filter
//! `<ModifyDamageMultiplicative>b__4_0` RVA `0xa470a`) every time it is
//! asked, and `CreatureCmd/<Damage>d__12::MoveNext` RVA `0x3e96c8` asks once
//! per receiver of every hit (`Hook::ModifyDamage` at IL_01b2). A card that
//! Hellraiser AutoPlays inside an open attack command
//! (`HellraiserPower/<AfterCardDrawnEarly>d__7::MoveNext` RVA `0x33c1a8`) is a
//! second such row, so the command's later damage instances read One.
//!
//! Every fixture is the KD13JGCDPB3U Toadpoles entry with the piles, the
//! Toadpoles' HP and the player's powers and relics replaced. Each passes
//! `admit_exact_solve` and `engine::admit` and is driven through
//! `apply_action_into`.
//!
//! The live engine agrees (headless harness, build v0.111.0 `41cef1ea`,
//! Ironclad against `TOADPOLES_WEAK`, `LethalityPower` 50 applied through
//! `PowerCmd.Apply`, Hellraiser played from hand); each test names its probe.
use serde_json::{Value, json};
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, Event},
    exact_solve_v1::ExactSolveActionV1,
    hot::HotState,
    solo_v1::admit_exact_solve,
};

const STRIKE: &str = "STRIKE_IRONCLAD";
const DEFEND: &str = "DEFEND_IRONCLAD";

fn card(id: &str, uid: u32) -> Value {
    json!({"id": id, "uid": uid, "upgrade": 0})
}

/// Lethality 50, Hellraiser 1 unless `hellraiser` is false, the Toadpoles at
/// 5 and 100 HP, `hand` (uids from 0) in Hand and `draw` (the following uids)
/// in the Draw pile.
fn root(hellraiser: bool, relics: &[&str], hand: &[&str], draw: &[&str]) -> Value {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    let player = &mut value["player"];
    player["hp"] = 100.into();
    player["max_hp"] = 100.into();
    player["lethality"] = 50.into();
    if hellraiser {
        player["hellraiser"] = 1.into();
        player["after_side_turn_end_power_order"] = json!([["hellraiser", 0]]);
        player["next_after_side_turn_end_power_uid"] = 1.into();
    }
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

/// The Thorns shape: the first Toadpole at 100 HP with Thorns 2.
fn thorny(mut value: Value) -> Value {
    value["monsters"][0]["hp"] = 100.into();
    value["monsters"][0]["max_hp"] = 100.into();
    value["monsters"][0]["thorns"] = 2.into();
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
/// point of the boundary.
fn cold(state: &HotState, catalog: &Catalog) -> Loaded {
    let wire = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    engine::admit(&wire, &cold, &cold_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
    Loaded {
        state: cold,
        catalog: cold_catalog,
    }
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

const PLAY_THE_FIRST_CARD: &str = r#"[{"kind":"play","uid":0}]"#;
const PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE: &str = r#"[{"kind":"play","uid":0,"target":0}]"#;

/// The issue's reproduction. Whirlwind at X = 3: the first hit kills the weak
/// Toadpole and deals 7 to the other (5 x 1.5, floored), Gremlin Horn draws
/// the Strike, Hellraiser plays it (6: it is the second Attack started this
/// turn), and the two remaining hits read a count of two and deal a bare 5.
/// `main` froze the multiplier at the command's entry and dealt 7 and 7,
/// leaving the survivor at 73.
///
/// Live engine, seed `PROBE2696B2` (the issue's log): 5 (killed), 7, the
/// Strike's 6, 5, 5; the survivor at 77 of 100.
#[test]
fn hits_after_a_nested_hellraiser_strike_read_no_lethality() {
    let loaded = load(root(true, &["GREMLIN_HORN"], &["WHIRLWIND"], &[STRIKE]));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD);
    assert_eq!(
        hits(&events),
        [(0, 7), (1, 7), (1, 6), (1, 5), (1, 5)],
        "{events:?}"
    );
    assert_eq!(hp(&state, 1), 77);
    assert_eq!(state.history.owner_attack_plays_started_this_turn, 2);
    cold(&state, &loaded.catalog);
}

/// No nested play: the Horn only draws the Strike, the count stays at one,
/// and every hit of the first Attack keeps Lethality. Unchanged from `main`.
#[test]
fn a_multi_hit_attack_with_no_nested_play_keeps_lethality_on_every_hit() {
    let loaded = load(root(false, &["GREMLIN_HORN"], &["WHIRLWIND"], &[STRIKE]));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD);
    assert_eq!(
        hits(&events),
        [(0, 7), (1, 7), (1, 7), (1, 7)],
        "{events:?}"
    );
    assert_eq!(hp(&state, 1), 79);
    assert_eq!(state.history.owner_attack_plays_started_this_turn, 1);
    cold(&state, &loaded.catalog);
}

/// The nested play starts after the command's last hit: nothing is left to
/// lose it. The outer Strike's 9 kills, the drawn Strike deals 6. Unchanged
/// from `main`.
#[test]
fn a_strike_nested_after_the_last_hit_changes_nothing() {
    let loaded = load(root(true, &["GREMLIN_HORN"], &[STRIKE], &[STRIKE]));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE);
    assert_eq!(hits(&events), [(0, 9), (1, 6)]);
    assert_eq!(hp(&state, 1), 94);
    cold(&state, &loaded.catalog);
}

/// A nested play inside a hit, behind the receiver's Thorns: Centennial
/// Puzzle draws three cards in the hit's `Hook.BeforeDamageReceived`
/// (`0x3e96c8` IL_0261), which is after that hit's `Hook.ModifyDamage`
/// (IL_01b2). So the hit that triggered the Strikes keeps its Lethality, and
/// each nested Strike, a later Attack play, reads none. Unchanged from `main`.
#[test]
fn the_hit_whose_thorns_nests_the_strikes_keeps_its_lethality() {
    let loaded = load(thorny(root(
        true,
        &["CENTENNIAL_PUZZLE"],
        &[STRIKE],
        &[STRIKE, STRIKE, STRIKE, DEFEND],
    )));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE);
    let hits = hits(&events);
    assert_eq!(hits.len(), 4, "{events:?}");
    assert!(hits[..3].iter().all(|(_, unblocked)| *unblocked == 6));
    assert_eq!(hits[3], (0, 9));
    assert_eq!(state.history.owner_attack_plays_started_this_turn, 4);
    cold(&state, &loaded.catalog);
}

/// The same nesting between two hits of one command. Twin Strike's first hit
/// was computed before its Thorns ran (7), its second after the Strike's
/// `CardPlayStarted` row (5). `main` dealt 7 and 7.
///
/// Live engine, seed `PROBE3671A1` (Thorns 2 on the target, Centennial
/// Puzzle): the Thorns result, the nested Strike's 6, then Twin Strike's 7,
/// a second Thorns result and 5; the target at 82 of 100.
#[test]
fn a_strike_nested_inside_the_first_hit_takes_lethality_from_the_second() {
    let loaded = load(thorny(root(
        true,
        &["CENTENNIAL_PUZZLE"],
        &["TWIN_STRIKE"],
        &[STRIKE, DEFEND, DEFEND, DEFEND],
    )));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE);
    let hits = hits(&events);
    assert_eq!(hits.len(), 3, "{events:?}");
    assert_eq!(hits[0].1, 6);
    assert_eq!(hits[1..], [(0, 7), (0, 5)]);
    let left: i32 = state.monsters.iter().map(|monster| monster.hp).sum();
    assert_eq!(200 - left, 6 + 7 + 5);
    cold(&state, &loaded.catalog);
}

/// Without Hellraiser the Puzzle only draws: both hits keep Lethality.
/// Unchanged from `main`.
#[test]
fn a_draw_inside_the_first_hit_that_plays_nothing_changes_nothing() {
    let loaded = load(thorny(root(
        false,
        &["CENTENNIAL_PUZZLE"],
        &["TWIN_STRIKE"],
        &[STRIKE, DEFEND, DEFEND, DEFEND],
    )));
    let (_, events) = run(&loaded, PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE);
    assert_eq!(hits(&events), [(0, 7), (0, 7)]);
}

/// One `CreatureCmd.Damage` over two receivers reads Lethality once per
/// receiver, in receiver order (`0x3e96c8` IL_01b2 inside the `targetList`
/// walk, IL_0261 before the next receiver). The first Toadpole's Thorns nests
/// the Strike, so the second receiver of the SAME hit already reads a count
/// of two: Whirlwind at X = 3 deals 7 and 5, then 5 and 5 twice. `main` dealt
/// 7 to all six.
///
/// Live engine, seed `B3671XK` (Thorns 2 on the first Toadpole, Centennial
/// Puzzle, an Energy Potion first): the Thorns result, the nested Strike's 6,
/// then 7 and 5, 5 and 5, 5 and 5; the Toadpoles at 77 and 85 of 100.
#[test]
fn the_second_receiver_of_a_hit_reads_lethality_after_the_first_receivers_thorns() {
    let loaded = load(thorny(root(
        true,
        &["CENTENNIAL_PUZZLE"],
        &["WHIRLWIND"],
        &[STRIKE, DEFEND, DEFEND, DEFEND],
    )));
    let (state, events) = run(&loaded, PLAY_THE_FIRST_CARD);
    let hits = hits(&events);
    assert_eq!(hits.len(), 7, "{events:?}");
    assert_eq!(hits[0].1, 6, "the nested Strike lands first");
    let nested = hits[0].0;
    assert_eq!(
        hits[1..],
        [(0, 7), (1, 5), (0, 5), (1, 5), (0, 5), (1, 5)],
        "{events:?}"
    );
    let strike = |uid: u32| if nested == uid { 6 } else { 0 };
    assert_eq!(hp(&state, 0), 100 - 17 - strike(0));
    assert_eq!(hp(&state, 1), 100 - 15 - strike(1));
    cold(&state, &loaded.catalog);
}

/// A nested selection parks inside the command. Seeker Strike, drawn by the
/// Puzzle inside Twin Strike's first hit, attacks and then publishes its
/// choice with the outer hit uncommitted. The park is carried by the action's
/// replay receipt, which re-executes the play with the answer, so every
/// per-hit read happens again in the same order: hot and from a cold load of
/// the parked document the results are identical, the first hit keeps its
/// Lethality and the second has lost it.
#[test]
fn a_selection_parked_inside_the_command_replays_the_per_hit_reads() {
    let loaded = load(thorny(root(
        true,
        &["CENTENNIAL_PUZZLE"],
        &["TWIN_STRIKE"],
        &["SEEKER_STRIKE", DEFEND, "BASH", DEFEND, DEFEND, DEFEND],
    )));
    let (parked, events) = run(&loaded, PLAY_THE_FIRST_CARD_AT_THE_FIRST_TOADPOLE);
    assert!(parked.pending.is_some(), "{events:?}");
    let reloaded = cold(&parked, &loaded.catalog);

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
        // Seeker Strike's DamageVar is 9 (`get_CanonicalVars` RVA `0xea710`).
        let left: i32 = done.monsters.iter().map(|monster| monster.hp).sum();
        assert_eq!(200 - left, 9 + 7 + 5, "{events:?}");
        let outer: Vec<i32> = hits(&events)
            .into_iter()
            .map(|(_, unblocked)| unblocked)
            .filter(|unblocked| *unblocked != 9)
            .collect();
        assert_eq!(outer, [7, 5], "{events:?}");
        cold(&done, &loaded.catalog);
    }
}

/// The boundary the nested play crosses is "a second owner Attack started
/// this turn", and it is the same one a sequential play crosses: Twin Strike
/// played first keeps Lethality on both hits, and the Strike after it reads
/// none. Unchanged from `main`.
#[test]
fn only_the_first_attack_of_the_turn_reads_lethality() {
    let mut value = root(false, &[], &["TWIN_STRIKE", STRIKE], &[DEFEND]);
    value["monsters"][0]["hp"] = 100.into();
    value["monsters"][0]["max_hp"] = 100.into();
    let loaded = load(value);
    let (state, events) = run(
        &loaded,
        r#"[{"kind":"play","uid":0,"target":0},{"kind":"play","uid":1,"target":0}]"#,
    );
    assert_eq!(hits(&events), [(0, 7), (0, 7), (0, 6)]);
    assert_eq!(state.history.owner_attack_plays_started_this_turn, 2);
}

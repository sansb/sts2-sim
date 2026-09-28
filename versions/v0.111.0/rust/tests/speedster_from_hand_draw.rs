//! #3047: SpeedsterPower damages on every Draw EXCEPT the turn-start hand draw.
//!
//! v0.111.0 `SpeedsterPower/<AfterCardDrawn>d__4::MoveNext` RVA `0x345abc`
//! IL_0020-0028 leaves when `fromHandDraw` is true. The only `fromHandDraw`
//! true caller in the DLL is `CombatManager.SetupPlayerTurn`
//! (`<SetupPlayerTurn>d__102::MoveNext` RVA `0x3f6c6c` IL_03cd); every card
//! body (Escape Plan, Backflip, Prepared, ...) passes false.

use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
    hot::HotState,
};

const SPEEDSTER: i64 = 2;

fn entry(speedster: bool) -> CanonicalStateV2 {
    let mut doc: CanonicalStateV2 = serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap();
    doc.player = serde_json::from_value(json!({
        "hp":100,"max_hp":100,"player_phase":3,"next_card_uid":20
    }))
    .unwrap();
    if speedster {
        doc.player.insert("speedster".into(), json!(SPEEDSTER));
        doc.player
            .insert("after_card_drawn_power_order".into(), json!(["speedster"]));
    }
    doc.monsters = serde_json::from_value(json!([
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2},
        {"kind":"TOADPOLE","hp":100,"max_hp":100,"loop_pos":2}
    ]))
    .unwrap();
    doc
}

fn apply(doc: &CanonicalStateV2, action: &Action) -> HotState {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let mut next = engine::apply_action(&state, &catalog, action)
        .unwrap()
        .state;
    // Resolve any parked selection (Prepared's discard) with its first answer.
    while !next.history.over {
        let choices = engine::legal_actions(&next, &catalog);
        if choices.is_empty() || choices.iter().any(|a| matches!(a, Action::EndTurn)) {
            break;
        }
        next = engine::apply_action(&next, &catalog, &choices[0])
            .unwrap()
            .state;
    }
    next
}

fn play(doc: &CanonicalStateV2) -> HotState {
    apply(
        doc,
        &Action::Play {
            uid: 0,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
}

fn hps(state: &HotState) -> Vec<i32> {
    state.monsters.iter().map(|monster| monster.hp).collect()
}

#[test]
fn card_body_draws_fire_speedster_on_every_monster() {
    // (card, cards drawn by its body at L0)
    for (id, drawn) in [("ESCAPE_PLAN", 1), ("BACKFLIP", 2), ("PREPARED", 1)] {
        let piles = json!({
            "hand": [{"id":id,"uid":0,"upgrade":0}],
            "draw": [
                {"id":"DEFEND_SILENT","uid":1,"upgrade":0},
                {"id":"DEFEND_SILENT","uid":2,"upgrade":0},
                {"id":"DEFEND_SILENT","uid":3,"upgrade":0}
            ],
            "discard": [],
        });
        let mut control = entry(false);
        control.piles = serde_json::from_value(piles.clone()).unwrap();
        let mut doc = entry(true);
        doc.piles = serde_json::from_value(piles).unwrap();

        let without = play(&control);
        let with = play(&doc);
        assert_eq!(hps(&without), vec![100, 100], "{id}: control is untouched");
        let expected = 100 - i32::try_from(SPEEDSTER * drawn).unwrap();
        assert_eq!(
            hps(&with),
            vec![expected, expected],
            "{id}: each Command draw hits every monster"
        );
        assert_eq!(with.cards_drawn_combat, without.cards_drawn_combat);
    }
}

#[test]
fn turn_start_hand_draw_does_not_fire_speedster() {
    let piles = json!({
        "hand": [],
        "draw": [
            {"id":"DEFEND_SILENT","uid":1,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":2,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":3,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":4,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":5,"upgrade":0},
            {"id":"DEFEND_SILENT","uid":6,"upgrade":0}
        ],
        "discard": [],
    });
    let mut control = entry(false);
    control.piles = serde_json::from_value(piles.clone()).unwrap();
    let mut doc = entry(true);
    doc.piles = serde_json::from_value(piles).unwrap();

    let without = apply(&control, &Action::EndTurn);
    let with = apply(&doc, &Action::EndTurn);
    assert!(
        with.piles.get(sts_sim::hot::PileId::Hand).len() >= 5,
        "the next turn's hand draw ran"
    );
    assert_eq!(
        with.cards_drawn_combat, without.cards_drawn_combat,
        "same draws in both runs"
    );
    assert_eq!(
        with.monsters, without.monsters,
        "the fromHandDraw draw never fires Speedster"
    );
}

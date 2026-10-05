//! #3679: a card that loses HP beside Rupture pays the same Strength whether
//! or not the fight's closure is replay-capable.
//!
//! `RupturePower/<AfterDamageReceived>d__9::MoveNext` RVA `0x342fec`
//! (v0.111.0 `sts2.dll`, SHA-256 `9cb4f1ad…`) adds the live Amount to the
//! played card's `playedCards` entry at IL_00fc-IL_0122 when the entry exists
//! (IL_0063-IL_0081), and `<AfterCardPlayed>d__10::MoveNext` RVA `0x342ec4`
//! removes the entry and applies its total at IL_0042-IL_0080. Neither reads
//! anything about how the play is carried.
//!
//! The document is the KD13JGCDPB3U Toadpoles entry with Rupture,
//! Bloodletting, Blood Wall and Burning Pact put beside the Stoke already in
//! the opening hand. Burning Pact selects and Stoke is a batch producer, so
//! the document's own closure runs every play under a frozen CardPlay
//! parent. The control swaps Burning Pact for a Strike, which leaves the
//! closure on the ordinary path. Nothing else differs, and neither swapped
//! card is played.
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action},
    exact_solve_v1::ExactSolveActionV1,
    ids::PowerId,
    solo_v1::admit_exact_solve,
};

fn run(selecting: bool, line: &str) -> (i32, Vec<(i64, i64, i64, i32)>) {
    let entry = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/search_refused_branch_toadpoles.json"
    );
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(entry).unwrap()).unwrap();
    value["piles"]["hand"][0]["id"] = "RUPTURE".into();
    value["piles"]["hand"][1]["id"] = "BLOODLETTING".into();
    value["piles"]["hand"][2]["id"] = "BLOOD_WALL".into();
    value["piles"]["hand"][3]["id"] = if selecting {
        "BURNING_PACT".into()
    } else {
        "STRIKE_IRONCLAD".into()
    };
    let document: CanonicalStateV2 = serde_json::from_value(value).unwrap();
    admit_exact_solve(&document).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
    engine::admit(&document, &state, &catalog).unwrap();
    let actions: Vec<ExactSolveActionV1> = serde_json::from_str(line).unwrap();
    let mut seen = Vec::new();
    for (index, action) in actions.into_iter().enumerate() {
        let action: Action = action.try_into().unwrap();
        state = engine::apply_action_into(&state, &catalog, &action, &mut Vec::new())
            .unwrap_or_else(|refusal| panic!("action {index} refused: {refusal:?}"));
        seen.push((
            i64::from(state.hp),
            i64::from(state.energy),
            i64::from(state.block),
            state.powers.value(PowerId::Strength),
        ));
    }
    (state.powers.value(PowerId::Strength), seen)
}

#[test]
fn an_hp_losing_card_pays_rupture_under_the_documents_own_replay_closure() {
    for line in [
        r#"[{"kind":"play","uid":0},{"kind":"play","uid":1},{"kind":"play","uid":2}]"#,
        r#"[{"kind":"play","uid":0},{"kind":"play","uid":2},{"kind":"play","uid":1}]"#,
    ] {
        let (strength, replay_capable) = run(true, line);
        let (control_strength, ordinary) = run(false, line);
        assert_eq!(control_strength, 2, "one Strength per HP-losing card");
        assert_eq!(
            strength, 2,
            "one Strength per HP-losing card: {replay_capable:?}"
        );
        assert_eq!(replay_capable, ordinary, "{line}");
    }
}

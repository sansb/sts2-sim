//! #3877 — Curl Up fires for a card Razor Tooth has just upgraded.
//!
//! The composition sweep met `CurlUp physical card source` on the first
//! Attack of `LOUSE_PROGENITOR_NORMAL & RELIC.RAZOR_TOOTH`, for every
//! character. `CurlUpPower::AfterDamageReceived` (RVA `0xa10cc`
//! IL_0056-IL_005e) remembers the card that dealt the powered hit, and
//! `CurlUpPower/<AfterCardPlayed>d__11::MoveNext` (`0x3384d0` IL_002c-IL_0042)
//! compares `cardPlay.Card` with it by reference before granting Block and
//! removing the power. `RazorTooth::AfterCardPlayed` (`0x9a1b8` IL_0060)
//! upgrades that same object earlier in the walk. The engine compared the
//! live card's spec with the spec the play ran with, which the upgrade had
//! just changed.
//!
//! The fixture is the document the sweep's root reached after one Defend
//! (already upgraded by Razor Tooth): a Defect with four Strikes in Hand
//! against a Louse Progenitor holding Curl Up 18. The `native_*` tests
//! compare the engine with the game itself.

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, SelectionRef},
    hot::{HotState, PileId},
    ids::PowerId,
};

const FIXTURE: &str = include_str!("../fixtures/issue3877_louse_progenitor_razor_tooth.json");

/// The game's own runs, recorded by `tests/native/probe.py` from
/// `tests/native/issue3877_curl_up_razor_tooth.spec.json` on the archived
/// v0.111.0 `sts2.dll` (headless harness): an Ironclad against
/// `ENCOUNTER.LOUSE_PROGENITOR_NORMAL` plays a Strike (or a Twin Strike) and
/// then a Strike, with Razor Tooth and without.
const NATIVE_RAZOR: &str = include_str!("native/curl_up_razor_tooth.native.json");
const NATIVE_RAZOR_TWIN: &str = include_str!("native/curl_up_razor_tooth_twin.native.json");
const NATIVE_PLAIN: &str = include_str!("native/curl_up_plain.native.json");

/// One play against the Louse, in terms both sides can state.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    hp_lost: i64,
    /// The Louse's Block after the play, as a share of its Curl Up amount:
    /// `(block, curl up amount at the root)`.
    block_of_curl: (i64, i64),
    curl_up_left: bool,
    /// The played card's upgrade level where it lies afterwards.
    played_upgrade: i64,
    played_in_discard: bool,
}

fn native_steps(recorded: &str) -> Vec<Step> {
    let all: serde_json::Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(all["build"], "v0.111.0");
    let trace = all["runs"][0]["trace"].as_array().unwrap();
    let louse = |entry: &serde_json::Value| entry["snap"]["creatures"][1].clone();
    let curl = |louse: &serde_json::Value| {
        louse["powers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|power| power[0] == "CurlUpPower")
            .map(|power| power[1].as_i64().unwrap())
    };
    let root_curl = curl(&louse(&trace[0])).unwrap();
    let mut steps = Vec::new();
    let mut before = louse(&trace[0]);
    for entry in &trace[1..] {
        let label = entry["label"].as_str().unwrap();
        let Some(card) = label
            .strip_suffix(":done")
            .and_then(|rest| rest.split_once(':'))
            .map(|(_, card)| card)
        else {
            continue;
        };
        assert_eq!(entry["state"], "Finished");
        let after = louse(entry);
        let discard: Vec<&str> = entry["snap"]["piles"]["DiscardPile"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| card.as_str().unwrap())
            .collect();
        // The card just played is the last one in Discard.
        let landed = discard.last().copied().unwrap_or("");
        steps.push(Step {
            hp_lost: before["hp"].as_i64().unwrap() - after["hp"].as_i64().unwrap(),
            block_of_curl: (after["block"].as_i64().unwrap(), root_curl),
            curl_up_left: curl(&after).is_some(),
            played_upgrade: landed.matches('+').count() as i64,
            played_in_discard: landed.trim_end_matches('+') == card,
        });
        before = after;
    }
    steps
}

/// The engine's plays of the same cards: the fixture's first Strike becomes
/// `first`, Razor Tooth is kept or dropped, and `first` then a Strike are
/// played on the Louse.
fn engine_steps(first: &str, razor_tooth: bool) -> Vec<Step> {
    let mut doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    doc.piles.get_mut("hand").unwrap()[0].id = first.to_string();
    if !razor_tooth {
        doc.player.insert(
            "relics_entering".to_string(),
            serde_json::json!(["RELIC.CRACKED_CORE"]),
        );
        doc.player.remove("razor_tooth");
    }
    // Both plays have to be affordable; the native scenario sets 9 too.
    doc.player.insert("energy".to_string(), 9.into());
    let catalog: Catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let mut state: HotState = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    let root_curl = i64::from(state.monsters[0].powers.value(PowerId::CurlUp));
    let mut steps = Vec::new();
    for uid in [0, 1] {
        let next = engine::apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let landed = next
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .find(|card| card.uid == uid)
            .and_then(|card| catalog.spec(card.atom));
        steps.push(Step {
            hp_lost: i64::from(state.monsters[0].hp) - i64::from(next.monsters[0].hp),
            block_of_curl: (i64::from(next.monsters[0].block), root_curl),
            curl_up_left: next.monsters[0].powers.value(PowerId::CurlUp) > 0,
            played_upgrade: landed.map_or(-1, |spec| i64::from(spec.identity.upgrade)),
            played_in_discard: landed.is_some(),
        });
        state = next;
    }
    steps
}

/// The two sides start from different Curl Up amounts (the game's Louse here
/// has 14, the fixture's 18), so Block is compared as what is left of it.
fn normalized(steps: Vec<Step>) -> Vec<(i64, i64, bool, i64, bool)> {
    steps
        .into_iter()
        .map(|step| {
            (
                step.hp_lost,
                step.block_of_curl.1 - step.block_of_curl.0,
                step.curl_up_left,
                step.played_upgrade,
                step.played_in_discard,
            )
        })
        .collect()
}

/// The play that refused. The Strike hits for 6 through no Block, Curl Up
/// grants its full amount and leaves, and the Strike is upgraded in Discard;
/// the next Strike takes 6 off that Block.
#[test]
fn native_curl_up_fires_for_a_strike_razor_tooth_upgraded() {
    let native = normalized(native_steps(NATIVE_RAZOR));
    assert_eq!(
        native,
        [(6, 0, false, 1, true), (0, 6, false, 1, true)],
        "the game: full Block after the first Strike, both Strikes upgraded"
    );
    assert_eq!(normalized(engine_steps("STRIKE_DEFECT", true)), native);
}

/// A two-hit card: Curl Up waits for the card to finish, so both hits land
/// on no Block, and the upgraded card is still the remembered one.
#[test]
fn native_curl_up_waits_for_both_hits_of_an_upgraded_twin_strike() {
    let native = normalized(native_steps(NATIVE_RAZOR_TWIN));
    assert_eq!(native[0], (10, 0, false, 1, true));
    assert_eq!(normalized(engine_steps("TWIN_STRIKE", true)), native);
}

/// The control without Razor Tooth: the same Block, and no upgrade.
#[test]
fn native_curl_up_without_razor_tooth_matches() {
    let native = normalized(native_steps(NATIVE_PLAIN));
    assert_eq!(native, [(6, 0, false, 0, true), (0, 6, false, 0, true)]);
    assert_eq!(normalized(engine_steps("STRIKE_DEFECT", false)), native);
}

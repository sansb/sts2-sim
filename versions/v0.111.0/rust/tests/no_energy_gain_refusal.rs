//! The Python oracle models NoEnergyGain. Rust can hydrate and authenticate its
//! native side-end object, but still names it at admission instead of silently
//! approximating the local Energy-gain modifier surface.

use serde_json::json;
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::engine::admission::{MissingCapability, admit};

const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

#[test]
fn live_no_energy_gain_is_hydratable_but_named_at_admission() {
    let mut document: CanonicalStateV2 =
        serde_json::from_str(FIXTURE).expect("canonical fixture parses");
    document
        .player
        .insert("no_energy_gain".to_owned(), json!(true));
    document.player.insert(
        "after_side_turn_end_power_order".to_owned(),
        json!([["no_energy_gain", 0]]),
    );
    document
        .player
        .insert("next_after_side_turn_end_power_uid".to_owned(), json!(1));
    document
        .player
        .insert("potion_slots".to_owned(), json!([null]));
    document
        .player
        .insert("fully_unlocked_potion_pool".to_owned(), json!(true));
    let catalog =
        HotBoundary::catalog_from_canonical(&document).expect("catalog is independent of power");

    let state = HotBoundary::from_canonical(&document, &catalog)
        .expect("all native side-end power objects are boundary-representable");
    assert_eq!(
        HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
        document
    );
    let refusal = admit(&document, &state, &catalog)
        .expect_err("local NoEnergyGain must stay closed until every gain site is modeled");
    assert_eq!(
        refusal.missing().collect::<Vec<_>>(),
        [MissingCapability::ArgumentShape(
            "NoEnergyGain local GainEnergy modifier",
        )]
    );
}

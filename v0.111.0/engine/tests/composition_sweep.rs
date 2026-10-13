//! #3877 — the composition sweep's self-test.
//!
//! The sweep (`examples/composition_sweep/`, `tools/composition_sweep.py`) is
//! a measurement tool and no workflow runs it. This runs a few dozen of its
//! roots so the tool cannot rot: the synthetic save still opens for every
//! character, each dimension still enumerates what it says, a root's lines do
//! not depend on which slice ran it, and every outcome is one of the stages
//! the aggregator knows.

#[allow(dead_code)]
#[path = "../examples/composition_sweep/sweep.rs"]
mod sweep;

use sts_sim::ids::{CardId, EnchantmentId, EncounterId, PotionId, RelicId};
use sweep::{CHARACTERS, Character, Mode, Play, Root};

const STAGES: [&str; 8] = [
    "entry",
    "opening",
    "boundary",
    "admission",
    "line",
    "stuck",
    "roundtrip",
    "panic",
];

fn play() -> Play {
    Play {
        rollouts: 3,
        depth: 120,
        ..Play::default()
    }
}

fn roots(mode: Mode, character: Character) -> Vec<Root> {
    let mut all = Vec::new();
    sweep::for_each_root(mode, character, &mut |root| {
        all.push(root);
        true
    });
    all
}

/// The starter kit alone opens, admits and plays for every character. Every
/// other row of the sweep is read against this one.
#[test]
fn the_control_root_is_clean_for_every_character() {
    for character in CHARACTERS {
        let control = &roots(Mode::Single, character)[0];
        assert_eq!((control.dim, control.item.as_str()), ("control", "base"));
        let findings = sweep::run_root(character, control, &play());
        assert!(findings.is_empty(), "{}: {findings:?}", character.name);
    }
}

#[test]
fn each_mode_enumerates_the_whole_content_surface() {
    let relics = RelicId::COUNT - 1; // every relic but the starter
    let (cards, potions, encounters) = (CardId::COUNT, PotionId::COUNT, EncounterId::COUNT);
    for character in CHARACTERS {
        let count = |mode, dim: &str| {
            roots(mode, character)
                .iter()
                .filter(|root| root.dim == dim)
                .count()
        };
        assert_eq!(
            count(Mode::Single, "card"),
            sts_sim::content_tables::CARD_ROWS.len()
        );
        assert_eq!(count(Mode::Single, "card_x2"), cards);
        assert_eq!(count(Mode::Single, "relic"), relics);
        assert_eq!(count(Mode::Single, "potion"), potions);
        assert_eq!(count(Mode::Single, "encounter"), encounters);
        assert_eq!(count(Mode::Enchant, "ench"), EnchantmentId::COUNT * cards);
    }
    // The pairs are the same for every character; one is enough to count.
    let pairs = roots(Mode::Pairs, CHARACTERS[0]);
    let count = |dim: &str| pairs.iter().filter(|root| root.dim == dim).count();
    assert_eq!(count("pair_rr"), relics * (relics - 1) / 2);
    assert_eq!(count("pair_rr_rev"), relics * (relics - 1) / 2);
    assert_eq!(count("pair_cc"), cards * (cards - 1) / 2);
    assert_eq!(count("pair_cr"), cards * relics);
    assert_eq!(count("pair_pc"), potions * cards);
    assert_eq!(count("pair_pr"), potions * relics);
    assert_eq!(count("pair_pp"), potions * potions);
    assert_eq!(count("pair_ec"), encounters * cards);
    assert_eq!(count("pair_er"), encounters * relics);
    assert_eq!(count("pair_ep"), encounters * potions);
}

/// A few dozen roots across every dimension: each outcome is a known stage,
/// and running a root twice gives the same lines.
#[test]
fn a_sample_of_every_dimension_runs_and_repeats() {
    let mut ran = 0;
    for (character, mode, stride) in [
        (CHARACTERS[0], Mode::Single, 151),
        (CHARACTERS[3], Mode::Single, 499),
        (CHARACTERS[2], Mode::Enchant, 1637),
        (CHARACTERS[4], Mode::Pairs, 60_013),
    ] {
        for root in roots(mode, character).iter().step_by(stride) {
            let findings = sweep::run_root(character, root, &play());
            for finding in &findings {
                assert!(STAGES.contains(&finding.stage), "{root:?}: {finding:?}");
                assert!(!finding.detail.is_empty(), "{root:?}: {finding:?}");
            }
            assert_eq!(
                findings,
                sweep::run_root(character, root, &play()),
                "{root:?}"
            );
            let line = sweep::outcome_json(7, character, root, &findings);
            assert_eq!(line["i"], 7);
            assert_eq!(line["ch"], character.name);
            assert_eq!(line["stage"] == "ok", findings.is_empty());
            ran += 1;
        }
    }
    assert!(ran >= 36, "only {ran} roots ran");
}

/// A custom root names real content, and builds the same root the sweep
/// would. No refusal is asserted here by name: this test must not fail the
/// day somebody models what a root used to refuse.
#[test]
fn a_custom_root_is_checked_and_matches_the_sweep() {
    let names = |items: &[&str]| -> Vec<String> { items.iter().map(ToString::to_string).collect() };
    let custom = sweep::custom_root(
        &names(&["BASH+1", "SHARP@ANGER"]),
        &names(&["VAJRA", "RELIC.ANCHOR"]),
        &names(&["FIRE_POTION"]),
        Some("CULTISTS_NORMAL"),
    )
    .unwrap();
    assert_eq!(custom.cards.len(), 2);
    assert_eq!(
        (custom.cards[0].id.as_str(), custom.cards[0].upgrade),
        ("BASH", 1)
    );
    assert_eq!(custom.cards[1].enchantment.as_deref(), Some("SHARP"));
    assert_eq!(custom.relics, ["RELIC.VAJRA", "RELIC.ANCHOR"]);
    for bad in [
        sweep::custom_root(&names(&["NOT_A_CARD"]), &[], &[], None),
        sweep::custom_root(&names(&["BASH+x"]), &[], &[], None),
        sweep::custom_root(&[], &names(&["NOT_A_RELIC"]), &[], None),
        sweep::custom_root(&[], &[], &names(&["NOT_A_POTION"]), None),
        sweep::custom_root(&[], &[], &[], Some("NOT_AN_ENCOUNTER")),
    ] {
        assert!(bad.is_err());
    }
    // The same pair, named by hand and enumerated, is the same save.
    let silent = CHARACTERS[1];
    let enumerated = roots(Mode::Pairs, silent)
        .into_iter()
        .find(|root| root.dim == "pair_cr")
        .unwrap();
    let cards: Vec<String> = enumerated
        .cards
        .iter()
        .map(|card| card.id.clone())
        .collect();
    let by_hand = sweep::custom_root(&cards, &enumerated.relics, &[], None).unwrap();
    assert_eq!(
        sweep::save_json(silent, &by_hand),
        sweep::save_json(silent, &enumerated)
    );
}

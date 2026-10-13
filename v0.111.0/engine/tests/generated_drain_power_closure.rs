//! #3877 — a Drain Power a listener mints upgrades cards the catalog knows.
//!
//! The composition sweep's largest mid-line class was `mid-combat card
//! identity ... was not pre-interned`, and 586 of its 662 roots were a
//! Necrobinder: her Hello World draws the Common pool and her Calamity the
//! Attack pool, both hold Drain Power, and the first generated Drain Power
//! that upgraded a Discard card (`DrainPower/<OnPlay>d__3::MoveNext` RVA
//! `0x399f7c`, `CardCmd::Upgrade` IL_0149) stopped the line: the catalog
//! closed upgrade successors for a Drain Power in the deck or from a
//! Crossbow, not for a generated one.
//!
//! These are the sweep's own roots and seeds (`examples/composition_sweep`),
//! so the lines below are the ones that refused: the lone `HELLO_WORLD` root
//! met the refusal on its second line at twelve lines per root, at
//! `Play DRAIN_POWER+0`.

#[allow(dead_code)]
#[path = "../examples/composition_sweep/sweep.rs"]
mod sweep;

use sweep::{Character, Mode, Play, Root};

fn single_root(character: Character, item: &str) -> Root {
    let mut found = None;
    sweep::for_each_root(Mode::Single, character, &mut |root| {
        if root.dim == "card" && root.item == item {
            found = Some(root);
            return false;
        }
        true
    });
    found.unwrap_or_else(|| panic!("the sweep enumerates {item}"))
}

/// Twelve lines per root, the single-card slice's setting.
fn play() -> Play {
    Play {
        rollouts: 12,
        ..Play::default()
    }
}

#[test]
fn a_necrobinder_plays_every_line_of_hello_world_and_calamity() {
    let necrobinder = Character::named("NECROBINDER").unwrap();
    for item in ["HELLO_WORLD+0", "HELLO_WORLD+1", "CALAMITY+0", "CALAMITY+1"] {
        let root = single_root(necrobinder, item);
        let findings = sweep::run_root(necrobinder, &root, &play());
        assert!(findings.is_empty(), "{item}: {findings:?}");
    }
}

/// The control: an owner whose pools hold no upgrade writer was already
/// clean, and stays so.
#[test]
fn an_ironclad_hello_world_is_unchanged() {
    let ironclad = Character::named("IRONCLAD").unwrap();
    let root = single_root(ironclad, "HELLO_WORLD+0");
    let findings = sweep::run_root(ironclad, &root, &play());
    assert!(findings.is_empty(), "{findings:?}");
}

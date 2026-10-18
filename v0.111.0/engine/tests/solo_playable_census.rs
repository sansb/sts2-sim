//! #3877 / #3703 — ten MultiplayerOnly cards one player may play.
//!
//! `CardModel::CanPlay` (RVA `0x7dca8`) tests the player count only for an
//! AnyAlly target (IL_0075-IL_00b1). Beacon of Hope, Blade Symphony, Energy
//! Surge, Gang Up, Glimpse Beyond, Hammer Time, Huddle Up, One for All, Plot and
//! Rally target Self, all allies or an enemy, so the game lets a
//! lone player play each of them. The engine admitted a root holding one and
//! never offered the play.
//!
//! Every test here compares the engine with the game itself: the runs under
//! `tests/native/solo_*.native.json`, recorded by `tests/native/probe.py`
//! from `tests/native/issue3877_solo_playable_group1.spec.json` on the
//! archived v0.111.0 `sts2.dll` (headless harness). One player against
//! `ENCOUNTER.TOADPOLES_WEAK` plays the card at each upgrade level, then the
//! cards that would show what it left behind, and ends the turn.
//! `native/replay.rs` gives the engine the recorded root (piles, HP, RNG
//! streams) and compares HP, Block, Energy, the enemies, the piles in order
//! and every RNG counter after each step.

#[allow(dead_code)]
#[path = "../examples/composition_sweep/sweep.rs"]
mod sweep;

#[allow(dead_code)]
#[path = "native/replay.rs"]
mod replay;

use replay::{Powers, View, compare};

macro_rules! native {
    ($name:literal) => {
        include_str!(concat!("native/", $name, ".native.json"))
    };
}

const NONE: Powers = Powers {
    player: &[],
    monster: &[],
};

fn step<'a>(views: &'a [View], label: &str) -> &'a View {
    views
        .iter()
        .find(|view| view.label.contains(label))
        .unwrap_or_else(|| panic!("a step labelled {label}"))
}

/// `EnergySurge/<OnPlay>d__9` (RVA `0x39b948`): `PlayerCmd.GainEnergy`
/// (IL_0118) for each living Player among the owner's teammates, the owner
/// included. Alone: 2 Energy, 3 upgraded, and the card exhausts.
#[test]
fn native_energy_surge_alone_gains_the_owner_energy() {
    for (recorded, gain) in [
        (native!("solo_energy_surge"), 2),
        (native!("solo_energy_surge_upgraded"), 3),
    ] {
        let views = compare(recorded, NONE);
        let played = step(&views, "ENERGY_SURGE");
        assert_eq!(played.energy, 9 - 1 + gain);
        assert_eq!(played.exhaust.len(), 1);
    }
}

/// `HuddleUp/<OnPlay>d__7` (RVA `0x3a60bc`): a Draw per living Player
/// (IL_0097). Alone: the owner draws 2, 3 upgraded.
#[test]
fn native_huddle_up_alone_draws_for_the_owner() {
    for (recorded, drawn) in [
        (native!("solo_huddle_up"), 2),
        (native!("solo_huddle_up_upgraded"), 3),
    ] {
        let views = compare(recorded, NONE);
        assert_eq!(step(&views, "HUDDLE_UP").hand.len(), 2 + drawn);
    }
}

/// `Plot/<OnPlay>d__5` (RVA `0x3b2b40`): `DrawCardsNextTurnPower` on each
/// living Player (IL_009d). Alone: the owner's next Hand is 5 + 2, 5 + 3
/// upgraded, and the power is gone after it.
#[test]
fn native_plot_alone_draws_more_next_turn() {
    let powers = Powers {
        player: &[("DrawCardsNextTurnPower", "draw_next_turn")],
        monster: &[],
    };
    for (recorded, extra) in [
        (native!("solo_plot"), 2),
        (native!("solo_plot_upgraded"), 3),
    ] {
        let views = compare(recorded, powers);
        assert_eq!(step(&views, "PLOT").player_powers, [extra]);
        let next = &views[2];
        assert_eq!(
            (next.hand.len() as i64, &next.player_powers[..]),
            (5 + extra, &[0][..])
        );
    }
}

/// `Rally/<OnPlay>d__7` (RVA `0x3b55e4`): `CreatureCmd.GainBlock` (IL_0089)
/// for each living Player. Alone: 12 Block, 17 upgraded.
#[test]
fn native_rally_alone_blocks_for_the_owner() {
    for (recorded, block) in [
        (native!("solo_rally"), 12),
        (native!("solo_rally_upgraded"), 17),
    ] {
        let views = compare(recorded, NONE);
        assert_eq!(step(&views, "RALLY").block, block);
    }
}

/// `GangUp/<>c__DisplayClass4_0::<get_CanonicalVars>b__1` (RVA `0x3a0c9c`)
/// counts this turn's powered hits on the target whose dealer is on the
/// owner's side and is not the owner (IL_0040-IL_0082). The owner's own
/// Strike does not count: 5 damage. Osty's Unleash does: 5 + 5, 5 + 7
/// upgraded.
#[test]
fn native_gang_up_alone_counts_only_the_pet() {
    for recorded in [native!("solo_gang_up"), native!("solo_gang_up_upgraded")] {
        let views = compare(recorded, NONE);
        let before = step(&views, "STRIKE_IRONCLAD").monsters[0].0;
        assert_eq!(step(&views, "GANG_UP").monsters[0].0, before - 5);
    }
    for (recorded, damage) in [
        (native!("solo_gang_up_osty"), 10),
        (native!("solo_gang_up_osty_upgraded"), 12),
    ] {
        let views = compare(recorded, NONE);
        let before = step(&views, "UNLEASH").monsters[0].0;
        assert_eq!(step(&views, "GANG_UP").monsters[0].0, before - damage);
    }
}

/// `GangUp/<OnPlay>d__5` (RVA `0x3a0d20`) only null-checks its target
/// (IL_0026-IL_0030). A Glam Gang Up whose first body kills a 4 HP Toadpole
/// plays its second body on the same creature, and the attack finds no
/// living receiver: the other Toadpole is untouched and the play finishes.
/// The engine refused the dead target by index (`GANG_UP & TRANSFIGURE` in
/// the sweep, once the card was offered).
#[test]
fn native_a_replayed_gang_up_whose_target_died_lands_on_nobody() {
    for recorded in [
        native!("solo_gang_up_glam_lethal"),
        native!("solo_gang_up_glam_lethal_upgraded"),
    ] {
        let views = compare(recorded, NONE);
        let played = step(&views, "GANG_UP");
        assert_eq!(played.monsters.len(), 1, "one Toadpole is left");
        // The survivor is at full HP until the Strike after it.
        let struck = step(&views, "STRIKE_IRONCLAD");
        assert_eq!(struck.monsters[0].0, played.monsters[0].0 - 6);
    }
}

/// Rally under Unmovable: the first card Block of the turn is doubled
/// (`UnmovablePower::ModifyBlockMultiplicative` RVA `0xaa578`), the Defend
/// after it is not. The engine rehearsed Rally's gain on a clone, which
/// counted against the play's own Unmovable window, and the real gain then
/// refused (`RALLY & UNMOVABLE` in the sweep).
#[test]
fn native_rally_alone_under_unmovable_is_doubled_once() {
    for (recorded, block) in [
        (native!("solo_rally_unmovable"), 12),
        (native!("solo_rally_unmovable_upgraded"), 17),
    ] {
        let views = compare(recorded, NONE);
        assert_eq!(step(&views, "RALLY").block, 2 * block);
        assert_eq!(step(&views, "DEFEND").block, 2 * block + 5);
    }
}

/// `BladeSymphony/<OnPlay>d__7` (RVA `0x38ce0c`): `Shiv.CreateInHand`
/// (IL_0134) `Cards` times for each living Player. Alone: 2 Shivs in the
/// owner's Hand at either level (the upgrade lowers the cost).
#[test]
fn native_blade_symphony_alone_gives_the_owner_shivs() {
    for recorded in [
        native!("solo_blade_symphony"),
        native!("solo_blade_symphony_upgraded"),
        native!("solo_blade_symphony_silent"),
        native!("solo_blade_symphony_silent_upgraded"),
    ] {
        let views = compare(recorded, NONE);
        let hand = &step(&views, "BLADE_SYMPHONY").hand;
        assert_eq!(hand.iter().filter(|card| *card == "SHIV").count(), 2);
    }
}

/// `GlimpseBeyond/<OnPlay>d__9` (RVA `0x3a1968`): `Soul.Create` `Cards`
/// times (IL_0124) into each living Player's Draw pile at a random position
/// (`CardPileCmd.AddGeneratedCardsToCombat`, IL_013a). Alone: 3 Souls, 4
/// upgraded, each placed by one Shuffle-stream draw.
#[test]
fn native_glimpse_beyond_alone_shuffles_souls_into_the_owner_s_draw_pile() {
    for (recorded, souls) in [
        (native!("solo_glimpse_beyond"), 3),
        (native!("solo_glimpse_beyond_upgraded"), 4),
    ] {
        let views = compare(recorded, NONE);
        let draw = &step(&views, "GLIMPSE_BEYOND").draw;
        assert_eq!(draw.iter().filter(|card| *card == "SOUL").count(), souls);
    }
}

/// `OneForAllPower::ModifyDamageAdditive` (RVA `0xa52d8`): a powered attack
/// from a card the power's owner owns, not X-cost, that spent no Energy
/// (IL_006f-IL_007c) and costs nothing now (IL_008a-IL_0095), deals `Amount`
/// more. Alone the power is the owner's: Beam Cell deals 3 + 3 (3 + 4
/// upgraded) and a Strike its 6.
#[test]
fn native_one_for_all_alone_adds_to_the_owner_s_free_attacks() {
    let powers = Powers {
        player: &[("OneForAllPower", "one_for_all")],
        monster: &[],
    };
    for (recorded, amount) in [
        (native!("solo_one_for_all"), 3),
        (native!("solo_one_for_all_upgraded"), 4),
        (native!("solo_one_for_all_defect"), 3),
        (native!("solo_one_for_all_defect_upgraded"), 4),
    ] {
        let views = compare(recorded, powers);
        let applied = step(&views, "ONE_FOR_ALL");
        assert_eq!(applied.player_powers, [amount]);
        let (first, second) = (applied.monsters[0].0, applied.monsters[1].0);
        assert_eq!(step(&views, "BEAM_CELL").monsters[0].0, first - 3 - amount);
        assert_eq!(step(&views, "STRIKE").monsters[1].0, second - 6);
    }
}

/// `BeaconOfHopePower/<AfterBlockGained>d__8` (RVA `0x335878`) gives half the
/// owner's Block to each other living Player (`<AfterBlockGained>b__8_0`
/// `0x9fc0b` drops the owner). Alone nobody receives it: a Defend is 5 Block.
#[test]
fn native_beacon_of_hope_alone_gives_block_to_nobody() {
    let powers = Powers {
        player: &[("BeaconOfHopePower", "beacon_of_hope")],
        monster: &[],
    };
    for recorded in [
        native!("solo_beacon_of_hope"),
        native!("solo_beacon_of_hope_upgraded"),
    ] {
        let views = compare(recorded, powers);
        assert_eq!(step(&views, "BEACON_OF_HOPE").player_powers, [1]);
        assert_eq!(step(&views, "DEFEND").block, 5);
    }
}

/// A lone owner who dies with Hammer Time live: the lost state still
/// projects and loads. The boundary required a living owner for the flag
/// (the party rule), so the sweep's `DENSE_VEGETATION_EVENT_ENCOUNTER &
/// HAMMER_TIME` root failed its round trip once the play was offered.
#[test]
fn a_lone_owner_who_dies_with_hammer_time_live_still_reloads() {
    use sts_sim::{
        boundary::HotBoundary,
        engine::{self, Action, SelectionRef},
    };
    let recorded: serde_json::Value = serde_json::from_str(native!("solo_hammer_time")).unwrap();
    let (mut state, catalog) = replay::root(&recorded);
    let uid = HotBoundary::try_to_canonical(&state, &catalog)
        .unwrap()
        .piles["hand"]
        .iter()
        .find(|card| card.id == "HAMMER_TIME")
        .and_then(|card| card.uid)
        .unwrap() as u32;
    let play = Action::Play {
        uid,
        target: None,
        selection: SelectionRef::NONE,
    };
    state = engine::apply_action(&state, &catalog, &play).unwrap().state;
    for _ in 0..200 {
        if state.history.over {
            break;
        }
        state = engine::apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
    }
    assert!(state.history.over && state.hp <= 0, "the Toadpoles win");
    let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert_eq!(doc.player.get("hammer_time"), Some(&1.into()));
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let reloaded = HotBoundary::from_canonical(&doc, &reloaded_catalog).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog).unwrap(),
        doc
    );
}

/// `HammerTimePower/<AfterForge>d__6` (RVA `0x33bbd8`) forges for every
/// other living Player (`<AfterForge>b__0` `0x33bbba` drops the forger).
/// Alone a Bulwark forges once: one Sovereign Blade, for the owner.
#[test]
fn native_hammer_time_alone_forges_for_nobody_else() {
    let powers = Powers {
        player: &[("HammerTimePower", "hammer_time")],
        monster: &[],
    };
    for recorded in [
        native!("solo_hammer_time"),
        native!("solo_hammer_time_upgraded"),
        native!("solo_hammer_time_regent"),
        native!("solo_hammer_time_regent_upgraded"),
    ] {
        let views = compare(recorded, powers);
        assert_eq!(step(&views, "HAMMER_TIME").player_powers, [1]);
        let hand = &step(&views, "BULWARK").hand;
        assert_eq!(
            hand.iter()
                .filter(|card| *card == "SOVEREIGN_BLADE")
                .count(),
            1
        );
    }
}

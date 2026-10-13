//! #3877 — a first Osty summoned in the middle of a fight takes the next
//! creature id, and the enemies spawned after it take the ones that follow.
//!
//! The composition sweep met `creature id counter untracked with a live pet`
//! on legal lines: any character without a Phylactery who summons Osty
//! (Afterlife, Bodyguard, Bone Brew, ...) in a fight that spawns enemies
//! (Fabricator, Fogmog, ...). `OstyCmd/<Summon>d__0::MoveNext` (RVA
//! `0x3ee040`) creates the creature through `PlayerCmd.AddPet` at `IL_01e9`
//! (`<AddPet>d__14`1` `0x3ee68c` `IL_0054`, `CreateCreature`), and
//! `CombatState::AttachCreature` (`0x13718f` `IL_0009`-`IL_0022`) stamps
//! `CombatId = _nextCreatureId++`. A root with no combat-start pet leaves the
//! counter untracked, and the summon did not start tracking it.
//!
//! The fixture is the document at the start of turn two, as the sweep's root
//! `IRONCLAD / FABRICATOR_NORMAL & AFTERLIFE` reached it: Fabricator (uid 0)
//! about to Fabricate, two bots (uids 1 and 2), Afterlife in Hand, no pet,
//! the counter untracked. The `native_*` tests below compare the engine's
//! creature ids with the game's own, for an Osty summoned before and after
//! a spawn (#3904).

use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, SelectionRef},
    hot::HotState,
};

const FIXTURE: &str =
    include_str!("../fixtures/issue3877_fabricator_before_a_first_osty_summon.json");
const AFTERLIFE_UID: u32 = 7;

fn load() -> (HotState, Catalog) {
    let doc: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn apply(state: &HotState, catalog: &Catalog, action: Action) -> HotState {
    engine::apply_action(state, catalog, &action).unwrap().state
}

fn uids(state: &HotState) -> Vec<u32> {
    state.monsters.iter().map(|monster| monster.uid).collect()
}

fn counter(state: &HotState, catalog: &Catalog) -> Option<u64> {
    HotBoundary::try_to_canonical(state, catalog)
        .unwrap()
        .player
        .get("next_creature_uid")
        .and_then(serde_json::Value::as_u64)
}

fn summon(state: &HotState, catalog: &Catalog) -> HotState {
    apply(
        state,
        catalog,
        Action::Play {
            uid: AFTERLIFE_UID,
            target: None,
            selection: SelectionRef::NONE,
        },
    )
}

/// The control: with no pet the counter stays untracked and Fabricator's
/// next two bots take `max(uid) + 1` and the one after.
#[test]
fn without_a_pet_the_spawn_takes_the_next_enemy_uid() {
    let (entry, catalog) = load();
    assert_eq!(uids(&entry), [1, 2, 0]);
    assert_eq!(counter(&entry, &catalog), None);
    let next = apply(&entry, &catalog, Action::EndTurn);
    assert_eq!(counter(&next, &catalog), None);
    let mut spawned: Vec<u32> = uids(&next);
    spawned.retain(|uid| !uids(&entry).contains(uid));
    assert_eq!(spawned, [3, 4], "Fabricator makes two bots");
}

/// Afterlife's Osty takes creature id 3, the counter is tracked from there,
/// and the bots Fabricator makes on the enemy turn take 4 and 5. This `EndTurn` is
/// the action that refused.
#[test]
fn a_first_osty_takes_the_next_creature_id_and_the_spawn_the_one_after() {
    let (entry, catalog) = load();
    let summoned = summon(&entry, &catalog);
    assert_eq!(counter(&summoned, &catalog), Some(4), "Osty is creature 3");
    let next = apply(&summoned, &catalog, Action::EndTurn);
    let mut spawned: Vec<u32> = uids(&next);
    spawned.retain(|uid| !uids(&entry).contains(uid));
    assert_eq!(spawned, [4, 5]);
    assert_eq!(counter(&next, &catalog), Some(6));
}

/// The state after the summon is a document: it reloads with the same
/// counter and plays the same enemy turn.
#[test]
fn the_summoned_state_reloads_with_its_counter() {
    let (entry, catalog) = load();
    let summoned = summon(&entry, &catalog);
    let document = HotBoundary::try_to_canonical(&summoned, &catalog).unwrap();
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let reloaded = HotBoundary::from_canonical(&document, &reloaded_catalog).unwrap();
    engine::admit(&document, &reloaded, &reloaded_catalog).unwrap();
    assert_eq!(counter(&reloaded, &reloaded_catalog), Some(4));
    let next = apply(&reloaded, &reloaded_catalog, Action::EndTurn);
    assert!(uids(&next).contains(&5) && !uids(&next).contains(&3));
}

// ---------------------------------------------------------------------------
// Native witnesses: creature ids in creation order, both ways round (#3904).
// ---------------------------------------------------------------------------

/// The game's own runs, recorded by `tests/native/probe.py` from
/// `tests/native/issue3904_creature_id_order.spec.json` on the archived
/// v0.111.0 `sts2.dll` (headless harness). An Ironclad plays Afterlife before
/// or after the encounter creates enemies: Fabricator's bots, and Gremlin
/// Merc's Surprise pair. Each snapshot lists `CombatState.Creatures` with
/// their `CombatId`; the player is 0, so an engine uid is the id minus one.
const NATIVE_BEFORE_FABRICATE: &str = include_str!("native/osty_before_fabricate.native.json");
const NATIVE_AFTER_FABRICATE: &str = include_str!("native/osty_after_fabricate.native.json");
const NATIVE_BEFORE_SURPRISE: &str = include_str!("native/osty_before_surprise.native.json");
const NATIVE_AFTER_SURPRISE: &str = include_str!("native/osty_after_surprise.native.json");

const FABRICATOR_ROOT: &str = include_str!("../fixtures/issue3904_fabricator_afterlife_root.json");
const GREMLIN_MERC_ROOT: &str =
    include_str!("../fixtures/issue3904_gremlin_merc_afterlife_root.json");

/// `(Osty's creature id, every enemy's creature id ascending)` in the native
/// snapshot whose label ends with `label`. Dead enemies the game has removed
/// are not listed, as they are not in `Creatures`.
fn native_ids(recorded: &str, label: &str) -> (Option<u64>, Vec<u64>) {
    let all: serde_json::Value = serde_json::from_str(recorded).unwrap();
    assert_eq!(all["build"], "v0.111.0");
    let runs = all["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "no choice is asked in these scenarios");
    let entry = runs[0]["trace"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["label"].as_str().unwrap().ends_with(label))
        .unwrap_or_else(|| panic!("no {label} entry"));
    if let Some(state) = entry.get("state") {
        assert_eq!(state, "Finished", "{label}");
    }
    let creatures = entry["snap"]["creatures"].as_array().unwrap();
    let osty = creatures
        .iter()
        .find(|creature| creature["name"] == "OSTY")
        .map(|creature| creature["id"].as_u64().unwrap());
    let mut enemies: Vec<u64> = creatures
        .iter()
        .filter(|creature| creature["name"] != "OSTY" && creature["name"] != "PLAYER")
        .map(|creature| creature["id"].as_u64().unwrap())
        .collect();
    enemies.sort_unstable();
    (osty, enemies)
}

/// The same pair for an engine state: the recorded Osty id and the uid of
/// every enemy the game would still list, each plus one. `gone` names the
/// uids of dead enemies native has removed from `Creatures` (the engine keeps
/// Gremlin Merc's corpse on the roster).
fn engine_ids(state: &HotState, catalog: &Catalog, gone: &[u32]) -> (Option<u64>, Vec<u64>) {
    let document = HotBoundary::try_to_canonical(state, catalog).unwrap();
    let osty = document
        .player
        .get("osty_creature_uid")
        .and_then(serde_json::Value::as_u64)
        .map(|uid| uid + 1);
    let mut enemies: Vec<u64> = state
        .monsters
        .iter()
        .filter(|monster| !gone.contains(&monster.uid))
        .map(|monster| u64::from(monster.uid) + 1)
        .collect();
    enemies.sort_unstable();
    (osty, enemies)
}

/// A sweep root (`IRONCLAD / <encounter> & AFTERLIFE`, the document its
/// opening produced) with Afterlife dealt into the opening Hand in place of
/// a Defend, and optionally the first enemy's HP lowered so one Strike kills.
fn root(
    fixture: &str,
    afterlife_in_hand: bool,
    first_enemy_hp: Option<i64>,
) -> (HotState, Catalog) {
    let mut doc: CanonicalStateV2 = serde_json::from_str(fixture).unwrap();
    if afterlife_in_hand {
        let draw = doc.piles.get_mut("draw").unwrap();
        let at = draw.iter().position(|card| card.id == "AFTERLIFE").unwrap();
        let hand_defend = doc.piles["hand"]
            .iter()
            .position(|card| card.id == "DEFEND_IRONCLAD")
            .unwrap();
        let afterlife = doc.piles.get_mut("draw").unwrap().remove(at);
        let defend = std::mem::replace(
            &mut doc.piles.get_mut("hand").unwrap()[hand_defend],
            afterlife,
        );
        doc.piles.get_mut("draw").unwrap().insert(at, defend);
    }
    if let Some(hp) = first_enemy_hp {
        doc.monsters[0].insert("hp".to_string(), hp.into());
    }
    let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
    let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
    engine::admit(&doc, &state, &catalog).unwrap();
    (state, catalog)
}

fn play(state: &HotState, catalog: &Catalog, id: &str, target: Option<u8>) -> HotState {
    let uid = state
        .piles
        .get(sts_sim::hot::PileId::Hand)
        .as_slice()
        .iter()
        .find(|card| {
            catalog
                .spec(card.atom)
                .is_some_and(|spec| spec.identity.id.as_str() == id)
        })
        .unwrap_or_else(|| panic!("no {id} in Hand"))
        .uid;
    apply(
        state,
        catalog,
        Action::Play {
            uid,
            target,
            selection: SelectionRef::NONE,
        },
    )
}

/// The roster index of the enemy holding `uid`.
fn index_of(state: &HotState, uid: u32) -> u8 {
    u8::try_from(
        state
            .monsters
            .iter()
            .position(|monster| monster.uid == uid)
            .unwrap(),
    )
    .unwrap()
}

/// Afterlife first, then Fabricator's first two bots: the Osty is creature 2
/// and the bots 3 and 4 (native), so uids 1, then 2 and 3.
#[test]
fn native_an_osty_summoned_before_fabricate_sits_below_the_bots() {
    let (entry, catalog) = root(FABRICATOR_ROOT, true, None);
    let summoned = play(&entry, &catalog, "AFTERLIFE", None);
    assert_eq!(
        engine_ids(&summoned, &catalog, &[]),
        native_ids(NATIVE_BEFORE_FABRICATE, ":AFTERLIFE:done")
    );
    let fabricated = apply(&summoned, &catalog, Action::EndTurn);
    let native = native_ids(NATIVE_BEFORE_FABRICATE, "2:end");
    assert_eq!(native, (Some(2), vec![1, 3, 4]));
    assert_eq!(engine_ids(&fabricated, &catalog, &[]), native);
}

/// Fabricator's first two bots, then Afterlife, then two more bots: bots 2
/// and 3, the Osty 4, bots 5 and 6 (native).
#[test]
fn native_an_osty_summoned_after_fabricate_sits_between_the_bots() {
    let (entry, catalog) = root(FABRICATOR_ROOT, false, None);
    let fabricated = apply(&entry, &catalog, Action::EndTurn);
    assert_eq!(
        engine_ids(&fabricated, &catalog, &[]),
        native_ids(NATIVE_AFTER_FABRICATE, "2:end")
    );
    let summoned = play(&fabricated, &catalog, "AFTERLIFE", None);
    assert_eq!(
        engine_ids(&summoned, &catalog, &[]),
        native_ids(NATIVE_AFTER_FABRICATE, ":AFTERLIFE:done")
    );
    let again = apply(&summoned, &catalog, Action::EndTurn);
    let native = native_ids(NATIVE_AFTER_FABRICATE, "4:end");
    assert_eq!(native, (Some(4), vec![1, 2, 3, 5, 6]));
    assert_eq!(engine_ids(&again, &catalog, &[]), native);
}

/// The order `main` played and the first draft of this change refused
/// (`Gremlin Merc attack entry`): the Merc dies, Surprise makes Fat 2 and
/// Sneaky 3, and Afterlife's Osty is creature 4 (native). The Strike on Fat
/// afterwards and the enemy turn both apply.
#[test]
fn native_an_osty_summoned_after_surprise_sits_above_the_pair() {
    let (entry, catalog) = root(GREMLIN_MERC_ROOT, true, Some(6));
    let surprised = play(&entry, &catalog, "STRIKE_IRONCLAD", Some(0));
    // Native removes the dead Merc from `Creatures`; the engine retains it.
    let merc = [0];
    assert_eq!(
        engine_ids(&surprised, &catalog, &merc),
        native_ids(NATIVE_AFTER_SURPRISE, "2:STRIKE_IRONCLAD:done")
    );
    let summoned = play(&surprised, &catalog, "AFTERLIFE", None);
    let native = native_ids(NATIVE_AFTER_SURPRISE, ":AFTERLIFE:done");
    assert_eq!(native, (Some(4), vec![2, 3]));
    assert_eq!(engine_ids(&summoned, &catalog, &merc), native);
    // Fat is creature 2, uid 1.
    let fat = index_of(&summoned, 1);
    let struck = play(&summoned, &catalog, "STRIKE_IRONCLAD", Some(fat));
    assert_eq!(
        summoned.monsters[usize::from(fat)].hp - struck.monsters[usize::from(fat)].hp,
        6
    );
    let next = apply(&struck, &catalog, Action::EndTurn);
    assert!(!next.history.over);
    assert_eq!(engine_ids(&next, &catalog, &merc).0, Some(4));
}

/// Afterlife first: the Osty is creature 2, and Surprise's Fat and Sneaky
/// are 3 and 4 (native).
#[test]
fn native_an_osty_summoned_before_surprise_sits_below_the_pair() {
    let (entry, catalog) = root(GREMLIN_MERC_ROOT, true, Some(6));
    let summoned = play(&entry, &catalog, "AFTERLIFE", None);
    assert_eq!(
        engine_ids(&summoned, &catalog, &[]),
        native_ids(NATIVE_BEFORE_SURPRISE, ":AFTERLIFE:done")
    );
    let surprised = play(&summoned, &catalog, "STRIKE_IRONCLAD", Some(0));
    let merc = [0];
    let native = native_ids(NATIVE_BEFORE_SURPRISE, "3:STRIKE_IRONCLAD:done");
    assert_eq!(native, (Some(2), vec![3, 4]));
    assert_eq!(engine_ids(&surprised, &catalog, &merc), native);
    // Fat is creature 3, uid 2.
    let fat = index_of(&surprised, 2);
    let struck = play(&surprised, &catalog, "STRIKE_IRONCLAD", Some(fat));
    assert_eq!(
        surprised.monsters[usize::from(fat)].hp - struck.monsters[usize::from(fat)].hp,
        6
    );
    let next = apply(&struck, &catalog, Action::EndTurn);
    assert!(!next.history.over);
}

/// Every state above is a document: the summoned-after-Surprise one reloads,
/// re-admits and plays the same Strike.
#[test]
fn the_osty_above_a_lineage_reloads_and_re_admits() {
    let (entry, catalog) = root(GREMLIN_MERC_ROOT, true, Some(6));
    let surprised = play(&entry, &catalog, "STRIKE_IRONCLAD", Some(0));
    let summoned = play(&surprised, &catalog, "AFTERLIFE", None);
    let document = HotBoundary::try_to_canonical(&summoned, &catalog).unwrap();
    assert_eq!(document.player["osty_creature_uid"], 3);
    assert_eq!(document.player["next_creature_uid"], 4);
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
    let reloaded = HotBoundary::from_canonical(&document, &reloaded_catalog).unwrap();
    engine::admit(&document, &reloaded, &reloaded_catalog).unwrap();
    let fat = index_of(&reloaded, 1);
    play(&reloaded, &reloaded_catalog, "STRIKE_IRONCLAD", Some(fat));
}

/// A recorded Osty id has to be one the counter handed out, to an Osty that
/// exists, and not an enemy's: each forgery refuses by name at admission.
#[test]
fn a_forged_osty_creature_id_refuses_at_admission() {
    // Fabricator 0, bots 1 and 2, the Osty 3, the counter 4.
    let (entry, catalog) = root(FABRICATOR_ROOT, false, None);
    let fabricated = apply(&entry, &catalog, Action::EndTurn);
    let summoned = play(&fabricated, &catalog, "AFTERLIFE", None);
    let document = HotBoundary::try_to_canonical(&summoned, &catalog).unwrap();
    assert_eq!(document.player["osty_creature_uid"], 3);
    let refuses = |edit: &dyn Fn(&mut CanonicalStateV2)| {
        let mut forged = document.clone();
        edit(&mut forged);
        let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
        let state = HotBoundary::from_canonical(&forged, &catalog).unwrap();
        let refusal = engine::admit(&forged, &state, &catalog).unwrap_err();
        format!("{refusal:?}").contains("Osty creature id")
    };
    // A bot's uid.
    assert!(refuses(&|doc| {
        doc.player.insert("osty_creature_uid".to_string(), 1.into());
    }));
    // An id the counter has not handed out.
    assert!(refuses(&|doc| {
        doc.player.insert("osty_creature_uid".to_string(), 4.into());
    }));
    // No Osty at all.
    assert!(refuses(&|doc| {
        doc.player.remove("ally");
    }));
}

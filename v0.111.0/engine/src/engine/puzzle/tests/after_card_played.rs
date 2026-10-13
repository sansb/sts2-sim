//! #3822: Game Piece's and Iron Club's AfterCardPlayed tail Draws on the
//! receipt-owned carrier. The fixture helpers are `puzzle::tests`'.
//!
//! # Native witnesses
//!
//! No captured fight reaches a choice inside these Draws, so the ground
//! truth is the game itself. `fixtures/issue3822_native/*.json` are traces
//! recorded from the archived v0.111.0 `sts2.dll` (SHA-256 `9cb4f1ad…`) by
//! `python/harness/probe_relic_hook_choice.py`: the game's own
//! `PlayCardAction` pipeline with its shipped `TestCardSelector` pushed as
//! the LOCAL selector, so the choice signals its context, the action really
//! pauses in `GatheringPlayerChoice`, and it resumes on the answer. Every
//! trace holds one run per answer sequence the game accepts, with the whole
//! observable state at the root, at each pause and after each action.
//!
//! [`native_trace`] rebuilds the root in this engine (same piles, relic
//! order, powers, monster HP and the nine RNG streams' words and counters),
//! replays the script, and requires this engine's state to equal the
//! game's at every recorded point and its legal answers to be in bijection
//! with the game's.

use super::*;
use serde_json::{Value, json};

// #3756: the side-end traces replay through this module's helpers.
mod constrict_native;

/// Native power class names and the engine power each one is.
const POWERS: [(&str, PowerId); 15] = [
    ("JuggernautPower", PowerId::Juggernaut),
    ("ReboundPower", PowerId::Rebound),
    ("NostalgiaPower", PowerId::Nostalgia),
    ("NoDrawPower", PowerId::NoDraw),
    ("ThornsPower", PowerId::Thorns),
    ("BurstPower", PowerId::Burst),
    ("HellraiserPower", PowerId::Hellraiser),
    ("StrengthPower", PowerId::Strength),
    ("DexterityPower", PowerId::Dexterity),
    ("StratagemPower", PowerId::Stratagem),
    ("VulnerablePower", PowerId::Vulnerable),
    ("WeakPower", PowerId::Weak),
    ("AdaptablePower", PowerId::Adaptable),
    ("EnragePower", PowerId::Enrage),
    ("SteamEruptionPower", PowerId::SteamPressure),
];

/// The nine combat streams in the order the traces key them.
const STREAMS: [(&str, crate::hot::RngStream); 9] = [
    ("ai", crate::hot::RngStream::Ai),
    ("combat_orbs", crate::hot::RngStream::CombatOrbs),
    ("energy_costs", crate::hot::RngStream::EnergyCosts),
    ("generation", crate::hot::RngStream::Generation),
    ("niche", crate::hot::RngStream::Niche),
    ("potion_generation", crate::hot::RngStream::PotionGeneration),
    ("rng", crate::hot::RngStream::Rng),
    ("sel", crate::hot::RngStream::Sel),
    ("targets", crate::hot::RngStream::Targets),
];

const PILES: [(&str, PileId); 5] = [
    ("Hand", PileId::Hand),
    ("DrawPile", PileId::Draw),
    ("DiscardPile", PileId::Discard),
    ("ExhaustPile", PileId::Exhaust),
    ("PlayPile", PileId::Play),
];

fn identity_of(label: &str) -> CardIdentity {
    let label = label.split('[').next().unwrap();
    let name = label.trim_end_matches('+');
    CardIdentity {
        id: CardId::from_str(name).unwrap_or_else(|| panic!("unknown card {name}")),
        upgrade: u8::try_from(label.len() - name.len()).unwrap(),
        enchantment: None,
    }
}

fn card_label(state: &HotState, catalog: &Catalog, card: HotCard) -> String {
    let spec = catalog.spec(card.atom).unwrap();
    let mut label = spec.row.name.to_owned();
    if spec.ethereal || state.card_states.get(card.uid).local_ethereal() {
        label.push_str("[Ethereal]");
    }
    label
}

/// Every live power, under its native class name. One this table does not
/// map keeps the engine's own name, which no native snapshot holds, so an
/// engine-only power fails the comparison.
fn powers_of(powers: &crate::powers::Slots<PowerId>) -> Value {
    let mut rows = powers
        .as_slice()
        .iter()
        .filter(|slot| slot.value != 0)
        .map(|slot| {
            let name = POWERS.iter().find(|(_, id)| *id == slot.key).map_or_else(
                || format!("ENGINE:{:?}", slot.key),
                |(name, _)| (*name).to_owned(),
            );
            json!([name, slot.value])
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row[0].as_str().unwrap().to_owned());
    Value::Array(rows)
}

/// This engine's state in the shape of one native snapshot ([`normalized`]).
/// `started` and `finished` are the CardPlay bodies started and finished
/// since the fight began: the root's native counts plus this engine's own.
/// Nothing here is read from the snapshot it will be compared with.
fn snapshot(state: &HotState, catalog: &Catalog, started: i64, finished: i64) -> Value {
    let piles = PILES
        .iter()
        .map(|(name, pile)| {
            let labels = state
                .piles
                .get(*pile)
                .as_slice()
                .iter()
                .map(|card| card_label(state, catalog, *card))
                .collect::<Vec<_>>();
            ((*name).to_owned(), json!(labels))
        })
        .collect::<serde_json::Map<_, _>>();
    let rng = STREAMS
        .iter()
        .map(|(name, stream)| {
            let live = state.rng.get(*stream);
            let words = ["_s0", "_s1", "_s2", "_s3"]
                .iter()
                .zip(live.words)
                .map(|(name, word)| json!([name, word]))
                .collect::<Vec<_>>();
            (
                (*name).to_owned(),
                json!({"counter": live.counter, "words": words}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let mons = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| monster.hp > 0 || kept_dead(monster))
        .map(|(index, monster)| {
            let mut powers = powers_of(&monster.powers);
            if monster.kind == MonsterKind::SkulkingColony {
                // `SkulkingColony/<AfterAddedToRoom>d__27` applies Hardened
                // Shell; this engine holds it as the kind, not as a slot.
                let rows = powers.as_array_mut().unwrap();
                rows.push(json!([
                    "HardenedShellPower",
                    crate::engine::damage::HARDENED_SHELL
                ]));
                rows.sort_by_key(|row| row[0].as_str().unwrap().to_owned());
            }
            json!({
                "id": index + 1,
                // The game floors `CurrentHp` at zero; this engine keeps a
                // kept corpse's overkill.
                "hp": monster.hp.max(0),
                "max_hp": monster.max_hp,
                "block": monster.block,
                "powers": powers,
            })
        })
        .collect::<Vec<_>>();
    let hand_costs = state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .map(|card| {
            let spec = catalog.spec(card.atom).unwrap();
            crate::engine::play::resolved_energy_cost(state, *card, spec)
        })
        .collect::<Vec<_>>();
    let mut relics = serde_json::Map::new();
    if catalog.hooks().owns(RelicId::RelicIronClub) {
        relics.insert(
            "IronClub".to_owned(),
            json!(state.fanouts.iron_club_cards()),
        );
    }
    let mut extra = serde_json::Map::new();
    // The three observables an encounter trace adds, each present only when
    // it is not the default (`probe_relic_hook_choice.py`'s `snap`).
    let display = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| monster.hp_display_is_infinite())
        .map(|(index, monster)| {
            // `HpDisplay` 1 is Hardened Shell's, 2 the Giant's
            // (`HotMonster::hp_display_is_infinite`).
            let name = if monster.kind == MonsterKind::WaterfallGiant {
                "InfiniteWithoutNumbers"
            } else {
                "InfiniteWithNumbers"
            };
            ((index + 1).to_string(), json!(name))
        })
        .collect::<serde_json::Map<_, _>>();
    if !display.is_empty() {
        extra.insert("hp_display".to_owned(), Value::Object(display));
    }
    if state.fanouts.hellraiser_infinite_auto_plays() != 0 {
        extra.insert(
            "hellraiser_infinite_auto_plays".to_owned(),
            json!(state.fanouts.hellraiser_infinite_auto_plays()),
        );
    }
    let dead = state
        .monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| kept_dead(monster))
        .map(|(index, _)| index + 1)
        .collect::<Vec<_>>();
    if !dead.is_empty() {
        extra.insert("dead".to_owned(), json!(dead));
    }
    if !state.fanouts.potion_slots().is_empty() {
        let belt = state
            .fanouts
            .potion_slots()
            .iter()
            .map(|slot| slot.map_or(Value::Null, |potion| json!(potion.as_str())))
            .collect::<Vec<_>>();
        extra.insert("potions".to_owned(), Value::Array(belt));
    }
    let mut ours = json!({
        "hp": state.hp,
        "max_hp": state.max_hp,
        "block": state.block,
        "energy": state.energy,
        "powers": powers_of(&state.powers),
        "mons": mons,
        "hand_costs": hand_costs,
        "piles": piles,
        "relics": relics,
        "plays_started": started,
        "plays_finished": finished + i64::from(state.history.card_plays_finished_combat),
        "over": state.history.over,
        "rng": rng,
    });
    ours.as_object_mut().unwrap().extend(extra);
    ours
}

/// A dead enemy the game keeps in `CombatState.Creatures`: a Test Subject
/// between forms (`AdaptablePower::ShouldCreatureBeRemovedFromCombatAfterDeath`
/// RVA `0x9f608`).
fn kept_dead(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::TestSubject && monster.test_subject_adaptable_reviving()
}

/// One native snapshot as [`snapshot`] spells it. Native never reduces
/// Iron Club's `CardsPlayed` (`IronClub::set_CardsPlayed` RVA `0x950f4`
/// stores the sum) and reads only `% Cards`; this engine stores that
/// residue.
fn normalized(native: &Value) -> Value {
    let mut native = native.clone();
    if let Some(club) = native["relics"].get_mut("IronClub") {
        *club = json!(club.as_u64().unwrap() % 4);
    }
    // Hellraiser's count runs on past nine in the game and is read only as
    // `>= 9` (`draw::hellraiser_should_auto_play`); this engine stores nine.
    if let Some(count) = native.get_mut("hellraiser_infinite_auto_plays") {
        *count = json!(
            count
                .as_u64()
                .unwrap()
                .min(u64::from(crate::hot::HELLRAISER_INFINITE_AUTO_PLAY_CAP))
        );
    }
    native
}

struct Root {
    state: HotState,
    catalog: Catalog,
    /// Native `plays_started` at the root.
    started: i64,
    /// Native `plays_finished` at the root.
    finished: i64,
}

/// The CardPlay bodies this engine has started, counted as the game's
/// history counts `CardPlayStartedEntry`. An answer on a receipt-owned park
/// re-executes its action from the root, and rehearsals run bodies on
/// clones, so the bodies of one root action are a set
/// (`play::take_test_body_starts`); an action's count is settled when it
/// finishes.
#[derive(Clone, Default)]
struct Started {
    /// Bodies of the actions already finished.
    settled: i64,
    /// Bodies of the action in progress.
    action: std::collections::BTreeSet<(u32, u8, i32)>,
}

impl Started {
    fn total(&self) -> i64 {
        self.settled + i64::try_from(self.action.len()).unwrap()
    }

    /// Fold in the bodies started since the last call.
    fn observe(&mut self) {
        self.action
            .extend(crate::engine::play::take_test_body_starts());
    }

    fn settle(&mut self) {
        self.settled = self.total();
        self.action.clear();
    }
}

/// Rebuild the native root: Ironclad at turn one against the trace's
/// Toadpoles, the spec's relics after Burning Blood as a vouched inventory.
fn root(spec: &Value, native: &Value) -> Root {
    let mut identities = std::collections::BTreeSet::new();
    for label in spec["deck"].as_array().unwrap() {
        identities.insert(identity_of(label.as_str().unwrap()));
    }
    let mut builder = CatalogBuilder::new();
    let atoms = identities
        .iter()
        .map(|identity| (*identity, builder.intern_reachable(*identity).unwrap()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let opened = spec
        .get("encounter")
        .map(|encounter| encounter_monsters(encounter.as_str().unwrap()));
    match &opened {
        Some(monsters) => {
            for monster in monsters {
                builder.intern_monster(monster.kind).unwrap();
            }
        }
        None => {
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
        }
    }
    let mut relics = vec![RelicId::RelicBurningBlood];
    for relic in spec["relics"].as_array().unwrap() {
        let name = format!("RELIC.{}", relic.as_str().unwrap());
        relics.push(RelicId::from_str(&name).unwrap_or_else(|| panic!("{name}")));
    }
    builder.set_relics_ordered(&relics, true).unwrap();
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    if let Some(potions) = spec.get("potions") {
        // The native belt: the spec's potions, then empty slots up to three.
        let mut slots = potions
            .as_array()
            .unwrap()
            .iter()
            .map(|name| crate::ids::PotionId::from_str(name.as_str().unwrap()))
            .collect::<Vec<_>>();
        assert!(slots.iter().all(Option::is_some), "unknown potion");
        slots.resize(3, None);
        assert!(
            state
                .fanouts
                .set_potion_belt(slots, false, false, false, false, true)
        );
    }
    // After the belt, which rewrites the same fanout word.
    if relics.contains(&RelicId::RelicUnceasingTop) {
        state.fanouts.set_unceasing_top(true);
    }
    if relics.contains(&RelicId::RelicGremlinHorn) {
        state.fanouts.set_gremlin_horn_owned(true);
    }
    if relics.contains(&RelicId::RelicVelvetChoker) {
        state.set_batch_six_deep_relic_ownership(false, false, false, true);
    }
    // Permafrost has not given its Block yet at a turn-one root.
    state.set_permafrost_armed(relics.contains(&RelicId::RelicPermafrost));
    state.hp = i32::try_from(native["hp"].as_i64().unwrap()).unwrap();
    state.max_hp = i32::try_from(native["max_hp"].as_i64().unwrap()).unwrap();
    state.energy = i16::try_from(native["energy"].as_i64().unwrap()).unwrap();
    state.player_phase = PHASE_ORDINARY_ACTIONS;
    state.exact_piles = true;
    // The probe starts its fight at ascension 0 (`_Fight(player, ..., 0,
    // ...)`).
    assert!(state.fanouts.set_ascension(0));
    for (name, stream) in STREAMS {
        let live = &native["rng"][name];
        let mut words = [0; 4];
        for (word, row) in words.iter_mut().zip(live["words"].as_array().unwrap()) {
            *word = row[1].as_u64().unwrap();
        }
        state.rng.set(
            stream,
            crate::hot::RngStreamState {
                words,
                counter: live["counter"].as_u64().unwrap(),
            },
        );
    }
    for (name, id) in POWERS {
        for row in native["powers"].as_array().unwrap() {
            if row[0] == name {
                let amount = i32::try_from(row[1].as_i64().unwrap()).unwrap();
                state.powers.set(id, SlotWire::Int, amount);
            }
        }
    }
    assert!(
        native["powers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| POWERS.iter().any(|(name, _)| row[0] == *name)),
        "unmapped root power"
    );
    crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
    let mut uid = 1;
    for (name, pile) in PILES {
        for label in native["piles"][name].as_array().unwrap() {
            let atom = atoms[&identity_of(label.as_str().unwrap())];
            state.piles.get_mut(pile).make_mut().push(card(uid, atom));
            uid += 1;
        }
    }
    state.next_card_uid = uid;
    let rows = native["mons"].as_array().unwrap();
    let monsters = if let Some(mut monsters) = opened {
        // The encounter as this engine opens it, with the root's HP and the
        // powers the scenario's setup applied.
        assert_eq!(monsters.len(), rows.len(), "{}", spec["encounter"]);
        for (monster, row) in monsters.iter_mut().zip(rows) {
            monster.hp = i32::try_from(row["hp"].as_i64().unwrap()).unwrap();
            monster.max_hp = i32::try_from(row["max_hp"].as_i64().unwrap()).unwrap();
            for power in row["powers"].as_array().unwrap() {
                if power[0] == "HardenedShellPower" {
                    assert_eq!(monster.kind, MonsterKind::SkulkingColony);
                    continue;
                }
                let id = POWERS
                    .iter()
                    .find(|(name, _)| power[0] == *name)
                    .unwrap_or_else(|| panic!("unmapped monster power {power}"))
                    .1;
                let amount = i32::try_from(power[1].as_i64().unwrap()).unwrap();
                monster.powers.set(id, SlotWire::Int, amount);
            }
        }
        monsters
    } else {
        let mut monsters = rows
            .iter()
            .map(|row| {
                assert!(row["powers"].as_array().unwrap().is_empty());
                let hp = i32::try_from(row["hp"].as_i64().unwrap()).unwrap();
                let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
                monster.max_hp = i32::try_from(row["max_hp"].as_i64().unwrap()).unwrap();
                monster
            })
            .collect::<Vec<_>>();
        assert_eq!(monsters.len(), 2, "ENCOUNTER.TOADPOLES_WEAK");
        // `ENCOUNTER.TOADPOLES_WEAK` (`encounters::weak::build_toadpoles_weak`)
        // opens its front Toadpole on the Spiken row and its back one on Whirl;
        // a monster's uid is its creation index. Only a script that ends the
        // turn reads the rows.
        for (index, (monster, opener)) in monsters.iter_mut().zip(["SPIKEN", "WHIRL"]).enumerate() {
            monster.loop_pos =
                crate::encounters::loop_position(MonsterKind::Toadpole, opener).unwrap();
            monster.slot = i32::try_from(index).unwrap();
            monster.uid = u32::try_from(index).unwrap();
        }
        monsters
    };
    state.monsters = std::sync::Arc::new(monsters);
    let (state, catalog) = cold(&state, &catalog);
    Root {
        state,
        catalog,
        started: native["plays_started"].as_i64().unwrap(),
        finished: native["plays_finished"].as_i64().unwrap(),
    }
}

/// The monsters of `encounter` as this engine's own opening makes them at
/// ascension 0 (the probe's), from the entry builder's fixture save. Only
/// the roster is taken: its kinds, uids, slots, opening rows and spawn
/// powers.
fn encounter_monsters(encounter: &str) -> Vec<HotMonster> {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../../fixtures/entry_builder_v1.json")).unwrap();
    let mut save = fixture["cases"][0]["save"].clone();
    save["ascension"] = json!(0);
    // The opening wants an exact unlock state; every epoch is one.
    save["players"][0]["unlock_state"]["unlocked_epochs"] = json!(
        crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101
            .iter()
            .map(|epoch| format!("EPOCH.{epoch}"))
            .collect::<Vec<_>>()
    );
    let save = serde_json::to_string(&save).unwrap();
    let node_type = if encounter.ends_with("_BOSS") {
        "boss"
    } else if encounter.ends_with("_ELITE") {
        "elite"
    } else {
        "monster"
    };
    let request = crate::entry::EntryRequest {
        input: crate::entry::EntryInput::Save(&save),
        encounter_id: Some(encounter),
        node_type: Some(node_type),
        game_build: crate::catalog::GameBuild::ALL[crate::catalog::GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let opening = match crate::entry::build_root(&request, None) {
        crate::entry::RootOutcome::Opened(opening) => opening,
        crate::entry::RootOutcome::EntryOnly(_, refusal) => panic!("{encounter}: {refusal}"),
        crate::entry::RootOutcome::Refused(refusal) => panic!("{encounter}: {refusal}"),
    };
    let catalog = HotBoundary::catalog_from_canonical(&opening.document).unwrap();
    let state = HotBoundary::from_canonical(&opening.document, &catalog).unwrap();
    state.monsters.as_ref().clone()
}

/// Compare this engine's state with one native snapshot.
fn assert_matches(root: &Root, state: &HotState, started: &Started, native: &Value, at: &str) {
    let mut ours = snapshot(
        state,
        &root.catalog,
        root.started + started.total(),
        root.finished,
    );
    let mut theirs = normalized(native);
    if theirs["over"] == json!(true) {
        // Past the last kill the game runs its combat-end flow (Burning
        // Blood's heal, the card cleanup, the history reset), which is not
        // this engine's.
        for key in [
            "hp",
            "piles",
            "hand_costs",
            "relics",
            "plays_started",
            "plays_finished",
        ] {
            ours[key] = Value::Null;
            theirs[key] = Value::Null;
        }
    }
    assert_eq!(ours, theirs, "{at}");
}

fn uid_of(state: &HotState, catalog: &Catalog, label: &str) -> u32 {
    state
        .piles
        .get(PileId::Hand)
        .as_slice()
        .iter()
        .find(|card| card_label(state, catalog, **card).split('[').next() == Some(label))
        .unwrap_or_else(|| panic!("no {label} in Hand"))
        .uid
}

/// What one native trace established, for the caller's own assertions.
struct Witness {
    /// Parked states, in script order, of the first run.
    parks: Vec<HotState>,
    /// Number of native runs (answer sequences).
    runs: usize,
    /// Legal answers at each park of the first run.
    options: Vec<usize>,
    /// The terminal state of every run.
    terminals: Vec<HotState>,
}

/// Replay one native trace file (see the module docs).
fn native_trace(text: &str) -> Witness {
    let file: Value = serde_json::from_str(text).unwrap();
    assert_eq!(file["build"], "v0.111.0");
    let spec = &file["spec"];
    let runs = file["runs"].as_array().unwrap();
    assert!(!runs.is_empty());
    let mut witness = Witness {
        parks: Vec::new(),
        runs: runs.len(),
        options: Vec::new(),
        terminals: Vec::new(),
    };
    // Every native answer sequence, as the engine answers that reproduce it.
    let mut used = std::collections::BTreeSet::new();
    let mut fanout = std::collections::BTreeMap::<Vec<String>, usize>::new();
    for (run_index, run) in runs.iter().enumerate() {
        let trace = run["trace"].as_array().unwrap();
        let entry = |label: &str| {
            trace
                .iter()
                .find(|entry| entry["label"] == label)
                .unwrap_or_else(|| panic!("no trace entry {label}"))
        };
        let mut root: Option<Root> = None;
        let mut state: Option<HotState> = None;
        let mut answers = Vec::new();
        let mut started = Started::default();
        for (step_index, step) in spec["script"].as_array().unwrap().iter().enumerate() {
            let kind = step[0].as_str().unwrap();
            match kind {
                "snap" => {
                    let label = step[1].as_str().unwrap();
                    let native = &entry(label)["snap"];
                    if root.is_none() {
                        let built = self::root(spec, native);
                        state = Some(built.state.clone());
                        root = Some(built);
                    }
                    let at = format!("{} run {run_index} {label}", file["name"]);
                    assert_matches(
                        root.as_ref().unwrap(),
                        state.as_ref().unwrap(),
                        &started,
                        native,
                        &at,
                    );
                }
                "play" => {
                    let root = root.as_ref().expect("a snap precedes the first play");
                    let card = step[1].as_str().unwrap();
                    // The trace names a play by its script position.
                    let label = format!("{step_index}:{card}");
                    let current = state.take().unwrap();
                    let target = step[2]
                        .as_u64()
                        .filter(|_| !card.starts_with('@'))
                        .map(|id| u8::try_from(id - 1).unwrap());
                    // `@END` ends the turn through the game's own
                    // `EndPlayerTurnAction`.
                    // `@POTION` uses the belt slot through `UsePotionAction`.
                    let play = if card == "@END" {
                        Action::EndTurn
                    } else if card == "@POTION" {
                        Action::UsePotion {
                            slot: u8::try_from(step[2].as_u64().unwrap()).unwrap(),
                            target: None,
                        }
                    } else {
                        Action::Play {
                            uid: uid_of(&current, &root.catalog, card),
                            target,
                            selection: SelectionRef::NONE,
                        }
                    };
                    let at = format!("{} run {run_index} {label}", file["name"]);
                    crate::engine::play::take_test_body_starts();
                    let mut next = apply_action(&current, &root.catalog, &play)
                        .unwrap_or_else(|error| panic!("{at}: {error:?}"))
                        .state;
                    started.observe();
                    let mut park = 0;
                    loop {
                        let parked = trace
                            .iter()
                            .find(|entry| entry["label"] == format!("{label}:parked-{park}"));
                        let Some(parked) = parked else {
                            assert!(next.pending.is_none(), "{at}: an extra choice");
                            break;
                        };
                        assert!(next.pending.is_some(), "{at}: no choice {park}");
                        // A queued hook action's choice is asked once the
                        // PlayCardAction has finished (`running` names it).
                        assert!(
                            parked["state"] == "GatheringPlayerChoice"
                                || parked.get("running").is_some(),
                            "{at}"
                        );
                        assert_matches(
                            root,
                            &next,
                            &started,
                            &parked["snap"],
                            &format!("{at} park {park}"),
                        );
                        // Every published park loads cold.
                        // (The reloaded catalog may number its atoms
                        // differently, so the two are compared by digest.)
                        let (loaded, loaded_catalog) = cold(&next, &root.catalog);
                        let legal = legal_actions(&next, &root.catalog);
                        assert_eq!(legal_actions(&loaded, &loaded_catalog), legal);
                        if run_index == 0 {
                            witness.parks.push(next.clone());
                            witness.options.push(legal.len());
                        }
                        fanout.insert(answers.clone(), legal.len());
                        // The native state after this answer is the next
                        // recorded point; exactly one legal answer reaches it.
                        let after = trace
                            .iter()
                            .find(|entry| entry["label"] == format!("{label}:parked-{}", park + 1))
                            .unwrap_or_else(|| entry(&format!("{label}:done")));
                        // Candidates are compared softly: sibling answers
                        // differ, and the chosen one is asserted below.
                        let wanted = normalized(&after["snap"]);
                        let mut matched = Vec::new();
                        for action in &legal {
                            crate::engine::play::take_test_body_starts();
                            let warm = apply_action(&next, &root.catalog, action)
                                .unwrap_or_else(|error| panic!("{at} {action:?}: {error:?}"))
                                .state;
                            let mut candidate = started.clone();
                            candidate.observe();
                            let from_cold = apply_action(&loaded, &loaded_catalog, action)
                                .unwrap()
                                .state;
                            assert_eq!(
                                HotBoundary::try_to_canonical(&from_cold, &loaded_catalog)
                                    .unwrap()
                                    .differential_digest(),
                                HotBoundary::try_to_canonical(&warm, &root.catalog)
                                    .unwrap()
                                    .differential_digest(),
                                "{at} {action:?}"
                            );
                            let ours = snapshot(
                                &warm,
                                &root.catalog,
                                root.started + candidate.total(),
                                root.finished,
                            );
                            if ours == wanted {
                                matched.push((*action, warm, candidate));
                            }
                        }
                        crate::engine::play::take_test_body_starts();
                        assert!(!matched.is_empty(), "{at} park {park}: no answer matches");
                        // Two answers the game cannot tell apart (two copies
                        // of one card) reach equal states; take the first
                        // this run has not used.
                        let (action, warm, candidate) = matched
                            .into_iter()
                            .find(|(action, _, _)| {
                                let mut key = answers.clone();
                                key.push(format!("{action:?}"));
                                !used.contains(&key)
                            })
                            .unwrap_or_else(|| panic!("{at} park {park}: answer reused"));
                        answers.push(format!("{action:?}"));
                        next = warm;
                        started = candidate;
                        park += 1;
                    }
                    assert_eq!(entry(&format!("{label}:done"))["state"], "Finished");
                    assert_matches(
                        root,
                        &next,
                        &started,
                        &entry(&format!("{label}:done"))["snap"],
                        &at,
                    );
                    started.settle();
                    if next.pending.is_none() {
                        assert!(next.frames.is_empty(), "{at}");
                        // A finished fight is not a root admission takes.
                        if !next.history.over {
                            cold(&next, &root.catalog);
                        }
                    }
                    state = Some(next);
                }
                // Root setup the first snapshot already carries.
                "energy" | "arrange" | "hp" | "power" | "mpower" => {
                    assert!(root.is_none(), "setup after the root")
                }
                other => panic!("unknown step {other}"),
            }
        }
        assert!(
            used.insert(answers),
            "{}: duplicate native runs",
            file["name"]
        );
        witness.terminals.push(state.unwrap());
    }
    // Bijection: the game's answer sequences are exactly this engine's.
    for (prefix, legal) in &fanout {
        let children = used
            .iter()
            .filter(|sequence| sequence.len() > prefix.len() && sequence.starts_with(prefix))
            .map(|sequence| sequence[prefix.len()].clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            children.len(),
            *legal,
            "{}: the game accepts {} answers after {prefix:?}, this engine offers {legal}",
            file["name"],
            children.len()
        );
    }
    witness
}

fn parked_caller(parked: &HotState) -> crate::hot::DrawCaller {
    let depth = parked_draw_depth(parked).expect("a receipt-owned Draw");
    let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[depth] else {
        unreachable!()
    };
    let draw = parked.frames.draw(record).unwrap();
    assert_eq!((draw.requested, draw.completed), (1, 0));
    draw.caller
}

macro_rules! trace {
    ($name:literal) => {
        native_trace(include_str!(concat!(
            "../../../../fixtures/issue3822_native/",
            $name,
            ".json"
        )))
    };
}

use crate::hot::DrawCaller::{GamePiece, IronClub};

/// Play the Hand card the IL-only roots put at uid 1.
const PLAY_POWER: Action = Action::Play {
    uid: 1,
    target: None,
    selection: SelectionRef::NONE,
};

/// The chain of the issue: a Power play, Game Piece's Draw, a Hellraiser
/// AutoPlay of Seeker Strike, and its `FromCombatPile` choice. The game
/// pauses the Power's own PlayCardAction with both cards in Play; each of
/// the three answers finishes the Strike, the relic walk and the Power.
#[test]
fn game_piece_seeker_strike_reproduces_the_native_pause_and_every_answer() {
    for (name, witness) in [
        ("seeker_basic", trace!("seeker_basic")),
        ("seeker_plus", trace!("seeker_plus")),
        // Nine Hand cards at the choice: the pick fills the Hand to its cap.
        ("seeker_hand_nine", trace!("seeker_hand_nine")),
    ] {
        assert_eq!(
            (witness.runs, &witness.options[..]),
            (3, &[3][..]),
            "{name}"
        );
        let parked = &witness.parks[0];
        assert_eq!(parked_caller(parked), GamePiece, "{name}");
        // The Power and the Strike are both still in Play at the pause.
        assert_eq!(parked.piles.get(PileId::Play).len(), 2, "{name}");
        for terminal in &witness.terminals {
            assert!(terminal.piles.get(PileId::Play).is_empty(), "{name}");
        }
    }
    let nine = trace!("seeker_hand_nine");
    assert_eq!(nine.parks[0].piles.get(PileId::Hand).len(), 9);
    for terminal in &nine.terminals {
        assert_eq!(terminal.piles.get(PileId::Hand).len(), 10);
    }
}

/// Sculpting Strike's `FromHand` choice in the same place, at both levels.
#[test]
fn game_piece_sculpting_strike_reproduces_the_native_pause_and_every_answer() {
    let basic = trace!("sculpting_basic");
    assert_eq!((basic.runs, &basic.options[..]), (3, &[3][..]));
    assert_eq!(parked_caller(&basic.parks[0]), GamePiece);
    let plus = trace!("sculpting_plus");
    assert_eq!((plus.runs, &plus.options[..]), (2, &[2][..]));
    assert_eq!(parked_caller(&plus.parks[0]), GamePiece);
}

/// A choice the game resolves without asking: one eligible card is taken,
/// and none leaves nothing to take. Neither pauses the action, in the game
/// or here.
#[test]
fn a_choice_with_one_option_or_none_does_not_pause() {
    for (name, witness) in [
        ("sculpting_one_option", trace!("sculpting_one_option")),
        ("sculpting_empty_hand", trace!("sculpting_empty_hand")),
        ("seeker_one_in_draw", trace!("seeker_one_in_draw")),
        ("seeker_empty_draw", trace!("seeker_empty_draw")),
    ] {
        assert_eq!(witness.runs, 1, "{name}");
        assert!(witness.parks.is_empty(), "{name}");
    }
}

/// The baseline the carrier must not disturb: a Strike with no choice.
#[test]
fn a_strike_without_a_choice_never_pauses() {
    let witness = trace!("plain_strike");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
}

/// Another AfterCardPlayed listener on each side of Game Piece. Rainbow
/// Ring completes its three types on the Power itself: recorded first, its
/// Strength is already applied when the Strike hits and is visible at the
/// pause; recorded second, it runs only after the answer.
#[test]
fn listeners_before_game_piece_run_before_the_pause_and_later_ones_after_the_answer() {
    let before = trace!("ring_before");
    let after = trace!("ring_after");
    for witness in [&before, &after] {
        assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
        assert_eq!(parked_caller(&witness.parks[0]), GamePiece);
        for terminal in &witness.terminals {
            assert_eq!(terminal.powers.value(PowerId::Strength), 1);
        }
    }
    assert_eq!(before.parks[0].powers.value(PowerId::Strength), 1);
    assert_eq!(after.parks[0].powers.value(PowerId::Strength), 0);
    // The Strike's hit reads the Strength only when the Ring ran first.
    let damage = |witness: &Witness| {
        witness.parks[0]
            .monsters
            .iter()
            .map(|monster| monster.max_hp - monster.hp)
            .sum::<i32>()
    };
    assert_eq!(damage(&before), damage(&after) + 1);
}

/// Mummified Hand picks a Hand card to make free. Recorded before Game
/// Piece it picks before the Draw; recorded after, it picks once the
/// answer has put Seeker Strike's card in the Hand.
#[test]
fn mummified_hand_picks_on_its_recorded_side_of_the_choice() {
    let before = trace!("mummified_before");
    let after = trace!("mummified_after");
    for witness in [&before, &after] {
        assert_eq!(witness.options, [3]);
        assert_eq!(parked_caller(&witness.parks[0]), GamePiece);
    }
    let sel = |state: &HotState| state.rng.get(crate::hot::RngStream::Sel).counter;
    // Seeker Strike's shuffle and Mummified Hand's pick share the stream:
    // the pick is already spent at the pause only when the Hand ran first.
    assert!(sel(&before.parks[0]) > sel(&after.parks[0]));
}

/// The AutoPlayed Strike kills. With an enemy left the fight goes on and
/// the choice opens; when it kills the last enemy the game opens no choice
/// (`CardSelectCmd` returns empty once the combat is ending).
#[test]
fn a_lethal_autoplayed_strike_pauses_only_while_the_fight_goes_on() {
    let last = trace!("kill_last_enemy");
    assert_eq!(last.runs, 1);
    assert!(last.parks.is_empty());
    assert!(last.terminals[0].history.over);
    let one = trace!("kill_one_of_two");
    assert_eq!(one.options, [3]);
    assert!(!one.terminals[0].history.over);
    assert_eq!(one.parks[0].monsters.iter().filter(|m| m.hp > 0).count(), 1);
}

/// A second Power after the resume draws through Game Piece again, and
/// parks again when that draw is Sculpting Strike.
#[test]
fn a_second_power_after_the_resume_parks_again() {
    let witness = trace!("second_power");
    assert!(witness.runs > 3, "{}", witness.runs);
    assert!(witness.parks.len() >= 2, "{}", witness.parks.len());
    for parked in &witness.parks {
        assert_eq!(parked_caller(parked), GamePiece);
    }
}

/// Iron Club's fourth-card Draw is the same shape, from a Skill play.
#[test]
fn iron_club_draw_reproduces_the_native_pause_and_every_answer() {
    let witness = trace!("iron_club");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(parked_caller(&witness.parks[0]), IronClub);
}

/// Both relics on one Power play. Game Piece first: its Draw parks, and
/// after the answer Iron Club (the next listener of the same walk) counts
/// the Power as the fourth card and its own Draw parks. Iron Club first:
/// the AutoPlayed Strike is the fourth card, so Iron Club's Draw runs
/// inside that Strike's AfterCardPlayed walk, nested in Game Piece's Draw.
#[test]
fn game_piece_and_iron_club_park_in_inventory_order_on_one_power_play() {
    let callers = |witness: &Witness| witness.parks.iter().map(parked_caller).collect::<Vec<_>>();
    let draws = |state: &HotState| {
        state
            .frames
            .as_slice()
            .iter()
            .filter(|frame| matches!(frame, crate::frame::Frame::Draw { .. }))
            .count()
    };
    let piece_first = trace!("piece_then_club");
    assert_eq!(callers(&piece_first), [GamePiece, IronClub]);
    assert_eq!(draws(&piece_first.parks[1]), 1);
    let club_first = trace!("club_then_piece");
    assert_eq!(callers(&club_first), [GamePiece, IronClub]);
    // Nested: Game Piece's Draw is still on the stack under the Strike.
    assert_eq!(draws(&club_first.parks[1]), 2);
}

/// The Stratagem twin: the relic's Draw reshuffles under a live Stratagem
/// and the reshuffle's pick pauses the play.
#[test]
fn a_stratagem_reshuffle_inside_either_relic_draw_reproduces_the_native_pause() {
    let piece = trace!("stratagem");
    assert_eq!(parked_caller(&piece.parks[0]), GamePiece);
    assert!(piece.runs > 1);
    let club = trace!("stratagem_iron_club");
    assert_eq!(parked_caller(&club.parks[0]), IronClub);
    assert!(club.runs > 1);
}

// ---- The independent review's scenarios (`r_*`) ----------------------------

/// The caller of each receipt-owned park, `None` for a park an ordinary
/// frame owns.
fn park_owners(witness: &Witness) -> Vec<Option<crate::hot::DrawCaller>> {
    witness
        .parks
        .iter()
        .map(|parked| parked_draw_depth(parked).map(|_| parked_caller(parked)))
        .collect()
}

/// The callers of every Draw frame of a parked state, bottom first.
fn draw_callers(parked: &HotState) -> Vec<crate::hot::DrawCaller> {
    parked
        .frames
        .as_slice()
        .iter()
        .filter_map(|frame| match *frame {
            crate::frame::Frame::Draw { record } => {
                Some(parked.frames.draw(record).unwrap().caller)
            }
            _ => None,
        })
        .collect()
}

/// Battle Trance's own Draw parks first on an ordinary CardPlay-owned frame
/// (Hellraiser plays the drawn Seeker Strike); the third drawn card is the
/// fourth play, so Iron Club's Draw then parks on the receipt, reached
/// through the first answer's continuation.
#[test]
fn an_ordinary_park_then_iron_club_park_inside_one_battle_trance() {
    let witness = trace!("r_trance_mixed");
    assert_eq!(witness.runs, 7);
    assert_eq!(park_owners(&witness), [None, Some(IronClub)]);
}

/// Iron Club's Draw nested in a plain Strike that Hellraiser AutoPlays out
/// of Pommel Strike's own card-program Draw.
#[test]
fn iron_club_draw_nests_under_a_card_program_draw() {
    let witness = trace!("r_pommel_club");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [Some(IronClub)]);
}

/// The outer Power is AutoPlayed by Havoc: natively the choice pauses the
/// Havoc play.
#[test]
fn a_power_autoplayed_by_havoc_reproduces_the_native_pause() {
    let witness = trace!("r_havoc_outer");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [Some(GamePiece)]);
}

/// Stratagem and Hellraiser both live inside one Game Piece Draw.
#[test]
fn stratagem_and_hellraiser_inside_one_game_piece_draw() {
    let witness = trace!("r_stratagem_hellraiser");
    assert_eq!(witness.runs, 6);
    assert_eq!(parked_caller(&witness.parks[0]), GamePiece);
}

/// Burst replays the third card; the replay body is the fourth CardPlay, so
/// Iron Club's Draw parks after the second body.
#[test]
fn a_burst_replay_is_iron_clubs_fourth_card() {
    let witness = trace!("r_burst_replay");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [Some(IronClub)]);
}

/// An EndTurn-rooted park: the next turn's hand draw AutoPlays a Strike as
/// the fourth card and Iron Club's Draw is Seeker Strike. The control
/// without Iron Club parks on the hand draw itself.
#[test]
fn iron_club_parks_inside_the_turn_start_hand_draw() {
    let club = trace!("r_turnstart_club");
    assert_eq!((club.runs, &club.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&club), [Some(IronClub)]);
    let plain = trace!("r_turnstart_plain");
    assert_eq!((plain.runs, &plain.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&plain), [None]);
}

// ---- Unceasing Top and the card-or-potion effect depth ---------------------

/// The Power is the last Hand card. Game Piece's Draw is a plain Strike
/// that Hellraiser AutoPlays inside the Power's `AfterCardPlayed` walk. The
/// Strike's own empty-hand check is inside the Power's effect bracket, so
/// Unceasing Top does not draw there (`play::CardOrPotionEffect`). The
/// Power's own check, after its End and routing, draws Seeker Strike, whose
/// choice is an ordinary frame-owned park under the Top's Draw.
#[test]
fn unceasing_top_waits_for_the_outer_power_to_end_its_effect() {
    let witness = trace!("r_top_nested_choice");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [None]);
    let parked = &witness.parks[0];
    assert_eq!(
        draw_callers(parked),
        [crate::hot::DrawCaller::UnceasingTopFinish]
    );
    // The Power has left Play: only Seeker Strike is being played.
    assert_eq!(parked.piles.get(PileId::Play).len(), 1);
}

/// The same with Mummified Hand between the two relics: it picks from the
/// Hand as the Power's walk leaves it, which is empty because the Top has
/// not drawn. The Top's later draw (Bash) therefore keeps its cost.
#[test]
fn mummified_hand_sees_the_hand_before_unceasing_top_draws() {
    let witness = trace!("r_top_mummified");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    // The trace's own `hand_costs` ([2]) is compared at every point.
    assert_eq!(witness.terminals[0].piles.get(PileId::Hand).len(), 1);
}

/// The control: with no effect executing, the last Hand card's own check
/// draws one card through the Top.
#[test]
fn unceasing_top_draws_after_a_plain_manual_play() {
    let witness = trace!("top_plain");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    assert_eq!(witness.terminals[0].piles.get(PileId::Hand).len(), 1);
}

/// After the outer Power: the Top's first draw is a second Strike, played
/// with no effect executing, so that Strike's own check draws again.
#[test]
fn unceasing_top_chains_once_the_outer_power_has_ended() {
    let witness = trace!("top_after_power");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    let end = &witness.terminals[0];
    // Hellraiser, Defend, Inflame and both Strikes.
    assert_eq!(end.history.card_plays_finished_combat, 5);
    assert_eq!(end.piles.get(PileId::Hand).len(), 1);
}

/// Iron Club in Game Piece's place: its Draw's Strike is played inside the
/// fourth card's walk; the Top's draw after that card is Seeker Strike,
/// whose choice parks under the Top's own Draw frame.
#[test]
fn unceasing_top_waits_for_iron_clubs_card_to_end_its_effect() {
    let witness = trace!("top_iron_club");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [None]);
    assert_eq!(
        draw_callers(&witness.parks[0]),
        [crate::hot::DrawCaller::UnceasingTopFinish]
    );
}

/// Havoc is the outer effect: the Strike it AutoPlays does not draw through
/// the Top, so Havoc reaches the Discard pile before the Top's Strike does.
#[test]
fn unceasing_top_waits_for_havoc_to_end_its_effect() {
    let witness = trace!("top_havoc");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    assert_eq!(witness.terminals[0].piles.get(PileId::Exhaust).len(), 1);
    assert_eq!(witness.terminals[0].piles.get(PileId::Discard).len(), 2);
}

/// A Burst replay runs inside the same effect as the first body: each body
/// of Shrug It Off draws its own Strike, and the Top draws once afterwards.
#[test]
fn unceasing_top_waits_for_every_replay_body() {
    let witness = trace!("top_burst");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    let end = &witness.terminals[0];
    assert_eq!(end.piles.get(PileId::Hand).len(), 1);
    assert_eq!(end.piles.get(PileId::Draw).len(), 3);
}

/// Hellraiser itself is the last Hand card: no effect is executing when the
/// Top draws, so each AutoPlayed Strike's own check draws the next card,
/// down to a card that stays in Hand or to Seeker Strike's choice.
#[test]
fn unceasing_top_chains_through_autoplayed_strikes_with_no_effect_executing() {
    let chain = trace!("top_chain");
    assert_eq!(chain.runs, 1);
    assert!(chain.parks.is_empty());
    let choice = trace!("top_chain_choice");
    assert_eq!((choice.runs, &choice.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&choice), [None]);
}

/// A potion is the outer effect. Distilled Chaos plays Fiend Fire (which
/// exhausts the Hand), a Strike and a Defend with the Hand empty throughout;
/// none of the three draws through the Top. The potion's own check, after
/// its effect has ended, draws one card.
#[test]
fn unceasing_top_waits_for_a_potion_to_end_its_effect() {
    let witness = trace!("top_potion");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    let end = &witness.terminals[0];
    assert_eq!(end.piles.get(PileId::Hand).len(), 1);
    assert_eq!(end.history.card_plays_finished_combat, 3);
}

/// Glowwater exhausts the Hand and draws ten. The two Strikes Hellraiser
/// plays out of that Draw leave the Hand empty inside the potion's effect:
/// exactly ten cards leave the Draw pile.
#[test]
fn unceasing_top_draws_nothing_inside_a_potions_own_draw() {
    let witness = trace!("top_potion_draw");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    let end = &witness.terminals[0];
    assert_eq!(end.piles.get(PileId::Hand).len(), 8);
    assert_eq!(end.piles.get(PileId::Draw).len(), 2);
}

// ---- The Top draws the card whose own play has just routed (#3756) ----------

/// Rebound puts the manual Strike on top of the Draw pile, the Top draws it
/// from inside that Strike's own wrapper, and Hellraiser plays it again. The
/// Top's next draw is a Defend. Nostalgia is the same route.
#[test]
fn unceasing_top_redraws_the_card_whose_wrapper_is_still_open() {
    for (name, witness) in [
        ("top_rebound", trace!("top_rebound")),
        ("top_nostalgia", trace!("top_nostalgia")),
    ] {
        assert_eq!(witness.runs, 1, "{name}");
        assert!(witness.parks.is_empty(), "{name}");
        let end = &witness.terminals[0];
        // Hellraiser, the Power or Skill, and the Strike twice.
        assert_eq!(end.history.card_plays_finished_combat, 4, "{name}");
        assert_eq!(end.piles.get(PileId::Hand).len(), 1, "{name}");
    }
    // Control: a Strike that goes to Discard is not redrawn.
    let control = trace!("top_strike_then_defend");
    assert_eq!(control.terminals[0].history.card_plays_finished_combat, 2);
}

/// With nothing but Strikes left to draw, one manual play does not return
/// until the combat is over: discard, reshuffle, draw, AutoPlay, again.
#[test]
fn unceasing_top_loops_strikes_until_the_combat_ends() {
    for (name, witness) in [
        ("top_lone_strike", trace!("top_lone_strike")),
        ("top_two_strikes", trace!("top_two_strikes")),
        ("top_three_strikes", trace!("top_three_strikes")),
        ("top_loop_seeker", trace!("top_loop_seeker")),
    ] {
        assert_eq!(witness.runs, 1, "{name}");
        assert!(witness.parks.is_empty(), "{name}");
        assert!(witness.terminals[0].history.over, "{name}");
    }
}

/// The reentry, then the Top's next draw is Seeker Strike: the action
/// pauses under the inner wrapper's Top Draw, itself under the outer one's.
#[test]
fn a_choice_parks_under_both_wrappers_of_the_redrawn_card() {
    let witness = trace!("top_rebound_seeker");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(park_owners(&witness), [None]);
    assert_eq!(
        draw_callers(&witness.parks[0]),
        [
            crate::hot::DrawCaller::UnceasingTopFinish,
            crate::hot::DrawCaller::UnceasingTopFinish
        ]
    );
}

// ---- Game Piece at its recorded place in the relic walk (#3756) -------------

/// Velvet Choker counts a play in its own `AfterCardPlayed`. The sixth card
/// is a Power: recorded before the Choker, Game Piece's draw runs first and
/// the Strike Hellraiser plays out of it is the Choker's sixth; recorded
/// after it, the Choker has six and the Strike is not played.
#[test]
fn velvet_choker_counts_the_power_at_its_recorded_side_of_game_piece() {
    let after = trace!("choker_after_piece");
    let before = trace!("choker_before_piece");
    for witness in [&after, &before] {
        assert_eq!(witness.runs, 1);
        assert!(witness.parks.is_empty());
    }
    // Five Defends, the Power, and the Strike or not.
    assert_eq!(after.terminals[0].history.card_plays_finished_combat, 7);
    assert_eq!(before.terminals[0].history.card_plays_finished_combat, 6);
    // One card short of the limit, the order does not show.
    for name_and_witness in [
        (
            "choker_after_piece_fifth",
            trace!("choker_after_piece_fifth"),
        ),
        (
            "choker_before_piece_fifth",
            trace!("choker_before_piece_fifth"),
        ),
    ] {
        let (name, witness) = name_and_witness;
        assert_eq!(
            witness.terminals[0].history.card_plays_finished_combat, 6,
            "{name}"
        );
    }
}

/// Permafrost's Block under Juggernaut and the Strike Hellraiser plays out
/// of Game Piece's draw each roll a target, in the relics' recorded order.
#[test]
fn game_piece_and_permafrost_roll_their_targets_in_recorded_order_under_juggernaut() {
    let piece_first = trace!("jug_piece_perma");
    let perma_first = trace!("jug_perma_piece");
    // Game Piece first: the Strike kills the weak Toadpole and Juggernaut
    // hits the other. Permafrost first: Juggernaut wounds the weak one and
    // the Strike finishes it.
    let hp = |witness: &Witness| {
        witness.terminals[0]
            .monsters
            .iter()
            .map(|monster| monster.hp)
            .collect::<Vec<_>>()
    };
    assert_ne!(hp(&piece_first), hp(&perma_first));
    // Without a nested play both orders end in one state.
    let plain = trace!("jug_piece_perma_plain");
    let plain_reversed = trace!("jug_perma_piece_plain");
    assert_eq!(hp(&plain), hp(&plain_reversed));
}

// ---- Compositions beside another choice owner (#3903) -----------------------

fn admission_of(fixture: &Fixture) -> Result<(), crate::engine::admission::AdmissionRefusal> {
    let wire = HotBoundary::try_to_canonical(&fixture.state, &fixture.catalog).unwrap();
    admit(&wire, &fixture.state, &fixture.catalog)
}

fn horn_frames(parked: &HotState) -> (Vec<crate::hot::DrawCaller>, bool) {
    (draw_callers(parked), parked_draw_depth(parked).is_some())
}

use crate::hot::DrawCaller::GremlinHorn;

/// Game Piece's Draw is a Strike that kills, and the Horn's Draw is Seeker
/// Strike. The choice is on the Horn listener's own context, so the game
/// asks it only once the Power's PlayCardAction has finished: the park is
/// the Horn's Draw with Seeker Strike in Play and the Strike discarded.
#[test]
fn a_horn_choice_begun_inside_game_pieces_draw_is_asked_after_the_power() {
    let witness = trace!("horn_piece");
    assert_eq!((witness.runs, witness.options.as_slice()), (3, &[3][..]));
    assert_eq!(horn_frames(&witness.parks[0]), (vec![GremlinHorn], false));
}

/// The retired wall `Gremlin Horn beside a Game Piece or Iron Club Draw
/// choice`. The Strike Game Piece draws kills; the Horn draws Seeker
/// Strike and its choice is queued; then Iron Club draws Sculpting Strike
/// and parks the Power's action with the hook action still queued. The
/// game asks the relic's choice first and the Horn's after the action,
/// and by then Sculpting Strike has left the Draw pile, so Seeker Strike's
/// short list offers one card fewer. Both inventory orders.
#[test]
fn a_relic_choice_is_asked_before_the_horn_choice_queued_behind_it() {
    for name in ["horn_piece_club", "horn_club_piece"] {
        crate::engine::hook_action::take_branch_hits();
        let witness = match name {
            "horn_piece_club" => trace!("horn_piece_club"),
            _ => trace!("horn_club_piece"),
        };
        let hits = crate::engine::hook_action::take_branch_hits();
        assert_eq!(witness.runs, 4, "{name}");
        assert_eq!(witness.options, vec![2, 2], "{name}");
        // The relic's park carries no queue and is the receipt's.
        let relic = &witness.parks[0];
        assert!(!relic.fanouts.deferred_hook_action_is_queued(), "{name}");
        assert_eq!(parked_caller(relic), IronClub, "{name}");
        assert!(!draw_callers(relic).contains(&GremlinHorn), "{name}");
        // The Horn's park follows it, on the same receipt.
        assert_eq!(
            horn_frames(&witness.parks[1]),
            (vec![GremlinHorn], false),
            "{name}"
        );
        assert!(hits.narrowed > 0 && hits.auto_taken == 0, "{name}");
    }
}

/// One short-listed card is left when the hook action starts, and the game
/// takes it without asking: the relic's choice is the only pause.
#[test]
fn a_horn_choice_left_with_one_option_is_taken_unasked() {
    crate::engine::hook_action::take_branch_hits();
    let witness = trace!("horn_seeker_short");
    let hits = crate::engine::hook_action::take_branch_hits();
    assert_eq!((witness.runs, witness.options.as_slice()), (2, &[2][..]));
    assert!(hits.auto_taken > 0);
    for end in &witness.terminals {
        assert!(end.piles.get(PileId::Draw).is_empty());
        assert_eq!(end.piles.get(PileId::Hand).len(), 3);
    }
}

/// The Horn's Sculpting Strike asks over the Hand as the hook action finds
/// it: Iron Club's later Draw has put a third card there.
#[test]
fn a_horn_choice_over_the_hand_reads_it_when_the_hook_action_starts() {
    let witness = trace!("horn_sculpting_moved");
    assert_eq!((witness.runs, witness.options.as_slice()), (3, &[3][..]));
    assert_eq!(horn_frames(&witness.parks[0]), (vec![GremlinHorn], false));
}

/// The relic's choice comes first and the kill after its answer, inside
/// the receipt's re-execution: the Horn's choice is queued there as it is
/// on a first run.
#[test]
fn a_horn_choice_begun_after_a_relic_answer_is_queued_by_the_re_execution() {
    let witness = trace!("horn_after_answer");
    assert_eq!(witness.runs, 7);
    assert_eq!(parked_caller(&witness.parks[0]), GamePiece);
    assert_eq!(horn_frames(&witness.parks[1]), (vec![GremlinHorn], false));
}

/// The retired wall `Iron Club Draw choice inside a deferred-choice
/// listener`, in the walk that has no `WhenAll`. Bash kills; the Horn
/// draws a plain Strike that is Iron Club's fourth card; Iron Club draws
/// Seeker Strike. That choice is the Horn listener's first, so the game
/// finishes Bash (Iron Club counts it fifth) and only then asks, with
/// both Strikes in Play. The park is Iron Club's Draw above the Horn's;
/// each answer runs the rest of the listener after the action.
#[test]
fn iron_clubs_draw_inside_the_horn_listener_is_the_listeners_hook_action() {
    crate::engine::hook_action::take_branch_hits();
    let witness = trace!("horn_club_nested");
    let hits = crate::engine::hook_action::take_branch_hits();
    assert_eq!((witness.runs, witness.options.as_slice()), (3, &[3][..]));
    assert_eq!(
        horn_frames(&witness.parks[0]),
        (vec![GremlinHorn, IronClub], true)
    );
    assert!(hits.receipt_choices_queued > 0 && hits.hook_actions_started >= 3);
    for end in &witness.terminals {
        assert!(end.piles.get(PileId::Play).is_empty());
        assert_eq!(end.fanouts.iron_club_cards(), 2);
    }
}

/// The three retired walls' roots admit: Gremlin Horn, a Whispering Earring
/// past its loop and an armed Centennial Puzzle, each beside either relic
/// with Hellraiser live. (Ahead of its
/// loop the Earring's own wall, `Whispering Earring bounded live Hand loop`,
/// refuses a root that reaches a selecting Strike, whatever else it
/// owns.)
#[test]
fn the_retired_walls_roots_admit() {
    for relic in [RelicId::RelicGamePiece, RelicId::RelicIronClub] {
        for beside in [
            RelicId::RelicGremlinHorn,
            RelicId::RelicWhisperingEarring,
            RelicId::RelicCentennialPuzzle,
        ] {
            let mut fixture = hellraiser_root(&[relic, beside], CardId::Inflame, &TAIL);
            fixture
                .state
                .fanouts
                .set_gremlin_horn_owned(beside == RelicId::RelicGremlinHorn);
            fixture
                .state
                .fanouts
                .set_puzzle_armed(beside == RelicId::RelicCentennialPuzzle);
            assert_eq!(admission_of(&fixture), Ok(()), "{relic:?} {beside:?}");
        }
    }
    // Ahead of the Earring's loop the root is the Earring's own to refuse.
    let mut ahead = hellraiser_root(
        &[RelicId::RelicGamePiece, RelicId::RelicWhisperingEarring],
        CardId::Inflame,
        &TAIL,
    );
    ahead.state.turn = 0;
    assert!(crate::engine::turn::whispering_earring_loop_is_ahead(
        &ahead.state
    ));
    assert!(crate::engine::puzzle::root_refuses_under_for_test(
        &ahead.state,
        &ahead.catalog,
        "Whispering Earring bounded live Hand loop"
    ));
    let mut bare = hellraiser_root(&[RelicId::RelicWhisperingEarring], CardId::Inflame, &TAIL);
    bare.state.turn = 0;
    assert!(crate::engine::puzzle::root_refuses_under_for_test(
        &bare.state,
        &bare.catalog,
        "Whispering Earring bounded live Hand loop"
    ));
}

/// A potion wrapper nested in a card's effect. Bloodletting's HP loss is
/// lethal and Fairy in a Bottle is used from inside the card's body, on the
/// Rust stack of a play that has no frame. The potion's own empty-hand
/// check finds the card's effect still executing
/// (`play::card_or_potion_effect_is_executing`), so the Top draws only
/// after Bloodletting has reached the Discard pile: it lies under both
/// Strikes there.
#[test]
fn unceasing_top_waits_for_the_card_around_fairy_in_a_bottle() {
    let witness = trace!("top_fairy");
    assert_eq!(witness.runs, 1);
    assert!(witness.parks.is_empty());
    let end = &witness.terminals[0];
    assert_eq!(end.fanouts.potion_slots(), [None, None, None]);
    assert_eq!(end.piles.get(PileId::Discard).len(), 3);
    assert_eq!(end.piles.get(PileId::Hand).len(), 1);
}

/// The reader itself: a synchronous card effect on the Rust stack, a
/// suspended card play's frame and a potion's Effect-stage frame each count;
/// a potion past its effect and an empty stack do not.
#[test]
fn an_effect_is_executing_on_the_rust_stack_or_on_a_frame() {
    use crate::engine::play::{
        card_or_potion_effect_is_executing, with_test_synchronous_effect_depth,
    };
    let fixture = hellraiser_root(&[RelicId::RelicGamePiece], CardId::Inflame, &TAIL);
    let (state, catalog) = rooted(&fixture);
    assert!(!card_or_potion_effect_is_executing(&state));
    assert!(with_test_synchronous_effect_depth(1, || {
        card_or_potion_effect_is_executing(&state)
    }));
    // The parked Seeker Strike is a suspended play: its frame counts.
    let parked = apply_action(&state, &catalog, &PLAY_POWER).unwrap().state;
    assert!(card_or_potion_effect_is_executing(&parked));
    for (stage, executing) in [
        (crate::frame::PotionFinishStage::Effect, true),
        (crate::frame::PotionFinishStage::AfterTop, false),
    ] {
        let mut potion = state.clone();
        potion
            .frames
            .push_potion_finish(&crate::hot::PotionFinishRecord {
                name: crate::ids::PotionId::SwiftPotion,
                stage,
                body_stage: crate::hot::PotionBodyStage::Synchronous,
                current_uid: None,
                aux: 0,
                candidates: Vec::new(),
                generation_options: Vec::new(),
            })
            .unwrap();
        assert_eq!(
            card_or_potion_effect_is_executing(&potion),
            executing,
            "{stage:?}"
        );
    }
}

// ---- IL-only witnesses (no native trace) ----------------------------------

const TAIL: [CardId; 4] = [
    CardId::SeekerStrike,
    CardId::DefendIronclad,
    CardId::DefendIronclad,
    CardId::DefendIronclad,
];

/// A Hellraiser root over a one-monster fight holding `relics` as a vouched
/// inventory, `hand` (uid 1) in Hand and `draw` on the Draw pile, top first.
fn hellraiser_root(relics: &[RelicId], hand: CardId, draw: &[CardId]) -> Fixture {
    root_with_power(relics, hand, draw, Some(PowerId::Hellraiser))
}

fn root_with_power(
    relics: &[RelicId],
    hand: CardId,
    draw: &[CardId],
    power: Option<PowerId>,
) -> Fixture {
    let mut ids = vec![hand];
    for id in draw {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    let identities = ids.iter().map(|id| plain(*id)).collect::<Vec<_>>();
    let mut fixture = fixture_in_order(&identities, relics, 2, true);
    if let Some(live) = power {
        fixture.state.powers.set(live, SlotWire::Int, 1);
    }
    crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut fixture.state);
    let atoms = fixture.atoms.clone();
    let atom = |id: CardId| atoms[ids.iter().position(|known| *known == id).unwrap()];
    let piles = &mut fixture.state.piles;
    piles
        .get_mut(PileId::Hand)
        .make_mut()
        .push(card(1, atom(hand)));
    for (index, id) in draw.iter().enumerate() {
        let uid = u32::try_from(index).unwrap() + 2;
        piles
            .get_mut(PileId::Draw)
            .make_mut()
            .push(card(uid, atom(*id)));
    }
    fixture
}

/// The claim `draw::game_piece_draw` rests on: Hellraiser plays only a
/// Strike-tagged card, and no Strike-tagged row of this build is a Power,
/// so the nested play cannot re-enter Game Piece.
#[test]
fn every_strike_tagged_row_is_an_attack() {
    let mut strikes = 0;
    for name in CardId::NAMES {
        let id = CardId::from_str(name).unwrap();
        for upgrade in 0..=1 {
            let Some(row) = crate::content_tables::card_row(id, upgrade) else {
                continue;
            };
            if row.strike_tag {
                strikes += 1;
                assert!(!row.is_power && !row.is_skill, "{}", row.name);
            }
        }
    }
    assert!(strikes > 30, "{strikes}");
}

/// With neither Hellraiser nor Stratagem live the certified synchronous
/// command runs and no frame is pushed.
#[test]
fn without_a_blocking_listener_the_relic_draw_stays_synchronous() {
    let fixture = root_with_power(&[RelicId::RelicGamePiece], CardId::Inflame, &TAIL, None);
    let (state, catalog) = rooted(&fixture);
    let next = apply_action(&state, &catalog, &PLAY_POWER).unwrap().state;
    assert!(next.pending.is_none() && next.frames.is_empty());
    // The Strike is drawn into the Hand, not played.
    assert_eq!(next.piles.get(PileId::Hand).len(), 1);
    assert_eq!(next.history.card_plays_finished_combat, 1);
}

/// Forged owner records refuse: a rewritten caller, the relic missing from
/// the catalog, a Draw already complete, and a widened request.
#[test]
fn forged_after_card_played_relic_owner_records_refuse() {
    for (relic, other, name, forged_name, plays) in [
        (
            RelicId::RelicGamePiece,
            RelicId::RelicIronClub,
            "game_piece",
            "iron_club",
            0,
        ),
        (
            RelicId::RelicIronClub,
            RelicId::RelicGamePiece,
            "iron_club",
            "game_piece",
            3,
        ),
    ] {
        let mut fixture = hellraiser_root(&[relic], CardId::Inflame, &TAIL);
        if relic == RelicId::RelicIronClub {
            assert!(fixture.state.fanouts.set_iron_club_cards(plays));
        }
        let (state, catalog) = rooted(&fixture);
        let parked = apply_action(&state, &catalog, &PLAY_POWER).unwrap().state;
        let depth = parked_draw_depth(&parked).unwrap();
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(
            wire.continuations[depth].fields.get("caller"),
            Some(&json!(name))
        );
        let loaded_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        assert!(HotBoundary::from_canonical(&wire, &loaded_catalog).is_ok());
        assert!(crate::engine::play::persisted_card_play_stack_is_exact(&parked, &catalog).is_ok());
        for (field, value) in [
            ("caller", json!("none")),
            // The other relic's caller, which this catalog does not own.
            ("caller", json!(forged_name)),
            ("completed", json!(1)),
            ("requested", json!(2)),
        ] {
            let mut forged = wire.clone();
            forged.continuations[depth]
                .fields
                .insert(field.to_owned(), value.clone());
            assert!(
                HotBoundary::from_canonical(&forged, &loaded_catalog).is_err(),
                "{name}: {field} = {value}"
            );
        }
        // The same stack against a catalog that owns only the other relic.
        let bare = hellraiser_root(&[other], CardId::Inflame, &TAIL);
        assert_eq!(bare.atoms, fixture.atoms);
        assert!(
            crate::engine::play::persisted_card_play_stack_is_exact(&parked, &bare.catalog)
                .is_err(),
            "{name}"
        );
    }
}

/// Under Whispering Earring's selector the game never offers the choice
/// (`puzzle::VAKUU_SELECTOR_CHOICE`), so the park and its answers refuse
/// by that name rather than publish a state the Earring would not reach.
#[test]
fn game_piece_choice_under_the_vakuu_selector_refuses_by_name() {
    use crate::engine::selection::VakuuSelectorScope;
    let refused = EngineRefusal::MalformedArgs(crate::engine::puzzle::VAKUU_SELECTOR_CHOICE);
    let fixture = hellraiser_root(&[RelicId::RelicGamePiece], CardId::Inflame, &TAIL);
    let (state, catalog) = rooted(&fixture);
    let parked = apply_action(&state, &catalog, &PLAY_POWER).unwrap().state;
    let answers = legal_actions(&parked, &catalog);
    assert_eq!(answers.len(), 3);
    let _vakuu = VakuuSelectorScope::enter();
    assert_eq!(
        apply_action(&state, &catalog, &PLAY_POWER).map(|next| next.state),
        Err(refused.clone())
    );
    for answer in &answers {
        assert_eq!(
            apply_action(&parked, &catalog, answer).map(|next| next.state),
            Err(refused.clone())
        );
    }
}

/// A Power AutoPlayed from the Draw pile by Havoc draws through Game Piece
/// inside the AutoPlay; the choice parks the Havoc play and every answer
/// finishes the Strike, the Power and Havoc once.
#[test]
fn a_power_autoplayed_by_havoc_parks_inside_the_autoplay() {
    let fixture = hellraiser_root(
        &[RelicId::RelicGamePiece],
        CardId::Havoc,
        &[
            CardId::Inflame,
            CardId::SeekerStrike,
            CardId::DefendIronclad,
            CardId::DefendIronclad,
            CardId::DefendIronclad,
        ],
    );
    let (state, catalog) = rooted(&fixture);
    let parked = apply_action(&state, &catalog, &PLAY_POWER).unwrap().state;
    assert_eq!(parked_caller(&parked), GamePiece);
    let (terminals, parks) = warm_and_cold_agree(&parked, &catalog);
    assert_eq!((terminals.len(), parks), (3, 1));
    for terminal in &terminals {
        // Havoc, Inflame and Seeker Strike each finish once.
        assert_eq!(terminal.history.card_plays_finished_combat, 3);
        assert_eq!(terminal.powers.value(PowerId::Strength), 2);
        assert!(terminal.piles.get(PileId::Play).is_empty());
    }
}

/// #3822: the two retired walls. A root that merely REACHES the chain
/// (Game Piece owned, Hellraiser and a selecting Strike in the catalog,
/// neither live nor held) admits, and so does a live Stratagem beside a
/// reachable Power.
#[test]
fn roots_that_only_reach_the_chain_admit() {
    let fixture = root_with_power(
        &[RelicId::RelicGamePiece],
        CardId::Inflame,
        &[
            CardId::Hellraiser,
            CardId::SeekerStrike,
            CardId::SculptingStrike,
            CardId::DefendIronclad,
        ],
        None,
    );
    let wire = HotBoundary::try_to_canonical(&fixture.state, &fixture.catalog).unwrap();
    assert_eq!(admit(&wire, &fixture.state, &fixture.catalog), Ok(()));

    let stratagem = root_with_power(
        &[RelicId::RelicGamePiece],
        CardId::Inflame,
        &[CardId::DefendIronclad, CardId::DefendIronclad],
        Some(PowerId::Stratagem),
    );
    let wire = HotBoundary::try_to_canonical(&stratagem.state, &stratagem.catalog).unwrap();
    assert_eq!(admit(&wire, &stratagem.state, &stratagem.catalog), Ok(()));
}

// ---- Hellraiser's nine AutoPlays a turn against an infinite HP display (#3929) ----

macro_rules! cap_trace {
    ($name:literal) => {
        native_trace(include_str!(concat!(
            "../../../../fixtures/issue3929_native/",
            $name,
            ".json"
        )))
    };
}

fn cap_count(state: &HotState) -> u8 {
    state.fanouts.hellraiser_infinite_auto_plays()
}

/// Skulking Colony under Unceasing Top with fifteen distinct Strikes. The
/// game makes thirteen plays: the manual one and two AutoPlays fill Hardened
/// Shell (6 + 6 + 6, then 2 of the fourth), nine more are counted against
/// the infinite display, and the next Strike the Top draws stays in Hand.
/// `main` played all fifteen. `cap_colony_13` is the same against a cycling
/// deck: the Strike left in Hand comes out of a reshuffle.
#[test]
fn skulking_colony_stops_hellraiser_after_nine_counted_auto_plays() {
    for (name, witness) in [
        ("cap_colony_15", cap_trace!("cap_colony_15")),
        ("cap_colony_13", cap_trace!("cap_colony_13")),
    ] {
        let end = &witness.terminals[0];
        assert_eq!(cap_count(end), 9, "{name}");
        assert_eq!(end.piles.get(PileId::Hand).len(), 1, "{name}");
        assert_eq!(end.history.card_plays_finished_combat, 13, "{name}");
        assert_eq!(
            end.monsters[0].skulking_colony_shell_window(),
            crate::engine::damage::HARDENED_SHELL,
            "{name}"
        );
        assert!(end.monsters[0].hp_display_is_infinite(), "{name}");
    }
}

/// Control, short of the shell: Battle Trance draws three Strikes, 18
/// damage, a finite display throughout, no count.
#[test]
fn skulking_colony_below_its_shell_counts_nothing() {
    let witness = cap_trace!("cap_colony_trance");
    let end = &witness.terminals[0];
    assert_eq!(cap_count(end), 0);
    assert!(!end.monsters[0].hp_display_is_infinite());
    assert!(end.piles.get(PileId::Hand).is_empty());
}

/// The turn after the stop. `HardenedShellPower::BeforeSideTurnStart` clears
/// the display at each side's start and Hellraiser's own `AfterSideTurnEnd`
/// the count, so the hand draw's Strikes play again (four of them refill the
/// shell before the Defend arrives).
#[test]
fn skulking_colony_next_turn_plays_again() {
    let witness = cap_trace!("cap_colony_next_turn");
    let end = &witness.terminals[0];
    assert_eq!(end.turn, 2);
    assert_eq!(cap_count(end), 0);
    assert!(end.monsters[0].hp_display_is_infinite());
}

/// A Waterfall Giant about to blow shows an infinite display from the kill
/// on: the manual Strike and nine AutoPlays, then the tenth drawn Strike
/// stays in Hand.
#[test]
fn a_giant_about_to_blow_stops_hellraiser_after_nine() {
    let witness = cap_trace!("cap_giant_15");
    let end = &witness.terminals[0];
    assert!(end.monsters[0].is_about_to_blow());
    assert_eq!(cap_count(end), 9);
    assert_eq!(end.piles.get(PileId::Hand).len(), 1);
    assert_eq!(end.history.card_plays_finished_combat, 10);
}

/// The Giant's display never clears, so the next turn's four AutoPlays are
/// the witness of `HellraiserPower::AfterSideTurnEnd`'s reset: without it
/// the count would still stand at nine and the hand draw would play nothing.
#[test]
fn hellraisers_side_end_resets_the_count_against_a_giant_still_about_to_blow() {
    let witness = cap_trace!("cap_giant_next_turn");
    let end = &witness.terminals[0];
    assert_eq!(end.turn, 2);
    assert!(end.monsters[0].is_about_to_blow());
    assert_eq!(cap_count(end), 4);
}

/// No hittable enemy: a Test Subject between forms. `All` over no enemy is
/// true, so every drawn Strike is counted. Its AutoPlay has no target and
/// puts the Strike in Discard unplayed (no CardPlay starts); after nine of
/// those (three Swift Potions) Offering's three Strikes stay in Hand.
#[test]
fn no_hittable_enemy_counts_and_stops_after_nine() {
    let witness = cap_trace!("cap_subject_potions");
    let end = &witness.terminals[0];
    assert!(end.monsters[0].test_subject_adaptable_reviving());
    assert_eq!(cap_count(end), 9);
    assert_eq!(end.piles.get(PileId::Hand).len(), 3);
    // The manual Strike and Offering: no AutoPlay started a CardPlay.
    assert_eq!(end.history.card_plays_finished_combat, 2);

    let trance = cap_trace!("cap_subject_trance");
    let end = &trance.terminals[0];
    assert_eq!(cap_count(end), 3);
    assert!(end.piles.get(PileId::Hand).is_empty());
}

/// The rule itself, one drawn Strike at a time
/// (`draw::hellraiser_should_auto_play`): a finite hittable enemy resets the
/// count and plays; an all-infinite roster, or no hittable enemy, counts and
/// plays nine times and then stops, still holding nine.
#[test]
fn the_count_runs_only_while_every_hittable_enemy_shows_an_infinite_display() {
    use crate::engine::draw::hellraiser_should_auto_play as rule;
    let colony = |window: i32, hp: i32| {
        let mut monster = HotMonster::new(MonsterKind::SkulkingColony, hp);
        assert!(monster.set_skulking_colony_shell_window(window));
        monster
    };
    let giant = |about_to_blow: bool| {
        let mut monster = HotMonster::new(MonsterKind::WaterfallGiant, 100);
        monster.set_is_about_to_blow(about_to_blow);
        monster
    };
    let state_of = |monsters: Vec<HotMonster>| {
        let mut state = HotState::at_defaults();
        state.monsters = std::sync::Arc::new(monsters);
        state
    };
    let shell = crate::engine::damage::HARDENED_SHELL;
    assert!(colony(shell, 50).hp_display_is_infinite());
    assert!(!colony(shell - 1, 50).hp_display_is_infinite());
    assert!(giant(true).hp_display_is_infinite());
    assert!(!giant(false).hp_display_is_infinite());
    assert!(!HotMonster::new(MonsterKind::Toadpole, 10).hp_display_is_infinite());

    // All infinite: nine plays, then none, and the count stays at nine.
    for monsters in [
        vec![colony(shell, 50)],
        vec![giant(true)],
        // A dead enemy is not hittable, whatever it shows.
        vec![giant(true), HotMonster::new(MonsterKind::Toadpole, 0)],
        // No hittable enemy at all.
        vec![HotMonster::new(MonsterKind::Toadpole, 0)],
        Vec::new(),
    ] {
        let mut state = state_of(monsters);
        for played in 1..=crate::hot::HELLRAISER_INFINITE_AUTO_PLAY_CAP {
            assert!(rule(&mut state));
            assert_eq!(cap_count(&state), played);
        }
        for _ in 0..3 {
            assert!(!rule(&mut state));
            assert_eq!(cap_count(&state), 9);
        }
    }
    // One finite hittable enemy: always played, and a running count resets.
    for monsters in [
        vec![colony(shell - 1, 50)],
        vec![giant(false)],
        vec![giant(true), HotMonster::new(MonsterKind::Toadpole, 10)],
    ] {
        let mut state = state_of(monsters);
        assert!(state.fanouts.set_hellraiser_infinite_auto_plays(9));
        assert!(rule(&mut state));
        assert_eq!(cap_count(&state), 0);
        assert!(rule(&mut state));
        assert_eq!(cap_count(&state), 0);
    }
    // The slot holds nothing past the cap.
    let mut state = state_of(Vec::new());
    assert!(!state.fanouts.set_hellraiser_infinite_auto_plays(10));
    assert_eq!(cap_count(&state), 0);
}

/// The count is document state: it projects under its own name only when
/// nonzero, reloads, and continues identically warm and cold; a value past
/// the cap does not load, and a count with no Hellraiser refuses at
/// admission.
#[test]
fn the_count_is_a_document_field() {
    const NAME: &str = "hellraiser_infinite_auto_plays_this_turn";
    let file: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/issue3929_native/cap_giant_15.json"
    ))
    .unwrap();
    let root = root(&file["spec"], &file["runs"][0]["trace"][0]["snap"]);
    let wire = HotBoundary::try_to_canonical(&root.state, &root.catalog).unwrap();
    assert!(!wire.player.contains_key(NAME));
    let strike = Action::Play {
        uid: uid_of(&root.state, &root.catalog, "STRIKE_IRONCLAD"),
        target: Some(0),
        selection: SelectionRef::NONE,
    };
    let stopped = apply_action(&root.state, &root.catalog, &strike)
        .unwrap()
        .state;
    let wire = HotBoundary::try_to_canonical(&stopped, &root.catalog).unwrap();
    assert_eq!(wire.player[NAME], json!(9));
    let (loaded, loaded_catalog) = cold(&stopped, &root.catalog);
    assert_eq!(cap_count(&loaded), 9);
    // The Strike left in Hand is an ordinary card: playing it by hand draws
    // the next one, which stays too, warm and cold alike.
    let again = |state: &HotState, catalog: &Catalog| {
        let play = Action::Play {
            uid: uid_of(state, catalog, "STRIKE_IRONCLAD"),
            target: Some(0),
            selection: SelectionRef::NONE,
        };
        let next = apply_action(state, catalog, &play).unwrap().state;
        assert_eq!(next.piles.get(PileId::Hand).len(), 1);
        assert_eq!(cap_count(&next), 9);
        HotBoundary::try_to_canonical(&next, catalog)
            .unwrap()
            .differential_digest()
    };
    assert_eq!(
        again(&stopped, &root.catalog),
        again(&loaded, &loaded_catalog)
    );

    let mut past = wire.clone();
    past.player.insert(NAME.to_owned(), json!(10));
    let catalog = HotBoundary::catalog_from_canonical(&past).unwrap();
    assert!(HotBoundary::from_canonical(&past, &catalog).is_err());

    // A root with no Hellraiser power, holding a count.
    let plain = root_with_power(
        &[RelicId::RelicGamePiece],
        CardId::Inflame,
        &[CardId::DefendIronclad, CardId::DefendIronclad],
        None,
    );
    let mut forged = HotBoundary::try_to_canonical(&plain.state, &plain.catalog).unwrap();
    forged.player.insert(NAME.to_owned(), json!(1));
    let catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
    let state = HotBoundary::from_canonical(&forged, &catalog).unwrap();
    let refusal = admit(&forged, &state, &catalog).unwrap_err();
    assert!(
        format!("{refusal:?}").contains("Hellraiser infinite AutoPlay count"),
        "{refusal:?}"
    );
}

/// A first Hellraiser is a new power object with a zeroed count
/// (`steps::templates::hellraiser`): a stale value cannot survive into it.
#[test]
fn a_new_hellraiser_starts_its_count_at_zero() {
    let fixture = root_with_power(
        &[RelicId::RelicGamePiece],
        CardId::Hellraiser,
        &[CardId::DefendIronclad, CardId::DefendIronclad],
        None,
    );
    let mut state = fixture.state.clone();
    assert!(state.fanouts.set_hellraiser_infinite_auto_plays(7));
    let played = apply_action(&state, &fixture.catalog, &PLAY_POWER)
        .unwrap()
        .state;
    assert_eq!(played.powers.value(PowerId::Hellraiser), 1);
    assert_eq!(cap_count(&played), 0);
}

/// The loop of #3756 against an infinite display: a lone Strike under
/// Unceasing Top. Each play routes the Strike to Discard, the Top reshuffles
/// and draws it, and Hellraiser plays it again inside the wrapper that has
/// just routed it. The game finishes the action: thirteen plays against
/// Skulking Colony (four to fill the shell, nine counted), ten against a
/// Giant about to blow, and the Strike is left in Hand both times. Without
/// the count this loop ran to the wrapper limit.
#[test]
fn the_same_uid_loop_ends_at_the_ninth_counted_auto_play() {
    for (name, witness, plays) in [
        ("cap_colony_lone", cap_trace!("cap_colony_lone"), 13),
        ("cap_giant_lone", cap_trace!("cap_giant_lone"), 10),
    ] {
        let end = &witness.terminals[0];
        assert!(end.pending.is_none() && end.frames.is_empty(), "{name}");
        assert_eq!(cap_count(end), 9, "{name}");
        assert_eq!(end.piles.get(PileId::Hand).len(), 1, "{name}");
        assert!(end.piles.get(PileId::Discard).is_empty(), "{name}");
        assert_eq!(end.history.card_plays_finished_combat, plays, "{name}");
    }
}

/// The same root against a Test Subject the manual Strike kills into its
/// revive. The Top does draw the Strike back; nothing can be hit, so the
/// draw is counted once and the AutoPlay discards the Strike without a
/// CardPlay. That play is not an `OnPlayWrapper`, so there is no second
/// `CheckForEmptyHand` and the action ends with the Hand empty.
#[test]
fn the_same_uid_redraw_with_no_hittable_enemy_is_counted_and_discarded() {
    let witness = cap_trace!("cap_subject_lone");
    let end = &witness.terminals[0];
    assert!(end.pending.is_none() && end.frames.is_empty());
    assert_eq!(cap_count(end), 1);
    assert!(end.piles.get(PileId::Hand).is_empty());
    assert_eq!(end.piles.get(PileId::Discard).len(), 1);
    assert_eq!(end.history.card_plays_finished_combat, 1);
}

/// The loop against finite enemies, on a fresh seed: the scenarios of the
/// independent review of #3921 (`rv_*` in the probe). Each relic counts
/// every AutoPlay of the loop at its own recorded place (Kunai, Shuriken,
/// Ornamental Fan, Nunchaku, Pen Nib, Pocketwatch, Letter Opener, Iron Club
/// before and after the Top), an enemy dies mid-loop, Havoc, Distilled Chaos
/// and Swift Potion start the loop, Rebound and Nostalgia feed it, and
/// Seeker Strike parks it with several wrappers open.
#[test]
fn the_loop_against_finite_enemies_matches_the_review_traces() {
    for (name, witness) in [
        ("rv_base", trace!("rv_base")),
        ("rv_kunai", trace!("rv_kunai")),
        ("rv_kunai_first", trace!("rv_kunai_first")),
        ("rv_shuriken", trace!("rv_shuriken")),
        ("rv_fan", trace!("rv_fan")),
        ("rv_nunchaku", trace!("rv_nunchaku")),
        ("rv_pennib", trace!("rv_pennib")),
        ("rv_pocketwatch", trace!("rv_pocketwatch")),
        ("rv_all2", trace!("rv_all2")),
        ("rv_kill_one", trace!("rv_kill_one")),
        ("rv_kill_one_b", trace!("rv_kill_one_b")),
        ("rv_seeker_deep", trace!("rv_seeker_deep")),
        ("rv_havoc", trace!("rv_havoc")),
        ("rv_havoc_lone", trace!("rv_havoc_lone")),
        ("rv_dchaos", trace!("rv_dchaos")),
        ("rv_dchaos_lone", trace!("rv_dchaos_lone")),
        ("rv_swift", trace!("rv_swift")),
        ("rv_rebound_loop", trace!("rv_rebound_loop")),
        ("rv_nostalgia_loop", trace!("rv_nostalgia_loop")),
        ("rv_club", trace!("rv_club")),
        ("rv_club_last", trace!("rv_club_last")),
    ] {
        assert_eq!((witness.runs, witness.parks.len()), (1, 0), "{name}");
    }
    // Seeker Strike drawn four wrappers deep: three answers.
    let deep = trace!("rv_seeker_deep4");
    assert_eq!((deep.runs, &deep.options[..]), (3, &[3][..]));
    // Iron Club's fourth card inside the loop draws Seeker Strike: two.
    let club = trace!("rv_club_seeker");
    assert_eq!((club.runs, &club.options[..]), (2, &[2][..]));
}

/// Velvet Choker ends the Unceasing Top loop: after the turn's sixth card
/// the next Strike drawn cannot be played, so its AutoPlay discards it with
/// no CardPlay and no further `CheckForEmptyHand`. The Hand ends empty, with
/// the Choker before or after the Top in the inventory, and with a lone
/// Strike cycling (scenarios of the independent review of #3926, fresh seed).
#[test]
fn velvet_choker_stops_the_unceasing_top_loop_at_six_cards() {
    for (name, witness) in [
        ("rv_top_vc", trace!("rv_top_vc")),
        ("rv_vc_top", trace!("rv_vc_top")),
        ("rv_vc_top_lone", trace!("rv_vc_top_lone")),
    ] {
        assert_eq!((witness.runs, witness.parks.len()), (1, 0), "{name}");
        let end = &witness.terminals[0];
        assert_eq!(end.history.card_plays_finished_combat, 6, "{name}");
        assert!(end.piles.get(PileId::Hand).is_empty(), "{name}");
    }
}

/// A selecting Strike drawn while the count runs. Seeker Strike is the
/// second counted AutoPlay against the filled shell and pauses on its
/// choice with the count at two; each of the three answers resumes that
/// same listener and leaves the count at two: a resume does not count the
/// draw again, warm or from a cold reload of the park.
#[test]
fn a_parked_selecting_strike_is_counted_once() {
    let witness = cap_trace!("cap_colony_seeker");
    assert_eq!((witness.runs, &witness.options[..]), (3, &[3][..]));
    assert_eq!(cap_count(&witness.parks[0]), 2);
    for terminal in &witness.terminals {
        assert_eq!(cap_count(terminal), 2);
        assert_eq!(terminal.history.card_plays_finished_combat, 6);
    }
}

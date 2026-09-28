//! `sts-sim diff-serve`: the persistent line-delimited JSON protocol the
//! trajectory differential drives (PORT_PLAN §4).
//!
//! K trajectories cost one process, not K `cargo run`s. The loop is binary-
//! local (it is not part of the `sts_sim` library surface) because it is a
//! transport, not a mechanic.
//!
//! # Protocol `diff-serve-v1`
//!
//! On start the server writes one greeting line:
//!
//! ```json
//! {"protocol":"diff-serve-v1"}
//! ```
//!
//! Then, one JSON object per input line:
//!
//! | request | meaning |
//! |---|---|
//! | `{"cmd":"load","entry":<canonical v2 document>}` | admit an entry |
//! | `{"cmd":"legal"}` | legal actions in the loaded state |
//! | `{"cmd":"apply","action":<action>}` | apply an action |
//! | `{"cmd":"apply","action":<action>,"describe_selection":true}` | also return ordered `selected_uids` for physical choices (null for nonphysical choices) |
//! | `{"cmd":"apply","action":<action>,"native_checkpoints":true}` | also return `native_checkpoints`: each native checkpoint boundary the action passed inside the one apply, in order, as `{"kind","state"}` (`engine::native_checkpoint`, #3242); `state` is null with a `refusal` detail when that state does not project |
//! | `{"cmd":"apply","action":<action>,"presentation_marks":true}` | also return `presentation_marks`: each automatic play the action ran, as ordered `{"kind":"autoplay_begin"\|"autoplay_end","source","card","state"}` with a visible-state summary (`engine::presentation`); review rows only, never compared |
//! | `{"cmd":"resolve_selection","uids":[…]}` | the unique `select` action applying exactly these uids in this order (`engine::recorded_selection_answer`): an ordered-extension answer kept out of `legal` (Ashwater/Gambler's Brew, #2524), else the one offered answer whose decode names them (#3125); `{"action":null}` when none does, a refusal when two do |
//! | `{"cmd":"project"}` | project the loaded state to canonical v2 |
//! | `{"cmd":"intents"}` | each monster's displayed intent (presentation only; see below) |
//! | `{"cmd":"manifest"}` | the derived capability manifest (D6) |
//! | `{"cmd":"coverage"[,"reset":true]}` | which kinds have been exercised so far |
//! | `{"cmd":"queen_burn_bright_smoke"}` | exact R41 Queen/Burn move-state witness |
//! | `{"cmd":"queen_puppet_strings_smoke"}` | exact R42 Queen/Puppet Strings move-state witness |
//! | `{"cmd":"spectral_hex_smoke"}` | exact R42 Spectral/Hex move-state witness |
//! | `{"cmd":"magi_dampen_smoke"}` | exact R47 Magi/Dampen move-state witness |
//! | `{"cmd":"whistle_smoke"}` | exact R46 Queen/Whistle card-step witness |
//! | `{"cmd":"thieving_hopper_smoke"}` | exact R44 Hopper three-move/readers witness |
//! | `{"cmd":"aeonglass_intensity_smoke"}` | exact R48 Aeonglass startup/cycle/Intensity witness |
//! | `{"cmd":"outbreak_smoke"}` | exact R49 Outbreak apply/trigger witness |
//! | `{"cmd":"gremlin_merc_attack_steal_smoke"}` | exact R43 Merc/Gimme move-state witness |
//! | `{"cmd":"fat_gremlin_escape_smoke"}` | exact R43 Fat/Flee move-state witness |
//! | `{"cmd":"quit"}` | end the session |
//!
//! `coverage` answers the question the manifest cannot: not "what does this
//! build claim to implement" but "what did the trajectories just played
//! actually run". It is recorded at the dispatch sites themselves
//! ([`sts_sim::coverage`]), needs no loaded fight, and with `"reset":true`
//! reports and clears — the per-trajectory form the differential drives, so a
//! smoke config claiming a kind no trajectory reached goes red instead of
//! passing untested (PORT_PLAN §4).
//!
//! `intents` is presentation metadata for review snapshots (#2806). It reads
//! the loaded state without changing it and answers, in roster order, one
//! object per monster: `uid`, `intent` (the move id it will act with, or
//! null), and `intent_damage`/`intent_hits` only where
//! [`sts_sim::engine::turn::monster_intents`] proves the displayed number
//! exact. It is a separate command rather than a canonical field because the
//! canonical projection is the pinned parity surface: a new field there would
//! reach boundary admission, the Python projector and every digest.
//!
//! `manifest` needs no loaded fight: it reports what this build of the engine
//! implements — which step and move kinds have bodies (out of how many, across
//! how many family modules), which powers and relics are modeled, and which
//! hook events are fired. Every field is derived from the generated dispatch
//! trees, so a driver can use it to decide which entries are worth synthesizing
//! instead of discovering the answer one refusal at a time.
//!
//! Every response is a single line carrying exactly one of `ok` or `refusal`:
//!
//! ```json
//! {"ok":{"quit":true}}
//! {"refusal":{"site":"load","kind":"not_admitted","detail":"..."}}
//! ```
//!
//! A malformed line is a typed refusal, never a panic and never a silent
//! exit; the session continues so a fuzzing driver can keep going. EOF ends
//! the session as `quit` does.
//!
//! # Actions on the wire
//!
//! ```json
//! {"kind":"play","uid":8,"target":1}
//! {"kind":"play","uid":2}
//! {"kind":"play","uid":12,"selection":37}
//! {"kind":"end"}
//! {"kind":"potion","slot":0}
//! {"kind":"select","answer":{"kind":"card_uid","uid":37}}
//! {"kind":"select","answer":{"kind":"option_index","index":4}}
//! ```
//!
//! `target` is the monster's **roster index** (`combat_sim`'s `choice`), which
//! is neither its `slot` nor its `uid`; `selection` is a second physical card
//! uid consumed by the played card's body. A `select` answer names either the
//! historical replay card's physical `uid` or the generic pending selector's
//! deterministic option `index`; the two domains are kept apart by the nested
//! `answer` object, so a payload-equal selected card can never be substituted
//! for its physical identity.
//! This is the same encoding
//! `tools/gen_slice_pins.py` emits, so a pinned line is replayable through
//! this protocol without translation.
//!
//! **There is exactly one action vocabulary in this crate** (#2477). It is the
//! stable v1 wire [`crate::exact_solve_v1::ExactSolveActionV1`] serializes,
//! and it is what the Python review side speaks: the recorded-line resolver
//! (`solver/rust_replay.py` `recorded_witness`) submits it and the replay
//! renderer (`solver/rust_review.py` `replay_line`) reads it back. The frozen
//! Python `_canonical_wire_action` that first fixed this spelling was deleted
//! with the Python review producer (#2827 item F1). `diff-serve` previously
//! spelled a `Select` flat
//! (`{"kind":"select","index":4}`), which was internally consistent — its own
//! `legal` output applied fine — and therefore invisible until a *recorded*
//! human line written in the canonical vocabulary was replayed through it and
//! refused as `malformed_action`. [`the_action_wire_is_the_stable_v1_wire`]
//! pins the two encoders against each other mechanically so they cannot drift
//! apart again.
//!
//! # Loading
//!
//! `load` runs three gates in order, each with its own refusal kind, so a
//! driver can tell "malformed", "unrepresentable", and "outside the slice"
//! apart: the canonical schema check, the canonical ⇄ hot boundary
//! (`unrepresentable_state`), and the engine's admission walk
//! (`not_admitted`, whose detail names every missing capability at once).
//! A successful load replaces whatever was loaded before.

use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::{CanonicalStateV2, Refusal, RefusalKind};
use sts_sim::catalog::{CardIdentity, Catalog, CatalogBuilder};
use sts_sim::engine::{self, Action, EngineRefusal, SelectionRef};
use sts_sim::hot::{HotCard, HotMonster, HotState, PileId, RngStream, RngStreamState};
use sts_sim::ids::{CardId, MonsterKind, PowerId};
use sts_sim::powers::SlotWire;

/// Protocol identifier written as the greeting line. Bumped, never
/// reinterpreted — a driver that does not recognise it must not proceed.
pub const PROTOCOL_V1: &str = "diff-serve-v1";

/// One loaded fight: the per-fight catalog and the live hot state.
struct Session {
    catalog: Catalog,
    state: HotState,
    events: Vec<Event>,
}

type Event = engine::Event;

impl Session {
    fn document(&self) -> Result<CanonicalStateV2, sts_sim::boundary::BoundaryRefusal> {
        HotBoundary::try_to_canonical(&self.state, &self.catalog)
    }
}

/// Serve the protocol until `quit`, EOF, or a write failure.
pub fn serve<R: BufRead, W: Write>(input: R, output: &mut W) -> std::io::Result<()> {
    // Coverage recording is off in every other process (a search must not pay
    // for it); this is the one that reports it.
    sts_sim::coverage::enable();
    sts_sim::coverage::reset();
    writeln!(output, "{}", json!({ "protocol": PROTOCOL_V1 }))?;
    output.flush()?;
    let mut session: Option<Session> = None;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (response, quit) = handle(&line, &mut session);
        writeln!(output, "{response}")?;
        output.flush()?;
        if quit {
            break;
        }
    }
    Ok(())
}

/// Handle one request line. Returns the response line and whether the
/// session should end.
fn handle(line: &str, session: &mut Option<Session>) -> (String, bool) {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return (
                protocol_refusal(
                    RefusalKind::MalformedRequest,
                    format!("request line is not JSON: {error}"),
                ),
                false,
            );
        }
    };
    let Some(object) = request.as_object() else {
        return (
            protocol_refusal(
                RefusalKind::MalformedRequest,
                "request is not a JSON object",
            ),
            false,
        );
    };
    let command = match object.get("cmd") {
        Some(Value::String(command)) => command.as_str(),
        Some(other) => {
            return (
                protocol_refusal(
                    RefusalKind::MalformedRequest,
                    format!("cmd is not a string: {other}"),
                ),
                false,
            );
        }
        None => {
            return (
                protocol_refusal(RefusalKind::MissingCommand, "request carries no cmd"),
                false,
            );
        }
    };

    match command {
        "quit" => (ok_line(json!({ "quit": true })), true),
        "load" => (load(object, session), false),
        "legal" => (legal(session), false),
        "apply" => (apply(object, session), false),
        "resolve_selection" => (resolve_selection(object, session), false),
        "project" => (project(session), false),
        "intents" => (intents(session), false),
        "manifest" => (manifest(), false),
        "coverage" => (coverage(object), false),
        "queen_burn_bright_smoke" => (queen_burn_bright_smoke(), false),
        "queen_puppet_strings_smoke" => (queen_puppet_strings_smoke(), false),
        "spectral_hex_smoke" => (spectral_hex_smoke(), false),
        "magi_dampen_smoke" => (magi_dampen_smoke(), false),
        "whistle_smoke" => (whistle_smoke(), false),
        "gremlin_merc_attack_steal_smoke" => (gremlin_merc_attack_steal_smoke(), false),
        "fat_gremlin_escape_smoke" => (fat_gremlin_escape_smoke(), false),
        "thieving_hopper_smoke" => (thieving_hopper_smoke(), false),
        "aeonglass_intensity_smoke" => (aeonglass_intensity_smoke(), false),
        "outbreak_smoke" => (outbreak_smoke(), false),
        other => (
            protocol_refusal(
                RefusalKind::UnknownCommand,
                format!(
                    "unknown cmd {other:?}; expected one of \
                     load, legal, apply, resolve_selection, project, intents, manifest, coverage, \
                     queen_burn_bright_smoke, queen_puppet_strings_smoke, \
                     spectral_hex_smoke, magi_dampen_smoke, whistle_smoke, gremlin_merc_attack_steal_smoke, \
                     fat_gremlin_escape_smoke, thieving_hopper_smoke, aeonglass_intensity_smoke, outbreak_smoke, quit"
                ),
            ),
            false,
        ),
    }
}

fn gremlin_merc_smoke_catalog() -> Result<Catalog, String> {
    let mut builder = CatalogBuilder::new();
    builder
        .intern_monster(MonsterKind::GremlinMerc)
        .map_err(|error| format!("{error:?}"))?;
    Ok(builder.build())
}

/// Exercise the exact initial Gimme row and record production move coverage.
fn gremlin_merc_attack_steal_smoke() -> String {
    let catalog = match gremlin_merc_smoke_catalog() {
        Ok(catalog) => catalog,
        Err(error) => return protocol_refusal(RefusalKind::EngineNotImplemented, error),
    };
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.gold = 20;
    let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 53);
    merc.max_hp = 53;
    state.monsters_mut().push(merc);
    let mut events = Vec::new();
    let successor =
        match engine::turn::gremlin_merc_attack_steal_smoke(&state, &catalog, &mut events) {
            Ok(successor) => successor,
            Err(refusal) => {
                return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
            }
        };
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "attack_steal",
        "events": events.len(),
        "player_hp": successor.hp,
        "gold": successor.gold,
        "stolen_gold": successor.monsters[0].powers.value(PowerId::StolenGold),
    }))
}

/// Exercise the exact post-Spawned Fat Flee row and record move coverage.
fn fat_gremlin_escape_smoke() -> String {
    let catalog = match gremlin_merc_smoke_catalog() {
        Ok(catalog) => catalog,
        Err(error) => return protocol_refusal(RefusalKind::EngineNotImplemented, error),
    };
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.rng.set(
        RngStream::Niche,
        RngStreamState {
            words: [5, 6, 7, 8],
            counter: 1,
        },
    );
    let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 53);
    merc.max_hp = 53;
    merc.powers.set(PowerId::StolenGold, SlotWire::Int, 20);
    state.monsters_mut().push(merc);
    if let Err(refusal) = engine::damage::damage_monster(
        &mut state,
        0,
        sts_sim::decimal::DotNetDecimal::from_i64(53),
        false,
        false,
        &mut Vec::new(),
    ) {
        return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
    }
    state.monsters_mut()[1].spawn_noop = false;
    state.monsters_mut()[2].spawn_noop = false;
    let mut events = Vec::new();
    let successor = match engine::turn::fat_gremlin_escape_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "escape",
        "events": events.len(),
        "over": successor.history.over,
        "monster_count": successor.monsters.len(),
    }))
}

/// Exercise all three R44 Hopper moves plus both power readers through the
/// ordinary public engine, using the exact empty-deck smoke foundation.
fn thieving_hopper_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    if builder.intern_monster(MonsterKind::ThievingHopper).is_err() {
        return protocol_refusal(
            RefusalKind::EngineNotImplemented,
            "Thieving Hopper smoke catalog",
        );
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.exact_piles = true;
    let mut hopper = HotMonster::new(MonsterKind::ThievingHopper, engine::THIEVING_HOPPER_HP);
    hopper.max_hp = engine::THIEVING_HOPPER_HP;
    hopper.powers.set(
        PowerId::EscapeArtist,
        SlotWire::Int,
        engine::THIEVING_HOPPER_ESCAPE_ARTIST,
    );
    state.monsters_mut().push(hopper);
    let mut events = Vec::new();
    let successor = match engine::turn::thieving_hopper_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "thieving_hopper",
        "events": events.len(),
        "over": successor.history.over,
        "monster_count": successor.monsters.len(),
    }))
}

/// Exercise Aeonglass construction and its complete deterministic cycle.
fn aeonglass_intensity_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    if builder.intern_aeonglass_smoke_foundation().is_err() {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Aeonglass smoke catalog");
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 300;
    state.max_hp = 300;
    state.player_phase = engine::admission::PHASE_ORDINARY_ACTIONS;
    state.exact_piles = true;
    state.rng.set(
        RngStream::Niche,
        RngStreamState {
            words: [11, 22, 33, 44],
            counter: 0,
        },
    );
    let niche_before = state.rng.get(RngStream::Niche).counter;
    let mut events = Vec::new();
    if let Err(refusal) = engine::construct_aeonglass_boss(&mut state, &catalog, &mut events) {
        return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
    }
    let successor = match engine::turn::aeonglass_intensity_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "aeonglass_intensity",
        "events": events.len(),
        "niche_draws": state.rng.get(RngStream::Niche).counter - niche_before,
        "wither_count": PileId::ALL.into_iter()
            .map(|pile| successor.piles.get(pile).len())
            .sum::<usize>(),
    }))
}

/// Exercise R49's player-applied Outbreak wave and null-applier trigger.
fn outbreak_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    let outbreak = match builder.intern(CardIdentity {
        id: CardId::Outbreak,
        upgrade: 0,
        enchantment: None,
    }) {
        Ok(atom) => atom,
        Err(_) => {
            return protocol_refusal(RefusalKind::EngineNotImplemented, "Outbreak smoke card");
        }
    };
    if builder.intern_monster(MonsterKind::Toadpole).is_err() {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Outbreak smoke catalog");
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 50;
    state.max_hp = 50;
    state.energy = 3;
    state.player_phase = engine::admission::PHASE_ORDINARY_ACTIONS;
    state.exact_piles = true;
    state.next_card_uid = 2;
    state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
        uid: 1,
        atom: outbreak,
        flags: 0,
    });
    let mut target = HotMonster::new(MonsterKind::Toadpole, 100);
    target.max_hp = 100;
    target.uid = 10;
    state.monsters_mut().push(target);

    let mut events = Vec::new();
    if let Err(refusal) = engine::play::play_card(&mut state, &catalog, 1, None, None, &mut events)
    {
        return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
    }
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "outbreak",
        "events": events.len(),
        "target_hp": state.monsters[0].hp,
        "poison": state.monsters[0].powers.value(PowerId::Poison),
        "poison_uid": state.monsters[0].poison_uid,
    }))
}

/// Exercise the coordinator-approved narrow R42 Queen/Puppet Strings witness.
///
/// The command constructs only the exact fresh row-0 stable roster, calls the
/// public state-authenticated witness, and reaches the production
/// `monster_act` dispatcher and coverage recorder.
fn queen_puppet_strings_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    if builder
        .intern_monster(MonsterKind::TorchHeadAmalgam)
        .is_err()
        || builder.intern_monster(MonsterKind::Queen).is_err()
    {
        return protocol_refusal(
            RefusalKind::EngineNotImplemented,
            "Queen Puppet Strings smoke catalog",
        );
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.exact_piles = true;
    let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, engine::TORCH_HEAD_AMALGAM_HP);
    amalgam.max_hp = engine::TORCH_HEAD_AMALGAM_HP;
    amalgam.loop_pos = 2;
    amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    let mut queen = HotMonster::new(MonsterKind::Queen, engine::QUEEN_HP);
    queen.max_hp = engine::QUEEN_HP;
    queen.slot = 1;
    queen.uid = 1;
    state.monsters_mut().extend([amalgam, queen]);

    let before = power_slots(&state);
    let mut events = Vec::new();
    let successor = match engine::turn::queen_puppet_strings_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    record_changed_powers(&before, &power_slots(&successor));
    ok_line(json!({
        "move": "queen_puppet_strings",
        "events": events.len(),
        "over": successor.history.over,
        "chains_of_binding": successor.powers.value(PowerId::ChainsOfBinding),
        "queen_loop_pos": successor.monsters[1].loop_pos,
    }))
}

/// Exercise R46's exact Whistle card body on the fresh Queen row.
fn whistle_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    let whistle = match builder.intern(CardIdentity {
        id: CardId::Whistle,
        upgrade: 0,
        enchantment: None,
    }) {
        Ok(atom) => atom,
        Err(_) => {
            return protocol_refusal(RefusalKind::EngineNotImplemented, "Whistle smoke card");
        }
    };
    if builder
        .intern_monster(MonsterKind::TorchHeadAmalgam)
        .is_err()
        || builder.intern_monster(MonsterKind::Queen).is_err()
    {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Whistle smoke catalog");
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.energy = 2;
    state.exact_piles = true;
    state.next_card_uid = 1;
    state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
        uid: 0,
        atom: whistle,
        flags: 0,
    });
    let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, engine::TORCH_HEAD_AMALGAM_HP);
    amalgam.max_hp = engine::TORCH_HEAD_AMALGAM_HP;
    amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    let mut queen = HotMonster::new(MonsterKind::Queen, engine::QUEEN_HP);
    queen.max_hp = engine::QUEEN_HP;
    queen.slot = 1;
    queen.uid = 1;
    state.monsters_mut().extend([amalgam, queen]);

    let mut events = Vec::new();
    if let Err(refusal) =
        engine::play::play_card(&mut state, &catalog, 0, Some(1), None, &mut events)
    {
        return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
    }
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "whistle",
        "events": events.len(),
        "queen_hp": state.monsters[1].hp,
        "queen_override": state.monsters[1].override_state.as_str(),
        "queen_follow_up": state.monsters[1].forced_follow_up.as_str(),
        "exhaust": state.piles.get(PileId::Exhaust).len(),
    }))
}

/// Exercise the coordinator-approved narrow R42 move-state witness.
///
/// Ordinary admission keeps refusing the Knights encounter until the other
/// two move bodies are modeled. This fixture authenticates only the exact A8+
/// fresh Spectral HEX row and reaches the ordinary `monster_act` dispatcher
/// and coverage recorder without widening that encounter surface.
fn spectral_hex_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    if builder.intern_spectral_hex_smoke_foundation().is_err()
        || builder.intern_monster(MonsterKind::FlailKnight).is_err()
        || builder.intern_monster(MonsterKind::MagiKnight).is_err()
    {
        return protocol_refusal(
            RefusalKind::EngineNotImplemented,
            "Spectral Hex smoke catalog",
        );
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.exact_piles = true;
    let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
    flail.max_hp = 108;
    if !flail.random_ai.set_next(Some(2)) || !flail.random_ai.set_log(&[2]) {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Spectral Hex smoke AI");
    }
    let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
    spectral.max_hp = 97;
    spectral.slot = 1;
    spectral.uid = 1;
    if !spectral.random_ai.set_next(Some(0)) || !spectral.random_ai.set_log(&[0]) {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Spectral Hex smoke AI");
    }
    let mut magi = HotMonster::new(MonsterKind::MagiKnight, 89);
    magi.max_hp = 89;
    magi.slot = 2;
    magi.uid = 2;
    state.monsters_mut().extend([flail, spectral, magi]);

    let before = power_slots(&state);
    let mut events = Vec::new();
    let successor = match engine::turn::spectral_hex_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    record_changed_powers(&before, &power_slots(&successor));
    ok_line(json!({
        "move": "hex_player",
        "events": events.len(),
        "over": successor.history.over,
        "hex_power": successor.powers.value(PowerId::HexPower),
    }))
}

fn magi_dampen_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    let l0 = CardIdentity {
        id: CardId::StrikeIronclad,
        upgrade: 0,
        enchantment: None,
    };
    let l1 = CardIdentity { upgrade: 1, ..l0 };
    if builder.intern_magi_dampen_smoke_foundation().is_err()
        || builder.intern_monster(MonsterKind::FlailKnight).is_err()
        || builder.intern_monster(MonsterKind::SpectralKnight).is_err()
        || builder.intern_dampen_card_pair(l0).is_err()
    {
        return protocol_refusal(
            RefusalKind::EngineNotImplemented,
            "Magi Dampen smoke catalog",
        );
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    state.exact_piles = true;
    state.next_card_uid = 1;
    state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
        uid: 0,
        atom: catalog.atom(&l1).expect("smoke L1 atom"),
        flags: sts_sim::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE | sts_sim::hot::CARD_FLAG_HEXED,
    });
    state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
    let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
    flail.max_hp = 108;
    if !flail.random_ai.set_next(Some(0)) || !flail.random_ai.set_log(&[2, 0]) {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Magi Dampen smoke AI");
    }
    let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
    spectral.max_hp = 97;
    spectral.slot = 1;
    spectral.uid = 1;
    if !spectral.random_ai.set_next(Some(1)) || !spectral.random_ai.set_log(&[0, 1]) {
        return protocol_refusal(RefusalKind::EngineNotImplemented, "Magi Dampen smoke AI");
    }
    let mut magi = HotMonster::new(MonsterKind::MagiKnight, 89);
    magi.max_hp = 89;
    magi.slot = 2;
    magi.uid = 2;
    magi.loop_pos = 1;
    state.monsters_mut().extend([flail, spectral, magi]);

    let mut events = Vec::new();
    let successor = match engine::turn::magi_dampen_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    ok_line(json!({
        "move": "dampen_player",
        "events": events.len(),
        "loop_pos": successor.monsters[2].loop_pos,
        "tracked": successor.card_states.dampen_tracked_len(),
        "upgrade": catalog.spec(successor.piles.get(PileId::Hand).as_slice()[0].atom)
            .expect("smoke atom").identity.upgrade,
    }))
}

/// Exercise the coordinator-approved narrow R41 move-state witness.
///
/// This does not load a Queen encounter and cannot bypass ordinary admission:
/// it constructs the one exact stable pre-Burn state, calls the public
/// state-authenticated witness, and therefore reaches the same `monster_act`
/// dispatch and coverage recorder as an admitted enemy phase.
fn queen_burn_bright_smoke() -> String {
    let mut builder = CatalogBuilder::new();
    if builder
        .intern_monster(MonsterKind::TorchHeadAmalgam)
        .is_err()
        || builder.intern_monster(MonsterKind::Queen).is_err()
    {
        return protocol_refusal(
            RefusalKind::EngineNotImplemented,
            "Queen Burn Bright smoke catalog",
        );
    }
    let catalog = builder.build();
    let mut state = HotState::at_defaults();
    state.hp = 100;
    state.max_hp = 100;
    let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, engine::TORCH_HEAD_AMALGAM_HP);
    amalgam.max_hp = engine::TORCH_HEAD_AMALGAM_HP;
    amalgam.loop_pos = 2;
    amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    let mut queen = HotMonster::new(MonsterKind::Queen, engine::QUEEN_HP);
    queen.max_hp = engine::QUEEN_HP;
    queen.loop_pos = 2;
    queen.slot = 1;
    queen.uid = 1;
    state.monsters_mut().extend([amalgam, queen]);

    let before = power_slots(&state);
    let mut events = Vec::new();
    let successor = match engine::turn::queen_burn_bright_smoke(&state, &catalog, &mut events) {
        Ok(successor) => successor,
        Err(refusal) => {
            return protocol_refusal(engine_refusal_kind(&refusal), refusal.to_string());
        }
    };
    sts_sim::coverage::record_events(&events);
    record_changed_powers(&before, &power_slots(&successor));
    ok_line(json!({
        "move": "queen_burn_bright",
        "events": events.len(),
        "over": successor.history.over,
        "amalgam_strength": successor.monsters[0].powers.value(PowerId::Strength),
        "queen_block": successor.monsters[1].block,
        "queen_loop_pos": successor.monsters[1].loop_pos,
    }))
}

fn load(request: &Map<String, Value>, session: &mut Option<Session>) -> String {
    let Some(entry) = request.get("entry") else {
        return refusal_line(
            Refusal::new(
                "load",
                RefusalKind::MissingField,
                "load requires an entry field carrying a canonical document",
            ),
            Map::new(),
        );
    };
    let document: CanonicalStateV2 = match serde_json::from_value(entry.clone()) {
        Ok(document) => document,
        Err(error) => {
            return refusal_line(
                Refusal::new(
                    "load",
                    RefusalKind::MalformedEntry,
                    format!("entry is not a canonical v2 document: {error}"),
                ),
                Map::new(),
            );
        }
    };
    if let Err(refusal) = document.validate_schema() {
        return refusal_line(refusal, Map::new());
    }
    let catalog = match HotBoundary::catalog_from_canonical(&document) {
        Ok(catalog) => catalog,
        Err(refusal) => {
            return refusal_line(
                Refusal::new(
                    "load",
                    RefusalKind::UnrepresentableState,
                    refusal.to_string(),
                ),
                Map::new(),
            );
        }
    };
    let state = match HotBoundary::from_canonical(&document, &catalog) {
        Ok(state) => state,
        Err(refusal) => {
            return refusal_line(
                Refusal::new(
                    "load",
                    RefusalKind::UnrepresentableState,
                    refusal.to_string(),
                ),
                Map::new(),
            );
        }
    };
    if let Err(refusal) = engine::admit(&document, &state, &catalog) {
        let mut extra = Map::new();
        extra.insert(
            "missing".to_string(),
            Value::Array(
                refusal
                    .missing()
                    .map(|item| Value::String(item.to_string()))
                    .collect(),
            ),
        );
        return refusal_line(
            Refusal::new("load", RefusalKind::NotAdmitted, refusal.to_string()),
            extra,
        );
    }

    let loaded = Session {
        catalog,
        state,
        events: Vec::new(),
    };
    // Echo the digest so a driver can check its Python-side projection
    // survived the wire unchanged before it plays a single action.
    let digest = match loaded.document() {
        Ok(document) => document.differential_digest(),
        Err(refusal) => {
            return refusal_line(
                Refusal::new(
                    "load",
                    RefusalKind::UnrepresentableState,
                    refusal.to_string(),
                ),
                Map::new(),
            );
        }
    };
    *session = Some(loaded);
    ok_line(json!({ "loaded": true, "digest": digest, "schema": document.schema }))
}

fn legal(session: &mut Option<Session>) -> String {
    let Some(session) = session.as_ref() else {
        return no_state("legal");
    };
    let actions: Vec<Value> = engine::legal_actions(&session.state, &session.catalog)
        .iter()
        .map(encode_action)
        .collect();
    ok_line(json!({ "actions": actions }))
}

/// Read-only: the unique accepted answer naming `uids`, in order
/// (`engine::recorded_selection_answer`: the ordered extension of #2524, then
/// every offered answer decoded in-process, #3125). `null` means no answer
/// names them.
fn resolve_selection(request: &Map<String, Value>, session: &mut Option<Session>) -> String {
    let Some(loaded) = session.as_ref() else {
        return no_state("resolve_selection");
    };
    let uids = request
        .get("uids")
        .and_then(Value::as_array)
        .and_then(|uids| {
            uids.iter()
                .map(|uid| uid.as_u64().and_then(|uid| u32::try_from(uid).ok()))
                .collect::<Option<Vec<u32>>>()
        });
    let Some(uids) = uids else {
        return refusal_line(
            Refusal::new(
                "resolve_selection",
                RefusalKind::MissingField,
                "resolve_selection requires a uids array of u32",
            ),
            Map::new(),
        );
    };
    match engine::recorded_selection_answer(&loaded.state, &loaded.catalog, &uids) {
        Ok(answer) => ok_line(json!({
            "action": answer.map(|answer| encode_action(&engine::Action::Select { answer })),
        })),
        Err(error) => refusal_line(
            Refusal::new(
                "resolve_selection",
                engine_refusal_kind(&error),
                error.to_string(),
            ),
            Map::new(),
        ),
    }
}

fn apply(request: &Map<String, Value>, session: &mut Option<Session>) -> String {
    let Some(loaded) = session.as_mut() else {
        return no_state("apply");
    };
    let Some(payload) = request.get("action") else {
        return refusal_line(
            Refusal::new(
                "apply",
                RefusalKind::MissingField,
                "apply requires an action field",
            ),
            Map::new(),
        );
    };
    let action = match decode_action(payload) {
        Ok(action) => action,
        Err(detail) => {
            return refusal_line(
                Refusal::new("apply", RefusalKind::MalformedAction, detail),
                Map::new(),
            );
        }
    };
    // Opt-in presentation metadata only; search and ordinary differential
    // traffic pay no enumeration cost and retain their existing wire shape.
    let selected_uids = if request.get("describe_selection") == Some(&Value::Bool(true)) {
        if let engine::Action::Select { answer } = action {
            match engine::selected_card_uids(&loaded.state, &loaded.catalog, answer) {
                Ok(uids) => uids,
                Err(error) => {
                    return refusal_line(
                        Refusal::new("apply", engine_refusal_kind(&error), error.to_string()),
                        Map::new(),
                    );
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let powers_before = power_slots(&loaded.state);
    let record_checkpoints = request.get("native_checkpoints") == Some(&Value::Bool(true));
    let record_marks = request.get("presentation_marks") == Some(&Value::Bool(true));
    let ((applied, recorded), marks) = engine::presentation::record_if(record_marks, || {
        if record_checkpoints {
            engine::native_checkpoint::record(|| {
                engine::apply_action_into(
                    &loaded.state,
                    &loaded.catalog,
                    &action,
                    &mut loaded.events,
                )
            })
        } else {
            (
                engine::apply_action_into(
                    &loaded.state,
                    &loaded.catalog,
                    &action,
                    &mut loaded.events,
                ),
                Vec::new(),
            )
        }
    });
    match applied {
        Ok(next) => {
            let document = match HotBoundary::try_to_canonical(&next, &loaded.catalog) {
                Ok(document) => document,
                Err(refusal) => {
                    loaded.events.clear();
                    return refusal_line(
                        Refusal::new(
                            "apply",
                            RefusalKind::EngineNotImplemented,
                            refusal.to_string(),
                        ),
                        Map::new(),
                    );
                }
            };
            loaded.state = next;
            // Powers have no single dispatch site, so their coverage is read
            // here, outside the hot loop: from the transition's own events and
            // from the power slots it changed.
            sts_sim::coverage::record_events(&loaded.events);
            record_changed_powers(&powers_before, &power_slots(&loaded.state));
            let mut result = json!({
                "digest": document.differential_digest(),
                "events": loaded.events.len(),
                "over": loaded.state.history.over,
            });
            if request.get("describe_selection") == Some(&Value::Bool(true)) {
                result["selected_uids"] = json!(selected_uids);
            }
            if record_checkpoints {
                result["native_checkpoints"] = native_checkpoints_json(&recorded, &loaded.catalog);
            }
            if record_marks {
                result["presentation_marks"] =
                    Value::Array(marks.iter().map(|mark| mark.to_json()).collect());
            }
            ok_line(result)
        }
        Err(refusal) => refusal_line(
            Refusal::new("apply", engine_refusal_kind(&refusal), refusal.to_string()),
            Map::new(),
        ),
    }
}

/// The recorded native checkpoint states of one apply, projected (#3242).
///
/// A state that does not project is reported with a null `state` and the
/// boundary's refusal, so a driver fails that checkpoint by name rather than
/// silently skipping it.
fn native_checkpoints_json(
    recorded: &[(engine::native_checkpoint::NativeCheckpointKind, HotState)],
    catalog: &Catalog,
) -> Value {
    Value::Array(
        recorded
            .iter()
            .map(
                |(kind, state)| match HotBoundary::try_to_canonical(state, catalog) {
                    Ok(document) => json!({
                        "kind": kind.as_str(),
                        "state": serde_json::to_value(&document)
                            .expect("canonical state serializes"),
                    }),
                    Err(refusal) => json!({
                        "kind": kind.as_str(),
                        "state": null,
                        "refusal": refusal.to_string(),
                    }),
                },
            )
            .collect(),
    )
}

/// Every `(creature, power, amount)` triple live in a state, ascending.
///
/// The player is keyed by `None`, a monster by its creation-order uid, so a
/// roster that grows or loses a member still compares cleanly.
fn power_slots(state: &HotState) -> Vec<(Option<u32>, PowerId, i32)> {
    let mut slots: Vec<(Option<u32>, PowerId, i32)> = state
        .powers
        .as_slice()
        .iter()
        .map(|slot| (None, slot.key, slot.value))
        .chain(state.monsters.iter().flat_map(|monster| {
            monster
                .powers
                .as_slice()
                .iter()
                .map(move |slot| (Some(monster.uid), slot.key, slot.value))
        }))
        .collect();
    slots.sort_unstable();
    slots
}

/// Record the powers a transition changed.
///
/// The engine's power *writes* are spread across whichever body owns each
/// power, and not every one of them emits a `PowerChanged` event — adding
/// events so the instrument could read them would be changing the engine to
/// suit the instrument. So the transport diffs the power slots either side of
/// the transition instead, which is the strong reading of "exercised": not
/// "this power was sitting on a creature", but "a body wrote it". `O(active
/// powers)` per transition, on the transport side of the boundary.
fn record_changed_powers(
    before: &[(Option<u32>, PowerId, i32)],
    after: &[(Option<u32>, PowerId, i32)],
) {
    for slot in before {
        if !after.contains(slot) {
            sts_sim::coverage::record_power(slot.1);
        }
    }
    for slot in after {
        if !before.contains(slot) {
            sts_sim::coverage::record_power(slot.1);
        }
    }
}

fn project(session: &mut Option<Session>) -> String {
    let Some(session) = session.as_ref() else {
        return no_state("project");
    };
    let document = match session.document() {
        Ok(document) => document,
        Err(refusal) => {
            return refusal_line(
                Refusal::new(
                    "project",
                    RefusalKind::EngineNotImplemented,
                    refusal.to_string(),
                ),
                Map::new(),
            );
        }
    };
    let digest = document.differential_digest();
    ok_line(json!({
        "state": serde_json::to_value(&document).expect("canonical state serializes"),
        "digest": digest,
    }))
}

/// Each monster's displayed intent in the loaded state (#2806).
///
/// Presentation only: it neither advances nor records coverage, and the
/// damage fields are omitted wherever the engine does not prove them exact.
fn intents(session: &mut Option<Session>) -> String {
    let Some(session) = session.as_ref() else {
        return no_state("intents");
    };
    let intents: Vec<Value> = engine::turn::monster_intents(&session.state, &session.catalog)
        .into_iter()
        .map(|intent| {
            let mut entry = json!({ "uid": intent.uid, "intent": intent.intent });
            if let (Some(damage), Some(hits)) = (intent.damage, intent.hits) {
                entry["intent_damage"] = json!(damage);
                entry["intent_hits"] = json!(hits);
            }
            entry
        })
        .collect();
    ok_line(json!({ "intents": intents }))
}

/// The derived capability manifest (D6), as JSON.
///
/// Stateless: a driver asks before it loads anything, to decide which entries
/// are worth synthesizing at all. Every field comes from
/// [`engine::capability_manifest`], which reads the generated dispatch trees —
/// there is no manifest literal anywhere in this crate to drift.
fn manifest() -> String {
    let manifest = engine::capability_manifest();
    let names = |items: Vec<String>| Value::Array(items.into_iter().map(Value::String).collect());
    ok_line(json!({
        "manifest": {
            "steps": {
                "implemented": names(
                    manifest.steps.iter().map(|kind| kind.as_str().to_string()).collect()),
                "total": manifest.step_total,
                "families": manifest.step_families,
            },
            "moves": {
                "implemented": names(
                    manifest.moves.iter().map(|kind| kind.as_str().to_string()).collect()),
                "total": manifest.move_total,
                "families": manifest.move_families,
            },
            "powers": names(
                manifest.powers.iter().map(|power| power.as_str().to_string()).collect()),
            "relics": names(
                manifest.relics.iter().map(|relic| relic.as_str().to_string()).collect()),
            "hooks": {
                "fired": names(
                    manifest.hooks_fired.iter().map(|event| event.as_str().to_string()).collect()),
                "total": sts_sim::hooks::HookEvent::COUNT,
                "effects_modeled": names(
                    manifest.hook_effects.iter().map(|verb| verb.as_str().to_string()).collect()),
            },
        }
    }))
}

/// Derived execution coverage (PORT_PLAN §4), as JSON.
///
/// Stateless with respect to the loaded fight and cumulative across loads, so
/// a driver can scope it however it likes: `{"cmd":"coverage","reset":true}`
/// reports and clears (per trajectory), plain `coverage` peeks (per run).
///
/// This is the *counterweight* to the manifest. The manifest says what the
/// build claims; this says what the trajectories ran. A wave PR whose smoke
/// config claims a kind that never appears here is red — "green because
/// untested" is not green.
fn coverage(request: &Map<String, Value>) -> String {
    let reset = match request.get("reset") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(other) => {
            return refusal_line(
                Refusal::new(
                    "coverage",
                    RefusalKind::MalformedRequest,
                    format!("reset is not a boolean: {other}"),
                ),
                Map::new(),
            );
        }
    };
    let coverage = if reset {
        sts_sim::coverage::take()
    } else {
        sts_sim::coverage::report()
    };
    ok_line(json!({
        "coverage": {
            "steps": coverage.steps,
            "moves": coverage.moves,
            "powers": coverage.powers,
            "relics": coverage.relics,
            "hooks": coverage.hooks,
            "transitions": coverage.transitions,
        }
    }))
}

/// Which wire kind an engine refusal reports as.
///
/// The split is the one a differential driver needs: "this action is not
/// applicable" (a disagreement about legality, which the legal-action
/// comparison would already have caught) versus "this mechanic is not
/// ported" (refusal parity — Python must be raising `NotImplementedError`
/// at the same site).
fn engine_refusal_kind(refusal: &EngineRefusal) -> RefusalKind {
    match refusal {
        EngineRefusal::NotAdmitted(_) => RefusalKind::NotAdmitted,
        EngineRefusal::CardNotInHand(_)
        | EngineRefusal::UnknownAtom(_)
        | EngineRefusal::UnknownMintIdentity(_)
        | EngineRefusal::NotEnoughEnergy { .. }
        | EngineRefusal::NotEnoughStars { .. }
        // A legality disagreement, never an unported mechanic: Python does not
        // raise on these, it simply never offers the card. Classifying it as
        // `EngineNotImplemented` would manufacture a refusal-parity mismatch
        // against a Python site that does not exist (#1613).
        | EngineRefusal::CardNotPlayableSolo(_)
        | EngineRefusal::TargetMismatch { .. }
        | EngineRefusal::SelectionMismatch { .. }
        | EngineRefusal::BadTarget(_)
        | EngineRefusal::BadPotionSlot(_)
        | EngineRefusal::PotionTargetMismatch { .. }
        // #2473: "not in my legal list" is an illegal action, and was reported
        // as `EngineNotImplemented` under `ContinuationNotModeled` until the
        // variant existed.
        | EngineRefusal::ActionNotLegal(_)
        | EngineRefusal::CombatOver => RefusalKind::IllegalAction,
        EngineRefusal::StepKindNotModeled(_)
        | EngineRefusal::MoveKindNotModeled(_)
        | EngineRefusal::OrbKindNotModeled(_)
        | EngineRefusal::MalformedArgs(_)
        | EngineRefusal::PotionsNotModeled
        | EngineRefusal::CounterOverflow(_)
        | EngineRefusal::PowerOrderNotModeled(_)
        | EngineRefusal::PowerRestackNotModeled(_)
        | EngineRefusal::MonsterLoopNotModeled(_)
        | EngineRefusal::HookNotModeled { .. }
        | EngineRefusal::PowerHookNotModeled { .. }
        | EngineRefusal::EndingSummonNotModeled(_)
        | EngineRefusal::EndingDamageNotModeled(_)
        | EngineRefusal::UntrackedCounterNotModeled(_)
        | EngineRefusal::FrozenCardVanished { .. }
        | EngineRefusal::ActiveCardNotUnique { .. }
        | EngineRefusal::PendingSelectionRouting(_)
        | EngineRefusal::NoLegalActions
        | EngineRefusal::ContinuationNotModeled => RefusalKind::EngineNotImplemented,
    }
}

fn encode_action(action: &Action) -> Value {
    match action {
        Action::EndTurn => json!({ "kind": "end" }),
        Action::Play {
            uid,
            target,
            selection,
        } => {
            let mut payload = Map::new();
            payload.insert("kind".to_owned(), Value::from("play"));
            payload.insert("uid".to_owned(), Value::from(*uid));
            if let Some(index) = target {
                payload.insert("target".to_owned(), Value::from(*index));
            }
            if let Some(selected) = selection.get() {
                payload.insert("selection".to_owned(), Value::from(selected));
            }
            Value::Object(payload)
        }
        Action::UsePotion { slot, target } => {
            let mut payload = Map::new();
            payload.insert("kind".to_owned(), Value::from("potion"));
            payload.insert("slot".to_owned(), Value::from(*slot));
            if let Some(index) = target {
                payload.insert("target".to_owned(), Value::from(*index));
            }
            Value::Object(payload)
        }
        Action::Select {
            answer: engine::SelectionAnswer::CardUid(uid),
        } => json!({ "kind": "select", "answer": { "kind": "card_uid", "uid": uid } }),
        Action::Select {
            answer: engine::SelectionAnswer::OptionIndex(index),
        } => json!({ "kind": "select", "answer": { "kind": "option_index", "index": index } }),
    }
}

fn decode_action(value: &Value) -> Result<Action, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("action is not a JSON object: {value}"))?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("action carries no kind: {value}"))?;
    match kind {
        "end" => Ok(Action::EndTurn),
        "play" => {
            let uid = object
                .get("uid")
                .and_then(Value::as_u64)
                .and_then(|uid| u32::try_from(uid).ok())
                .ok_or_else(|| format!("play action carries no uid: {value}"))?;
            let target = match object.get("target") {
                None | Some(Value::Null) => None,
                Some(index) => Some(
                    index
                        .as_u64()
                        .and_then(|index| u8::try_from(index).ok())
                        .ok_or_else(|| format!("play target is not a roster index: {index}"))?,
                ),
            };
            let selection = SelectionRef::new(match object.get("selection") {
                None | Some(Value::Null) => None,
                Some(selected) => Some(
                    selected
                        .as_u64()
                        .and_then(|selected| u32::try_from(selected).ok())
                        .ok_or_else(|| format!("play selection is not a card uid: {selected}"))?,
                ),
            });
            Ok(Action::Play {
                uid,
                target,
                selection,
            })
        }
        "potion" => {
            if object.len() > 3
                || object.len() < 2
                || object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "kind" | "slot" | "target"))
            {
                return Err(format!("potion action has a noncanonical shape: {value}"));
            }
            let slot = object
                .get("slot")
                .and_then(Value::as_u64)
                .and_then(|slot| u8::try_from(slot).ok())
                .ok_or_else(|| format!("potion action carries no slot: {value}"))?;
            let target = match object.get("target") {
                None | Some(Value::Null) => None,
                Some(index) => Some(
                    index
                        .as_u64()
                        .and_then(|index| u8::try_from(index).ok())
                        .ok_or_else(|| format!("potion target is not a roster index: {index}"))?,
                ),
            };
            Ok(Action::UsePotion { slot, target })
        }
        "select" => {
            if object.len() != 2 {
                return Err(format!("select action has a noncanonical shape: {value}"));
            }
            let answer = object
                .get("answer")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("select action must carry one answer: {value}"))?;
            let domain = answer
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("select answer carries no kind: {value}"))?;
            if answer.len() != 2 {
                return Err(format!("select answer has a noncanonical shape: {value}"));
            }
            let answer = match domain {
                "card_uid" => engine::SelectionAnswer::CardUid(
                    answer
                        .get("uid")
                        .and_then(Value::as_u64)
                        .and_then(|uid| u32::try_from(uid).ok())
                        .ok_or_else(|| format!("select uid is invalid: {value}"))?,
                ),
                "option_index" => engine::SelectionAnswer::OptionIndex(
                    answer
                        .get("index")
                        .and_then(Value::as_u64)
                        .and_then(|index| u32::try_from(index).ok())
                        .ok_or_else(|| format!("select index is invalid: {value}"))?,
                ),
                other => return Err(format!("unknown select answer kind {other:?}")),
            };
            Ok(Action::Select { answer })
        }
        other => Err(format!("unknown action kind {other:?}")),
    }
}

fn no_state(site: &str) -> String {
    refusal_line(
        Refusal::new(
            site,
            RefusalKind::NoStateLoaded,
            "no state is loaded; load an entry first",
        ),
        Map::new(),
    )
}

fn protocol_refusal(kind: RefusalKind, detail: impl Into<String>) -> String {
    refusal_line(Refusal::new("protocol", kind, detail), Map::new())
}

fn ok_line(payload: Value) -> String {
    json!({ "ok": payload }).to_string()
}

fn refusal_line(refusal: Refusal, extra: Map<String, Value>) -> String {
    let mut object = Map::new();
    object.insert(
        "refusal".to_string(),
        serde_json::to_value(refusal).expect("refusal serializes"),
    );
    object.extend(extra);
    Value::Object(object).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");
    const PINS: &str = include_str!("../fixtures/slice_line_v1.json");

    fn drive(requests: &str) -> Vec<Value> {
        let mut output = Vec::new();
        serve(requests.as_bytes(), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).expect("every response line is JSON"))
            .collect()
    }

    fn refusal_kind(response: &Value) -> &str {
        response["refusal"]["kind"].as_str().expect("typed refusal")
    }

    fn load_line() -> String {
        let entry: Value = serde_json::from_str(FIXTURE).unwrap();
        json!({ "cmd": "load", "entry": entry }).to_string()
    }

    /// The manifest is stateless, derived, and honest about the distance to
    /// full coverage — a driver reads it to decide what to synthesize at all.
    #[test]
    fn the_manifest_reports_the_derived_capabilities_without_a_loaded_fight() {
        let responses = drive(&format!("{}\n", json!({ "cmd": "manifest" })));
        let manifest = &responses[1]["ok"]["manifest"];
        // Derived, not enumerated: an engine slice or a wave PR that lands a
        // body must not turn this test red in a file it may not edit (#1353's
        // second wall). What is pinned is the *shape* — sorted, unique, drawn
        // from the family registries — plus the R0.5 kinds as a floor.
        let steps = manifest["steps"]["implemented"]
            .as_array()
            .expect("the implemented step list");
        for kind in ["attack", "block", "vulnerable"] {
            assert!(steps.iter().any(|name| name == kind), "{kind}");
        }
        assert!(steps.windows(2).all(|pair| {
            pair[0].as_str().expect("a kind name") < pair[1].as_str().expect("a kind name")
        }));
        // 334 since #3322 added the Rust-owned `mad_science_chaos_exact`.
        assert_eq!(manifest["steps"]["total"], json!(334));
        assert_eq!(manifest["steps"]["families"], json!(30));
        let moves = manifest["moves"]["implemented"]
            .as_array()
            .expect("the implemented move list");
        for kind in ["attack", "buff_thorns", "spit_attack"] {
            assert!(moves.iter().any(|name| name == kind), "{kind}");
        }
        assert!(moves.windows(2).all(|pair| {
            pair[0].as_str().expect("a kind name") < pair[1].as_str().expect("a kind name")
        }));
        assert_eq!(manifest["moves"]["total"], json!(93));
        assert_eq!(manifest["moves"]["families"], json!(5));
        // Derived from `IMPLEMENTED_POWERS`, so an engine slice that reads a
        // new power back does not turn this red — the R0.5 three are the floor.
        let powers = manifest["powers"].as_array().expect("the power list");
        for power in ["strength", "thorns", "vuln"] {
            assert!(powers.iter().any(|name| name == power), "{power}");
        }
        assert_eq!(
            manifest["relics"],
            json!(
                sts_sim::engine::admission::IMPLEMENTED_RELICS
                    .iter()
                    .map(|relic| relic.as_str())
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(manifest["hooks"]["total"], json!(19));
        assert_eq!(
            manifest["hooks"]["effects_modeled"],
            json!(
                sts_sim::hooks::TemplateEffect::IMPLEMENTED
                    .iter()
                    .map(|effect| effect.as_str())
                    .collect::<Vec<_>>()
            )
        );
        // 17 since #2528 E4a added the two combat-start fire points. The
        // manifest reports what the engine *can* fire; the coverage test below
        // is what pins that a loaded document does not replay them.
        assert_eq!(
            manifest["hooks"]["fired"].as_array().map(Vec::len),
            Some(17)
        );
    }

    /// Coverage is derived from what ran, not from what the tables said might
    /// run: an untouched session reports nothing, and the pinned line reports
    /// exactly the slice's implemented kinds.
    #[test]
    fn coverage_reports_the_kinds_the_played_line_actually_exercised() {
        let pins: Value = serde_json::from_str(PINS).unwrap();
        let mut script = String::new();
        // Before anything is played: empty, in a session whose recorder was
        // just reset by `serve`.
        script.push_str("{\"cmd\":\"coverage\",\"reset\":true}\n");
        script.push_str(&format!("{}\n", load_line()));
        for step in pins["steps"].as_array().unwrap() {
            script.push_str(&json!({ "cmd": "apply", "action": step["action"] }).to_string());
            script.push('\n');
        }
        script.push_str("{\"cmd\":\"coverage\",\"reset\":true}\n");
        script.push_str("{\"cmd\":\"coverage\"}\n");
        let responses = drive(&script);

        let before = &responses[1]["ok"]["coverage"];
        assert_eq!(before["steps"], json!([]));
        assert_eq!(before["transitions"], json!(0));

        let after = &responses[responses.len() - 2]["ok"]["coverage"];
        // The starter deck plays Strike/Defend/Bash, so all three implemented
        // step kinds run; TOADPOLE's loop runs all three implemented moves.
        assert_eq!(after["steps"], json!(["attack", "block", "vulnerable"]));
        assert_eq!(
            after["moves"],
            json!(["attack", "buff_thorns", "spit_attack"])
        );
        assert_eq!(after["powers"], json!(["thorns", "vuln"]));
        assert_eq!(
            after["transitions"],
            json!(pins["steps"].as_array().unwrap().len())
        );
        // Every fire point the turn structure reaches is recorded, and none
        // the engine does not fire.
        //
        // The two combat-start hooks are the exception, and pinning that is
        // the point of this block. Since #2528 they ARE fire points — the
        // opening fires them once per fight — but `diff-serve` loads an
        // already-opened canonical document, so replaying them here would
        // apply Data Disk's Focus or Fake Anchor's Block a second time on top
        // of a state that already reflects them.
        const OPENING_ONLY: [sts_sim::hooks::HookEvent; 2] = [
            sts_sim::hooks::HookEvent::AfterRoomEntered,
            sts_sim::hooks::HookEvent::BeforeCombatStart,
        ];
        let hooks: Vec<String> = serde_json::from_value(after["hooks"].clone()).unwrap();
        for event in sts_sim::engine::FIRE_POINTS {
            if OPENING_ONLY.contains(&event) {
                continue;
            }
            assert!(
                hooks.contains(&event.as_str().to_string()),
                "fire point {} was never recorded: {hooks:?}",
                event.as_str()
            );
        }
        for event in OPENING_ONLY {
            assert!(
                !hooks.contains(&event.as_str().to_string()),
                "{} was replayed on a loaded post-opening document",
                event.as_str()
            );
        }

        // `reset: true` cleared it, so the peek that follows is empty.
        assert_eq!(
            responses[responses.len() - 1]["ok"]["coverage"]["steps"],
            json!([])
        );
    }

    #[test]
    fn queen_burn_bright_smoke_is_narrow_and_records_the_ordinary_move() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"queen_burn_bright_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("queen_burn_bright"));
        assert_eq!(responses[2]["ok"]["amalgam_strength"], json!(1));
        assert_eq!(responses[2]["ok"]["queen_block"], json!(20));
        assert_eq!(responses[2]["ok"]["queen_loop_pos"], json!(2));
        assert!(
            responses[3]["ok"]["coverage"]["moves"]
                .as_array()
                .unwrap()
                .contains(&json!("queen_burn_bright"))
        );
    }

    #[test]
    fn queen_puppet_strings_smoke_is_narrow_and_records_the_ordinary_move() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"queen_puppet_strings_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("queen_puppet_strings"));
        assert_eq!(responses[2]["ok"]["chains_of_binding"], json!(3));
        assert_eq!(responses[2]["ok"]["queen_loop_pos"], json!(1));
        assert!(
            responses[3]["ok"]["coverage"]["moves"]
                .as_array()
                .unwrap()
                .contains(&json!("queen_puppet_strings"))
        );
    }

    #[test]
    fn whistle_smoke_is_narrow_and_records_the_exact_step() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"whistle_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("whistle"));
        assert_eq!(responses[2]["ok"]["queen_hp"], json!(engine::QUEEN_HP - 33));
        assert_eq!(responses[2]["ok"]["queen_override"], json!("STUNNED"));
        assert_eq!(
            responses[2]["ok"]["queen_follow_up"],
            json!("PUPPET_STRINGS_MOVE")
        );
        assert_eq!(responses[2]["ok"]["exhaust"], json!(1));
        assert!(
            responses[3]["ok"]["coverage"]["steps"]
                .as_array()
                .unwrap()
                .contains(&json!("stun_target"))
        );
    }

    #[test]
    fn spectral_hex_smoke_is_narrow_and_records_the_ordinary_move() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"spectral_hex_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("hex_player"));
        assert_eq!(responses[2]["ok"]["hex_power"], json!(2));
        assert!(
            responses[3]["ok"]["coverage"]["moves"]
                .as_array()
                .unwrap()
                .contains(&json!("hex_player"))
        );
    }

    #[test]
    fn magi_dampen_smoke_is_narrow_and_records_the_ordinary_move() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"magi_dampen_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("dampen_player"));
        assert_eq!(responses[2]["ok"]["loop_pos"], json!(2));
        assert_eq!(responses[2]["ok"]["tracked"], json!(1));
        assert_eq!(responses[2]["ok"]["upgrade"], json!(0));
        assert!(
            responses[3]["ok"]["coverage"]["moves"]
                .as_array()
                .unwrap()
                .contains(&json!("dampen_player"))
        );
    }

    #[test]
    fn gremlin_merc_move_witnesses_record_attack_steal_and_escape() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"gremlin_merc_attack_steal_smoke\"}\n\
             {\"cmd\":\"fat_gremlin_escape_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("attack_steal"));
        assert_eq!(responses[2]["ok"]["gold"], json!(0));
        assert_eq!(responses[2]["ok"]["stolen_gold"], json!(20));
        assert_eq!(responses[3]["ok"]["move"], json!("escape"));
        assert_eq!(responses[3]["ok"]["monster_count"], json!(2));
        let moves = responses[4]["ok"]["coverage"]["moves"].as_array().unwrap();
        assert!(moves.contains(&json!("attack_steal")));
        assert!(moves.contains(&json!("escape")));
    }

    #[test]
    fn thieving_hopper_witness_records_all_three_moves_and_power_readers() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"thieving_hopper_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("thieving_hopper"));
        assert_eq!(responses[2]["ok"]["over"], json!(true));
        assert_eq!(responses[2]["ok"]["monster_count"], json!(0));
        let coverage = &responses[3]["ok"]["coverage"];
        let moves = coverage["moves"].as_array().unwrap();
        for kind in ["hopper_thievery", "hopper_flutter", "hopper_escape"] {
            assert!(moves.contains(&json!(kind)), "missing {kind}: {moves:?}");
        }
        let powers = coverage["powers"].as_array().unwrap();
        for power in ["escape_artist", "flutter"] {
            assert!(
                powers.contains(&json!(power)),
                "missing {power}: {powers:?}"
            );
        }
    }

    #[test]
    fn aeonglass_witness_records_constructor_draw_and_intensity() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"aeonglass_intensity_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("aeonglass_intensity"));
        assert_eq!(responses[2]["ok"]["niche_draws"], json!(1));
        assert_eq!(responses[2]["ok"]["wither_count"], json!(2));
        let moves = responses[3]["ok"]["coverage"]["moves"].as_array().unwrap();
        assert!(moves.contains(&json!("aeonglass_intensity")));
    }

    #[test]
    fn outbreak_witness_records_the_exact_step() {
        let responses = drive(
            "{\"cmd\":\"coverage\",\"reset\":true}\n\
             {\"cmd\":\"outbreak_smoke\"}\n\
             {\"cmd\":\"coverage\",\"reset\":true}\n",
        );
        assert_eq!(responses[2]["ok"]["move"], json!("outbreak"));
        assert_eq!(responses[2]["ok"]["target_hp"], json!(91));
        assert_eq!(responses[2]["ok"]["poison"], json!(8));
        assert_eq!(responses[2]["ok"]["poison_uid"], json!(0));
        assert!(
            responses[3]["ok"]["coverage"]["steps"]
                .as_array()
                .unwrap()
                .contains(&json!("outbreak_exact"))
        );
    }

    #[test]
    fn coverage_refuses_a_non_boolean_reset_without_ending_the_session() {
        let responses = drive("{\"cmd\":\"coverage\",\"reset\":7}\n{\"cmd\":\"quit\"}\n");
        assert_eq!(refusal_kind(&responses[1]), "malformed_request");
        assert_eq!(responses[2], json!({ "ok": { "quit": true } }));
    }

    #[test]
    fn the_session_opens_with_a_versioned_greeting() {
        let responses = drive("");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0], json!({ "protocol": PROTOCOL_V1 }));
    }

    #[test]
    fn quit_ends_the_session_and_ignores_trailing_input() {
        let responses = drive("{\"cmd\":\"quit\"}\n{\"cmd\":\"legal\"}\n");
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[1], json!({ "ok": { "quit": true } }));
    }

    /// #3242: `native_checkpoints` is opt-in, leaves the transition alone,
    /// and reports an empty list for an action that spans no boundary.
    #[test]
    fn apply_reports_native_checkpoints_only_when_asked() {
        let end = json!({ "cmd": "apply", "action": { "kind": "end" } });
        let recorded = json!({
            "cmd": "apply", "action": { "kind": "end" }, "native_checkpoints": true,
        });
        let responses = drive(&format!(
            "{}\n{end}\n{}\n{recorded}\n",
            load_line(),
            load_line()
        ));
        let plain = &responses[2]["ok"];
        let asked = &responses[4]["ok"];
        assert!(plain.get("native_checkpoints").is_none());
        assert_eq!(asked["native_checkpoints"], json!([]));
        assert_eq!(asked["digest"], plain["digest"]);
    }

    /// Each recorded state projects, or names why it does not (#3242).
    #[test]
    fn native_checkpoint_states_project_or_refuse_by_name() {
        use engine::native_checkpoint::NativeCheckpointKind;
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let reported = native_checkpoints_json(
            &[(NativeCheckpointKind::VoidFormEndTurnRequest, state.clone())],
            &catalog,
        );
        assert_eq!(reported[0]["kind"], "void_form_end_turn_request");
        assert_eq!(
            reported[0]["state"],
            serde_json::to_value(HotBoundary::try_to_canonical(&state, &catalog).unwrap()).unwrap()
        );
        assert!(reported[0].get("refusal").is_none());

        // Against a catalog that interns none of its cards the same state
        // cannot project: the entry names the refusal instead of vanishing.
        let empty = CatalogBuilder::new().build();
        assert!(HotBoundary::try_to_canonical(&state, &empty).is_err());
        let reported = native_checkpoints_json(
            &[(NativeCheckpointKind::AutoPostHookFinished, state)],
            &empty,
        );
        assert_eq!(reported.as_array().unwrap().len(), 1);
        assert_eq!(reported[0]["kind"], "auto_post_hook_finished");
        assert_eq!(reported[0]["state"], Value::Null);
        assert!(
            reported[0]["refusal"]
                .as_str()
                .is_some_and(|detail| !detail.is_empty())
        );
    }

    #[test]
    fn load_admits_the_fixture_and_echoes_its_digest() {
        let responses = drive(&format!("{}\n", load_line()));
        let response = &responses[1];
        assert_eq!(response["ok"]["loaded"], Value::Bool(true));
        let state: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(response["ok"]["digest"], state.differential_digest());
        assert_eq!(
            response["ok"]["schema"],
            sts_sim::canonical::STATE_SCHEMA_V2
        );
    }

    /// `intents` names every Toadpole's next loop row, prices only the exact
    /// attack, leaves the loaded state untouched, and the priced WHIRL is
    /// what the enemy turn then deals to the unblocked player.
    #[test]
    fn intents_price_the_attack_the_enemy_turn_then_deals() {
        let script = format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n",
            load_line(),
            json!({ "cmd": "project" }),
            json!({ "cmd": "intents" }),
            json!({ "cmd": "project" }),
            json!({ "cmd": "apply", "action": { "kind": "end" } }),
            json!({ "cmd": "project" }),
        );
        let responses = drive(&script);
        assert_eq!(
            responses[3]["ok"]["intents"],
            json!([
                { "uid": 0, "intent": "SPIKEN" },
                { "uid": 1, "intent": "WHIRL", "intent_damage": 8, "intent_hits": 1 },
            ])
        );
        assert_eq!(responses[2]["ok"]["digest"], responses[4]["ok"]["digest"]);
        let hp = |response: &Value| response["ok"]["state"]["player"]["hp"].as_i64().unwrap();
        assert_eq!(hp(&responses[2]) - hp(&responses[6]), 8);
        assert_eq!(
            refusal_kind(&drive(&format!("{}\n", json!({ "cmd": "intents" })))[1]),
            "no_state_loaded"
        );
    }

    /// #2828: the query prices the fight's tier, and the enemy turn deals
    /// it. WHIRL is `Toadpole::get_WhirlDamage` RVA `0xc2722`,
    /// `GetValueIfAscension(9, 8, 7)`: 7 at A8, 8 at A9.
    #[test]
    fn intents_price_the_fights_ascension_tier() {
        for (ascension, whirl) in [(8, 7), (9, 8)] {
            let mut entry: Value = serde_json::from_str(FIXTURE).unwrap();
            entry["player"]["ascension"] = json!(ascension);
            let script = format!(
                "{}\n{}\n{}\n{}\n{}\n",
                json!({ "cmd": "load", "entry": entry }),
                json!({ "cmd": "project" }),
                json!({ "cmd": "intents" }),
                json!({ "cmd": "apply", "action": { "kind": "end" } }),
                json!({ "cmd": "project" }),
            );
            let responses = drive(&script);
            assert_eq!(
                responses[3]["ok"]["intents"][1],
                json!({ "uid": 1, "intent": "WHIRL", "intent_damage": whirl, "intent_hits": 1 }),
                "A{ascension}"
            );
            let hp = |response: &Value| response["ok"]["state"]["player"]["hp"].as_i64().unwrap();
            assert_eq!(hp(&responses[2]) - hp(&responses[5]), whirl, "A{ascension}");
        }
    }

    /// #2814: a sleeping Lagavulin Matriarch shows its native `SLEEP_MOVE`
    /// (a `SleepIntent`: named, no damage) for each of its three sleeping
    /// sides, and once the countdown wakes it the SLASH it will deal.
    #[test]
    fn intents_name_a_sleeping_lagavulin_then_price_its_waking_slash() {
        let mut entry: Value = serde_json::from_str(FIXTURE).unwrap();
        entry["monsters"] = json!([{
            "asleep": 3, "block": 12, "hp": 233, "kind": "LAGAVULIN_MATRIARCH",
            "max_hp": 233, "mplating": 12,
        }]);
        let end = json!({ "cmd": "apply", "action": { "kind": "end" } });
        let intents = json!({ "cmd": "intents" });
        let script = format!(
            "{}\n{intents}\n{end}\n{intents}\n{end}\n{intents}\n{end}\n{intents}\n",
            json!({ "cmd": "load", "entry": entry }),
        );
        let responses = drive(&script);
        let sleeping = json!([{ "uid": 0, "intent": "SLEEP_MOVE" }]);
        for response in [2, 4, 6] {
            assert_eq!(responses[response]["ok"]["intents"], sleeping, "{response}");
        }
        assert_eq!(
            responses[8]["ok"]["intents"],
            json!([{ "uid": 0, "intent": "SLASH_MOVE", "intent_damage": 21, "intent_hits": 1 }])
        );
    }

    #[test]
    fn legal_returns_the_canonical_action_encoding() {
        let responses = drive(&format!("{}\n{{\"cmd\":\"legal\"}}\n", load_line()));
        assert_eq!(
            responses[2]["ok"]["actions"],
            json!([
                { "kind": "play", "uid": 0, "target": 1 },
                { "kind": "play", "uid": 0, "target": 0 },
                { "kind": "play", "uid": 2 },
                { "kind": "end" },
            ])
        );
    }

    #[test]
    fn the_action_wire_round_trips_a_second_card_selection() {
        let action = Action::Play {
            uid: 12,
            target: None,
            selection: SelectionRef::new(Some(37)),
        };
        let encoded = encode_action(&action);

        assert_eq!(
            encoded,
            json!({ "kind": "play", "uid": 12, "selection": 37 })
        );
        assert_eq!(decode_action(&encoded).unwrap(), action);

        for (answer, encoded) in [
            (
                engine::SelectionAnswer::CardUid(37),
                json!({ "kind": "select", "answer": { "kind": "card_uid", "uid": 37 } }),
            ),
            (
                engine::SelectionAnswer::OptionIndex(4),
                json!({ "kind": "select", "answer": { "kind": "option_index", "index": 4 } }),
            ),
        ] {
            let action = Action::Select { answer };
            assert_eq!(encode_action(&action), encoded);
            assert_eq!(decode_action(&encoded).unwrap(), action);
        }
        for malformed in [
            // The deprecated flat spelling. It is refused, not quietly
            // accepted beside the canonical one: two live spellings are how
            // #2477 stayed invisible.
            json!({ "kind": "select", "uid": 37 }),
            json!({ "kind": "select", "index": 4 }),
            json!({ "kind": "select", "uid": 3, "index": 4 }),
            // A mixed or over-wide answer object cannot be interpreted by
            // whichever branch is convenient.
            json!({ "kind": "select", "answer": { "kind": "card_uid", "index": 4 } }),
            json!({
                "kind": "select",
                "answer": { "kind": "option_index", "index": 4, "uid": 3 },
            }),
            json!({ "kind": "select", "answer": { "kind": "slot", "index": 4 } }),
            json!({ "kind": "select", "answer": { "kind": "option_index", "index": -1 } }),
            json!({ "kind": "select" }),
            json!({
                "kind": "select",
                "answer": { "kind": "option_index", "index": 4 },
                "uid": 37,
            }),
        ] {
            assert!(
                decode_action(&malformed).is_err(),
                "noncanonical select accepted: {malformed}"
            );
        }
    }

    /// The crate has exactly one action vocabulary (#2477).
    ///
    /// `diff-serve` and `exact-solve` v1 are the two places an action crosses
    /// a wire, and the Python review side (`rust_replay.recorded_witness`,
    /// `rust_review.replay_line`) speaks the v1 spelling in both directions. Comparing the two encoders against each other —
    /// rather than each against a hand-written literal — is what makes a
    /// future divergence a failing test instead of a refused human line.
    #[test]
    fn the_action_wire_is_the_stable_v1_wire() {
        for action in [
            Action::EndTurn,
            Action::Play {
                uid: 12,
                target: None,
                selection: SelectionRef::new(None),
            },
            Action::Play {
                uid: 8,
                target: Some(1),
                selection: SelectionRef::new(None),
            },
            Action::Play {
                uid: 12,
                target: None,
                selection: SelectionRef::new(Some(37)),
            },
            Action::UsePotion {
                slot: 0,
                target: None,
            },
            Action::UsePotion {
                slot: 3,
                target: Some(2),
            },
            Action::Select {
                answer: engine::SelectionAnswer::CardUid(37),
            },
            Action::Select {
                answer: engine::SelectionAnswer::OptionIndex(0),
            },
            Action::Select {
                answer: engine::SelectionAnswer::OptionIndex(9),
            },
        ] {
            let stable =
                serde_json::to_value(sts_sim::exact_solve_v1::ExactSolveActionV1::from(action))
                    .expect("the stable v1 action serializes");
            assert_eq!(
                encode_action(&action),
                stable,
                "diff-serve emits a shape exact-solve v1 does not: {action:?}"
            );
            assert_eq!(
                decode_action(&stable).unwrap(),
                action,
                "diff-serve refuses the shape exact-solve v1 emits: {action:?}"
            );
        }
    }

    #[test]
    fn potion_action_wire_round_trips_target_and_rejects_reserved_shape() {
        for action in [
            Action::UsePotion {
                slot: 2,
                target: None,
            },
            Action::UsePotion {
                slot: 255,
                target: Some(254),
            },
        ] {
            let encoded = encode_action(&action);
            assert_eq!(decode_action(&encoded).unwrap(), action);
        }
        for malformed in [
            json!({"kind": "potion"}),
            json!({"kind": "potion", "slot": 256}),
            json!({"kind": "potion", "slot": 0, "target": 256}),
            json!({"kind": "potion", "slot": 0, "reserved": 0}),
            json!({"kind": "potion", "slot": 0, "target": null, "reserved": 0}),
        ] {
            assert!(decode_action(&malformed).is_err(), "accepted {malformed}");
        }
    }

    #[test]
    fn apply_then_project_agrees_with_the_python_pins() {
        // The whole pinned line, through the wire protocol rather than the
        // library API: the transport must not be able to lose or reorder a
        // field the in-process differential already proved.
        let pins: Value = serde_json::from_str(PINS).unwrap();
        let steps = pins["steps"].as_array().unwrap();
        let mut script = format!("{}\n", load_line());
        for step in steps {
            script.push_str(&json!({ "cmd": "apply", "action": step["action"] }).to_string());
            script.push('\n');
        }
        script.push_str("{\"cmd\":\"project\"}\n");
        let responses = drive(&script);
        for (index, step) in steps.iter().enumerate() {
            let response = &responses[index + 2];
            assert_eq!(
                response["ok"]["digest"], step["digest"],
                "step {index} diverged over the wire: {response}"
            );
        }
        let projected = &responses[steps.len() + 2];
        assert_eq!(
            projected["ok"]["digest"],
            steps.last().unwrap()["digest"],
            "project must agree with the last apply"
        );
        // `project` returns a full canonical document, not just a digest.
        let document: CanonicalStateV2 =
            serde_json::from_value(projected["ok"]["state"].clone()).unwrap();
        assert_eq!(
            document.differential_digest(),
            steps.last().unwrap()["digest"].as_str().unwrap()
        );
    }

    #[test]
    fn an_entry_outside_the_slice_refuses_with_the_whole_missing_list() {
        let mut entry: Value = serde_json::from_str(FIXTURE).unwrap();
        // Crusher used to be a stable multi-refusal carrier, but slice 6
        // implements one of its two missing rows. Test Subject deliberately
        // remains outside this slice on several independent boss mechanics.
        entry["monsters"][0]["kind"] = Value::String("TEST_SUBJECT".to_string());
        let responses = drive(&format!("{}\n", json!({ "cmd": "load", "entry": entry })));
        assert_eq!(refusal_kind(&responses[1]), "not_admitted");
        let missing = responses[1]["missing"].as_array().unwrap();
        assert!(
            missing.len() > 1,
            "admission must list every missing capability: {missing:?}"
        );
        // And nothing was loaded: the session is unchanged.
        let after = drive(&format!(
            "{}\n{{\"cmd\":\"legal\"}}\n",
            json!({ "cmd": "load", "entry": entry })
        ));
        assert_eq!(refusal_kind(&after[2]), "no_state_loaded");
    }

    #[test]
    fn an_unrepresentable_entry_refuses_before_admission() {
        let mut entry: Value = serde_json::from_str(FIXTURE).unwrap();
        entry["player"]["surprise_power"] = Value::from(3);
        let responses = drive(&format!("{}\n", json!({ "cmd": "load", "entry": entry })));
        assert_eq!(refusal_kind(&responses[1]), "unrepresentable_state");
    }

    #[test]
    fn invalid_potion_slot_is_illegal_while_a_valid_unmodeled_body_has_its_own_kind() {
        let responses = drive(&format!(
            "{}\n{}\n{}\n",
            load_line(),
            json!({ "cmd": "apply", "action": { "kind": "play", "uid": 99 } }),
            json!({ "cmd": "apply", "action": { "kind": "potion", "slot": 0 } }),
        ));
        assert_eq!(refusal_kind(&responses[2]), "illegal_action");
        assert_eq!(refusal_kind(&responses[3]), "illegal_action");
        assert_eq!(
            engine_refusal_kind(&EngineRefusal::PotionsNotModeled),
            RefusalKind::EngineNotImplemented,
        );
    }

    #[test]
    fn every_state_dependent_command_refuses_without_a_loaded_state() {
        for command in ["legal", "apply", "resolve_selection", "project"] {
            let responses = drive(&format!("{{\"cmd\":\"{command}\"}}\n"));
            assert_eq!(refusal_kind(&responses[1]), "no_state_loaded");
            assert_eq!(responses[1]["refusal"]["site"], command);
        }
    }

    #[test]
    fn malformed_input_refuses_without_ending_the_session() {
        let responses = drive(
            "not json\n\
             [1,2,3]\n\
             {\"cmd\":7}\n\
             {\"nope\":1}\n\
             {\"cmd\":\"teleport\"}\n\
             {\"cmd\":\"load\"}\n\
             {\"cmd\":\"load\",\"entry\":{\"schema\":\"sts-sim-canonical-v2\"}}\n\
             {\"cmd\":\"quit\"}\n",
        );
        let kinds: Vec<&str> = responses[1..responses.len() - 1]
            .iter()
            .map(refusal_kind)
            .collect();
        assert_eq!(
            kinds,
            [
                "malformed_request",
                "malformed_request",
                "malformed_request",
                "missing_command",
                "unknown_command",
                "missing_field",
                "malformed_entry",
            ]
        );
        assert_eq!(
            responses[responses.len() - 1],
            json!({ "ok": { "quit": true } })
        );
    }

    #[test]
    fn a_malformed_action_refuses_without_ending_the_session() {
        let responses = drive(&format!(
            "{}\n{}\n{}\n{{\"cmd\":\"apply\"}}\n{{\"cmd\":\"quit\"}}\n",
            load_line(),
            json!({ "cmd": "apply", "action": 7 }),
            json!({ "cmd": "apply", "action": { "kind": "teleport" } }),
        ));
        assert_eq!(refusal_kind(&responses[2]), "malformed_action");
        assert_eq!(refusal_kind(&responses[3]), "malformed_action");
        assert_eq!(refusal_kind(&responses[4]), "missing_field");
    }

    #[test]
    fn an_entry_with_a_foreign_schema_refuses_on_the_schema() {
        let mut entry: Value = serde_json::from_str(FIXTURE).unwrap();
        entry["schema"] = Value::String("sts-kernel-state/v1".to_string());
        let responses = drive(&format!("{}\n", json!({ "cmd": "load", "entry": entry })));
        assert_eq!(refusal_kind(&responses[1]), "unsupported_schema");
    }

    #[test]
    fn blank_lines_are_skipped_rather_than_answered() {
        let responses = drive("\n   \n{\"cmd\":\"quit\"}\n");
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[1], json!({ "ok": { "quit": true } }));
    }

    #[test]
    fn every_response_carries_exactly_one_of_ok_or_refusal() {
        let responses = drive(&format!(
            "{}\n{{\"cmd\":\"legal\"}}\nbroken\n{{\"cmd\":\"project\"}}\n{{\"cmd\":\"quit\"}}\n",
            load_line()
        ));
        for response in &responses[1..] {
            let object = response.as_object().unwrap();
            assert_eq!(
                object.contains_key("ok") as u8 + object.contains_key("refusal") as u8,
                1,
                "response must carry exactly one of ok/refusal: {response}"
            );
        }
    }
}

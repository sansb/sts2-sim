//! Encounter roster builders — the content half of `make_monsters` (#2529).
//!
//! One module per family, mirroring the `content/encounters/*.py` pool shards
//! the Python oracle is sharded into. A builder is a pure function of the
//! fight's RNG context: it returns the roster **in creation order** and every
//! number it puts in that roster comes from a generated table
//! (`content_tables::MONSTER_MODELS` for HP bands and spawn-time power
//! amounts, `content_tables::encounter_pool_constants` for the per-slot
//! starter orders that live in IL branch structure). PORT_PLAN §5: a number
//! typed into a Rust file by hand is a review rejection even when it is
//! correct.
//!
//! # What is deliberately NOT here
//!
//! The engine half is #2528 (E4a), §A8 of
//! `solver/invariant-walks/2026-09-16-issue2528-entry-opening-spec.md`, and a
//! content PR may not inline any of it:
//!
//! * `make_monsters`' substring dispatch over the registered keys. The
//!   generated `content_tables::ENCOUNTER_ROSTER_BUILDERS` is keyed by
//!   `EncounterId`; turning a wire id such as `ENCOUNTER.DECIMILLIPEDE_ELITE`
//!   into the `Decimillipede` key is the engine's, and it is a **substring**
//!   match, not equality.
//! * The `_EncounterCtx` equivalent — the live `Niche` carrier, the per-fight
//!   `Encounter` stream derivation (one site, with its `total_floor`
//!   refusal), and the creation-order uid stamping `done` performs. This
//!   module declares the interface (`EncounterCtx`) the builders are written
//!   against; nothing here implements it against a real fight.
//! * The roster -> `HotMonster` admission edge. `MonsterSpec` is data;
//!   turning it into hot state, including mapping `initial_state` onto
//!   `HotMonster`'s powers, is engine.
//! * Turn-1 initial-RAND intent rolls and `_initialize_random_ai`.
//!
//! So until E4a lands, `ENCOUNTER_ROSTER_BUILDERS` has no caller in the
//! engine: the builders are exercised by their own oracle tests against
//! Python's `make_monsters`, which is what E4b's acceptance measures.

pub mod boss;
pub mod elite;
pub mod event;
pub mod normal_a;
pub mod normal_b;
pub mod normal_c;
pub mod weak;

use crate::content_tables::{AscensionTier, MONSTER_MODELS, MonsterModel};
use crate::ids::MonsterKind;

/// An exactness failure a roster builder cannot resolve (SOLVER_INVARIANTS
/// I5). Every variant names the fact that is missing rather than standing in
/// a plausible value; E4a maps these into the engine's refusal vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterRefusal {
    /// `content_tables::MONSTER_MODELS` has no exactly-read row for this
    /// kind — the assembly reader recorded why instead of guessing.
    MonsterModelUnread {
        /// The kind whose native model could not be read.
        kind: MonsterKind,
        /// The reader's reason, verbatim from the generated table.
        why: &'static str,
    },
    /// A builder asked for a single fixed HP on a kind whose native band is
    /// a range. `Creature::SetUniqueMonsterHpValue` would roll, not pin.
    MonsterHpNotFixed {
        /// The kind whose `MinInitialHp` and `MaxInitialHp` differ.
        kind: MonsterKind,
    },
    /// A builder asked for a spawn-time power the assembly does not apply at
    /// this kind's `AfterAddedToRoom`.
    MonsterPowerAbsent {
        /// The kind whose spawn-time powers were searched.
        kind: MonsterKind,
        /// The native power class the builder expected.
        power: &'static str,
    },
    /// The per-fight `Encounter` stream cannot be derived. It is seeded from
    /// the run seed, the floor and the encounter's `Id.Entry`, so a missing
    /// floor would silently pick a different roster.
    EncounterStreamUnavailable {
        /// What the draw was for, for the message.
        roll: &'static str,
    },
    /// A builder asked for a rotation `content_tables::LOOPS` does not carry
    /// for this kind — a random-AI kind has no deterministic rotation, so a
    /// starting position in one is not a fact about it.
    MonsterLoopAbsent {
        /// The kind whose rotation was searched.
        kind: MonsterKind,
    },
    /// A builder named a `MoveState` that is not in this kind's rotation.
    /// Reading a starting position by move name rather than by index is what
    /// keeps the index out of the Rust source (PORT_PLAN §5), so a name that
    /// does not resolve is a codegen/content disagreement, not a default.
    MonsterLoopMoveAbsent {
        /// The kind whose rotation was searched.
        kind: MonsterKind,
        /// The `MoveState` name the builder asked for.
        move_name: &'static str,
    },
}

/// One monster of a roster, in creation order — the content lane's output.
///
/// Mirrors exactly the `combat_sim.Monster` keyword arguments the
/// `content/encounters/*.py` builders set, and nothing else: the rest of
/// `Monster`'s fields are engine defaults that the admission edge fills in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonsterSpec {
    /// `Monster.kind`.
    pub kind: MonsterKind,
    /// `Monster.hp`.
    pub hp: i32,
    /// `Monster.max_hp` — the rolled or adjusted maximum.
    pub max_hp: i32,
    /// `Monster.slot` — the encounter slot index. Creation order is the
    /// roster's own order; `slot` is only set where a builder pins it.
    pub slot: i32,
    /// `Monster.loop_pos` — index into the rotation for the NEXT move.
    pub loop_pos: i32,
    /// `Monster.next_move` — the random-AI rolled move; `""` when unset.
    pub next_move: &'static str,
    /// `Monster.move_log` — the random-AI move log.
    pub move_log: &'static [&'static str],
    /// Spawn-time roster state, as `(combat_sim.Monster field, value)` in
    /// application order, each value carrying the field's OWN Python type.
    ///
    /// Named by the Python field rather than by the native power class on
    /// purpose: which of a creature's `AfterAddedToRoom` powers is roster
    /// state and which is engine behaviour is the port's modeling, and this
    /// is the side of that boundary the oracle compares.
    pub initial_state: Vec<(&'static str, SpawnValue)>,
}

/// One spawn-time field's value, tagged with the type `combat_sim` declares
/// for that field.
///
/// The tag is load-bearing rather than cosmetic (#2790). `project_state.py`'s
/// zero-default elision requires `type(value) is type(default)`, and
/// `canonical_json` spells `1` and `true` differently, so an amount and a
/// flag are different documents at the same field — different bytes, a
/// different `differential_digest`, and a different fight. The builder API
/// used to take `i64` for everything, which forced `with_state("shriek", 1)`
/// on a field the frozen Python (deleted #2827) declares `shriek: bool = False`; the
/// hybrid census's opening-parity gate compared with Python `==`, where
/// `1 == True`, and substituted that root on `fc4cf049784d8f31`
/// TERROR_EEL_ELITE, whose per-action lockstep then diverged everywhere.
///
/// Which fields are flags is not a judgement call made here: it was read from
/// the oracle's own dataclass. `tools/projection_parity_census.py` re-derived
/// every `with_state`/`with_flag` site under `src/` and every field's type
/// out of `combat_sim.Monster`, and refused on any disagreement (deleted
/// #2999 with the simulator it read; `PROJECTION_PARITY_CENSUS.md` is its
/// frozen last run).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnValue {
    /// A field `combat_sim` declares `int` — a power Amount, a counter, a
    /// stack count.
    Amount(i64),
    /// A field `combat_sim` declares `bool` — a power whose PRESENCE is the
    /// whole state, such as the Eel's `ShriekPower`.
    Flag(bool),
    /// A field `combat_sim` declares `str` — today only `Monster.override`
    /// (frozen Python, deleted #2827; default `""`), which the Slumbering Beetle's
    /// roster sets to `"SNORE"` (#2848). The boundary already carries it as
    /// `FieldDefault::Text("")`; this is the roster-vocabulary half.
    Text(&'static str),
}

impl SpawnValue {
    /// This value as an integer, for an Amount or a Flag (`0`/`1`), and
    /// `None` for Text.
    ///
    /// This is the reading a caller asking for a power amount wants (the
    /// opening's Artifact lookup); a text field has no integer reading.
    pub fn as_i64(self) -> Option<i64> {
        match self {
            Self::Amount(value) => Some(value),
            Self::Flag(flag) => Some(i64::from(flag)),
            Self::Text(_) => None,
        }
    }

    /// This value as the roster pins record it.
    ///
    /// `tools/gen_roster_pins.py::_initial_state` writes `int(value)` for a
    /// bool, so a flag pins as `0`/`1`, and writes a `str` field as the
    /// string itself. That is the PIN's encoding, not the wire's — the
    /// document keeps the type (`entry::opening::monster_entity`).
    pub fn pin(self) -> serde_json::Value {
        match self {
            Self::Text(text) => serde_json::Value::from(text),
            integer => serde_json::Value::from(integer.as_i64().expect("an integer spawn value")),
        }
    }
}

impl MonsterSpec {
    /// A monster with the engine defaults for everything a builder does not
    /// set (`combat_sim.Monster`'s dataclass defaults).
    pub fn new(kind: MonsterKind, hp: i32) -> Self {
        Self {
            kind,
            hp,
            max_hp: hp,
            slot: 0,
            loop_pos: 0,
            next_move: "",
            move_log: &[],
            initial_state: Vec::new(),
        }
    }

    /// Pin `Monster.slot`.
    pub fn slot(mut self, slot: i32) -> Self {
        self.slot = slot;
        self
    }

    /// Pin `Monster.loop_pos`.
    pub fn loop_pos(mut self, loop_pos: i32) -> Self {
        self.loop_pos = loop_pos;
        self
    }

    /// Pin the random-AI opening move and its log.
    pub fn opening_move(
        mut self,
        next_move: &'static str,
        move_log: &'static [&'static str],
    ) -> Self {
        self.next_move = next_move;
        self.move_log = move_log;
        self
    }

    /// Carry one spawn-time `combat_sim.Monster` field as an AMOUNT.
    ///
    /// For a field the oracle declares `bool`, use [`Self::with_flag`]: the
    /// two produce different canonical bytes and the parity census refuses a
    /// site that picks the wrong one (#2790).
    pub fn with_state(mut self, field: &'static str, value: i64) -> Self {
        self.initial_state.push((field, SpawnValue::Amount(value)));
        self
    }

    /// Carry one spawn-time `combat_sim.Monster` field that is a FLAG.
    ///
    /// The oracle's own spelling: `content/encounters/elite.py::_eel` (frozen
    /// Python, deleted #2827) writes
    /// `ctx.fixed(EEL, 150, shriek=True)`, not `shriek=1`.
    pub fn with_flag(mut self, field: &'static str, value: bool) -> Self {
        self.initial_state.push((field, SpawnValue::Flag(value)));
        self
    }

    /// Carry one spawn-time `combat_sim.Monster` field that is TEXT.
    ///
    /// The oracle's own spelling: `content/encounters/normal.py::_slumbering_beetle`
    /// (frozen Python, deleted #2827)
    /// writes `Monster(SLUMBERING_BEETLE, ..., override="SNORE")` (#2848).
    /// `tools/projection_parity_census.py` checked every call site against the
    /// field's declared `str` type, as it did for the other two kinds (deleted
    /// #2999).
    pub fn with_text(mut self, field: &'static str, value: &'static str) -> Self {
        self.initial_state.push((field, SpawnValue::Text(value)));
        self
    }
}

/// The fight-scoped RNG context a roster builder is handed.
///
/// This is the interface half of the `_EncounterCtx` equivalent #2528 §A8
/// assigns to the engine lane. Every method is exactly one `combat_sim`
/// operation, so a builder reads as its Python counterpart does.
pub trait EncounterCtx {
    /// `RunState.AscensionLevel` of the fight being built — what every
    /// [`tier`] in a roster is selected by (#2539). `_EncounterCtx.ascension`.
    fn ascension(&self) -> u8;

    /// One draw on the fight's `Niche` stream — `Rng.next_int(lo, hi)`, with
    /// `hi` exclusive. `_EncounterCtx.fixed` spends exactly one of these for
    /// a fixed-HP monster, which is why fixed-HP monsters still consume the
    /// stream.
    fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32;

    /// One draw on this fight's `Encounter` stream —
    /// `_EncounterCtx.encounter_rng(roll).next_int(lo, hi)`.
    ///
    /// A *separate* stream per encounter, seeded by
    /// `EncounterModel::GenerateMonstersWithSlots` from the run-set seed, the
    /// total floor and the encounter's `Id.Entry`, always at counter 0. The
    /// derivation is one engine site and is never re-derived per family; a
    /// missing floor refuses through `RosterRefusal`.
    fn encounter_next_int(
        &mut self,
        roll: &'static str,
        lo: i32,
        hi: i32,
    ) -> Result<i32, RosterRefusal>;

    /// `Creature::SetUniqueMonsterHpValue`, ported from
    /// `combat_sim._roll_unique_hp`: one `Niche` draw over `[lo, hi]` minus
    /// the values already taken, iterated ascending; when nothing is left the
    /// draw is over the whole band.
    ///
    /// A provided method because it is a pure function of `niche_next_int`
    /// and nothing else — E4a's context may take it as-is.
    fn unique_hp(&mut self, lo: i32, hi: i32, taken: &[i32]) -> i32 {
        let available = (lo..=hi).filter(|v| !taken.contains(v)).count();
        if available == 0 {
            return self.niche_next_int(lo, hi + 1);
        }
        let pick = self.niche_next_int(0, available as i32);
        (lo..=hi)
            .filter(|v| !taken.contains(v))
            .nth(pick as usize)
            .expect("the draw is bounded by the candidate count")
    }
}

/// A roster builder: the content half of `make_monsters`.
///
/// Takes the fight's `_EncounterCtx` equivalent and returns the roster in
/// creation order. The generated `content_tables::ENCOUNTER_ROSTER_BUILDERS`
/// is the registry of these; the engine half that dispatches into it is
/// #2528's.
pub type RosterBuilder = fn(&mut dyn EncounterCtx) -> Result<Vec<MonsterSpec>, RosterRefusal>;

/// The roster builder registered for `encounter`, if any.
///
/// Keyed by the **registered** `EncounterId`, not by the wire id:
/// `make_monsters` matches a registered key as a substring of the wire id, and
/// that match is the engine's (#2528 §A8).
pub fn roster_builder(encounter: crate::ids::EncounterId) -> Option<RosterBuilder> {
    crate::content_tables::ENCOUNTER_ROSTER_BUILDERS
        .binary_search_by_key(&(encounter as u16), |(e, _)| *e as u16)
        .ok()
        .map(|index| crate::content_tables::ENCOUNTER_ROSTER_BUILDERS[index].1)
}

/// The ascension level a root carries when nothing says otherwise: the tier
/// every root was modeled at before #2539 (at or above every monster gate the
/// build has), `combat_sim.MODELED_ASCENSION`. It is also the wire default
/// the boundary elides, so an A10 document keeps its pre-#2539 bytes.
pub const MODELED_ASCENSION: u8 = 10;

/// The highest `AscensionLevel` a v0.111.0 run carries
/// (`combat_sim.MAX_ASCENSION`). A level above it is refused at the boundary
/// rather than read as "at or above every gate".
pub const MAX_ASCENSION: u8 = 10;

/// One native ascension-tiered constant, at `ascension` (#2539).
///
/// Native authority, v0.111.0 `sts2.dll` sha256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `AscensionHelper::GetValueIfAscension(level, atOrAbove, below)` (RVAs
/// `0x106c4c`, `0x106c5e`, `0x106c70`, one per overload, identical bodies)
/// is `RunManager::HasAscension(level) ? atOrAbove : below` (IL_0001-IL_0011).
/// `RunManager::HasAscension` (`0x4dbcc`) is false outside a run in progress
/// and otherwise `AscensionManager::HasLevel(level)` (IL_000b-IL_0017), and
/// `AscensionManager::HasLevel` (`0x11fa83`) is `ldfld _level; ldarg.1; clt;
/// ldc.i4.0; ceq` (IL_0001-IL_000d) — `!(_level < level)`, i.e. the
/// comparison is **`>=`**: the at-or-above tier applies AT the gate. A combat
/// is always inside a run in progress. `gate: None` is an untiered `ldc; ret`
/// getter, whose two fields hold the same value.
///
/// This replaced `modeled`, which returned `at_or_above` unconditionally
/// because the frozen oracle hard-coded that tier (#2539: 58 of the eval
/// manifest's 201 fights are below A8).
pub const fn tier(tier: AscensionTier, ascension: u8) -> i64 {
    match tier.gate {
        Some(gate) if ascension < gate => tier.below,
        _ => tier.at_or_above,
    }
}

/// The generated native model row for `kind`, or its recorded refusal.
pub fn monster_model(kind: MonsterKind) -> Result<&'static MonsterModel, RosterRefusal> {
    let model = &MONSTER_MODELS[kind as usize];
    match (model.min_initial_hp, model.max_initial_hp) {
        (Some(_), Some(_)) => Ok(model),
        _ => Err(RosterRefusal::MonsterModelUnread {
            kind,
            why: model.refused,
        }),
    }
}

/// `(MinInitialHp, MaxInitialHp)` at `ascension` ([`tier`]).
pub fn hp_band(kind: MonsterKind, ascension: u8) -> Result<(i32, i32), RosterRefusal> {
    let model = monster_model(kind)?;
    Ok((
        tier(model.min_initial_hp.expect("checked above"), ascension) as i32,
        tier(model.max_initial_hp.expect("checked above"), ascension) as i32,
    ))
}

/// The single native `MaxInitialHp` of a fixed-HP kind at `ascension`, or
/// `None` when the kind's row is unread or its band at that tier is a range.
///
/// The engine's roster validators pin a fixed-HP owner's `max_hp` with this,
/// so the pin follows the fight's tier instead of hard-coding the A8+ value.
pub fn fixed_hp(kind: MonsterKind, ascension: u8) -> Option<i32> {
    match hp_band(kind, ascension) {
        Ok((lo, hi)) if lo == hi => Some(lo),
        _ => None,
    }
}

/// Whether `max_hp` is in `kind`'s native band at `ascension`. False for a
/// kind whose row is unread.
pub fn hp_in_band(kind: MonsterKind, ascension: u8, max_hp: i32) -> bool {
    hp_band(kind, ascension).is_ok_and(|(lo, hi)| (lo..=hi).contains(&max_hp))
}

/// The amount of one spawn-time `Apply<XPower>`, at `ascension` ([`tier`]).
pub fn initial_power(
    kind: MonsterKind,
    power: &'static str,
    ascension: u8,
) -> Result<i64, RosterRefusal> {
    let model = monster_model(kind)?;
    model
        .initial_powers
        .iter()
        .find(|(name, _)| *name == power)
        .map(|(_, value)| tier(*value, ascension))
        .ok_or(RosterRefusal::MonsterPowerAbsent { kind, power })
}

/// Where `move_name` sits in `kind`'s deterministic rotation.
///
/// The oracle's builders carry these as bare indices with the move name in a
/// comment (`for i, start in enumerate((2, 1)):  # SPIKEN, WHIRL`). The index
/// is a number, so it may not be typed into a Rust file (PORT_PLAN §5); the
/// name is not, and `content_tables::LOOPS` is generated, so looking the index
/// up by name reads the same fact out of codegen instead.
pub fn loop_position(kind: MonsterKind, move_name: &'static str) -> Result<i32, RosterRefusal> {
    let rotation = crate::content_tables::monster_loop(kind)
        .ok_or(RosterRefusal::MonsterLoopAbsent { kind })?;
    rotation
        .iter()
        .position(|entry| entry.name == move_name)
        .map(|index| index as i32)
        .ok_or(RosterRefusal::MonsterLoopMoveAbsent { kind, move_name })
}

/// How many `MoveState`s `kind`'s rotation has.
///
/// The length is a number the native bodies use as a modulus (`idx % 3`), as a
/// draw bound (`Rng::NextInt(3)`) and as a count of distinct starting offsets,
/// so reading it from the generated rotation is what keeps it out of the Rust
/// source (PORT_PLAN §5) wherever a builder staggers same-kind monsters around
/// one loop.
pub fn rotation_len(kind: MonsterKind) -> Result<i32, RosterRefusal> {
    Ok(crate::content_tables::monster_loop(kind)
        .ok_or(RosterRefusal::MonsterLoopAbsent { kind })?
        .len() as i32)
}

/// `_EncounterCtx.fixed`: a monster whose native band is a single value.
///
/// Spends the one `Niche` draw `Creature::SetUniqueMonsterHpValue` spends for
/// a single-value set, then builds the spec. A kind whose band is a real
/// range refuses rather than silently pinning an endpoint.
pub fn fixed(ctx: &mut dyn EncounterCtx, kind: MonsterKind) -> Result<MonsterSpec, RosterRefusal> {
    let (lo, hi) = hp_band(kind, ctx.ascension())?;
    if lo != hi {
        return Err(RosterRefusal::MonsterHpNotFixed { kind });
    }
    ctx.niche_next_int(0, 1);
    Ok(MonsterSpec::new(kind, lo))
}

#[cfg(test)]
pub(crate) mod oracle;

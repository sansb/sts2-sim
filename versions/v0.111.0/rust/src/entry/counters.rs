//! The RNG streams the fight is entered with.
//!
//! Oracle: `live_coach.stream_counters`, and the nine keyword arguments
//! `mcr_replay.start_from_save` derives from it.
//!
//! # Why there is no seeding scheme here
//!
//! A schema >= 19 save records each stream's **counter and its four xoshiro
//! words**, so the builder reads state rather than deriving it. That the
//! derivation would agree is a falsifiable claim, and it was falsified in the
//! only direction that matters — it held:
//! `live_coach.verify_stream_seeding(save, "v0.111.0")` derives each stream as
//! `RunRngSet(seed).rngs[Name]` fast-forwarded by the recorded counter and
//! compares the four words with the save's. Measured 2026-09-16 over the local
//! corpus: **3,382 save files scanned, 3,092 at schema 20, zero saves with a
//! disagreeing stream** (`fixtures/entry_builder_v1.json`,
//! `seeding_falsification`).
//!
//! The consequence is structural, not a convenience. `rng::stream_seed` has no
//! `v0.111.0` arm and zero callers in this crate; it stays uncalled, the
//! derivation is a cross-check rather than a dependency, and the next build's
//! fork inherits no seeding scheme to re-verify for these nine streams. It
//! also retires I10 for this path: the Shuffle-counter *prediction* existed
//! because a `.run` gave counters without state.
//!
//! Native authority: `Entities.Rngs.RunRngType` (12 members) and
//! `Saves.SerializableRng` (`counter`, `state0..state3`), v0.111.0 `sts2.dll`
//! sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.

use std::collections::BTreeMap;

use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{RUN_RNG_STREAMS, SerializedRng, SerializedRun};

/// The nine run streams that are combat state, paired with the canonical v2
/// `rng` key each projects to.
///
/// One-to-one with `project_state.RNG_FIELDS` and `hot::RngStream`. The other
/// three run streams (`up_front`, `unknown_map_point`, `treasure_room_relics`)
/// and all three player streams (`rewards`, `shops`, `transformations`) are
/// run state, not combat state, and are not projected.
pub const COMBAT_STREAMS: [(&str, &str); 9] = [
    ("shuffle", "rng"),
    ("niche", "niche"),
    ("monster_ai", "ai"),
    ("combat_card_selection", "sel"),
    ("combat_card_generation", "generation"),
    ("combat_targets", "targets"),
    ("combat_energy_costs", "energy_costs"),
    ("combat_orbs", "combat_orbs"),
    ("combat_potion_generation", "potion_generation"),
];

/// The six the oracle indexes directly (`ctr["shuffle"]`, …), so their absence
/// is a hard error there and a named refusal here.
pub const REQUIRED_COMBAT_STREAMS: [&str; 6] = [
    "shuffle",
    "niche",
    "monster_ai",
    "combat_card_selection",
    "combat_targets",
    "combat_energy_costs",
];

/// The three the oracle reads with `.get`, handing `start_combat` `None` when
/// the save omits them.
///
/// Absence is a *fact about the save*, not a malformed save, so it is carried
/// through rather than refused: `start_combat`'s own exactness gates then
/// refuse any fight that would actually consume the missing stream. Every one
/// of the 3,092 schema-20 corpus saves records all twelve; the split exists
/// because the checked-in synthetic fixture does not, and because inventing a
/// refusal the oracle does not have would break entry parity.
pub const OPTIONAL_COMBAT_STREAMS: [&str; 3] = [
    "combat_card_generation",
    "combat_potion_generation",
    "combat_orbs",
];

/// Every recorded run stream, by the save's own key.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunStreams {
    pub seed: String,
    pub streams: BTreeMap<String, SerializedRng>,
}

impl RunStreams {
    /// The entering counter of one combat stream.
    pub fn counter(&self, stream: &'static str) -> Result<u64, EntryRefusal> {
        self.streams
            .get(stream)
            .map(|state| state.counter)
            .ok_or(EntryRefusal::MissingRngStream(stream))
    }

    /// Every recorded counter, which is what `stream_counters` returns — all
    /// twelve run streams, not only the nine combat ones.
    pub fn counters(&self) -> BTreeMap<String, u64> {
        self.streams
            .iter()
            .map(|(name, state)| (name.clone(), state.counter))
            .collect()
    }
}

/// Read the run's streams, refusing an untaught stream name and a missing
/// combat stream by name.
///
/// The unknown-name refusal is a deliberate tightening over the oracle, which
/// carries an unrecognised key along harmlessly: an unmodeled stream is an
/// unmodeled fact, and a new `RunRngType` member is exactly the build-bump
/// event I11 wants visible. No corpus save exercises it — all 3,092 carry
/// exactly the twelve IL members.
pub fn run_streams(run: &SerializedRun) -> Result<RunStreams, EntryRefusal> {
    let rng = run.run_rng()?;
    // `stream_counters` refuses an rng block with neither `counters`
    // (schema <= 18) nor `rngs` (schema >= 19). Schema 20 is the `rngs` form,
    // and this parser admits no other.
    let rngs = rng.rngs.as_ref().ok_or(EntryRefusal::MissingSaveField {
        owner: "run.rng",
        field: "rngs",
    })?;
    for name in rngs.keys() {
        if !RUN_RNG_STREAMS.contains(&name.as_str()) {
            return Err(EntryRefusal::UnknownRngStream(name.clone()));
        }
    }
    for stream in REQUIRED_COMBAT_STREAMS {
        if !rngs.contains_key(stream) {
            return Err(EntryRefusal::MissingRngStream(stream));
        }
    }
    Ok(RunStreams {
        seed: rng.seed.clone(),
        streams: rngs.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save_with(streams: &str) -> String {
        format!(
            r#"{{"schema_version": 20, "rng": {{"seed": "ZPJHU3WSH2", "rngs": {{{streams}}}}},
                 "players": []}}"#
        )
    }

    fn all_twelve() -> String {
        let bodies: Vec<String> = RUN_RNG_STREAMS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                format!(r#""{name}": {{"counter": {index}, "s0": 1, "s1": 2, "s2": 3, "s3": 4}}"#)
            })
            .collect();
        save_with(&bodies.join(", "))
    }

    #[test]
    fn every_recorded_counter_is_returned_not_only_the_nine() {
        let run = SerializedRun::parse(&all_twelve()).unwrap();
        let streams = run_streams(&run).unwrap();
        assert_eq!(streams.counters().len(), 12);
        assert_eq!(streams.seed, "ZPJHU3WSH2");
        assert_eq!(streams.counter("shuffle").unwrap(), 1);
    }

    #[test]
    fn an_rng_block_without_an_rngs_key_refuses_by_name() {
        // Key presence is the schema >= 19 proxy the oracle vouches for
        // dispatch order on, so absence must be distinguishable from empty.
        let run =
            SerializedRun::parse(r#"{"schema_version": 20, "rng": {"seed": "S"}, "players": []}"#)
                .unwrap();
        assert_eq!(
            run_streams(&run),
            Err(EntryRefusal::MissingSaveField {
                owner: "run.rng",
                field: "rngs",
            })
        );
    }

    #[test]
    fn a_missing_combat_stream_refuses_by_stream_name() {
        let text = all_twelve().replace(r#""niche""#, r#""niche_typo""#);
        let run = SerializedRun::parse(&text).unwrap();
        // The untaught name is caught before the missing one.
        assert_eq!(
            run_streams(&run),
            Err(EntryRefusal::UnknownRngStream("niche_typo".to_string()))
        );
    }

    fn without(stream: &str) -> SerializedRun {
        let bodies: Vec<String> = RUN_RNG_STREAMS
            .iter()
            .filter(|name| **name != stream)
            .map(|name| {
                format!(r#""{name}": {{"counter": 0, "s0": 1, "s1": 2, "s2": 3, "s3": 4}}"#)
            })
            .collect();
        SerializedRun::parse(&save_with(&bodies.join(", "))).unwrap()
    }

    #[test]
    fn dropping_a_required_combat_stream_refuses_by_name() {
        assert_eq!(
            run_streams(&without("niche")),
            Err(EntryRefusal::MissingRngStream("niche"))
        );
    }

    #[test]
    fn dropping_an_optional_combat_stream_is_a_fact_not_a_refusal() {
        // The oracle reads these three with `.get` and passes `None` on, so a
        // refusal here would be a divergence, not a tightening. The checked-in
        // synthetic fixture save omits `combat_orbs`.
        for stream in OPTIONAL_COMBAT_STREAMS {
            let streams = run_streams(&without(stream)).unwrap();
            assert!(!streams.counters().contains_key(stream));
            assert_eq!(
                streams.counter(stream),
                Err(EntryRefusal::MissingRngStream(stream))
            );
        }
    }

    #[test]
    fn the_nine_combat_streams_are_distinct_and_all_run_streams() {
        let mut save_names: Vec<&str> = COMBAT_STREAMS.iter().map(|(name, _)| *name).collect();
        let mut canonical: Vec<&str> = COMBAT_STREAMS.iter().map(|(_, key)| *key).collect();
        save_names.sort_unstable();
        canonical.sort_unstable();
        let total = save_names.len();
        save_names.dedup();
        canonical.dedup();
        assert_eq!(save_names.len(), total);
        assert_eq!(canonical.len(), total);
        for (name, _) in COMBAT_STREAMS {
            assert!(
                RUN_RNG_STREAMS.contains(&name),
                "{name} is not a RunRngType"
            );
        }
    }

    #[test]
    fn required_and_optional_partition_the_nine() {
        let mut split: Vec<&str> = REQUIRED_COMBAT_STREAMS
            .iter()
            .chain(OPTIONAL_COMBAT_STREAMS.iter())
            .copied()
            .collect();
        let mut all: Vec<&str> = COMBAT_STREAMS.iter().map(|(name, _)| *name).collect();
        split.sort_unstable();
        all.sort_unstable();
        assert_eq!(split, all);
    }
}

//! The `--mcr` opening-checksum RNG splice, as a typed, opt-in step.
//!
//! # What the splice is
//!
//! `mcr_validate.checksum_opening_state` (`mcr_validate.py:512-536`) takes the
//! capture's **first** checksum, requires its context to be exactly
//! `"After player turn start"`, and then overwrites **all nine** RNG
//! attributes of the already-opened combat from `checksums[0].full_state.rng`.
//!
//! # Three obligations, from the E4 spec walk §A5
//!
//! 1. **Opt-in and typed.** Before #2528 the CLI refused `--mcr` outright with
//!    `mcr_splice_without_opening`, because there was no opening for it to
//!    overwrite. E4a turns that refusal into a step, not into a default: a
//!    caller that does not ask for it gets an unspliced document.
//! 2. **The document must say it happened.** A spliced document and an
//!    unspliced one are different documents; conflating them would let an
//!    opening-consumption defect pass digest parity.
//! 3. **It must not become the regression net.** The splice masks RNG-counter
//!    drift in the opening while leaving pile and monster drift visible — a
//!    simulator that consumed the wrong number of `Shuffle` draws would still
//!    show a wrong hand, but its counters would look right. The net is the
//!    **pre-splice** measurement (#2527): opening counters and all four
//!    xoshiro words match the capture on 318/318 checksummed fights.
//!
//! The `.mcr` decoder is still not ported. This module takes the first
//! checksum's `rng` block as already-decoded JSON, which is all the splice
//! needs.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::canonical::CanonicalRngV2;
use crate::entry::counters::COMBAT_STREAMS;
use crate::entry::opening::refusal::OpeningRefusal;

/// The context string `checksum_opening_state` requires of the first checksum.
pub const OPENING_CHECKSUM_CONTEXT: &str = "After player turn start";

/// The nine streams of a capture's opening checksum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McrOpeningChecksum {
    pub streams: BTreeMap<String, CanonicalRngV2>,
}

impl McrOpeningChecksum {
    /// Read the block out of a decoded `.mcr` payload.
    ///
    /// `payload` is `checksums[0]`: an object carrying `context` and
    /// `full_state.rng`, where `rng` is keyed by the **canonical** stream names
    /// (`rng`, `niche`, `ai`, …) — the same keys `project_state.RNG_FIELDS`
    /// uses, because the checksum's `full_state` is a projected state.
    ///
    /// All nine must be present: the splice overwrites all nine
    /// unconditionally, so a partial block would leave a mix of spliced and
    /// computed streams that no measurement could interpret.
    pub fn from_first_checksum(payload: &Value) -> Result<Self, OpeningRefusal> {
        let context = payload
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if context != OPENING_CHECKSUM_CONTEXT {
            return Err(OpeningRefusal::McrChecksumContext {
                context: context.to_string(),
            });
        }
        let rng = payload.get("full_state").and_then(|state| state.get("rng"));
        let mut streams = BTreeMap::new();
        for (_save_name, canonical) in COMBAT_STREAMS {
            let block = rng
                .and_then(|rng| rng.get(canonical))
                .ok_or(OpeningRefusal::McrChecksumIncomplete { stream: canonical })?;
            let parsed: CanonicalRngV2 = serde_json::from_value(block.clone())
                .map_err(|_| OpeningRefusal::McrChecksumIncomplete { stream: canonical })?;
            streams.insert(canonical.to_string(), parsed);
        }
        Ok(Self { streams })
    }

    /// Overwrite every one of the document's nine streams.
    ///
    /// Unconditional, exactly as the oracle is: it does not compare first and
    /// does not skip a stream whose value already agrees.
    pub fn splice_into(&self, rng: &mut BTreeMap<String, CanonicalRngV2>) {
        for (name, state) in &self.streams {
            rng.insert(name.clone(), state.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn block() -> Value {
        let mut rng = serde_json::Map::new();
        for (index, (_save, canonical)) in COMBAT_STREAMS.iter().enumerate() {
            rng.insert(
                (*canonical).to_string(),
                json!({"counter": index, "words": [1, 2, 3, 4]}),
            );
        }
        json!({
            "context": OPENING_CHECKSUM_CONTEXT,
            "full_state": {"rng": Value::Object(rng)},
        })
    }

    #[test]
    fn the_first_checksum_must_be_the_opening_state() {
        let mut payload = block();
        payload["context"] = Value::from("Turn 2 start");
        let refusal = McrOpeningChecksum::from_first_checksum(&payload).unwrap_err();
        assert_eq!(refusal.class(), "mcr_checksum_context");
        assert!(refusal.to_string().contains("Turn 2 start"));
    }

    #[test]
    fn a_missing_stream_refuses_rather_than_splicing_eight_of_nine() {
        let mut payload = block();
        payload["full_state"]["rng"]
            .as_object_mut()
            .unwrap()
            .remove("sel");
        let refusal = McrOpeningChecksum::from_first_checksum(&payload).unwrap_err();
        assert_eq!(
            refusal,
            OpeningRefusal::McrChecksumIncomplete { stream: "sel" }
        );
    }

    #[test]
    fn the_splice_overwrites_every_stream_unconditionally() {
        let checksum = McrOpeningChecksum::from_first_checksum(&block()).unwrap();
        let mut rng = BTreeMap::new();
        rng.insert(
            "rng".to_string(),
            CanonicalRngV2 {
                counter: 99,
                words: [9, 9, 9, 9],
            },
        );
        checksum.splice_into(&mut rng);
        assert_eq!(rng.len(), COMBAT_STREAMS.len());
        assert_eq!(rng["rng"].words, [1, 2, 3, 4]);
    }
}

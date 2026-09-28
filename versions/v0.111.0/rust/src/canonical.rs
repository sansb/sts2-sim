//! The `sts-sim-canonical-v2` wire schema (PORT_PLAN §3 D3, §4).
//!
//! This is the deliberately slow, string-keyed representation both engines
//! meet at: the entry format the admission gate consumes, the document the
//! trajectory differential compares, and the shape
//! `tools/project_state.py` produces from a Python `combat_sim.State`. The
//! hot search-owned state (D3) is a separate type built from this at the
//! boundary; nothing here is on a per-node path.
//!
//! # Schema
//!
//! ```json
//! {
//!   "schema": "sts-sim-canonical-v2",
//!   "game_build": "v0.111.0",
//!   "player":   {"<State field>": <value>, ...},
//!   "monsters": [{"<Monster field>": <value>, ...}, ...],
//!   "piles":    {"hand": [<card>, ...], ...},
//!   "rng":      {"<stream>": {"words": [w0, w1, w2, w3], "counter": n}, ...},
//!   "continuations": [{"type": "<FrameName>", "fields": {...}}, ...],
//!   "refusal":  {"site": "...", "kind": "...", "detail": "..."}
//! }
//! ```
//!
//! Entity field bags are name→value maps rather than 517 struct fields: on
//! this side of the boundary that *is* the schema (D3 — the struct-per-power
//! shape is Python's canonical form, and the hot form is neither).
//!
//! # Zero-default elision
//!
//! Both sides omit a field whose value equals that field's dataclass default.
//! Python computes the defaults from `dataclasses`; the serde structs mirror
//! it with `skip_serializing_if`. Elision is part of the contract, not a
//! size optimization: a present key means "not at its default".
//!
//! # Digest agreement
//!
//! [`CanonicalStateV2::differential_digest`] is the sha256 of
//! [`CanonicalStateV2::canonical_json`], which serializes through
//! [`serde_json::Value`] so map keys are sorted (serde_json's `Map` is a
//! `BTreeMap` unless `preserve_order` is enabled) and separators are compact.
//! That is byte-for-byte what Python's
//! `json.dumps(doc, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
//! produces. `tests::digest_agrees_with_the_python_projection` pins the
//! agreement against a fixture produced by `project_state.py --emit`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// Wire schema identifier. Bumped, never reinterpreted.
pub const STATE_SCHEMA_V2: &str = "sts-sim-canonical-v2";

/// A `State`/`Monster` field bag: field name → projected value.
pub type CanonicalEntityV2 = BTreeMap<String, Value>;

/// One `(s0, s1, s2, s3, counter)` xoshiro stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRngV2 {
    pub counter: u64,
    pub words: [u64; 4],
}

/// One physical card instance.
///
/// `id`/`upgrade` are the payload head; the named slots are the documented
/// physical-card slots 2..7; `extra` keeps the opaque tagged `CardModel`-local
/// tail (afflictions, dupe tags, deck rows) positional rather than inventing
/// names for it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalCardV2 {
    pub id: String,
    pub upgrade: i64,
    /// Absent only for a legacy payload tuple without physical identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<u64>,
    /// A selection-action reference rather than a pile resident.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pick: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enchantment: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transient_keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enchantment_state: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sovereign_blade: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_state: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<Value>,
}

/// One resumable engine frame from the continuation stack.
///
/// `type` is the Python frame class name; `fields` is its per-field dict, both
/// derived mechanically from the `NamedTuple` rather than enumerated by hand.
/// The typed `Frame` enum (D3) lands with the hot state in R0.4; on the wire
/// the tag stays a string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalFrameV2 {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub fields: BTreeMap<String, Value>,
}

/// Typed refusal kinds carried on the wire (D6).
///
/// The port never approximates and never panics on a missing mechanic: it
/// names the kind. Message text is evidence, not identity — refusal parity is
/// matched on `site` + `kind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    /// The request line was not a JSON object.
    MalformedRequest,
    /// The request object carried no `cmd`.
    MissingCommand,
    /// `cmd` named something outside the protocol.
    UnknownCommand,
    /// A command was missing a required field.
    MissingField,
    /// The entry payload did not parse as a canonical document.
    MalformedEntry,
    /// The entry's `schema` is not the admitted one.
    UnsupportedSchema,
    /// A state-dependent command arrived with no loaded state.
    NoStateLoaded,
    /// The entry parsed and is well formed, but its content closure is
    /// outside the implemented slice (PORT_PLAN D6). The detail names every
    /// missing capability, not the first one.
    NotAdmitted,
    /// The entry parsed but cannot be represented hot: the canonical ⇄ hot
    /// boundary refused a field, a frame, or a value shape.
    UnrepresentableState,
    /// An `apply` payload did not decode as an action.
    MalformedAction,
    /// The action decoded but is not applicable in the loaded state.
    IllegalAction,
    /// The action reached a mechanic this engine does not implement.
    EngineNotImplemented,
    /// A fight root could not be built from its raw input (a save, or an
    /// uploaded entry projection) — the boundary #1282 D2 moved into Rust.
    ///
    /// Reached *before* a document exists, which is why the entry builder
    /// carries its own [`crate::entry::refusal::EntryRefusal`] vocabulary
    /// rather than widening `BoundaryRefusal` or the admission refusal. One
    /// kind, many variants: `site` + `kind` is what refusal parity matches on,
    /// and the variant's name travels beside the document as `refusal_class`.
    EntryNotBuildable,
}

impl RefusalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedRequest => "malformed_request",
            Self::MissingCommand => "missing_command",
            Self::UnknownCommand => "unknown_command",
            Self::MissingField => "missing_field",
            Self::MalformedEntry => "malformed_entry",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::NoStateLoaded => "no_state_loaded",
            Self::NotAdmitted => "not_admitted",
            Self::UnrepresentableState => "unrepresentable_state",
            Self::MalformedAction => "malformed_action",
            Self::IllegalAction => "illegal_action",
            Self::EngineNotImplemented => "engine_not_implemented",
            Self::EntryNotBuildable => "entry_not_buildable",
        }
    }
}

impl fmt::Display for RefusalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A refusal as it appears on the wire: site + kind + detail.
///
/// Placeholder shape for R0.3. When the engine lands, `site` becomes an
/// interned dispatch site and `detail` gains structured operands; the wire
/// keeps strings either way (this is the slow format).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    pub site: String,
    pub kind: RefusalKind,
    pub detail: String,
}

impl Refusal {
    pub fn new(site: &str, kind: RefusalKind, detail: impl Into<String>) -> Self {
        Self {
            site: site.to_string(),
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.site, self.kind, self.detail)
    }
}

/// A canonical combat state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalStateV2 {
    pub schema: String,
    /// Caller-supplied provenance: the build whose simulator produced this.
    /// `State` carries no build of its own, so this is optional rather than
    /// invented (parity is always within a build — `SIM_VERSIONING.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_build: Option<String>,
    pub player: CanonicalEntityV2,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monsters: Vec<CanonicalEntityV2>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub piles: BTreeMap<String, Vec<CanonicalCardV2>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rng: BTreeMap<String, CanonicalRngV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub continuations: Vec<CanonicalFrameV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
}

impl CanonicalStateV2 {
    pub fn validate_schema(&self) -> Result<(), Refusal> {
        if self.schema == STATE_SCHEMA_V2 {
            Ok(())
        } else {
            Err(Refusal::new(
                "canonical",
                RefusalKind::UnsupportedSchema,
                format!(
                    "unsupported canonical state schema {:?}; expected {STATE_SCHEMA_V2}",
                    self.schema
                ),
            ))
        }
    }

    /// The exact wire bytes: sorted keys, compact separators, UTF-8.
    ///
    /// Routed through [`serde_json::Value`] so key order comes from
    /// `serde_json::Map`'s `BTreeMap` rather than from struct declaration
    /// order — the digest cannot drift when a field is added or moved.
    pub fn canonical_json(&self) -> String {
        let value = serde_json::to_value(self).expect("canonical state serializes");
        serde_json::to_string(&value).expect("canonical value serializes")
    }

    /// Stable comparison digest, byte-identical to the Python projection's.
    pub fn differential_digest(&self) -> String {
        format!("{:x}", Sha256::digest(self.canonical_json().as_bytes()))
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Produced by `python3 tools/project_state.py --emit` against the
    /// v0.111.0 solver (Ironclad starter deck vs `TOADPOLES_WEAK`).
    const FIXTURE: &str = include_str!("../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    /// `python3 tools/project_state.py --self-check` reported this sha256
    /// (the CLI was deleted by #2999; the fixture is frozen data pinned by
    /// `tools/frozen_oracle_data.py`). Refresh both together, deliberately, or
    /// not at all.
    const FIXTURE_DIGEST: &str = "97be893cf44c5451b13de2c2468096712b724f9ea67484b0131ea043321ed83c";

    fn fixture() -> CanonicalStateV2 {
        serde_json::from_str(FIXTURE).expect("fixture parses as canonical v2")
    }

    #[test]
    fn digest_agrees_with_the_python_projection() {
        let state = fixture();
        state.validate_schema().unwrap();
        // Byte agreement, not just digest agreement: if these ever diverge
        // the digest equality below would be coincidence.
        assert_eq!(state.canonical_json(), FIXTURE.trim_end());
        assert_eq!(state.differential_digest(), FIXTURE_DIGEST);
    }

    #[test]
    fn the_fixture_round_trips_through_the_typed_schema() {
        let state = fixture();
        let reparsed: CanonicalStateV2 = serde_json::from_str(&state.canonical_json()).unwrap();
        assert_eq!(state, reparsed);
        assert_eq!(state.differential_digest(), reparsed.differential_digest());
    }

    #[test]
    fn the_fixture_carries_the_structural_sections() {
        let state = fixture();
        assert_eq!(state.game_build.as_deref(), Some("v0.111.0"));
        assert_eq!(state.monsters.len(), 2);
        assert_eq!(state.monsters[0]["kind"], "TOADPOLE");
        assert_eq!(state.piles["hand"].len(), 5);
        assert_eq!(state.piles["hand"][0].id, "STRIKE_IRONCLAD");
        assert_eq!(state.piles["hand"][0].uid, Some(0));
        // An empty pile whose Python field has no dataclass default is still
        // emitted; elision is by default-equality, not by emptiness.
        assert!(state.piles["discard"].is_empty());
        assert_eq!(state.rng["rng"].counter, 9);
        assert_eq!(state.rng["rng"].words[0], 14_118_584_688_975_139_523);
        assert!(state.continuations.is_empty());
        assert!(state.refusal.is_none());
    }

    #[test]
    fn the_digest_is_independent_of_construction_order() {
        let state = fixture();
        let mut shuffled = state.clone();
        let monsters = std::mem::take(&mut shuffled.monsters);
        shuffled.monsters = monsters;
        let piles = std::mem::take(&mut shuffled.piles);
        shuffled.piles = piles.into_iter().rev().collect();
        assert_eq!(state.differential_digest(), shuffled.differential_digest());
    }

    #[test]
    fn a_changed_field_changes_the_digest() {
        let state = fixture();
        let mut mutated = state.clone();
        mutated
            .player
            .insert("hp".to_string(), Value::from(67))
            .expect("hp is projected in the fixture");
        assert_ne!(state.differential_digest(), mutated.differential_digest());
    }

    #[test]
    fn an_unknown_schema_refuses() {
        let mut state = fixture();
        state.schema = "sts-kernel-state/v1".to_string();
        let refusal = state.validate_schema().unwrap_err();
        assert_eq!(refusal.kind, RefusalKind::UnsupportedSchema);
        assert_eq!(refusal.site, "canonical");
    }

    #[test]
    fn an_unknown_document_field_refuses_rather_than_being_dropped() {
        let injected = FIXTURE.trim_end().replacen('{', "{\"surprise\":1,", 1);
        let error = serde_json::from_str::<CanonicalStateV2>(&injected).unwrap_err();
        assert!(
            error.to_string().contains("surprise"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn refusal_kinds_serialize_as_their_wire_strings() {
        for kind in [
            RefusalKind::MalformedRequest,
            RefusalKind::MissingCommand,
            RefusalKind::UnknownCommand,
            RefusalKind::MissingField,
            RefusalKind::MalformedEntry,
            RefusalKind::UnsupportedSchema,
            RefusalKind::NoStateLoaded,
            RefusalKind::NotAdmitted,
            RefusalKind::UnrepresentableState,
            RefusalKind::MalformedAction,
            RefusalKind::IllegalAction,
            RefusalKind::EngineNotImplemented,
        ] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                Value::from(kind.as_str())
            );
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }

    #[test]
    fn a_refusal_round_trips_on_the_wire() {
        let refusal = Refusal::new(
            "load",
            RefusalKind::EngineNotImplemented,
            "no engine at R0.3",
        );
        let encoded = serde_json::to_string(&refusal).unwrap();
        assert_eq!(
            encoded,
            r#"{"site":"load","kind":"engine_not_implemented","detail":"no engine at R0.3"}"#
        );
        assert_eq!(serde_json::from_str::<Refusal>(&encoded).unwrap(), refusal);
    }
}

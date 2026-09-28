//! The typed I5 refusal vocabulary of the entry builder (#2511, #1282 D2).
//!
//! A third enum, sibling to [`crate::boundary::BoundaryRefusal`] and the
//! engine's admission refusal rather than a widening of either: both of those
//! are reached *after* a canonical document exists, and this one is reached
//! before there is a document at all.
//!
//! Two rules travel with it, from the invariant walk
//! `solver/invariant-walks/2026-09-16-rust-entry-builder.md` (I5):
//!
//! * **No catch-all.** Every variant names the mechanic or the field it could
//!   not derive, following `BoundaryRefusal`'s precedent. There is deliberately
//!   no `Other(String)`.
//! * **Rust does not inherit Python's refusals, and does not inherit its
//!   admissions either.** Each fact is re-derived from the save or refused by
//!   a Rust name.
//!
//! On the wire every variant surfaces as the single
//! [`crate::canonical::RefusalKind::EntryNotBuildable`] with `site: "entry"`;
//! the variant's `Display` form is the `detail`. `RefusalKind` is the only
//! wire-stable refusal vocabulary and parity matches on `site` + `kind`, so
//! adding one kind rather than one kind per variant is deliberate.

use std::fmt;

/// Why a fight root could not be built from its input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryRefusal {
    /// The input was not JSON at all.
    MalformedSaveJson(String),
    /// A key outside the schema-20 field surface the builder was taught, or a
    /// value of the wrong JSON type for a key that is.
    ///
    /// This is the structural half of I8's "a field the save records that the
    /// builder does not read is a silent violation": `entry/save.rs` parses
    /// with `deny_unknown_fields`, so an untaught key is this refusal rather
    /// than a dropped fact.
    UnknownSaveField(String),
    /// `schema_version` is outside the admitted set (schema 20 only).
    UnsupportedSaveSchema(u32),
    /// `--build` named a build outside `versions/admitted_builds.json`.
    ///
    /// I11: the builder takes the build explicitly and never defaults, so a
    /// fifth call site cannot reopen the implicit-build hole #1265 closed.
    UnadmittedGameBuild(String),
    /// `players` does not hold exactly one player.
    MultiplayerSave(usize),
    /// A field the builder reads is absent from the save.
    MissingSaveField {
        owner: &'static str,
        field: &'static str,
    },
    /// `visited_map_coords` is empty, so there is no current node.
    EmptyVisitedMapCoords,
    /// `current_act_index >= 1` with no `map_point_history`: the global node
    /// index cannot be derived, and the per-act index is not a substitute
    /// (the #826 act-2 reseeding drift).
    MissingMapPointHistory,
    /// `current_act_index` is outside `acts`.
    ActIndexOutOfRange { index: usize, acts: usize },
    /// The saved act map spells a point type outside `MapPointType`.
    UnknownNodeType(String),
    /// The current coordinate is not a point on the current act's saved map,
    /// so the node's kind is underivable.
    NodeNotOnSavedMap { col: i64, row: i64 },
    /// The node kind carries no encounter in the act's room lists and no
    /// `--encounter` was supplied.
    NoEncounterForNode { node_type: String },
    /// The act's visited counter points past the end of its encounter list.
    EncounterIndexOutOfRange {
        list: &'static str,
        index: usize,
        len: usize,
    },
    /// A model entry id is empty, or carries the wrong category prefix.
    MalformedModelId {
        category: &'static str,
        value: String,
    },
    /// A `SavedProperties` group name outside the seven the IL declares.
    SavedPropertyGroupUnhandled { owner: String, group: String },
    /// A `SavedProperties` row that is not exactly `{name, value}`, or whose
    /// value is not the group's type.
    SavedPropertyRowShape {
        owner: String,
        group: String,
        detail: String,
    },
    /// A `rng.rngs` key outside `RunRngType`.
    UnknownRngStream(String),
    /// One of the nine projected combat streams is absent from `rng.rngs`.
    MissingRngStream(&'static str),
    /// A review-provenance entry projection does not carry a fact the entry
    /// needs. A *different* input (a full save) can answer it, so this is I13
    /// "not yet", not "never".
    ProvenanceEntryIncomplete { fact: &'static str },
    /// A review-provenance payload whose `save_schema_version` is outside the
    /// admitted set.
    UnsupportedProvenanceSchema(u32),
    /// A replay capture's decoded run (`--capture-run`) carries a key outside
    /// the taught decoder surface, lacks one, or holds a value of the wrong
    /// JSON type (`entry/capture.rs`, `deny_unknown_fields`).
    UnknownCaptureField(String),
    /// A capture field the mapping does not carry onto the save surface,
    /// because no capture has exercised it.
    CaptureFieldUnmapped { field: &'static str },
    /// A decoded `SavedProperties` name outside the IL's `[SavedProperty]`
    /// set, so its group cannot be recovered.
    CapturePropertyUnknown { owner: String, name: String },
    /// A decoded `SavedProperties` value whose JSON type contradicts the
    /// property's declared type.
    CapturePropertyType {
        owner: String,
        name: String,
        group: &'static str,
    },
    /// A decoded enum member name outside the enum the IL declares.
    CaptureEnumUnknown {
        enumeration: &'static str,
        value: String,
    },
    /// A capture stream with a counter but no state words, or the reverse.
    CaptureRngStateMissing { stream: String },
    /// An entry-facts document (`--facts`, `entry/facts.rs`) is not JSON.
    MalformedFactsJson(String),
    /// An entry-facts document carries a key outside the `sts-sim-entry-v1`
    /// surface, lacks a required one, or holds a value of the wrong JSON type.
    UnknownFactsField(String),
    /// An entry-facts document names a schema other than `sts-sim-entry-v1`.
    UnsupportedFactsSchema(String),
    /// An entry-facts document's `game_build` is not the `--build` requested.
    FactsBuildMismatch { facts: String, requested: String },
    /// Two entry facts contradict each other, or a fact holds a value the save
    /// path can never produce (a non-canonical id, `props` on a card that
    /// carries none, a `relic_counters` key for an unowned relic, ...).
    FactsInconsistent { field: &'static str, detail: String },
    /// A stream's recorded words are not the state `RunRngSet` derives from the
    /// seed at the recorded counter.
    FactsStreamDisagreesWithSeed { stream: String },
    /// A stream's counter is beyond the bound the derivation check walks.
    FactsStreamCounterUnverifiable { stream: String, counter: u64 },
    /// A relic's saved `IsMelted`/`IsWax` flags are not a state native can
    /// reach: `Player.MeltRelicInternal` (RVA 0x11737c, IL_000d) throws unless
    /// the relic is wax, so `IsMelted: true` without `IsWax: true` (or a
    /// repeated or non-boolean flag) is unprovable.
    MeltedRelicFlags { relic: String, detail: &'static str },
    /// A melted relic that native still reads through `Player.GetRelic<T>` or
    /// `Player.Relics`, outside the `IsMelted`-filtered hook walk. Dropping it
    /// from the inventory would be wrong and keeping it would dispatch its
    /// hooks, so the entry refuses it by name.
    MeltedRelicStillRead { relic: String, reader: &'static str },
    /// An event-started combat whose option chain has a pre-combat effect the
    /// entry does not model, or whose encounter no read chain covers
    /// (`entry/event_combat.rs`, #3248). The save predates the event, so the
    /// root would be the pre-event player.
    EventCombatPathNotModeled {
        encounter: String,
        event: &'static str,
        reason: &'static str,
    },
    /// An event-started combat where the player holds a listener of a hook the
    /// event's room entry or option chain fires before the fight (#3248).
    EventCombatListenerNotModeled {
        encounter: String,
        listener: String,
        hook: &'static str,
    },
    /// An event-started combat in a run with modifiers, which are hook
    /// listeners the event path does not model (#3248).
    EventCombatRunModifiers {
        encounter: String,
        modifiers: Vec<String>,
    },
    /// An entry-facts document for an event-started combat: the facts cannot
    /// say whether they are the pre-event snapshot or the state after the
    /// event's option (#3248).
    EventCombatFactsUnvouched { encounter: String },
}

impl EntryRefusal {
    /// A short, stable class name for census tallies.
    ///
    /// The census groups by this; `Display` carries the evidence. Keeping the
    /// two separate is what lets a reworded refusal be told apart from a new
    /// one (the `eval_suite` refusal-class precedent).
    pub fn class(&self) -> &'static str {
        match self {
            Self::MalformedSaveJson(_) => "malformed_save_json",
            Self::UnknownSaveField(_) => "unknown_save_field",
            Self::UnsupportedSaveSchema(_) => "unsupported_save_schema",
            Self::UnadmittedGameBuild(_) => "unadmitted_game_build",
            Self::MultiplayerSave(_) => "multiplayer_save",
            Self::MissingSaveField { .. } => "missing_save_field",
            Self::EmptyVisitedMapCoords => "empty_visited_map_coords",
            Self::MissingMapPointHistory => "missing_map_point_history",
            Self::ActIndexOutOfRange { .. } => "act_index_out_of_range",
            Self::UnknownNodeType(_) => "unknown_node_type",
            Self::NodeNotOnSavedMap { .. } => "node_not_on_saved_map",
            Self::NoEncounterForNode { .. } => "no_encounter_for_node",
            Self::EncounterIndexOutOfRange { .. } => "encounter_index_out_of_range",
            Self::MalformedModelId { .. } => "malformed_model_id",
            Self::SavedPropertyGroupUnhandled { .. } => "saved_property_group_unhandled",
            Self::SavedPropertyRowShape { .. } => "saved_property_row_shape",
            Self::UnknownRngStream(_) => "unknown_rng_stream",
            Self::MissingRngStream(_) => "missing_rng_stream",
            Self::ProvenanceEntryIncomplete { .. } => "provenance_entry_incomplete",
            Self::UnsupportedProvenanceSchema(_) => "unsupported_provenance_schema",
            Self::UnknownCaptureField(_) => "unknown_capture_field",
            Self::CaptureFieldUnmapped { .. } => "capture_field_unmapped",
            Self::CapturePropertyUnknown { .. } => "capture_property_unknown",
            Self::CapturePropertyType { .. } => "capture_property_type",
            Self::CaptureEnumUnknown { .. } => "capture_enum_unknown",
            Self::CaptureRngStateMissing { .. } => "capture_rng_state_missing",
            Self::MalformedFactsJson(_) => "malformed_facts_json",
            Self::UnknownFactsField(_) => "unknown_facts_field",
            Self::UnsupportedFactsSchema(_) => "unsupported_facts_schema",
            Self::FactsBuildMismatch { .. } => "facts_build_mismatch",
            Self::FactsInconsistent { .. } => "facts_inconsistent",
            Self::FactsStreamDisagreesWithSeed { .. } => "facts_stream_disagrees_with_seed",
            Self::FactsStreamCounterUnverifiable { .. } => "facts_stream_counter_unverifiable",
            Self::MeltedRelicFlags { .. } => "melted_relic_flags",
            Self::MeltedRelicStillRead { .. } => "melted_relic_still_read",
            Self::EventCombatPathNotModeled { .. } => "event_combat_path_not_modeled",
            Self::EventCombatListenerNotModeled { .. } => "event_combat_listener_not_modeled",
            Self::EventCombatRunModifiers { .. } => "event_combat_run_modifiers",
            Self::EventCombatFactsUnvouched { .. } => "event_combat_facts_unvouched",
        }
    }
}

impl fmt::Display for EntryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedSaveJson(detail) => write!(f, "save is not JSON: {detail}"),
            Self::UnknownSaveField(detail) => write!(
                f,
                "save carries a field outside the taught schema-20 surface: {detail}"
            ),
            Self::UnsupportedSaveSchema(version) => write!(
                f,
                "save schema_version {version} is outside the admitted set {{20}}"
            ),
            Self::UnadmittedGameBuild(build) => {
                write!(f, "unadmitted game build {build:?}")
            }
            Self::MultiplayerSave(count) => write!(
                f,
                "save contains {count} players; the entry builder supports exactly one \
                 and refuses multiplayer rather than silently selecting players[0]"
            ),
            Self::MissingSaveField { owner, field } => {
                write!(
                    f,
                    "{owner}: required field {field:?} is absent from the save"
                )
            }
            Self::EmptyVisitedMapCoords => {
                f.write_str("visited_map_coords is empty, so the save has no current node")
            }
            Self::MissingMapPointHistory => f.write_str(
                "current_act_index >= 1 with no map_point_history: the GLOBAL node index \
                 is underivable and the per-act index is not a substitute",
            ),
            Self::ActIndexOutOfRange { index, acts } => {
                write!(f, "current_act_index {index} is outside acts (len {acts})")
            }
            Self::UnknownNodeType(value) => {
                write!(f, "saved map point type {value:?} is outside MapPointType")
            }
            Self::NodeNotOnSavedMap { col, row } => write!(
                f,
                "current coordinate (col {col}, row {row}) is not a point on the \
                 current act's saved map, so the node kind is underivable"
            ),
            Self::NoEncounterForNode { node_type } => write!(
                f,
                "node type {node_type:?} carries no encounter in the act's room lists \
                 and none was supplied"
            ),
            Self::EncounterIndexOutOfRange { list, index, len } => {
                write!(f, "{list}[{index}] is out of range (len {len})")
            }
            Self::MalformedModelId { category, value } => {
                write!(f, "invalid {category} model-entry id {value:?}")
            }
            Self::SavedPropertyGroupUnhandled { owner, group } => write!(
                f,
                "{owner}: SavedProperties group {group:?} is outside the seven the IL \
                 declares (ints, bools, strings, int_arrays, model_ids, cards, card_arrays)"
            ),
            Self::SavedPropertyRowShape {
                owner,
                group,
                detail,
            } => write!(f, "{owner}: SavedProperties {group} row: {detail}"),
            Self::UnknownRngStream(name) => {
                write!(f, "rng.rngs key {name:?} is outside RunRngType")
            }
            Self::MissingRngStream(name) => {
                write!(f, "rng.rngs is missing the combat stream {name:?}")
            }
            Self::ProvenanceEntryIncomplete { fact } => write!(
                f,
                "review-provenance entry projection does not carry {fact:?}; a full \
                 save answers it"
            ),
            Self::UnsupportedProvenanceSchema(version) => write!(
                f,
                "review-provenance save_schema_version {version} is outside the \
                 admitted set {{20}}"
            ),
            Self::UnknownCaptureField(detail) => write!(
                f,
                "capture run carries a field outside the taught decoder surface: {detail}"
            ),
            Self::CaptureFieldUnmapped { field } => write!(
                f,
                "capture run field {field:?} is present, and no capture has exercised \
                 its mapping onto the save surface"
            ),
            Self::CapturePropertyUnknown { owner, name } => write!(
                f,
                "{owner}: decoded SavedProperties name {name:?} is outside the IL's \
                 [SavedProperty] set, so its group is unrecoverable"
            ),
            Self::CapturePropertyType { owner, name, group } => write!(
                f,
                "{owner}: decoded SavedProperties {name:?} is declared as a {group} \
                 property but its value has another JSON type"
            ),
            Self::CaptureEnumUnknown { enumeration, value } => {
                write!(
                    f,
                    "capture {enumeration} member {value:?} is outside the enum"
                )
            }
            Self::CaptureRngStateMissing { stream } => write!(
                f,
                "capture rng stream {stream:?} does not carry both a counter and its \
                 four state words"
            ),
            Self::MalformedFactsJson(detail) => write!(f, "entry facts are not JSON: {detail}"),
            Self::UnknownFactsField(detail) => write!(
                f,
                "entry facts carry a field outside the taught sts-sim-entry-v1 surface: \
                 {detail}"
            ),
            Self::UnsupportedFactsSchema(schema) => write!(
                f,
                "entry facts schema {schema:?} is not \"sts-sim-entry-v1\""
            ),
            Self::FactsBuildMismatch { facts, requested } => write!(
                f,
                "entry facts were built for {facts:?}, and --build requested {requested:?}"
            ),
            Self::FactsInconsistent { field, detail } => {
                write!(f, "entry facts {field}: {detail}")
            }
            Self::FactsStreamDisagreesWithSeed { stream } => write!(
                f,
                "entry facts stream {stream:?} is not the state RunRngSet derives from \
                 the seed at its counter"
            ),
            Self::FactsStreamCounterUnverifiable { stream, counter } => write!(
                f,
                "entry facts stream {stream:?} counter {counter} is beyond the bound the \
                 seed derivation check walks"
            ),
            Self::MeltedRelicFlags { relic, detail } => {
                write!(f, "{relic}: melted-wax flags are unprovable: {detail}")
            }
            Self::MeltedRelicStillRead { relic, reader } => write!(
                f,
                "{relic} is melted, but native still reads it outside the hook walk \
                 ({reader}); melted-relic ownership for that reader is unmodeled"
            ),
            Self::EventCombatPathNotModeled {
                encounter,
                event,
                reason,
            } => write!(
                f,
                "{encounter} is started by event {event}, and the save predates the \
                 event: {reason}"
            ),
            Self::EventCombatListenerNotModeled {
                encounter,
                listener,
                hook,
            } => write!(
                f,
                "{encounter} is started by an event after the save, and {listener} \
                 listens to {hook} on that path, which the entry does not model"
            ),
            Self::EventCombatRunModifiers {
                encounter,
                modifiers,
            } => write!(
                f,
                "{encounter} is started by an event after the save, and the run's \
                 modifiers {modifiers:?} are hook listeners that path does not model"
            ),
            Self::EventCombatFactsUnvouched { encounter } => write!(
                f,
                "{encounter} is started by an event, and an entry-facts document cannot \
                 vouch whether its player is the pre-event snapshot"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_name_is_distinct() {
        // Two variants sharing a class would silently merge two census rows.
        let refusals = [
            EntryRefusal::MalformedSaveJson(String::new()),
            EntryRefusal::UnknownSaveField(String::new()),
            EntryRefusal::UnsupportedSaveSchema(19),
            EntryRefusal::UnadmittedGameBuild(String::new()),
            EntryRefusal::MultiplayerSave(2),
            EntryRefusal::MissingSaveField {
                owner: "player",
                field: "gold",
            },
            EntryRefusal::EmptyVisitedMapCoords,
            EntryRefusal::MissingMapPointHistory,
            EntryRefusal::ActIndexOutOfRange { index: 3, acts: 3 },
            EntryRefusal::UnknownNodeType(String::new()),
            EntryRefusal::NodeNotOnSavedMap { col: 0, row: 0 },
            EntryRefusal::NoEncounterForNode {
                node_type: String::new(),
            },
            EntryRefusal::EncounterIndexOutOfRange {
                list: "normal_encounter_ids",
                index: 0,
                len: 0,
            },
            EntryRefusal::MalformedModelId {
                category: "CARD",
                value: String::new(),
            },
            EntryRefusal::SavedPropertyGroupUnhandled {
                owner: String::new(),
                group: String::new(),
            },
            EntryRefusal::SavedPropertyRowShape {
                owner: String::new(),
                group: String::new(),
                detail: String::new(),
            },
            EntryRefusal::UnknownRngStream(String::new()),
            EntryRefusal::MissingRngStream("shuffle"),
            EntryRefusal::ProvenanceEntryIncomplete { fact: "current_hp" },
            EntryRefusal::UnsupportedProvenanceSchema(19),
            EntryRefusal::UnknownCaptureField(String::new()),
            EntryRefusal::CaptureFieldUnmapped { field: "x" },
            EntryRefusal::CapturePropertyUnknown {
                owner: String::new(),
                name: String::new(),
            },
            EntryRefusal::CapturePropertyType {
                owner: String::new(),
                name: String::new(),
                group: "ints",
            },
            EntryRefusal::CaptureEnumUnknown {
                enumeration: "GameMode",
                value: String::new(),
            },
            EntryRefusal::CaptureRngStateMissing {
                stream: String::new(),
            },
            EntryRefusal::MalformedFactsJson(String::new()),
            EntryRefusal::UnknownFactsField(String::new()),
            EntryRefusal::UnsupportedFactsSchema(String::new()),
            EntryRefusal::FactsBuildMismatch {
                facts: String::new(),
                requested: String::new(),
            },
            EntryRefusal::FactsInconsistent {
                field: "entry",
                detail: String::new(),
            },
            EntryRefusal::FactsStreamDisagreesWithSeed {
                stream: String::new(),
            },
            EntryRefusal::FactsStreamCounterUnverifiable {
                stream: String::new(),
                counter: 0,
            },
            EntryRefusal::EventCombatPathNotModeled {
                encounter: String::new(),
                event: "",
                reason: "",
            },
            EntryRefusal::EventCombatListenerNotModeled {
                encounter: String::new(),
                listener: String::new(),
                hook: "",
            },
            EntryRefusal::EventCombatRunModifiers {
                encounter: String::new(),
                modifiers: Vec::new(),
            },
            EntryRefusal::EventCombatFactsUnvouched {
                encounter: String::new(),
            },
        ];
        let mut classes: Vec<&str> = refusals.iter().map(EntryRefusal::class).collect();
        let total = classes.len();
        classes.sort_unstable();
        classes.dedup();
        assert_eq!(classes.len(), total);
    }

    #[test]
    fn display_never_renders_empty() {
        // The census stores the detail verbatim; an empty one is unreadable
        // evidence.
        assert!(!EntryRefusal::EmptyVisitedMapCoords.to_string().is_empty());
        assert!(
            EntryRefusal::UnsupportedSaveSchema(16)
                .to_string()
                .contains("16")
        );
    }
}

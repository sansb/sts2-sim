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
    /// Key order comes from sorting, never from struct declaration order, so
    /// the digest cannot drift when a field is added or moved. The bytes are
    /// those of routing through [`serde_json::Value`] (`serde_json::Map` is a
    /// `BTreeMap`); [`sorted_json`] writes them without building that tree,
    /// and falls back to it for a shape it does not write (#3420).
    pub fn canonical_json(&self) -> String {
        let Ok(bytes) = sorted_json::to_vec(self) else {
            return self.canonical_json_via_value();
        };
        let json = String::from_utf8(bytes).expect("serde_json writes UTF-8");
        debug_assert_eq!(json, self.canonical_json_via_value());
        json
    }

    /// The reference route: build the [`Value`] tree, then write it.
    fn canonical_json_via_value(&self) -> String {
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

/// A JSON writer that sorts every object's keys, producing the bytes of
/// `serde_json::to_string(&serde_json::to_value(value)?)` without building
/// (and then dropping) the intermediate [`Value`] tree (#3420).
///
/// Equivalence, by construction:
///
/// * Objects (structs, maps, and every variant wrapper) sort their keys
///   bytewise, which is `serde_json::Map`'s `BTreeMap<String, _>` order, and a
///   repeated key keeps its last value, as `Map::insert` does.
/// * Integers, booleans, strings, units and `None` are written by serde_json's
///   own serializer, the same formatter the `Value` route ends in. Floats,
///   `char`, bytes and 128-bit integers take the `Value` route per leaf.
/// * serde_json's private `Number` token (this crate enables
///   `arbitrary_precision`) is written raw, as `Value::Number` is.
///
/// Any other shape (a non-string map key, another private serde_json token)
/// is an error, and [`CanonicalStateV2::canonical_json`] falls back to the
/// `Value` route for the whole document.
///
/// Entries are written in arrival order, key included. An object whose keys
/// arrived strictly ascending (every `BTreeMap` and `serde_json::Map`) is
/// already canonical; any other is reordered on close through one scratch
/// buffer and one entry stack shared by the whole document, which is sound
/// because a nested object always closes before its parent's next entry.
mod sorted_json {
    use serde::ser::{self, Serialize};
    use serde_json::{Error, Value};

    const NUMBER_TOKEN: &str = "$serde_json::private::Number";
    const PRIVATE_PREFIX: &str = "$serde_json::private::";

    pub(super) fn to_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Error> {
        let mut ctx = Ctx {
            out: Vec::with_capacity(8192),
            scratch: Vec::new(),
            entries: Vec::new(),
        };
        value.serialize(Sorted { ctx: &mut ctx })?;
        Ok(ctx.out)
    }

    struct Ctx {
        out: Vec<u8>,
        scratch: Vec<u8>,
        /// Open objects' entries, innermost last.
        entries: Vec<Entry>,
    }

    /// One written `"key":value`, as a byte range of `Ctx::out`.
    struct Entry {
        key: Key,
        start: usize,
        end: usize,
    }

    enum Key {
        Static(&'static str),
        Owned(String),
    }

    impl Key {
        fn as_str(&self) -> &str {
            match self {
                Key::Static(key) => key,
                Key::Owned(key) => key,
            }
        }
    }

    fn direct<T: Serialize + ?Sized>(out: &mut Vec<u8>, value: &T) -> Result<(), Error> {
        serde_json::to_writer(out, value)
    }

    fn via_value<T: Serialize + ?Sized>(out: &mut Vec<u8>, value: &T) -> Result<(), Error> {
        serde_json::to_writer(out, &serde_json::to_value(value)?)
    }

    fn unsupported(what: &str) -> Error {
        ser::Error::custom(format!("sorted_json: unsupported {what}"))
    }

    struct Sorted<'a> {
        ctx: &'a mut Ctx,
    }

    impl Sorted<'_> {
        /// `{"variant":` — the single-key object serde_json wraps a
        /// non-unit variant in; the caller closes it.
        fn open_variant(&mut self, variant: &'static str) -> Result<(), Error> {
            self.ctx.out.push(b'{');
            direct(&mut self.ctx.out, variant)?;
            self.ctx.out.push(b':');
            Ok(())
        }
    }

    impl<'a> ser::Serializer for Sorted<'a> {
        type Ok = ();
        type Error = Error;
        type SerializeSeq = Seq<'a>;
        type SerializeTuple = Seq<'a>;
        type SerializeTupleStruct = Seq<'a>;
        type SerializeTupleVariant = Seq<'a>;
        type SerializeMap = Object<'a>;
        type SerializeStruct = Object<'a>;
        type SerializeStructVariant = Object<'a>;

        fn serialize_bool(self, v: bool) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_i8(self, v: i8) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_i16(self, v: i16) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_i32(self, v: i32) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_i64(self, v: i64) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_i128(self, v: i128) -> Result<(), Error> {
            via_value(&mut self.ctx.out, &v)
        }
        fn serialize_u8(self, v: u8) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_u16(self, v: u16) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_u32(self, v: u32) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_u64(self, v: u64) -> Result<(), Error> {
            direct(&mut self.ctx.out, &v)
        }
        fn serialize_u128(self, v: u128) -> Result<(), Error> {
            via_value(&mut self.ctx.out, &v)
        }
        fn serialize_f32(self, v: f32) -> Result<(), Error> {
            via_value(&mut self.ctx.out, &v)
        }
        fn serialize_f64(self, v: f64) -> Result<(), Error> {
            via_value(&mut self.ctx.out, &v)
        }
        fn serialize_char(self, v: char) -> Result<(), Error> {
            via_value(&mut self.ctx.out, &v)
        }
        fn serialize_str(self, v: &str) -> Result<(), Error> {
            direct(&mut self.ctx.out, v)
        }
        fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
            via_value(&mut self.ctx.out, v)
        }
        fn serialize_none(self) -> Result<(), Error> {
            direct(&mut self.ctx.out, &())
        }
        fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
            value.serialize(self)
        }
        fn serialize_unit(self) -> Result<(), Error> {
            direct(&mut self.ctx.out, &())
        }
        fn serialize_unit_struct(self, _name: &'static str) -> Result<(), Error> {
            direct(&mut self.ctx.out, &())
        }
        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
        ) -> Result<(), Error> {
            direct(&mut self.ctx.out, variant)
        }
        fn serialize_newtype_struct<T: Serialize + ?Sized>(
            self,
            name: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            if name.starts_with(PRIVATE_PREFIX) {
                return Err(unsupported(name));
            }
            value.serialize(self)
        }
        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            mut self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            self.open_variant(variant)?;
            value.serialize(Sorted {
                ctx: &mut *self.ctx,
            })?;
            self.ctx.out.push(b'}');
            Ok(())
        }
        fn serialize_seq(self, _len: Option<usize>) -> Result<Seq<'a>, Error> {
            Ok(Seq::open(self.ctx, false))
        }
        fn serialize_tuple(self, _len: usize) -> Result<Seq<'a>, Error> {
            Ok(Seq::open(self.ctx, false))
        }
        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Seq<'a>, Error> {
            Ok(Seq::open(self.ctx, false))
        }
        fn serialize_tuple_variant(
            mut self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
            _len: usize,
        ) -> Result<Seq<'a>, Error> {
            self.open_variant(variant)?;
            Ok(Seq::open(self.ctx, true))
        }
        fn serialize_map(self, _len: Option<usize>) -> Result<Object<'a>, Error> {
            Ok(Object::open(self.ctx, Mode::Object))
        }
        fn serialize_struct(self, name: &'static str, _len: usize) -> Result<Object<'a>, Error> {
            if name == NUMBER_TOKEN {
                return Ok(Object::open(self.ctx, Mode::Number));
            }
            if name.starts_with(PRIVATE_PREFIX) {
                return Err(unsupported(name));
            }
            Ok(Object::open(self.ctx, Mode::Object))
        }
        fn serialize_struct_variant(
            mut self,
            _name: &'static str,
            _index: u32,
            variant: &'static str,
            _len: usize,
        ) -> Result<Object<'a>, Error> {
            self.open_variant(variant)?;
            Ok(Object::open(self.ctx, Mode::VariantObject))
        }
    }

    struct Seq<'a> {
        ctx: &'a mut Ctx,
        first: bool,
        variant: bool,
    }

    impl<'a> Seq<'a> {
        fn open(ctx: &'a mut Ctx, variant: bool) -> Self {
            ctx.out.push(b'[');
            Self {
                ctx,
                first: true,
                variant,
            }
        }

        fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            if !self.first {
                self.ctx.out.push(b',');
            }
            self.first = false;
            value.serialize(Sorted {
                ctx: &mut *self.ctx,
            })
        }

        fn close(self) -> Result<(), Error> {
            self.ctx.out.push(b']');
            if self.variant {
                self.ctx.out.push(b'}');
            }
            Ok(())
        }
    }

    impl ser::SerializeSeq for Seq<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            self.element(value)
        }
        fn end(self) -> Result<(), Error> {
            self.close()
        }
    }

    impl ser::SerializeTuple for Seq<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            self.element(value)
        }
        fn end(self) -> Result<(), Error> {
            self.close()
        }
    }

    impl ser::SerializeTupleStruct for Seq<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            self.element(value)
        }
        fn end(self) -> Result<(), Error> {
            self.close()
        }
    }

    impl ser::SerializeTupleVariant for Seq<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            self.element(value)
        }
        fn end(self) -> Result<(), Error> {
            self.close()
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Object,
        /// A struct variant: close the `{"variant":` wrapper too.
        VariantObject,
        /// serde_json's `Number`: one field holding the digits, written raw.
        Number,
    }

    struct Object<'a> {
        ctx: &'a mut Ctx,
        /// Offset of this object's `{` in `Ctx::out`.
        base: usize,
        /// This object's first entry in `Ctx::entries`.
        first_entry: usize,
        /// Every key so far arrived strictly after its predecessor.
        ascending: bool,
        mode: Mode,
        pending_key: Option<String>,
        number_fields: usize,
    }

    impl<'a> Object<'a> {
        fn open(ctx: &'a mut Ctx, mode: Mode) -> Self {
            let base = ctx.out.len();
            if mode != Mode::Number {
                ctx.out.push(b'{');
            }
            let first_entry = ctx.entries.len();
            Self {
                ctx,
                base,
                first_entry,
                ascending: true,
                mode,
                pending_key: None,
                number_fields: 0,
            }
        }

        fn entry<T: Serialize + ?Sized>(&mut self, key: Key, value: &T) -> Result<(), Error> {
            let previous = self.ctx.entries[self.first_entry..].last();
            if let Some(previous) = previous {
                if previous.key.as_str() >= key.as_str() {
                    self.ascending = false;
                }
                self.ctx.out.push(b',');
            }
            let start = self.ctx.out.len();
            direct(&mut self.ctx.out, key.as_str())?;
            self.ctx.out.push(b':');
            value.serialize(Sorted {
                ctx: &mut *self.ctx,
            })?;
            let end = self.ctx.out.len();
            self.ctx.entries.push(Entry { key, start, end });
            Ok(())
        }

        fn field<T: Serialize + ?Sized>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            if self.mode == Mode::Number {
                if key != NUMBER_TOKEN || self.number_fields != 0 {
                    return Err(unsupported("Number shape"));
                }
                let Value::String(digits) = serde_json::to_value(value)? else {
                    return Err(unsupported("Number digits"));
                };
                self.ctx.out.extend_from_slice(digits.as_bytes());
                self.number_fields += 1;
                return Ok(());
            }
            self.entry(Key::Static(key), value)
        }

        fn finish(self) -> Result<(), Error> {
            let Object {
                ctx,
                base,
                first_entry,
                ascending,
                mode,
                number_fields,
                ..
            } = self;
            if mode == Mode::Number {
                return if number_fields == 1 {
                    Ok(())
                } else {
                    Err(unsupported("Number shape"))
                };
            }
            if !ascending {
                let Ctx {
                    out,
                    scratch,
                    entries,
                } = ctx;
                let body = base + 1;
                scratch.clear();
                scratch.extend_from_slice(&out[body..]);
                out.truncate(body);
                let entries = &mut entries[first_entry..];
                // Stable, so equal keys stay in arrival order and the last wins.
                entries.sort_by(|left, right| left.key.as_str().cmp(right.key.as_str()));
                let mut first = true;
                for (index, entry) in entries.iter().enumerate() {
                    if entries
                        .get(index + 1)
                        .is_some_and(|next| next.key.as_str() == entry.key.as_str())
                    {
                        continue;
                    }
                    if !first {
                        out.push(b',');
                    }
                    first = false;
                    out.extend_from_slice(&scratch[entry.start - body..entry.end - body]);
                }
            }
            ctx.entries.truncate(first_entry);
            ctx.out.push(b'}');
            if mode == Mode::VariantObject {
                ctx.out.push(b'}');
            }
            Ok(())
        }
    }

    impl ser::SerializeMap for Object<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
            let Value::String(key) = serde_json::to_value(key)? else {
                return Err(unsupported("non-string map key"));
            };
            self.pending_key = Some(key);
            Ok(())
        }
        fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
            let key = self
                .pending_key
                .take()
                .ok_or_else(|| unsupported("map value without a key"))?;
            self.entry(Key::Owned(key), value)
        }
        fn end(self) -> Result<(), Error> {
            self.finish()
        }
    }

    impl ser::SerializeStruct for Object<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            self.field(key, value)
        }
        fn end(self) -> Result<(), Error> {
            self.finish()
        }
    }

    impl ser::SerializeStructVariant for Object<'_> {
        type Ok = ();
        type Error = Error;
        fn serialize_field<T: Serialize + ?Sized>(
            &mut self,
            key: &'static str,
            value: &T,
        ) -> Result<(), Error> {
            self.field(key, value)
        }
        fn end(self) -> Result<(), Error> {
            self.finish()
        }
    }
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

    /// `sorted_json` against the `Value` route it replaces (#3420), over
    /// every shape it writes: arrival order sorted and unsorted, duplicate
    /// keys, each variant kind, the leaves it hands to serde_json, and keys
    /// whose byte order differs from their escaped form.
    #[test]
    fn sorted_json_writes_the_value_route_bytes_for_every_shape() {
        use serde::ser::{SerializeMap, Serializer};
        use std::collections::HashMap;

        fn value_route<T: Serialize + ?Sized>(value: &T) -> Vec<u8> {
            serde_json::to_vec(&serde_json::to_value(value).unwrap()).unwrap()
        }
        fn check<T: Serialize + ?Sized>(label: &str, value: &T) {
            let sorted = sorted_json::to_vec(value).expect(label);
            assert_eq!(
                String::from_utf8(sorted).unwrap(),
                String::from_utf8(value_route(value)).unwrap(),
                "{label}"
            );
        }

        #[derive(Serialize)]
        struct Unit;
        #[derive(Serialize)]
        struct Newtype(i32);
        #[derive(Serialize)]
        struct Pair(u8, &'static str);
        #[derive(Serialize)]
        enum Shape {
            Unit,
            Newtype(Vec<i8>),
            Tuple(i32, Option<bool>),
            Struct { zeta: u16, alpha: Option<u16> },
        }
        #[derive(Serialize)]
        struct Leaves {
            zulu: f64,
            yankee: f32,
            xray: i128,
            whiskey: u128,
            victor: char,
            uniform: Unit,
            tango: Newtype,
            sierra: Pair,
            romeo: (i64, u64),
            quebec: Option<String>,
            papa: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            oscar: Option<u8>,
            november: Vec<Shape>,
            mike: Vec<Value>,
        }
        /// A map that repeats a key: `Map::insert` keeps the last value.
        struct Repeats;
        impl Serialize for Repeats {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("b", &1)?;
                map.serialize_entry("a", &2)?;
                map.serialize_entry("b", &3)?;
                map.serialize_entry("a", &4)?;
                map.serialize_entry("c", &5)?;
                map.end()
            }
        }

        let leaves = Leaves {
            zulu: -0.1,
            yankee: 1.1,
            xray: i128::MIN,
            whiskey: u128::MAX,
            victor: '"',
            uniform: Unit,
            tango: Newtype(-7),
            sierra: Pair(255, "tab\there"),
            romeo: (i64::MIN, u64::MAX),
            quebec: Some("é\u{1F600}\\".to_owned()),
            papa: None,
            oscar: None,
            november: vec![
                Shape::Unit,
                Shape::Newtype(vec![-1, 0, 1]),
                Shape::Tuple(3, None),
                Shape::Struct {
                    zeta: 2,
                    alpha: Some(1),
                },
            ],
            mike: vec![
                serde_json::json!({"z": [1, -2, 3.5, 1e300, 18446744073709551615u64], "a": {"y": null, "b": true}}),
                serde_json::json!([]),
                serde_json::json!({}),
            ],
        };
        check("leaves and variants", &leaves);
        check("nested Value", &leaves.mike);
        check("repeated keys", &Repeats);

        // Unsorted arrival, and keys whose raw byte order is not the order of
        // their escaped JSON (`"` escapes to `\"`, above `A`; `é` sorts after
        // every ASCII key).
        let mut keys = HashMap::new();
        for (index, key) in ["zebra", "\"quote", "A", "é", "a\u{0}", "a", "", "\\", "Z"]
            .into_iter()
            .enumerate()
        {
            keys.insert(key.to_owned(), vec![index; index % 3]);
        }
        check("unsorted string keys", &keys);
        let sorted: BTreeMap<String, Vec<usize>> = keys.clone().into_iter().collect();
        check("sorted string keys", &sorted);
        check(
            "map in a variant in a map",
            &BTreeMap::from([(
                "k",
                Shape::Struct {
                    zeta: 9,
                    alpha: None,
                },
            )]),
        );
        check(
            "empty containers",
            &(Vec::<u8>::new(), BTreeMap::<String, u8>::new()),
        );

        // A shape it does not write is an error, so `canonical_json` falls
        // back to the `Value` route rather than guessing.
        let numeric_keys = BTreeMap::from([(10_u32, "ten"), (2, "two")]);
        assert!(sorted_json::to_vec(&numeric_keys).is_err());
        assert_eq!(
            serde_json::to_string(&serde_json::to_value(&numeric_keys).unwrap()).unwrap(),
            r#"{"10":"ten","2":"two"}"#
        );
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

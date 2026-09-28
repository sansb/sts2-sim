//! Per-fight roster parity against Python's `make_monsters` (#2532, #2531).
//!
//! The acceptance measurement for every landed E4b family, replayed rather
//! than asserted: the Python generator walked the local capture corpus, kept
//! every fight whose encounter dispatched to a `content/encounters/<shard>.py`
//! builder, and recorded what `combat_sim.make_monsters` produced together
//! with the exact RNG state it produced it from. This module replays every one
//! of those cases through the Rust builders and compares the roster, the HP
//! rolls, the per-monster initial state and the Niche counter after creation.
//!
//! The cases are **frozen data** since #2999: #2827 item F deletes the Python
//! simulator, so the roster half can no longer be regenerated, and
//! `tools/gen_roster_pins.py` keeps only its stdlib `--relabel` mode (the last
//! generator is in git history before #2999).
//!
//! Frozen is not infallible: where the Python oracle disagreed with the game,
//! the pin is hand-corrected to the IL and the correction is named here.
//! #3048 removed `("block", 13)` from every `CUBEX_CONSTRUCT` case in
//! `normal_a_rosters_v1.json` (5) and `normal_c_rosters_v1.json` (26):
//! `CubexConstruct/<AfterAddedToRoom>d__27`'s `GainBlock` runs before combat
//! is in progress and is a no-op (`normal_a::build_cubex_construct_normal`).
//!
//! One pin file per family, and [`PINS`] is the whole list — a family is
//! measured here or it is not measured. F1 (weak) generalised the single-file
//! shape F2 (elite) landed with rather than copying it.
//!
//! # Why the streams come from the fixture
//!
//! `RunRngSet`'s per-stream seeding and `Rng.for_encounter`'s per-fight
//! Encounter derivation are #2528 §A4 engine sites, escalated rather than
//! inlined here — `rng::stream_seed` deliberately has no `v0.111.0` arm and
//! no caller in this crate (`entry/counters.rs`). So the fixture carries the
//! four xoshiro words and the counter of each stream, and this context starts
//! from those. That is also how a real fight will reach the builders: a
//! schema >= 19 save *records* the Niche stream's state, and the entry builder
//! reads it.
//!
//! # The fixture labels are checked against the manifest, here
//!
//! Each case's `fixture` is the eval-manifest id of the same `(seed, node)`,
//! or null. That label is the one input to the pins that lives in git rather
//! than in the local corpus, and it is the one that drifted: the manifest was
//! re-seeded on 2026-09-17 (`bd7717a5`) and `elite_rosters_v1.json` kept the
//! 2026-09-16 labels for four days with nothing red, because the generator's
//! `--check` needs `~/sts2-captures` and the counts here were compared with
//! themselves. [`every_fixture_label_is_the_eval_manifests`] re-derives the
//! labels from the committed `eval/manifest.json` instead, in both directions,
//! so a manifest change that relabels a pooled fight fails this lane until
//! `tools/gen_roster_pins.py --relabel` has rewritten the labels. That mode
//! is standard library, so a re-seed (#2919) keeps working after item F.
//!
//! The reverse direction has two ways to be satisfied. A pooled manifest
//! fixture is either a pinned case carrying its own label, or it is listed in
//! the pin file's `unpinned_fixtures`. That list holds the fixtures a re-seed
//! added after the freeze, which no Python roster was ever measured for; the
//! `.mcr` certification census covers them against the game's own checksums
//! instead. It must be exact in both directions, so a pool's membership still
//! cannot move silently. `--relabel` writes it. Until #2999 the generator
//! rooted such a fixture from its provenance's entry save (#2787) and counted
//! it in `explicit_fixture_fights`, which now records that last generation.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::encounters::{EncounterCtx, MonsterSpec, RosterRefusal, roster_builder};
use crate::ids::{EncounterId, MonsterKind};
use crate::rng::Xoshiro256StarStar;

/// Every landed slice's committed pin file.
///
/// Usually one per family. `scrolls_of_biting` is the exception the partition
/// predicted: one Python builder behind two registered keys in two different
/// families (F3's `SCROLLS_OF_BITING_WEAK`, F6's `SCROLLS_OF_BITING_NORMAL`),
/// ported once ahead of both waves. Its pin file is that pair and nothing
/// else, so F3 and F6 still pin their remaining ids under their own names and
/// [`the_crate_builds_exactly_the_ported_pools`] keeps counting every
/// registered builder exactly once.
const PINS: [(&str, &str); 8] = [
    ("boss", include_str!("../../fixtures/boss_rosters_v1.json")),
    (
        "elite",
        include_str!("../../fixtures/elite_rosters_v1.json"),
    ),
    (
        "event",
        include_str!("../../fixtures/event_rosters_v1.json"),
    ),
    (
        "normal_a",
        include_str!("../../fixtures/normal_a_rosters_v1.json"),
    ),
    (
        "normal_b",
        include_str!("../../fixtures/normal_b_rosters_v1.json"),
    ),
    (
        "normal_c",
        include_str!("../../fixtures/normal_c_rosters_v1.json"),
    ),
    (
        "scrolls_of_biting",
        include_str!("../../fixtures/scrolls_of_biting_rosters_v1.json"),
    ),
    ("weak", include_str!("../../fixtures/weak_rosters_v1.json")),
];

/// The eval manifest every pin file's `fixture` labels were read from.
///
/// `tools/gen_roster_pins.py` keys a fixture on `(seed, node)`; this is the
/// same file, read the same way, so the labels can be re-derived without the
/// capture corpus. It is the only pin input that changes between corpus
/// re-derivations (six manifest commits between 2026-09-14 and 2026-09-17).
const MANIFEST: &str = include_str!("../../../eval/manifest.json");

#[derive(Deserialize)]
struct Manifest {
    fights: Vec<ManifestFight>,
}

#[derive(Deserialize)]
struct ManifestFight {
    id: String,
    seed: String,
    node: i64,
    encounter: Option<String>,
}

#[derive(Deserialize)]
struct Pins {
    build: String,
    corpus_fights: usize,
    /// Of `corpus_fights`, the cases rooted from an eval fixture's own
    /// provenance because no watcher-named corpus capture reached it (#2787).
    explicit_fixture_fights: usize,
    fixture_fights: usize,
    /// Cases rooted from a made-up seed, floor and Niche counter rather than
    /// a real run (#2536) — `gen_roster_pins.py::synthetic_cases`. Absent
    /// from a slice that does not ask for them.
    #[serde(default)]
    synthetic_fights: usize,
    #[serde(default)]
    per_encounter_synthetic: BTreeMap<String, usize>,
    registered_keys: Vec<String>,
    per_encounter: BTreeMap<String, Counts>,
    cases: Vec<Case>,
    /// Pooled manifest fixtures with no pinned case, sorted by id: fights a
    /// re-seed added after the #2999 freeze. Absent while there are none.
    #[serde(default)]
    unpinned_fixtures: Vec<String>,
}

#[derive(Deserialize)]
struct Counts {
    corpus: usize,
    fixtures: usize,
}

#[derive(Deserialize)]
struct Stream {
    counter: u64,
    words: [u64; 4],
}

#[derive(Deserialize)]
struct Case {
    encounter: String,
    registered_key: String,
    seed: String,
    node_index: Option<i64>,
    /// The save's `ascension`, which `make_monsters` selected every tier by
    /// when the pin was generated (#2539).
    ascension: u8,
    fixture: Option<String>,
    /// A synthetic root: measured exactly like a corpus case, but no real run
    /// reached it, so it never carries a fixture label and is counted apart.
    #[serde(default)]
    synthetic: bool,
    niche_in: Stream,
    niche_out: Stream,
    encounter_stream: Option<Stream>,
    roster: Vec<PinnedMonster>,
}

#[derive(Deserialize)]
struct PinnedMonster {
    uid: usize,
    kind: String,
    hp: i32,
    max_hp: i32,
    slot: i32,
    loop_pos: i32,
    next_move: String,
    move_log: Vec<String>,
    initial_state: Vec<(String, serde_json::Value)>,
}

/// The fight-scoped RNG context, replayed from recorded stream state.
struct ReplayCtx {
    niche: Xoshiro256StarStar,
    encounter: Option<Xoshiro256StarStar>,
    ascension: u8,
}

impl ReplayCtx {
    fn new(case: &Case) -> Self {
        Self {
            niche: Xoshiro256StarStar {
                words: case.niche_in.words,
                counter: case.niche_in.counter,
            },
            encounter: case.encounter_stream.as_ref().map(|s| Xoshiro256StarStar {
                words: s.words,
                counter: s.counter,
            }),
            ascension: case.ascension,
        }
    }
}

/// `Rng.next_int(lo, hi)` — `hi` exclusive, exactly one draw.
fn next_int(rng: &mut Xoshiro256StarStar, lo: i32, hi: i32) -> i32 {
    assert!(lo < hi, "Minimum must be lower than maximum");
    rng.next_bounded(hi - lo).expect("bounded by lo < hi") + lo
}

impl EncounterCtx for ReplayCtx {
    fn ascension(&self) -> u8 {
        self.ascension
    }

    fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32 {
        next_int(&mut self.niche, lo, hi)
    }

    fn encounter_next_int(
        &mut self,
        roll: &'static str,
        lo: i32,
        hi: i32,
    ) -> Result<i32, RosterRefusal> {
        match self.encounter.as_mut() {
            Some(rng) => Ok(next_int(rng, lo, hi)),
            None => Err(RosterRefusal::EncounterStreamUnavailable { roll }),
        }
    }
}

fn pins() -> Vec<(&'static str, Pins)> {
    PINS.iter()
        .map(|(pool, text)| {
            (
                *pool,
                serde_json::from_str::<Pins>(text)
                    .unwrap_or_else(|e| panic!("{pool} roster pins parse: {e}")),
            )
        })
        .collect()
}

fn build(case: &Case) -> Vec<MonsterSpec> {
    let encounter = EncounterId::from_str(&case.registered_key)
        .unwrap_or_else(|| panic!("{}: no EncounterId", case.registered_key));
    let builder = roster_builder(encounter)
        .unwrap_or_else(|| panic!("{}: no registered roster builder", case.registered_key));
    let mut ctx = ReplayCtx::new(case);
    let roster = builder(&mut ctx)
        .unwrap_or_else(|refusal| panic!("{} ({}): {refusal:?}", case.encounter, case.seed));
    assert_eq!(
        ctx.niche.counter, case.niche_out.counter,
        "{} ({} node {:?}): Niche counter after creation",
        case.encounter, case.seed, case.node_index
    );
    assert_eq!(
        ctx.niche.words, case.niche_out.words,
        "{} ({} node {:?}): Niche stream words after creation",
        case.encounter, case.seed, case.node_index
    );
    roster
}

fn assert_roster(case: &Case, roster: &[MonsterSpec]) {
    let where_ = format!(
        "{} ({} node {:?}{})",
        case.encounter,
        case.seed,
        case.node_index,
        case.fixture
            .as_ref()
            .map(|f| format!(", fixture {f}"))
            .unwrap_or_default()
    );
    assert_eq!(roster.len(), case.roster.len(), "{where_}: roster size");
    for (index, (got, want)) in roster.iter().zip(&case.roster).enumerate() {
        assert_eq!(index, want.uid, "{where_}: creation order is uid order");
        assert_eq!(
            got.kind,
            MonsterKind::from_str(&want.kind).expect("a modeled kind"),
            "{where_}[{index}]: kind"
        );
        assert_eq!(got.hp, want.hp, "{where_}[{index}]: hp");
        assert_eq!(got.max_hp, want.max_hp, "{where_}[{index}]: max_hp");
        assert_eq!(got.slot, want.slot, "{where_}[{index}]: slot");
        assert_eq!(got.loop_pos, want.loop_pos, "{where_}[{index}]: loop_pos");
        assert_eq!(
            got.next_move, want.next_move,
            "{where_}[{index}]: next_move"
        );
        assert_eq!(got.move_log, want.move_log, "{where_}[{index}]: move_log");
        // `gen_roster_pins.py::_initial_state` writes `int(value)` for a bool,
        // so a flag compares as 0/1, and a `str` field (#2848) as the string.
        // The pin's encoding, not the wire's — `monster_entity` keeps the type.
        let state: Vec<(String, serde_json::Value)> = got
            .initial_state
            .iter()
            .map(|(field, value)| ((*field).to_string(), value.pin()))
            .collect();
        assert_eq!(
            state, want.initial_state,
            "{where_}[{index}]: initial state"
        );
    }
}

#[test]
fn every_ported_corpus_fight_and_fixture_matches_python_make_monsters() {
    for (pool, pins) in pins() {
        assert_eq!(pins.build, "v0.111.0", "{pool}");
        assert!(!pins.cases.is_empty(), "{pool}: the pins carry cases");
        let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut synthetic: BTreeMap<String, usize> = BTreeMap::new();
        for case in &pins.cases {
            let roster = build(case);
            assert_roster(case, &roster);
            if case.synthetic {
                assert!(
                    case.fixture.is_none(),
                    "{pool}/{}: a synthetic root carries a fixture label",
                    case.encounter
                );
                *synthetic.entry(case.encounter.clone()).or_default() += 1;
                continue;
            }
            let row = seen.entry(case.encounter.clone()).or_insert((0, 0));
            row.0 += 1;
            if case.fixture.is_some() {
                row.1 += 1;
            }
        }
        // Per encounter, not summarised: a family's acceptance is a table.
        for (encounter, counts) in &pins.per_encounter {
            let got = seen.get(encounter).copied().unwrap_or((0, 0));
            assert_eq!(
                got,
                (counts.corpus, counts.fixtures),
                "{pool}/{encounter}: replayed corpus/fixture counts"
            );
        }
        assert_eq!(
            synthetic, pins.per_encounter_synthetic,
            "{pool}: replayed synthetic roots per encounter"
        );
        // Every builder the slice registers has at least one parity witness,
        // real or synthetic — a zero-demand id may not land unmeasured.
        for key in &pins.registered_keys {
            assert!(
                pins.cases.iter().any(|case| &case.registered_key == key),
                "{pool}/{key}: registered but no pinned case replays it"
            );
        }
        assert_eq!(
            pins.cases.len(),
            pins.corpus_fights + pins.synthetic_fights,
            "{pool}: every recorded fight is replayed"
        );
        assert_eq!(
            pins.cases.iter().filter(|c| c.fixture.is_some()).count(),
            pins.fixture_fights,
            "{pool}: every recorded eval fixture is replayed"
        );
        // An explicitly added case exists only because a manifest fixture
        // named it at the last Python generation. A later relabel may drop
        // that fixture from the manifest, so this bounds the cases, not the
        // labels (#2999).
        assert!(
            pins.explicit_fixture_fights <= pins.corpus_fights,
            "{pool}: {} explicitly added cases but only {} corpus cases",
            pins.explicit_fixture_fights,
            pins.corpus_fights
        );
    }
}

/// The manifest's labelling, as `gen_roster_pins.py::fixture_ids` reads it.
fn manifest_labels(manifest: &str) -> BTreeMap<(String, i64), String> {
    let manifest: Manifest =
        serde_json::from_str(manifest).unwrap_or_else(|e| panic!("eval manifest parse: {e}"));
    let mut labels = BTreeMap::new();
    for fight in manifest.fights {
        let previous = labels.insert((fight.seed.clone(), fight.node), fight.id.clone());
        assert!(
            previous.is_none(),
            "eval manifest: two fixtures at ({}, node {})",
            fight.seed,
            fight.node
        );
    }
    labels
}

/// `gen_roster_pins.py::dispatch` — substring, exactly one owner or none.
fn pooled_key<'a>(registered_keys: &'a [String], wire_id: &str) -> Option<&'a str> {
    let mut owners = registered_keys
        .iter()
        .filter(|key| wire_id.contains(key.as_str()));
    let first = owners.next()?;
    assert!(
        owners.next().is_none(),
        "{wire_id}: claimed by more than one registered key"
    );
    Some(first)
}

#[test]
fn every_fixture_label_is_the_eval_manifests() {
    for (pool, pins) in pins() {
        assert_labels_are_the_manifests(pool, &pins, MANIFEST);
    }
}

/// One pool's labels against one manifest, in both directions.
fn assert_labels_are_the_manifests(pool: &str, pins: &Pins, manifest: &str) {
    let labels = manifest_labels(manifest);
    let regenerate = "relabel with `python3 versions/v0.111.0/rust/tools/gen_roster_pins.py \
                      --relabel` (the manifest changed under the pins)";
    // Forward: every recorded case carries the manifest's label for its
    // fight, or none where the manifest has no fixture there.
    let mut recorded = BTreeMap::new();
    for case in &pins.cases {
        let node = case.node_index.unwrap_or_else(|| {
            panic!(
                "{pool}/{} ({}): a pinned case has no node",
                case.encounter, case.seed
            )
        });
        let want = labels.get(&(case.seed.clone(), node));
        assert_eq!(
            case.fixture.as_ref(),
            want,
            "{pool}/{} ({} node {node}): fixture label — {regenerate}",
            case.encounter,
            case.seed
        );
        recorded.insert((case.seed.clone(), node), case.fixture.clone());
    }
    // Reverse: every manifest fixture whose encounter dispatches to this
    // pool is one of the recorded cases, or is listed as unpinned. A
    // fixture the pins neither record nor list means the manifest moved
    // the family's membership without a relabel.
    let manifest: Manifest = serde_json::from_str(manifest).expect("parsed above");
    let mut unpinned = Vec::new();
    for fight in manifest.fights {
        let Some(encounter) = fight.encounter.as_deref() else {
            continue;
        };
        if pooled_key(&pins.registered_keys, encounter).is_none() {
            continue;
        }
        match recorded.get(&(fight.seed.clone(), fight.node)) {
            Some(label) => assert_eq!(
                label.as_ref(),
                Some(&fight.id),
                "{pool}/{encounter} ({} node {}): fixture label — {regenerate}",
                fight.seed,
                fight.node
            ),
            None => unpinned.push(fight.id.clone()),
        }
    }
    unpinned.sort();
    assert_eq!(
        unpinned, pins.unpinned_fixtures,
        "{pool}: the pooled manifest fixtures with no pinned case are not the pin \
         file's `unpinned_fixtures` — {regenerate}"
    );
}

/// A one-case pool for the label controls: `SKULKING_COLONY` at
/// `(SEED, node 14)`, labelled `aaaa000000000001`, plus whatever
/// `unpinned_fixtures` the control lists.
fn control_pins(unpinned: &[&str]) -> Pins {
    let mut pins: serde_json::Value = serde_json::json!({
        "build": "v0.111.0",
        "corpus_fights": 1,
        "explicit_fixture_fights": 0,
        "fixture_fights": 1,
        "registered_keys": ["SKULKING_COLONY"],
        "per_encounter": {"ENCOUNTER.SKULKING_COLONY_ELITE": {"corpus": 1, "fixtures": 1}},
        "cases": [{
            "encounter": "ENCOUNTER.SKULKING_COLONY_ELITE",
            "registered_key": "SKULKING_COLONY",
            "seed": "SEED",
            "node_index": 14,
            "ascension": 10,
            "fixture": "aaaa000000000001",
            "niche_in": {"counter": 0, "words": [1, 2, 3, 4]},
            "niche_out": {"counter": 1, "words": [1, 2, 3, 4]},
            "encounter_stream": null,
            "roster": []
        }]
    });
    if !unpinned.is_empty() {
        pins["unpinned_fixtures"] = serde_json::json!(unpinned);
    }
    serde_json::from_value(pins).unwrap()
}

/// The pinned fixture, plus one pooled fixture a re-seed added after the
/// freeze (no case at `(SEED, node 30)`) and one out-of-pool fixture.
const CONTROL_MANIFEST: &str = r#"{"fights": [
    {"id": "aaaa000000000001", "seed": "SEED", "node": 14,
     "encounter": "ENCOUNTER.SKULKING_COLONY_ELITE"},
    {"id": "bbbb000000000002", "seed": "SEED", "node": 30,
     "encounter": "ENCOUNTER.SKULKING_COLONY_ELITE"},
    {"id": "cccc000000000003", "seed": "SEED", "node": 32,
     "encounter": "ENCOUNTER.THE_INSATIABLE_BOSS"}
]}"#;

#[test]
fn a_pooled_fixture_added_after_the_freeze_passes_when_listed_unpinned() {
    assert_labels_are_the_manifests(
        "control",
        &control_pins(&["bbbb000000000002"]),
        CONTROL_MANIFEST,
    );
}

#[test]
#[should_panic(expected = "`unpinned_fixtures`")]
fn a_pooled_fixture_added_after_the_freeze_fails_until_relabelled() {
    assert_labels_are_the_manifests("control", &control_pins(&[]), CONTROL_MANIFEST);
}

#[test]
#[should_panic(expected = "`unpinned_fixtures`")]
fn a_stale_unpinned_entry_fails_once_the_manifest_drops_it() {
    let manifest = r#"{"fights": [
        {"id": "aaaa000000000001", "seed": "SEED", "node": 14,
         "encounter": "ENCOUNTER.SKULKING_COLONY_ELITE"}
    ]}"#;
    assert_labels_are_the_manifests("control", &control_pins(&["bbbb000000000002"]), manifest);
}

#[test]
#[should_panic(expected = "fixture label")]
fn a_renamed_pinned_fixture_fails_until_relabelled() {
    let manifest = CONTROL_MANIFEST.replace("aaaa000000000001", "dddd000000000004");
    assert_labels_are_the_manifests("control", &control_pins(&["bbbb000000000002"]), &manifest);
}

#[test]
fn the_crate_builds_exactly_the_ported_pools() {
    let mut registered = 0;
    for (pool, pins) in pins() {
        for key in &pins.registered_keys {
            let encounter =
                EncounterId::from_str(key).unwrap_or_else(|| panic!("{key}: no EncounterId"));
            assert!(
                roster_builder(encounter).is_some(),
                "{key}: registered in Python's {pool} pool but no Rust builder"
            );
        }
        registered += pins.registered_keys.len();
    }
    assert_eq!(
        crate::content_tables::ENCOUNTER_ROSTER_BUILDERS.len(),
        registered,
        "the crate builds exactly the pooled families that carry pins — an \
         extra row means a builder landed without its acceptance measurement"
    );
}

#[test]
fn the_roster_registry_is_sorted_for_binary_search() {
    let table = &crate::content_tables::ENCOUNTER_ROSTER_BUILDERS;
    for window in table.windows(2) {
        assert!(
            (window[0].0 as u16) < (window[1].0 as u16),
            "ENCOUNTER_ROSTER_BUILDERS must be ascending by discriminant"
        );
    }
}

#[test]
fn a_kind_whose_native_model_is_unread_refuses_by_name() {
    // `TOUGH_EGG`'s `Apply<HatchPower>` amount is a local the manifest
    // reader does not decode, so its row carries a reason and no facts. A
    // builder that reached for it must refuse, not approximate (I5).
    // (`TEST_SUBJECT` was this witness until #2535 taught the reader its
    // two-hop getter delegation, and `AXEBOT` until #2534 taught it the
    // straight-line getter its HP is computed through.)
    let refusal =
        crate::encounters::hp_band(MonsterKind::ToughEgg, crate::encounters::MODELED_ASCENSION)
            .expect_err("TOUGH_EGG has no exactly-read HP band");
    match refusal {
        RosterRefusal::MonsterModelUnread { kind, why } => {
            assert_eq!(kind, MonsterKind::ToughEgg);
            assert!(!why.is_empty(), "the refusal names the reason");
        }
        other => panic!("unexpected refusal {other:?}"),
    }
}

#[test]
fn a_ranged_band_cannot_be_taken_as_a_fixed_hp() {
    struct Dead;
    impl EncounterCtx for Dead {
        fn ascension(&self) -> u8 {
            crate::encounters::MODELED_ASCENSION
        }
        fn niche_next_int(&mut self, _lo: i32, _hi: i32) -> i32 {
            panic!("the draw must not be spent before the band is checked")
        }
        fn encounter_next_int(
            &mut self,
            roll: &'static str,
            _lo: i32,
            _hi: i32,
        ) -> Result<i32, RosterRefusal> {
            Err(RosterRefusal::EncounterStreamUnavailable { roll })
        }
    }
    let refusal = crate::encounters::fixed(&mut Dead, MonsterKind::PhrogParasite)
        .expect_err("PHROG_PARASITE's band is a range");
    assert_eq!(
        refusal,
        RosterRefusal::MonsterHpNotFixed {
            kind: MonsterKind::PhrogParasite
        }
    );
}

#[test]
fn a_rotation_position_is_read_by_move_name_and_refuses_an_unknown_one() {
    // The oracle carries these as bare indices with the move name in a
    // comment; reading the index out of the generated rotation by name is
    // what keeps the number out of the Rust source (PORT_PLAN §5). So the
    // lookup has to be exact in both directions.
    assert_eq!(
        crate::encounters::loop_position(MonsterKind::Toadpole, "SPIKEN"),
        Ok(2)
    );
    assert_eq!(
        crate::encounters::loop_position(MonsterKind::Toadpole, "WHIRL"),
        Ok(1)
    );
    assert_eq!(
        crate::encounters::loop_position(MonsterKind::Toadpole, "NOT_A_MOVE"),
        Err(RosterRefusal::MonsterLoopMoveAbsent {
            kind: MonsterKind::Toadpole,
            move_name: "NOT_A_MOVE",
        })
    );
    // A random-AI kind has no deterministic rotation at all, so a starting
    // position in one is not a fact about it.
    assert_eq!(
        crate::encounters::loop_position(MonsterKind::SludgeSpinner, "OIL_SPRAY"),
        Err(RosterRefusal::MonsterLoopAbsent {
            kind: MonsterKind::SludgeSpinner
        })
    );
}

#[test]
fn a_builder_that_needs_the_encounter_stream_refuses_without_a_floor() {
    struct NoFloor(Xoshiro256StarStar);
    impl EncounterCtx for NoFloor {
        fn ascension(&self) -> u8 {
            crate::encounters::MODELED_ASCENSION
        }
        fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32 {
            next_int(&mut self.0, lo, hi)
        }
        fn encounter_next_int(
            &mut self,
            roll: &'static str,
            _lo: i32,
            _hi: i32,
        ) -> Result<i32, RosterRefusal> {
            Err(RosterRefusal::EncounterStreamUnavailable { roll })
        }
    }
    let mut ctx = NoFloor(Xoshiro256StarStar::from_seed(1));
    let refusal = crate::encounters::elite::build_decimillipede(&mut ctx)
        .expect_err("no floor, no Encounter stream");
    assert!(matches!(
        refusal,
        RosterRefusal::EncounterStreamUnavailable { .. }
    ));
}

/// #2539: `tier` is native `GetValueIfAscension` — `>=` the gate selects
/// `at_or_above` (`AscensionManager::HasLevel` `0x11fa83`). A gate-8 HP field
/// (Soul Fysh `(8, 221, 211)`) and a gate-9 spawn amount (Corpse Slug's
/// `RavenousPower` `(9, 5, 4)`) at ascensions 7, 8 and 9: A8 is the one level
/// where the two disagree about which side of their gate the fight is on.
#[test]
fn tier_selection_is_at_or_above_the_gate() {
    use crate::content_tables::AscensionTier;
    use crate::encounters::{fixed_hp, hp_band, initial_power, tier};
    assert_eq!(
        [7, 8, 9].map(|a| hp_band(MonsterKind::SoulFysh, a).unwrap()),
        [(211, 211), (221, 221), (221, 221)]
    );
    assert_eq!(
        [7, 8, 9].map(|a| fixed_hp(MonsterKind::SoulFysh, a)),
        [Some(211), Some(221), Some(221)]
    );
    assert_eq!(
        [7, 8, 9].map(|a| initial_power(MonsterKind::CorpseSlug, "RavenousPower", a).unwrap()),
        [4, 4, 5]
    );
    // A ranged band is never a fixed HP, and an untiered getter ignores the level.
    assert_eq!(fixed_hp(MonsterKind::Toadpole, 10), None);
    let untiered = AscensionTier {
        gate: None,
        at_or_above: 9,
        below: 9,
    };
    assert_eq!([0, 10].map(|a| tier(untiered, a)), [9, 9]);
    assert_eq!(
        initial_power(MonsterKind::Exoskeleton, "HardToKillPower", 0),
        initial_power(MonsterKind::Exoskeleton, "HardToKillPower", 10)
    );
}

/// Byrdonis is a fixed 90 at A8+ but an 81..=84 band below (#2539), so its
/// builder rolls the tier's band; over the fixed band the one draw is the
/// same `Niche` draw `fixed` would spend.
#[test]
fn byrdonis_rolls_its_below_gate_band() {
    struct Ctx(Xoshiro256StarStar, u8);
    impl EncounterCtx for Ctx {
        fn ascension(&self) -> u8 {
            self.1
        }
        fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32 {
            next_int(&mut self.0, lo, hi)
        }
        fn encounter_next_int(
            &mut self,
            roll: &'static str,
            _lo: i32,
            _hi: i32,
        ) -> Result<i32, RosterRefusal> {
            Err(RosterRefusal::EncounterStreamUnavailable { roll })
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..32 {
        let mut ctx = Ctx(Xoshiro256StarStar::from_seed(seed), 7);
        let roster = crate::encounters::elite::build_byrdonis(&mut ctx).unwrap();
        assert_eq!(ctx.0.counter, 1, "one Niche draw");
        seen.insert(roster[0].max_hp);
    }
    assert!(seen.iter().all(|hp| (81..=84).contains(hp)), "{seen:?}");
    assert!(seen.len() > 1, "a real band: {seen:?}");
    let mut ctx = Ctx(Xoshiro256StarStar::from_seed(3), 8);
    let roster = crate::encounters::elite::build_byrdonis(&mut ctx).unwrap();
    assert_eq!((roster[0].max_hp, ctx.0.counter), (90, 1));
}

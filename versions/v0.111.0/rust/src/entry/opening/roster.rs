//! `make_monsters` — the engine half of encounter creation (#2528 §A8.2–A8.4).
//!
//! [`crate::encounters`] is the **content** half, landed by #2529's family
//! lanes: the `EncounterCtx` interface, `MonsterSpec`, `RosterRefusal`, the
//! generated `content_tables::ENCOUNTER_ROSTER_BUILDERS` registry, and one
//! `build_*` per registered encounter. Its module doc names four things a
//! content PR may not inline, and this module is those four:
//!
//! 1. the substring dispatch from a wire id onto a registered key;
//! 2. the `_EncounterCtx` equivalent — the live `Niche` carrier, the per-fight
//!    `Encounter` stream derived at **one** site, and the creation-order uid
//!    stamping `_EncounterCtx.done` performs;
//! 3. the roster → state admission edge;
//! 4. (turn-1 initial-RAND intent rolls, which the builders express as
//!    `MonsterSpec::opening_move` rather than rolling here.)
//!
//! Native authority: `EncounterModel::GenerateMonstersWithSlots` (v0.111.0 RVA
//! `0x7f88c`) and `CombatState::CreateCreature` (`0x137074`), read from the
//! archived `sts2.dll` sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.

use crate::encounters::{EncounterCtx, MonsterSpec, RosterRefusal, roster_builder};
use crate::ids::EncounterId;
use crate::rng::Xoshiro256StarStar;

/// The roster a builder produced, in creation order.
///
/// Creation order **is** slot order and **is** uid order: `_EncounterCtx.done`
/// stamps `m.uid = i` over the returned list (frozen Python, deleted #2827), and
/// the game appends each created creature to
/// `CurrentMapPointHistoryEntry.Rooms.Last().MonsterIds` in
/// `MonstersWithSlots` order (`CombatState::CreateCreature` `0x137074`). The
/// `.mcr` target ids are those uids, so a roster returned in another order
/// silently re-targets every recorded action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roster {
    pub monsters: Vec<MonsterSpec>,
    /// The `Niche` stream as it stands after creation. Carried into
    /// `State.niche` so mid-fight spawns continue the same stream
    /// (`make_monsters` docstring, frozen Python, deleted #2827).
    pub niche: Xoshiro256StarStar,
}

/// The `EncounterId` a wire id resolves to — by **substring**, not equality.
///
/// `EncounterId::NAMES` is the *registered key* axis
/// (`content_tables::ENCOUNTER_MATCH_ORDER`), and `make_monsters` matches a
/// key as a substring of the wire id:
///
/// ```text
/// for sub, builder in ENCOUNTER_BUILDERS:
///     if sub in encounter_id:
///         return builder(ctx)
/// ```
///
/// Ten of the twelve elite keys are proper substrings of theirs —
/// `ENCOUNTER.DECIMILLIPEDE_ELITE` resolves to `EncounterId::Decimillipede`.
/// That is not a hypothetical: an equality lookup here reported "unknown
/// encounter" for **84 of the corpus's 495 encountered fights** while the
/// totals still looked plausible, which is exactly the hazard
/// `ENCOUNTER_COVERAGE_CENSUS.md` names.
///
/// `make_monsters`' docstring *asserts* the registered keys are disjoint.
/// This **checks** it, the same way `tools/encounter_coverage_census.py`'s own
/// `dispatch` does: an id claimed by two keys refuses instead of taking the
/// first.
pub fn registered_key(encounter: &str) -> Result<EncounterId, EncounterDispatchRefusal> {
    let entry = strip_category(encounter);
    // An exact name wins outright: a wire id that *is* a registered key
    // resolves to itself without consulting the substring rule.
    if let Some(id) = EncounterId::from_str(entry) {
        return Ok(id);
    }
    let owners: Vec<EncounterId> = EncounterId::ALL
        .into_iter()
        .filter(|id| entry.contains(id.as_str()))
        .collect();
    match owners.len() {
        1 => Ok(owners[0]),
        0 => Err(EncounterDispatchRefusal::UnknownEncounterId(
            encounter.to_string(),
        )),
        _ => Err(EncounterDispatchRefusal::AmbiguousEncounterKey {
            encounter: owners[0].as_str(),
            keys: owners.into_iter().map(|id| id.as_str()).collect(),
        }),
    }
}

/// `"ENCOUNTER.SLIMES_WEAK"` → `"SLIMES_WEAK"`, the `ModelId.Entry` half.
///
/// This is the part `EncounterModel::GenerateMonstersWithSlots` hashes into
/// the per-fight stream seed. It is **not** always a registered key; see
/// [`registered_key`].
pub fn strip_category(encounter: &str) -> &str {
    encounter.rsplit('.').next().unwrap_or(encounter)
}

/// Why a wire id could not be turned into a roster.
///
/// Separate from [`RosterRefusal`], which is the content lane's vocabulary for
/// a builder that ran and could not finish exactly. These are reached before
/// any builder is called.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EncounterDispatchRefusal {
    /// The wire id resolves to no registered key, so the build's own id axis
    /// does not name it.
    UnknownEncounterId(String),
    /// Two registered keys are substrings of the same wire id.
    AmbiguousEncounterKey {
        encounter: &'static str,
        keys: Vec<&'static str>,
    },
    /// The id resolves, and `content_tables::ENCOUNTER_ROSTER_BUILDERS` has no
    /// builder for it. The E4b frontier, named per fight.
    ///
    /// I13 "not yet": the family PR that registers the key answers it.
    EncounterNotBuilt(EncounterId),
}

/// The live context a real opening hands a builder.
///
/// Implements the content lane's [`EncounterCtx`]: owns the `Niche` stream, so
/// the surviving state can be carried into `State.niche`, and derives the
/// per-fight `Encounter` stream lazily at the one site §A8.3 requires. A
/// family module that re-derived that seed would duplicate the build switch
/// #637 spent an issue collapsing into one place, which is why the trait
/// exposes the *draw* and never the stream.
pub struct LiveEncounterCtx<'a> {
    niche: Xoshiro256StarStar,
    /// `RunRngSet.Seed` — the 64-bit run set seed, one of the three addends of
    /// the per-fight stream.
    run_set_seed: u64,
    /// `RunState.TotalFloor`, `None` when the entry could not derive it.
    total_floor: Option<i64>,
    /// The **wire** id's `Entry` half, which is what the seed hashes.
    ///
    /// Borrowed from the caller's wire id, never taken from the dispatched
    /// [`EncounterId`]: `_EncounterCtx.encounter_rng` hashes
    /// `self.encounter_id.split(".")[-1]` of the id `make_monsters` was
    /// *called* with (frozen Python, deleted #2827), and that id is a proper
    /// superstring of the registered key for ten of the twelve elite
    /// encounters. See [`strip_category`] and [`registered_key`].
    entry: &'a str,
    encounter_stream: Option<Xoshiro256StarStar>,
    /// `RunState.AscensionLevel`, read from the save (#2539): the tier every
    /// builder selects its HP bands and spawn amounts by.
    ascension: u8,
}

impl<'a> LiveEncounterCtx<'a> {
    pub fn new(
        niche: Xoshiro256StarStar,
        run_set_seed: u64,
        total_floor: Option<i64>,
        entry: &'a str,
        ascension: u8,
    ) -> Self {
        Self {
            niche,
            run_set_seed,
            total_floor,
            entry,
            encounter_stream: None,
            ascension,
        }
    }

    /// The surviving `Niche` state, for `State.niche`.
    pub fn into_niche(self) -> Xoshiro256StarStar {
        self.niche
    }
}

/// `Rng.next_int(lo, hi)`: `int(NextDouble() * (hi - lo)) + lo`, one draw.
///
/// The game throws `"Minimum must be lower than maximum."` when `lo >= hi`.
/// The content lane's `niche_next_int` is infallible by contract, so an
/// inverted range is a programming error in a builder rather than a fact about
/// a fight, and it panics here instead of silently returning `lo`.
fn draw(stream: &mut Xoshiro256StarStar, lo: i32, hi: i32) -> i32 {
    assert!(
        lo < hi,
        "next_int({lo}, {hi}): minimum must be lower than maximum"
    );
    stream
        .next_bounded(hi - lo)
        .expect("the span is positive, so the bound is valid")
        + lo
}

impl EncounterCtx for LiveEncounterCtx<'_> {
    fn ascension(&self) -> u8 {
        self.ascension
    }

    fn niche_next_int(&mut self, lo: i32, hi: i32) -> i32 {
        draw(&mut self.niche, lo, hi)
    }

    fn encounter_next_int(
        &mut self,
        roll: &'static str,
        lo: i32,
        hi: i32,
    ) -> Result<i32, RosterRefusal> {
        if self.encounter_stream.is_none() {
            let floor = self
                .total_floor
                .ok_or(RosterRefusal::EncounterStreamUnavailable { roll })?;
            self.encounter_stream = Some(crate::rng::encounter_stream(
                self.run_set_seed,
                floor,
                self.entry,
            ));
        }
        let stream = self
            .encounter_stream
            .as_mut()
            .expect("the per-fight Encounter stream was just derived");
        Ok(draw(stream, lo, hi))
    }
}

/// `combat_sim.make_monsters`: create the encounter's roster, replaying the
/// `Niche` stream for HP.
///
/// The two streams this consumes — `Niche` and the per-fight `Encounter` — are
/// disjoint from `Shuffle`, which is the whole reason the opening may create
/// monsters *after* instantiating the deck even though the game creates them
/// before (`CombatRoom::StartCombat` `0x58d4c` runs
/// `GenerateMonstersWithSlots` `0x7f88c` before `CombatManager::SetUpCombat`
/// `0x135900`). The moment an opening step consumes two streams that freedom
/// disappears.
///
/// The **two** id axes are kept apart on purpose, and conflating them is a
/// silent-divergence defect rather than a cosmetic one: the builder is chosen
/// by [`registered_key`]'s substring rule, while the per-fight `Encounter`
/// stream is seeded from [`strip_category`] of the id this function was
/// *called* with. `ENCOUNTER.DECIMILLIPEDE_ELITE` therefore dispatches to
/// `Decimillipede`'s builder and hashes `DECIMILLIPEDE_ELITE` — exactly what
/// `_EncounterCtx.encounter_rng` does with `self.encounter_id`
/// (frozen Python, deleted #2827), which `make_monsters` sets from its own
/// `encounter_id` parameter (`_roll_unique_hp`). Seeding from the key instead moves the
/// starter-move roll to a different stream and silently re-rosters the fight.
pub fn make_monsters(
    encounter: &str,
    niche: Xoshiro256StarStar,
    run_set_seed: u64,
    total_floor: Option<i64>,
    ascension: u8,
) -> Result<Result<Roster, RosterRefusal>, EncounterDispatchRefusal> {
    let id = registered_key(encounter)?;
    let builder = roster_builder(id).ok_or(EncounterDispatchRefusal::EncounterNotBuilt(id))?;
    let mut ctx = LiveEncounterCtx::new(
        niche,
        run_set_seed,
        total_floor,
        strip_category(encounter),
        ascension,
    );
    Ok(match builder(&mut ctx) {
        Ok(monsters) => Ok(Roster {
            monsters,
            niche: ctx.into_niche(),
        }),
        Err(refusal) => Err(refusal),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// High-tier review finding P1-1 (#2544): the `Encounter` stream is seeded
    /// from the **wire** id, not from the registered key it dispatched to.
    ///
    /// Vectors printed by the oracle, not re-derived from the same expression:
    ///
    /// ```text
    /// python3.12 -c "import combat_sim as s, sts2_rng as r
    /// b='v0.111.0'; seed='ZPJHU3WSH2'; floor=8
    /// rs = r.RunRngSet(seed, build=b); print(rs.seed, rs['Niche'].seed)
    /// for e in ('ENCOUNTER.DECIMILLIPEDE_ELITE', 'ENCOUNTER.DECIMILLIPEDE'):
    ///     ms, _ = s.make_monsters(e, seed, 0, floor, build=b)
    ///     print(e, [(m.hp, m.loop_pos) for m in ms])"
    /// ```
    ///
    /// prints `loop_pos` `[1, 0, 2]` for the wire id and `[0, 2, 1]` for the
    /// bare key. Seeding from `id.as_str()` produced the second list — a
    /// rostered fight that never happened, with no refusal anywhere.
    #[test]
    fn the_encounter_stream_hashes_the_wire_entry_not_the_registered_key() {
        // The oracle's own `RunRngSet` numbers for `ZPJHU3WSH2` at v0.111.0.
        let run_set_seed = crate::rng::run_set_seed_v109("ZPJHU3WSH2");
        assert_eq!(run_set_seed, 11_137_324_208_666_220_282);
        let niche = Xoshiro256StarStar::from_seed(12_800_230_507_224_994_239);
        assert_eq!(
            niche.words,
            [
                14_073_480_017_042_375_735,
                2_014_339_587_019_794_725,
                10_333_115_920_123_273_490,
                7_680_740_182_681_148_870,
            ],
            "the Niche stream this pin rides on is the oracle's"
        );

        let elite = make_monsters(
            "ENCOUNTER.DECIMILLIPEDE_ELITE",
            niche,
            run_set_seed,
            Some(8),
            crate::encounters::MODELED_ASCENSION,
        )
        .expect("the elite wire id dispatches to the Decimillipede builder")
        .expect("the builder has every fact it needs");
        assert_eq!(
            elite
                .monsters
                .iter()
                .map(|m| m.loop_pos)
                .collect::<Vec<_>>(),
            vec![1, 0, 2],
            "the starter roll must come off the DECIMILLIPEDE_ELITE stream"
        );
        assert_eq!(
            elite.monsters.iter().map(|m| m.hp).collect::<Vec<_>>(),
            vec![52, 46, 48]
        );

        // The bare key is a *different* fight, which is what makes the two
        // axes non-interchangeable rather than merely untidy.
        let bare = make_monsters(
            "ENCOUNTER.DECIMILLIPEDE",
            niche,
            run_set_seed,
            Some(8),
            crate::encounters::MODELED_ASCENSION,
        )
        .expect("the bare key is also registered")
        .expect("the builder has every fact it needs");
        assert_eq!(
            bare.monsters.iter().map(|m| m.loop_pos).collect::<Vec<_>>(),
            vec![0, 2, 1]
        );
    }

    #[test]
    fn the_live_ctx_carries_the_wire_entry_half() {
        let mut ctx = LiveEncounterCtx::new(
            Xoshiro256StarStar::from_seed(1),
            42,
            Some(3),
            strip_category("ENCOUNTER.DECIMILLIPEDE_ELITE"),
            crate::encounters::MODELED_ASCENSION,
        );
        let drawn = ctx.encounter_next_int("starter", 0, 3).unwrap();
        let mut expected = crate::rng::encounter_stream(42, 3, "DECIMILLIPEDE_ELITE");
        assert_eq!(drawn, expected.next_bounded(3).unwrap());
        assert_eq!(ctx.encounter_stream.expect("derived").words, expected.words);
        // And it is genuinely a different stream from the registered key's.
        assert_ne!(
            crate::rng::encounter_stream(42, 3, "DECIMILLIPEDE_ELITE").words,
            crate::rng::encounter_stream(42, 3, "DECIMILLIPEDE").words
        );
    }

    #[test]
    fn a_wire_id_resolves_to_its_registered_key_by_substring() {
        // The hazard the coverage census names: the key is a proper substring
        // of the wire id for ten of the twelve elite encounters.
        assert_eq!(
            registered_key("ENCOUNTER.DECIMILLIPEDE_ELITE").unwrap(),
            EncounterId::Decimillipede
        );
        // An id that IS a registered key still resolves to itself.
        assert_eq!(
            registered_key("ENCOUNTER.TOADPOLES_WEAK").unwrap(),
            EncounterId::ToadpolesWeak
        );
    }

    #[test]
    fn every_registered_key_resolves_to_itself_and_only_itself() {
        // The disjointness `make_monsters` asserts, checked over the whole
        // 88-id axis rather than on the one id a test happened to pick.
        for id in EncounterId::ALL {
            assert_eq!(
                registered_key(&format!("ENCOUNTER.{}", id.as_str())).unwrap(),
                id,
                "{} does not resolve to itself",
                id.as_str()
            );
        }
    }

    #[test]
    fn an_id_outside_the_axis_refuses_by_name() {
        assert_eq!(
            registered_key("ENCOUNTER.NOT_A_REAL_ENCOUNTER"),
            Err(EncounterDispatchRefusal::UnknownEncounterId(
                "ENCOUNTER.NOT_A_REAL_ENCOUNTER".to_string()
            ))
        );
    }

    #[test]
    fn the_entry_half_of_a_wire_id_is_what_seeds_the_stream() {
        assert_eq!(strip_category("ENCOUNTER.SLIMES_WEAK"), "SLIMES_WEAK");
        assert_eq!(strip_category("SLIMES_WEAK"), "SLIMES_WEAK");
    }

    #[test]
    fn the_encounter_stream_refuses_without_a_floor() {
        let mut ctx = LiveEncounterCtx::new(
            Xoshiro256StarStar::from_seed(1),
            42,
            None,
            EncounterId::Decimillipede.as_str(),
            crate::encounters::MODELED_ASCENSION,
        );
        assert_eq!(
            ctx.encounter_next_int("starter move offset", 0, 3),
            Err(RosterRefusal::EncounterStreamUnavailable {
                roll: "starter move offset"
            })
        );
    }

    #[test]
    fn the_encounter_stream_is_derived_once_and_then_continues() {
        let mut ctx = LiveEncounterCtx::new(
            Xoshiro256StarStar::from_seed(1),
            42,
            Some(3),
            EncounterId::Decimillipede.as_str(),
            crate::encounters::MODELED_ASCENSION,
        );
        ctx.encounter_next_int("first", 0, 100).unwrap();
        ctx.encounter_next_int("second", 0, 100).unwrap();
        // Two draws on ONE stream, not two streams at counter 1.
        assert_eq!(ctx.encounter_stream.expect("derived").counter, 2);
    }

    #[test]
    fn a_fixed_hp_monster_still_spends_its_niche_draw() {
        // `_EncounterCtx.fixed` spends `next_int(0, 1)` explicitly
        // (frozen Python, deleted #2827) — one Niche draw per monster, fixed-HP
        // monsters included. Skipping it would desynchronise every later
        // monster and every mid-fight spawn.
        let mut ctx = LiveEncounterCtx::new(
            Xoshiro256StarStar::from_seed(7),
            0,
            None,
            EncounterId::ToadpolesWeak.as_str(),
            crate::encounters::MODELED_ASCENSION,
        );
        ctx.niche_next_int(0, 1);
        assert_eq!(ctx.into_niche().counter, 1);
    }

    #[test]
    fn unique_hp_excludes_taken_values_and_spends_exactly_one_draw() {
        let mut ctx = LiveEncounterCtx::new(
            Xoshiro256StarStar::from_seed(3),
            0,
            None,
            EncounterId::ToadpolesWeak.as_str(),
            crate::encounters::MODELED_ASCENSION,
        );
        // Only 11 survives the exclusion, so the single draw cannot change it.
        assert_eq!(ctx.unique_hp(10, 12, &[10, 12]), 11);
        assert_eq!(ctx.into_niche().counter, 1);
    }
}

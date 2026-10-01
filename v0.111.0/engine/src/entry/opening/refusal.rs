//! The typed I5 refusal vocabulary of the opening (#2528 E4a).
//!
//! A fourth enum, sibling to [`crate::entry::refusal::EntryRefusal`],
//! [`crate::encounters::RosterRefusal`], [`crate::boundary::BoundaryRefusal`]
//! and the engine's [`crate::engine::EngineRefusal`] rather than a widening of
//! any of them. The boundary between them is *when* they are reached:
//!
//! | enum | reached |
//! |---|---|
//! | `EntryRefusal` | before any RNG exists — the save's own facts |
//! | `OpeningRefusal` | while the run RNG is being consumed |
//! | `RosterRefusal` | inside encounter creation |
//! | `BoundaryRefusal` | admitting the state the opening built |
//! | `EngineRefusal` | firing a hook or dealing the first hand |
//!
//! The last three are **carried**, not restated: a wrapped refusal keeps its
//! own name and detail, so a census can tell an unbuilt encounter from an
//! unrepresentable state from an unmodeled hook body.
//!
//! # The rule this vocabulary exists to enforce
//!
//! From the E4 spec walk (I5, Part C): *"a class whose fact **is** in the save
//! must be admitted **or** carry a Rust-named refusal — never a transcribed
//! Python one"*. Every variant below either names a mechanic this port does
//! not run, or names a decision that has not been taken. None of them is
//! "Python refused, so we refuse".

use std::fmt;

use crate::boundary::BoundaryRefusal;
use crate::encounters::RosterRefusal;
use crate::engine::EngineRefusal;
use crate::entry::opening::roster::EncounterDispatchRefusal;

/// Why a fight's opening could not be built exactly.
#[derive(Clone, Debug, PartialEq)]
pub enum OpeningRefusal {
    /// The wire id could not be turned into a registered key, or no builder
    /// is registered for the key it resolves to.
    ///
    /// The E4b frontier, named per fight. Reached before any builder runs.
    EncounterDispatch(EncounterDispatchRefusal),
    /// A registered builder ran and could not finish exactly.
    ///
    /// The content lane's own vocabulary (`encounters::RosterRefusal`),
    /// carried rather than restated so a census can tell an unread monster
    /// model from a missing floor.
    Roster(RosterRefusal),
    /// The opening built a state the canonical ⇄ hot boundary cannot
    /// represent.
    Boundary(BoundaryRefusal),
    /// A combat-start hook body or the first hand draw reached a mechanic the
    /// engine does not implement.
    ///
    /// This is where the spec's `suspending_listener` class lands: a
    /// continuation through the turn-start listener walk is engine structure,
    /// not an entry or content question (Part C7).
    Engine(EngineRefusal),

    // ---- the 177's per-class dispositions -------------------------------
    /// A generated-card source whose per-character pool this port does not
    /// model.
    ///
    /// Part C1, and the **open decision**: the class Python calls
    /// `owner_provenance` is misnamed. `players[0].character_id` is recorded
    /// on 3,092 of 3,092 schema-20 saves, so the character is known; what is
    /// missing is the Necrobinder / Defect / Silent generation pools, which
    /// are DLL facts belonging in `dll_content.py` codegen. Until that scope
    /// call is taken (option (a) per-generator pool extraction, or option (b)
    /// this refusal as the permanent answer) the pools are **not** ported
    /// here, and the fight refuses by a name that says which generator and
    /// which character.
    ///
    /// The genuinely-unknowable case is different and must not be collapsed
    /// with this one: a **multiplayer** deck can hold a teammate's card, and
    /// `SerializableCard` carries no `Owner`. No fight in the corpus is
    /// multiplayer (512/512 single-player), so that case is unreachable here —
    /// but it is "never", where this is "not yet" (I13).
    GenerationPoolUnmodeled { source: String, character: String },
    /// A generation pool whose reachable card set depends on an unlock profile
    /// that is recorded but not modeled.
    ///
    /// Part C3: all 25 `epoch_provenance` refusals name *missing* epochs, not
    /// absent provenance — `unlock_state.unlocked_epochs` is present on
    /// 3,092/3,092 saves. Building the pool from the recorded epoch set is
    /// admissible work; it is one piece of work with the
    /// `splash_unlock_epochs` partial-unlock profile and is not taken here.
    ///
    /// Since #3336 the owner-pool generators reach this only without a
    /// recorded profile, and Infernal Blade (whose engine pool is frozen) also
    /// under a profile hiding an Ironclad gating epoch
    /// (`opening::owner_pool_generation_profile_is_recorded`).
    GenerationPoolPartialUnlock {
        source: String,
        missing: Vec<String>,
    },
    /// A deck card whose `BeforeHandDraw` listener draws from its owner's pool
    /// (`HELLO_WORLD`, `CALL_OF_THE_VOID`, `CREATIVE_AI`) on a fight that lacks
    /// what the pool's provenance needs: an owner with a character card pool
    /// (any character since #3375; the frozen Python `start_combat`, deleted
    /// #2827, demanded the card's own), or a recorded unlock profile.
    /// The opening writes that provenance
    /// (`entry::opening::OWNER_LISTENER_POOL_FIELDS`), so it refuses here
    /// rather than emitting a document without it.
    ListenerPoolProvenance {
        source: &'static str,
        requires: &'static str,
    },
    /// Counter-relic dispatch cannot be represented under the recorded order.
    ///
    /// Part C4. The order *is* recorded at schema >= 19 and
    /// `relics_entering_dispatch_ordered` vouches for it; this refusal fires
    /// only where the recorded order places a same-hook peer somewhere the
    /// fixed counter-relic suffix cannot represent. Ported from
    /// frozen Python `start_combat`, deleted #2827, so the *shape* of the gate is the oracle's
    /// and the *name* is Rust's.
    RelicDispatchOrderUnmodeled {
        hook: &'static str,
        relics: Vec<String>,
    },
    /// A relic with a body on one of the hooks the opening and the first deal
    /// run, and no Rust subscriber to receive it.
    ///
    /// The two lists in `entry::opening` — `OPENING_WINDOW_RELIC_BODIES`,
    /// derived from `combat_sim._current_relic_hooks`, and
    /// `TURN_ONE_ORB_RELICS` — carry the derivation and the measurement. The
    /// short version: this crate reaches a subscriber for those hooks only
    /// through `content_tables::TEMPLATE_RELIC_STEPS`, so firing them for a
    /// hand-authored relic walks an empty list and the opening emits a state
    /// the oracle disagrees with, **silently**. Before this refusal existed,
    /// 14 of 14 corpus fights holding `RELIC.CRACKED_CORE` and 5 of 5 holding
    /// `RELIC.VAJRA` did exactly that. (Vajra's body has since been modeled,
    /// #2827.)
    ///
    /// Named per relic rather than as one class, so the census says which body
    /// is missing rather than how many fights hold one.
    RoomEntryRelicNotModeled { relic: String },
    /// A relic whose **turn-1** body the opening runs and this crate has no
    /// subscriber for (#2731).
    ///
    /// A distinct variant rather than a reuse of
    /// [`Self::RoomEntryRelicNotModeled`], on two grounds. The name would
    /// mislead: these bodies do not run at room entry, they run inside
    /// `start_combat`'s closing `begin_player_turn`, and a reader chasing a
    /// census row to the wrong half of the opening is exactly the cost a typed
    /// vocabulary exists to avoid. And the *reason* differs in a way a census
    /// should be able to count: the five-hook table is about a template
    /// subscriber this crate never compiles, while these seven relics have no
    /// Rust body anywhere — including for the post-opening turn walk. The
    /// follow-ups that retire them are per-relic modeling slices, and a
    /// separate class is how the census shows that wall coming down.
    ///
    /// `entry::opening::TURN_ONE_RELIC_BODIES` carries the derivation, the
    /// per-relic IL citation for the turn-1 guard, and the argument that a
    /// post-opening root is unaffected.
    TurnOneRelicBodyNotModeled { relic: String },
    /// A turn-1 `AfterPlayerTurnStart` relic co-owned with a same-hook peer
    /// whose relative order the oracle refuses to guess (#2827).
    ///
    /// Native dispatches `Hook::AfterPlayerTurnStart` relic listeners in
    /// `Player.Relics` order, which the run payload does not vouch for, while
    /// both engines run the hook as one fixed suffix
    /// (`combat_sim._continue_after_toasty_mittens`,
    /// `engine::relics::continue_after_toasty_mittens`). `start_combat` refuses
    /// the co-ownerships where that fixed order is observable: Gambling Chip's
    /// and Toasty Mittens' Hand choices, the generation sources' Hand writes,
    /// Bone Tea's second pass, and a second enemy-damage body or Royal Poison
    /// beside Festive Popper. `entry::opening`'s
    /// `after_player_turn_start_peers_are_ordered` carries the line citations.
    /// `owner` is the modeled relic whose body the gate protects; `peers` are
    /// the co-owned relics that make the order observable.
    ///
    /// Since #2884 two pairs read their order from a vouched inventory instead:
    /// Choices Paradox before Bellows, and Festive Popper beside Toasty Mittens
    /// in either order. For the first pair the refusal also covers a vouched
    /// inventory whose recorded order (Bellows first) this engine does not run.
    TurnStartRelicOrderUnrecorded {
        owner: &'static str,
        peers: Vec<String>,
    },
    /// Turn-1 all-enemy debuff relics meeting a monster whose entering
    /// `ArtifactPower` blocks **some but not all** of them (#2693).
    ///
    /// The oracle's own `partial_artifact` gate, ported rather than
    /// approximated: frozen Python `start_combat` (deleted #2827) refuses when any created
    /// monster has `0 < m.artifact < len(relics & _TURN_ONE_ALL_ENEMY_DEBUFF_RELICS)`.
    /// Artifact consumes one stack per blocked **application**, so which of the
    /// owner's debuffs lands is decided by their position in `Player.Relics`
    /// (`CombatState::IterateHookListeners` `0x137409`), and the run payload
    /// does not vouch for relic acquisition order on a `BeforeSideTurnStart`
    /// walk. A guess there is a different document, not a slower one.
    ///
    /// A distinct variant rather than a reuse of
    /// [`Self::TurnOneRelicBodyNotModeled`]: the bodies **are** modeled
    /// (`engine::relics::turn_one_all_enemy_debuffs`), and what is missing is an
    /// ordering fact about the inventory — the same distinction
    /// [`Self::RelicDispatchOrderUnmodeled`] draws for `AfterCardPlayed`.
    /// `entry.relic_entry.dispatch_ordered` is a possible future narrowing and
    /// is deliberately not read here: the oracle refuses unconditionally, and
    /// matching it keeps this port on the over-refusal side of a fact no
    /// witness has yet pinned.
    TurnOneDebuffPartialArtifact {
        /// The owned debuff relics, sorted — the `len` the oracle compares.
        relics: Vec<String>,
        /// `(slot, kind, artifact)` per offending monster, the oracle's tuple.
        monsters: Vec<String>,
    },
    /// A relic whose **per-fight state** `start_combat` seeds and
    /// `entry::opening::pre_hook_document` does not write (#2693, narrowed by
    /// #2756).
    ///
    /// A third class rather than a reuse of either neighbour, because it is
    /// neither a missing subscriber nor a missing body: for four of the six the
    /// engine body existed and read a flag the opening left at its `State`
    /// default, so the relic was silently **inert for the whole fight** while its
    /// immutable inventory mirror said it was owned. Nothing downstream can
    /// notice — the document is well-formed, admissible and wrong.
    ///
    /// Found by measurement, not by reading: the room-entry relic bodies #2755
    /// retired let two corpus fights past the gate, and both came back
    /// digest-divergent from Python on exactly
    /// `player.throwing_axe_available`. A per-relic sweep of the oracle
    /// (add one relic to the fixture save, diff Python's projected player
    /// fields against Rust's) then found six.
    ///
    /// **#2756 modeled five of the six** — the four flags in
    /// `entry::opening::PER_FIGHT_RELIC_STATE_SEEDED`, plus Blood Vial's turn-1
    /// heal in `engine::relics::after_player_turn_start_late`. What still
    /// reaches this variant is `entry::opening::PER_FIGHT_RELIC_STATE_UNSEEDED`,
    /// empty since #3320 modeled its last member, `RELIC.DRAGON_FRUIT` (no
    /// instance field; its `dragon_fruit` is a catalog ownership mirror). The
    /// variant stays for the #2779 field derivation.
    PerFightRelicStateNotSeeded { relic: String, field: &'static str },
    /// `RELIC.RUINED_HELMET` with two room-entry Strength sources of
    /// **different** amounts.
    ///
    /// Each source is its own positive `Apply<StrengthPower>` inside the
    /// acquisition-ordered `AfterRoomEntered` relic group, and Ruined Helmet
    /// doubles only the first positive application it sees
    /// (`RuinedHelmet::TryModifyPowerAmountReceived`, and
    /// `engine::damage::apply_owner_strength`'s latch). The run payload does
    /// not preserve acquisition order, so with distinct amounts the final
    /// Strength depends on an unrecorded fact. Equal amounts commute — either
    /// ordering adds the same one extra time — which is why the gate is on
    /// *distinctness* and not on the count.
    RuinedHelmetStrengthOrder { sources: Vec<String> },
    /// `RELIC.SYMBIOTIC_VIRUS`'s turn-1 Dark channel beside a same-hook peer
    /// whose order around it is observable (#2827).
    ///
    /// The oracle's four `start_combat` refusals (frozen Python, deleted #2827),
    /// by the same conditions: Infused Core at any capacity; Runic Capacitor
    /// at `BaseOrbSlotCount` 0; Cracked Core at `BaseOrbSlotCount` 0 with
    /// Fencing Manual or Brimstone. In each, whether the Dark channel first
    /// bootstraps the queue, overflows it, or auto-evokes a Lightning that can
    /// kill before another listener's snapshot depends on relic acquisition
    /// order, which the run payload does not record. `engine::admission`
    /// carries the same four by name for a loaded document; the opening does
    /// not run admission, so it refuses them itself.
    SymbioticVirusTurnStartOrder { peers: Vec<String> },
    /// A relic combination `start_combat` refuses outright, whatever the fight
    /// (#2827).
    ///
    /// Each is ported from one oracle gate in `start_combat`'s pure
    /// construction half, where the reason is either an ownership the game
    /// cannot produce or a same-hook acquisition order the run payload does
    /// not record. The opening does not run `engine::admission`, so a
    /// combination the oracle refuses has to refuse here, or the opening would
    /// emit a document for a fight Python never roots. `reason` is the
    /// oracle's own reason, shortened; `entry::opening`'s
    /// `refuse_incompatible_phylacteries`,
    /// `refuse_unordered_kusarigama_peers` and
    /// `refuse_unordered_blessed_antler_peers` carry the citations.
    RelicCombinationRefused {
        relics: Vec<String>,
        reason: &'static str,
    },
    /// A relic that heals on room entry, where the node type decides whether
    /// it fires.
    ///
    /// Part C6, narrowed by #3162: PLANISPHERE on an `unknown` (`?`) map
    /// point, or one the save does not name (`entry::opening`'s
    /// `planisphere_heals_here`, `Planisphere/<AfterRoomEntered>d__5::MoveNext`
    /// `0x32e318`). Whether the entry save's HP already carries that heal, and
    /// whether the fight is the point's first room, are not established.
    /// PANTOGRAPH's boss-room heal left this refusal in #3162: the opening runs
    /// it (`pantograph_before_combat_start`).
    RoomEntryHealUnmodeled { relic: &'static str },
    /// `RELIC.STONE_CRACKER` over a draw-pile card whose upgrade ladder this
    /// crate does not carry.
    ///
    /// The hook filters the pile by `CardModel::get_IsUpgradable` (RVA
    /// `0x7cee5`) before its `CombatCardSelection` shuffle, so one card whose
    /// `MaxUpgradeLevel` is unknown changes the pool's length and with it the
    /// stream's advance. The oracle refuses the same fight at its ladder gate
    /// (`MaxUpgradeLevel unknown`, frozen Python `start_combat`, deleted #2827).
    StoneCrackerUpgradeLadderUnknown { card: String },
    /// `RELIC.VENERABLE_TEA_SET` or `RELIC.FAKE_VENERABLE_TEA_SET` whose
    /// saved `GainEnergyInNextCombat` the entry could not read as an exact
    /// bool, so whether its turn-1 `AfterEnergyReset` grant fires is unknown
    /// (frozen Python `start_combat`, deleted #2827).
    TeaSetChargeUndated { relic: &'static str },
    /// `RELIC.BOOMING_CONCH` on a fight whose node type is unknown, so whether
    /// it is an elite combat cannot be decided.
    BoomingConchRoomTypeUnknown,
    /// `RELIC.SLING_OF_COURAGE` on a fight whose node type is unknown, so
    /// whether it is an elite combat, and whether its Strength applies, cannot
    /// be decided (frozen Python `start_combat`, deleted #2827).
    SlingOfCourageRoomTypeUnknown,
    /// A relic with a persistent entering counter this fight does not carry.
    ///
    /// `seeded()` (frozen Python `start_combat`, deleted #2827): the counter is run state, and
    /// a guessed one silently changes which turn the relic fires on.
    RelicCounterUnseeded { relic: &'static str },
    /// A relic whose persistent counter is present but not an exact
    /// non-negative integer.
    RelicCounterNotExact { relic: &'static str, value: String },
    /// A relic that procures into the potion belt on a fight whose belt
    /// carries no positive `max_potion_slot_count`. (A slot row outside the
    /// recorded capacity is [`Self::PotionBeltRowMalformed`], for every
    /// fight, since #2791.)
    ///
    /// `PotionCmd::TryToProcure` fills the **first empty** slot, so the
    /// result depends on the belt's trailing empties, which only the recorded
    /// capacity names. The oracle refuses the same fight
    /// (`combat_sim.start_combat`'s `entry_potion_relics` gate, frozen Python, deleted #2827, through `_potion_slots_from_entry`).
    PotionBeltNotExact {
        relic: &'static str,
        capacity: Option<i64>,
    },
    /// A potion-belt row the native loader cannot place: no `slot_index`, a
    /// negative one, or one at or past the recorded `max_potion_slot_count`.
    ///
    /// `Player.LoadPotions` (`0x117a0c`, `IL_0035`–`IL_0043`) hands each
    /// saved row's `SlotIndex` to `Player.AddPotionInternal` (`0x117670`),
    /// against a `_potionSlots` list that `SetMaxPotionCountInternal`
    /// (`0x117584`, `IL_0028`–`IL_003a`) grew with nulls to exactly the
    /// recorded capacity. There a negative index is redirected to the first
    /// empty slot (`IL_001f`–`IL_002f`) and one at or past the count throws
    /// out of `List.get_Item` (`IL_003f`). The game never writes any of the
    /// three: `SerializablePotion.Serialize` (`0x41609`, `IL_000f`–`IL_0015`)
    /// always writes the index, and `Player.ToSerializable` (`0x116dec`,
    /// `IL_0052`) writes the capacity as `_potionSlots.Count`. Eliding the
    /// belt instead would open a well-formed document that silently lost the
    /// saved potions (#2791).
    PotionBeltRowMalformed {
        potion: String,
        slot_index: Option<i64>,
        capacity: Option<i64>,
    },
    /// An orb-channelling fight whose character is undetermined.
    ///
    /// `BaseOrbSlotCount` is 3 for Defect (`0x280b6f`) and 0 for every other
    /// character (`CharacterModel` default `0x22960e`), so an undetermined
    /// character cannot be defaulted to 0 without mis-modelling a Defect fight
    /// (frozen Python `start_combat`, deleted #2827).
    OrbBaseSlotsUndetermined,
    /// The save's `ascension` is absent or not a v0.111.0 `AscensionLevel`
    /// (`0..=10`). Every ascension-tiered monster constant is selected by it
    /// (#2539), so defaulting it would silently pick a roster's HP tier;
    /// `start_combat` refuses the same entry (`combat_sim.MAX_ASCENSION`).
    AscensionNotExact { recorded: Option<i64> },
    /// A node whose `total_floor` could not be derived, on a fight that needs
    /// it. `total_floor = node_index + 1` (frozen Python `start_combat`, deleted #2827).
    TotalFloorUnknown,
    /// One of the nine combat streams a consumer in this fight reads is absent
    /// from the save.
    CombatStreamAbsent { stream: &'static str },
    /// The `--mcr` splice was requested and the capture's first checksum is
    /// not the opening state.
    ///
    /// `mcr_validate.checksum_opening_state` (`mcr_validate.py:512-536`)
    /// requires the first checksum's context to be exactly
    /// `"After player turn start"`, then overwrites all nine RNG attributes
    /// from it. A splice against any other context would install a mid-fight
    /// stream into an opening state.
    McrChecksumContext { context: String },
    /// The `--mcr` payload did not carry all nine streams of the first
    /// checksum's `full_state.rng` block.
    McrChecksumIncomplete { stream: &'static str },
    /// A creature whose turn-1 intent is rolled during combat setup is
    /// outside the exact domain the opening's roll covers.
    ///
    /// `_initialize_random_ai` (frozen Python, deleted #2827) walks the living
    /// INITIAL-RAND instances in roster order and rolls each one's opening
    /// move off the shared `MonsterAi` stream. At v0.111.0 the set is
    /// `{FABRICATOR, FLYCONID, LEAF_SLIME_S}` plus `(EXOSKELETON, slot 3)`,
    /// and `roll_initial_random_ai` ports every one of them from the DLL. It
    /// refuses an enrolled instance that enters with a non-empty move state
    /// (every weight reading it relies on is for an empty `StateLog`), and a
    /// Fabricator whose setup roster already holds four living enemies (the
    /// DISINTEGRATE follow no corpus roster reaches): a wrong roll both
    /// mis-states the intent and desynchronises `MonsterAi` for the whole
    /// fight.
    InitialRandomAiNotModeled { kind: &'static str },
    /// The entering deck holds an Innate card, or one enchanted `IMBUED`.
    ///
    /// `_apply_turn_one_pile_fixups` (frozen Python `_commit_live_card_piles`, deleted #2827) rewrites the
    /// draw pile at `SetupPlayerTurn`'s seam — `IMBUED` copies to the bottom,
    /// then the Innate ones reversed onto the front — and sets
    /// `innate_min_draw`. Ported when the deck's keywords come from its card
    /// specs; this refusal covers the case a card's **effective** keywords
    /// differ from its spec's, which the opening cannot see.
    InnateKeywordProvenance { card: String },
    /// A deck card whose saved per-instance `props` are outside the exact
    /// native domain `start_combat` validates.
    ///
    /// The three cards `entry::deck::PER_INSTANCE_PROP_CARDS` carries have
    /// saved mutable/immutable native fields, and `start_combat` checks each
    /// one's exact names, types and `Current == base + Increased` relation
    /// before instantiating the copy. Guessing past a payload that fails that
    /// check would enter combat with a card whose growth is invented.
    CardEntryPropsNotExact { card: &'static str, detail: String },
    /// A deck card whose saved per-instance variant this port cannot play.
    ///
    /// `MAD_SCIENCE` carries two immutable saved integers (`TinkerTimeType`,
    /// `TinkerTimeRider`) that make each copy one of eighteen different cards
    /// ([`crate::catalog::MadScienceVariant`], #2942). A legal variant whose
    /// body is not ported
    /// ([`crate::content_tables::MadScienceVariantRow::unmodeled`]: Improvement
    /// since #3427 ported Curious), or a deck whose copies carry two different
    /// variants (which the fight-level variant axis does not represent),
    /// refuses here by name rather than entering combat as a different card.
    CardEntryVariantNotModeled { card: &'static str, detail: String },
    /// Card identity allocation diverged from the oracle's.
    ///
    /// The opening stamps physical-card uids before the hooks, in the order
    /// the oracle's first `_normalize_card_identities` sees: the post-fixup
    /// pile, or the shuffled pile for a Ghost Seed owner, whose body
    /// allocates ahead of the fixup (`stamp_card_identities`). That equals
    /// the oracle's allocation **only while the deal is a prefix draw of that
    /// pile**. The premise is checked after the deal rather than assumed; a
    /// hook that reorders or inserts cards makes it false, as does a fixup
    /// that reordered a Tea of Discourtesy owner's deck (held to the shuffled
    /// order), and a silently different uid re-targets every recorded action
    /// in a `.mcr` line.
    CardIdentityAllocationDiverged { detail: String },
    /// Two or more entering deck cards enchanted `IMBUED`, each of which
    /// native auto-plays on turn one. A single one is played by
    /// `engine::play::autoplay_imbued_turn_one` (#3381); the relative order of
    /// several, and the listener list's live-membership recheck between them,
    /// are not witnessed.
    ///
    /// `Imbued::AfterAutoPrePlayPhaseEntered` (v0.111.0 RVA `0xd60b8`, body
    /// `<AfterAutoPrePlayPhaseEntered>d__5::MoveNext` `0x3882a8`) returns
    /// unless the enchanted card's owner is the phase's player
    /// (`IL_001d`-`IL_002e`) and `TurnNumber <= 1` (`IL_0036`-`IL_0046`), then
    /// awaits `CardCmd.AutoPlay` on the card (`IL_004d`-`IL_005d`). The pile
    /// placement is `turn_one_fixup_order`
    /// (`get_ShouldStartAtBottomOfDrawPile`, `0xd60af`).
    ImbuedAutoPlayNotModeled { card: String },
}

impl From<RosterRefusal> for OpeningRefusal {
    fn from(refusal: RosterRefusal) -> Self {
        Self::Roster(refusal)
    }
}

impl From<EncounterDispatchRefusal> for OpeningRefusal {
    fn from(refusal: EncounterDispatchRefusal) -> Self {
        Self::EncounterDispatch(refusal)
    }
}

impl From<BoundaryRefusal> for OpeningRefusal {
    fn from(refusal: BoundaryRefusal) -> Self {
        Self::Boundary(refusal)
    }
}

impl From<EngineRefusal> for OpeningRefusal {
    fn from(refusal: EngineRefusal) -> Self {
        Self::Engine(refusal)
    }
}

impl OpeningRefusal {
    /// A short, stable class name for census tallies.
    ///
    /// A wrapped refusal reports the **inner** class, so an unbuilt encounter
    /// and an unrepresentable state are different census rows rather than one
    /// `opening_refused` bucket.
    pub fn class(&self) -> String {
        match self {
            Self::EncounterDispatch(inner) => match inner {
                EncounterDispatchRefusal::UnknownEncounterId(_) => "unknown_encounter_id",
                EncounterDispatchRefusal::AmbiguousEncounterKey { .. } => "ambiguous_encounter_key",
                EncounterDispatchRefusal::EncounterNotBuilt(_) => "encounter_not_built",
            }
            .to_string(),
            Self::Roster(inner) => match inner {
                RosterRefusal::MonsterModelUnread { .. } => "monster_model_unread",
                RosterRefusal::MonsterHpNotFixed { .. } => "monster_hp_not_fixed",
                RosterRefusal::MonsterPowerAbsent { .. } => "monster_power_absent",
                RosterRefusal::EncounterStreamUnavailable { .. } => "encounter_stream_unavailable",
                RosterRefusal::MonsterLoopAbsent { .. } => "monster_loop_absent",
                RosterRefusal::MonsterLoopMoveAbsent { .. } => "monster_loop_move_absent",
            }
            .to_string(),
            Self::Boundary(_) => "boundary_unrepresentable".to_string(),
            Self::Engine(_) => "engine_not_implemented".to_string(),
            Self::GenerationPoolUnmodeled { .. } => "generation_pool_unmodeled".to_string(),
            Self::GenerationPoolPartialUnlock { .. } => {
                "generation_pool_partial_unlock".to_string()
            }
            Self::ListenerPoolProvenance { .. } => "listener_pool_provenance".to_string(),
            Self::RelicDispatchOrderUnmodeled { .. } => "relic_dispatch_order".to_string(),
            Self::RoomEntryRelicNotModeled { .. } => "room_entry_relic_not_modeled".to_string(),
            Self::PerFightRelicStateNotSeeded { .. } => {
                "per_fight_relic_state_not_seeded".to_string()
            }
            Self::TurnOneRelicBodyNotModeled { .. } => {
                "turn_one_relic_body_not_modeled".to_string()
            }
            Self::TurnOneDebuffPartialArtifact { .. } => {
                "turn_one_debuff_partial_artifact".to_string()
            }
            Self::TurnStartRelicOrderUnrecorded { .. } => {
                "turn_start_relic_order_unrecorded".to_string()
            }
            Self::RuinedHelmetStrengthOrder { .. } => "ruined_helmet_strength_order".to_string(),
            Self::SymbioticVirusTurnStartOrder { .. } => {
                "symbiotic_virus_turn_start_order".to_string()
            }
            Self::RelicCombinationRefused { .. } => "relic_combination_refused".to_string(),
            Self::RoomEntryHealUnmodeled { .. } => "room_entry_heal_unmodeled".to_string(),
            Self::StoneCrackerUpgradeLadderUnknown { .. } => {
                "stone_cracker_upgrade_ladder_unknown".to_string()
            }
            Self::TeaSetChargeUndated { .. } => "tea_set_charge_undated".to_string(),
            Self::BoomingConchRoomTypeUnknown => "booming_conch_room_type_unknown".to_string(),
            Self::SlingOfCourageRoomTypeUnknown => "sling_of_courage_room_type_unknown".to_string(),
            Self::RelicCounterUnseeded { .. } => "relic_counter_unseeded".to_string(),
            Self::RelicCounterNotExact { .. } => "relic_counter_not_exact".to_string(),
            Self::PotionBeltNotExact { .. } => "potion_belt_not_exact".to_string(),
            Self::PotionBeltRowMalformed { .. } => "potion_belt_row_malformed".to_string(),
            Self::OrbBaseSlotsUndetermined => "orb_base_slots_undetermined".to_string(),
            Self::AscensionNotExact { .. } => "ascension_not_exact".to_string(),
            Self::TotalFloorUnknown => "total_floor_unknown".to_string(),
            Self::CombatStreamAbsent { .. } => "combat_stream_absent".to_string(),
            Self::McrChecksumContext { .. } => "mcr_checksum_context".to_string(),
            Self::McrChecksumIncomplete { .. } => "mcr_checksum_incomplete".to_string(),
            Self::InitialRandomAiNotModeled { .. } => "initial_random_ai_not_modeled".to_string(),
            Self::InnateKeywordProvenance { .. } => "innate_keyword_provenance".to_string(),
            Self::CardEntryPropsNotExact { .. } => "card_entry_props_not_exact".to_string(),
            Self::CardEntryVariantNotModeled { .. } => "card_entry_variant_not_modeled".to_string(),
            Self::CardIdentityAllocationDiverged { .. } => {
                "card_identity_allocation_diverged".to_string()
            }
            Self::ImbuedAutoPlayNotModeled { .. } => "imbued_auto_play_not_modeled".to_string(),
        }
    }
}

impl fmt::Display for OpeningRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncounterDispatch(EncounterDispatchRefusal::UnknownEncounterId(value)) => {
                write!(
                    f,
                    "encounter id {value:?} resolves to no registered key, so the \
                     build's own id axis does not name it"
                )
            }
            Self::EncounterDispatch(EncounterDispatchRefusal::AmbiguousEncounterKey {
                encounter,
                keys,
            }) => write!(
                f,
                "{encounter}: {} registered roster keys claim it ({keys:?}); \
                 make_monsters would silently take the first",
                keys.len()
            ),
            Self::EncounterDispatch(EncounterDispatchRefusal::EncounterNotBuilt(id)) => write!(
                f,
                "no Rust roster builder is registered for {:?}; the encounter \
                 builder wave (#2529 E4b) registers it, and guessing a roster \
                 would mis-seed every HP roll (SOLVER_INVARIANTS.md I5)",
                id.as_str()
            ),
            Self::Roster(inner) => write!(
                f,
                "the registered roster builder could not finish exactly: \
                 {inner:?} (SOLVER_INVARIANTS.md I5)"
            ),
            Self::Boundary(inner) => write!(f, "{inner:?}"),
            Self::Engine(inner) => write!(f, "{inner}"),
            Self::GenerationPoolUnmodeled { source, character } => write!(
                f,
                "{source} generates cards from the {character} pool, which this \
                 port does not model; the character IS recorded, so this is a \
                 content gap awaiting a scope decision (per-generator pool \
                 extraction vs this refusal), not a provenance gap \
                 (SOLVER_INVARIANTS.md I5/I13)"
            ),
            Self::GenerationPoolPartialUnlock { source, missing } => write!(
                f,
                "{source} draws from a generation pool under a partial unlock \
                 profile (missing {missing:?}); the recorded epoch set answers \
                 it, and building the pool from it is not done here \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::ListenerPoolProvenance { source, requires } => write!(
                f,
                "{source} draws from its owner's card pool and requires {requires} \
                 (SOLVER_INVARIANTS.md I5/I8)"
            ),
            Self::RelicDispatchOrderUnmodeled { hook, relics } => write!(
                f,
                "recorded {hook} dispatch order places relic peers {relics:?} \
                 where the fixed counter-relic suffix cannot represent them \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::RoomEntryRelicNotModeled { relic } => write!(
                f,
                "{relic} has a body on a hook the opening or the first deal \
                 fires, and this crate carries no subscriber for it, so firing \
                 the hook would do nothing rather than refuse \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::TurnOneRelicBodyNotModeled { relic } => write!(
                f,
                "{relic} has a body on the player's FIRST turn-start walk, \
                 which start_combat's closing begin_player_turn runs inside \
                 the opening, and this crate carries no subscriber for it — so \
                 the document would be missing the effect with nothing to \
                 refuse it (SOLVER_INVARIANTS.md I5)"
            ),
            Self::TurnOneDebuffPartialArtifact { relics, monsters } => write!(
                f,
                "turn-1 all-enemy debuff relics with partial Artifact: \
                 BeforeSideTurnStart relic acquisition order is unrecorded \
                 (relics={relics:?}, monsters={monsters:?}); which debuff the \
                 Artifact eats depends on it (SOLVER_INVARIANTS.md I5)"
            ),
            Self::TurnStartRelicOrderUnrecorded { owner, peers } => write!(
                f,
                "{owner} with same-AfterPlayerTurnStart relic peers {peers:?}: \
                 relic acquisition order decides the turn-1 Hand/terminal \
                 outcome and is unrecorded (SOLVER_INVARIANTS.md I5)"
            ),
            Self::PerFightRelicStateNotSeeded { relic, field } => write!(
                f,
                "{relic} carries per-fight state start_combat seeds as \
                 State.{field} and the opening does not write, so the relic \
                 would be inert for the whole fight while its inventory mirror \
                 says it is owned (SOLVER_INVARIANTS.md I5)"
            ),
            Self::RuinedHelmetStrengthOrder { sources } => write!(
                f,
                "RELIC.RUINED_HELMET with distinct AfterRoomEntered Strength \
                 amounts {sources:?}: relic acquisition order decides which one \
                 is doubled and is unrecorded (SOLVER_INVARIANTS.md I5)"
            ),
            Self::SymbioticVirusTurnStartOrder { peers } => write!(
                f,
                "RELIC.SYMBIOTIC_VIRUS with {peers:?}: the turn-1 \
                 AfterSideTurnStart Dark channel's order against these peers \
                 changes the orb queue, an evoke or a later snapshot, and relic \
                 acquisition order is unrecorded (SOLVER_INVARIANTS.md I5)"
            ),
            Self::RelicCombinationRefused { relics, reason } => write!(
                f,
                "start_combat refuses the relic combination {relics:?}: \
                 {reason} (SOLVER_INVARIANTS.md I5)"
            ),
            Self::RoomEntryHealUnmodeled { relic } => write!(
                f,
                "{relic} heals on entering this node type and the heal is not \
                 modeled (SOLVER_INVARIANTS.md I5)"
            ),
            Self::StoneCrackerUpgradeLadderUnknown { card } => write!(
                f,
                "RELIC.STONE_CRACKER filters the draw pile by IsUpgradable \
                 before its CombatCardSelection shuffle, and {card}'s \
                 MaxUpgradeLevel is unknown (SOLVER_INVARIANTS.md I5)"
            ),
            Self::TeaSetChargeUndated { relic } => write!(
                f,
                "{relic} charge state is undated for this fight \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::BoomingConchRoomTypeUnknown => f.write_str(
                "RELIC.BOOMING_CONCH needs the combat's room type and this \
                 entry carries none (SOLVER_INVARIANTS.md I5)",
            ),
            Self::SlingOfCourageRoomTypeUnknown => f.write_str(
                "RELIC.SLING_OF_COURAGE needs the combat's room type and this \
                 entry carries none (SOLVER_INVARIANTS.md I5)",
            ),
            Self::RelicCounterUnseeded { relic } => write!(
                f,
                "{relic} counter is unseeded for this fight; it is run state and \
                 a guess changes which turn the relic fires on \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::RelicCounterNotExact { relic, value } => write!(
                f,
                "{relic} requires an exact non-negative persistent entering \
                 counter, got {value} (SOLVER_INVARIANTS.md I5/I8)"
            ),
            Self::PotionBeltNotExact { relic, capacity } => write!(
                f,
                "{relic} procures into the first empty potion slot, which needs \
                 an exact positive max_potion_slot_count and in-range slot rows; \
                 got capacity {capacity:?} (SOLVER_INVARIANTS.md I5/I8)"
            ),
            Self::PotionBeltRowMalformed {
                potion,
                slot_index,
                capacity,
            } => write!(
                f,
                "potion belt row {potion} has slot_index {slot_index:?}, which \
                 the native loader cannot place in a belt of recorded capacity \
                 {capacity:?}; eliding the belt would silently lose the saved \
                 potions (SOLVER_INVARIANTS.md I5)"
            ),
            Self::OrbBaseSlotsUndetermined => f.write_str(
                "orb-channelling content with an undetermined character: \
                 BaseOrbSlotCount is 3 for Defect and 0 otherwise, and \
                 defaulting to 0 would silently run a Defect fight at the wrong \
                 capacity (SOLVER_INVARIANTS.md I8)",
            ),
            Self::AscensionNotExact { recorded } => write!(
                f,
                "combat-entry ascension is not an exact v0.111.0 AscensionLevel \
                 in 0..=10: {recorded:?}; every tiered monster constant is \
                 selected by it (SOLVER_INVARIANTS.md I5)"
            ),
            Self::TotalFloorUnknown => f.write_str(
                "total_floor is underivable for this entry (node_index + 1), and \
                 the per-fight Encounter stream is seeded from it \
                 (SOLVER_INVARIANTS.md I5)",
            ),
            Self::CombatStreamAbsent { stream } => write!(
                f,
                "this fight consumes the {stream:?} stream and the save does not \
                 record it (SOLVER_INVARIANTS.md I5)"
            ),
            Self::McrChecksumContext { context } => write!(
                f,
                "--mcr splice needs the capture's FIRST checksum to be the \
                 opening state (context \"After player turn start\"), got \
                 {context:?}"
            ),
            Self::McrChecksumIncomplete { stream } => write!(
                f,
                "--mcr splice overwrites all nine RNG streams and the capture's \
                 first checksum carries no {stream:?}"
            ),
            Self::InitialRandomAiNotModeled { kind } => write!(
                f,
                "{kind} rolls its turn-1 intent during combat setup \
                 (_initialize_random_ai) from a state outside the exact \
                 empty-log domain the opening's roll covers, and a wrong roll \
                 desynchronises MonsterAi for the whole fight \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::InnateKeywordProvenance { card } => write!(
                f,
                "{card}: the turn-one pile fixup reads a card's EFFECTIVE \
                 keywords, and this instance's differ from its spec's \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::CardEntryPropsNotExact { card, detail } => write!(
                f,
                "{card} entering props are outside the exact native domain \
                 start_combat validates: {detail} (SOLVER_INVARIANTS.md I5/I8)"
            ),
            Self::CardEntryVariantNotModeled { card, detail } => write!(
                f,
                "{card} carries a saved per-instance variant that selects a \
                 different card spec, and this crate cannot play it: {detail} \
                 (SOLVER_INVARIANTS.md I5)"
            ),
            Self::CardIdentityAllocationDiverged { detail } => write!(
                f,
                "physical-card uid allocation diverged from the oracle's \
                 post-deal normalization: {detail} (SOLVER_INVARIANTS.md I5)"
            ),
            Self::ImbuedAutoPlayNotModeled { card } => write!(
                f,
                "{card} is one of several IMBUED cards, each of which native \
                 auto-plays at turn one's AutoPre phase, and their relative \
                 order is not modeled (SOLVER_INVARIANTS.md I5)"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{EncounterId, MonsterKind};

    #[test]
    fn a_wrapped_refusal_reports_the_inner_class() {
        let refusal = OpeningRefusal::EncounterDispatch(
            EncounterDispatchRefusal::EncounterNotBuilt(EncounterId::ToadpolesWeak),
        );
        assert_eq!(refusal.class(), "encounter_not_built");
        assert!(refusal.to_string().contains("TOADPOLES_WEAK"));

        // And the content lane's own vocabulary keeps its own class, so a
        // census can tell an unbuilt encounter from a builder that ran and
        // could not finish.
        let inner = OpeningRefusal::Roster(RosterRefusal::MonsterModelUnread {
            kind: MonsterKind::Toadpole,
            why: "the reader's reason",
        });
        assert_eq!(inner.class(), "monster_model_unread");
        assert!(inner.to_string().contains("the reader's reason"));
    }

    #[test]
    fn every_class_name_is_distinct() {
        let refusals = [
            OpeningRefusal::GenerationPoolUnmodeled {
                source: "JACKPOT".to_string(),
                character: "CHARACTER.NECROBINDER".to_string(),
            },
            OpeningRefusal::GenerationPoolPartialUnlock {
                source: "WHITE_NOISE".to_string(),
                missing: vec!["DEFECT6_EPOCH".to_string()],
            },
            OpeningRefusal::ListenerPoolProvenance {
                source: "HELLO_WORLD",
                requires: "an explicit character owner with a card pool",
            },
            OpeningRefusal::RelicDispatchOrderUnmodeled {
                hook: "AfterCardPlayed",
                relics: vec!["RELIC.LETTER_OPENER".to_string()],
            },
            OpeningRefusal::RoomEntryRelicNotModeled {
                relic: "RELIC.SNECKO_EYE".to_string(),
            },
            OpeningRefusal::TurnOneRelicBodyNotModeled {
                relic: "RELIC.RED_MASK".to_string(),
            },
            OpeningRefusal::TurnStartRelicOrderUnrecorded {
                owner: "RELIC.FESTIVE_POPPER",
                peers: vec!["RELIC.GAMBLING_CHIP".to_string()],
            },
            OpeningRefusal::PerFightRelicStateNotSeeded {
                relic: "RELIC.THROWING_AXE".to_string(),
                field: "throwing_axe_available",
            },
            OpeningRefusal::RoomEntryHealUnmodeled {
                relic: "RELIC.PLANISPHERE",
            },
            OpeningRefusal::RuinedHelmetStrengthOrder {
                sources: vec!["RELIC.EMBER_TEA=2".to_string()],
            },
            OpeningRefusal::SymbioticVirusTurnStartOrder {
                peers: vec!["RELIC.INFUSED_CORE".to_string()],
            },
            OpeningRefusal::RelicCombinationRefused {
                relics: vec![
                    "RELIC.MUSIC_BOX".to_string(),
                    "RELIC.KUSARIGAMA".to_string(),
                ],
                reason: "AfterCardPlayed acquisition order",
            },
            OpeningRefusal::StoneCrackerUpgradeLadderUnknown {
                card: "X+0".to_string(),
            },
            OpeningRefusal::TeaSetChargeUndated {
                relic: "RELIC.VENERABLE_TEA_SET",
            },
            OpeningRefusal::BoomingConchRoomTypeUnknown,
            OpeningRefusal::SlingOfCourageRoomTypeUnknown,
            OpeningRefusal::RelicCounterUnseeded {
                relic: "RELIC.HAPPY_FLOWER",
            },
            OpeningRefusal::RelicCounterNotExact {
                relic: "RELIC.IRON_CLUB",
                value: "null".to_string(),
            },
            OpeningRefusal::PotionBeltNotExact {
                relic: "RELIC.PETRIFIED_TOAD",
                capacity: Some(0),
            },
            OpeningRefusal::PotionBeltRowMalformed {
                potion: "POTION.FIRE_POTION".to_string(),
                slot_index: Some(-1),
                capacity: Some(3),
            },
            OpeningRefusal::OrbBaseSlotsUndetermined,
            OpeningRefusal::AscensionNotExact { recorded: None },
            OpeningRefusal::TotalFloorUnknown,
            OpeningRefusal::CombatStreamAbsent { stream: "niche" },
            OpeningRefusal::McrChecksumContext {
                context: "Turn 2".to_string(),
            },
            OpeningRefusal::McrChecksumIncomplete { stream: "sel" },
            OpeningRefusal::InitialRandomAiNotModeled {
                kind: "LEAF_SLIME_S",
            },
            OpeningRefusal::InnateKeywordProvenance {
                card: "WRITHE".to_string(),
            },
            OpeningRefusal::CardEntryPropsNotExact {
                card: "GENETIC_ALGORITHM",
                detail: "absent".to_string(),
            },
            OpeningRefusal::CardEntryVariantNotModeled {
                card: "MAD_SCIENCE",
                detail: String::new(),
            },
            OpeningRefusal::CardIdentityAllocationDiverged {
                detail: String::new(),
            },
            OpeningRefusal::ImbuedAutoPlayNotModeled {
                card: "DEFEND_IRONCLAD".to_string(),
            },
        ];
        let mut classes: Vec<String> = refusals.iter().map(OpeningRefusal::class).collect();
        let total = classes.len();
        classes.sort();
        classes.dedup();
        assert_eq!(classes.len(), total);
        for refusal in &refusals {
            assert!(!refusal.to_string().is_empty());
        }
    }
}
